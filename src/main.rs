use clap::Parser;
use std::process::ExitCode;

use texttv::cli::{Args, Mode, Size, Source, print_sections};
use texttv::config::Config;
use texttv::fetch;
use texttv::mosaic;
use texttv::parse::{extract_page, parse_texttv_nu};
use texttv::timing;
use texttv::render::{
    RenderOptions, render_colored, render_images, render_text, stdout_is_tty,
};

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(a) => a,
        Err(e) => {
            let _ = e.print();
            return match e.kind() {
                clap::error::ErrorKind::DisplayHelp
                | clap::error::ErrorKind::DisplayVersion => ExitCode::from(0),
                _ => ExitCode::from(1),
            };
        }
    };
    match run(args) {
        Ok(()) => ExitCode::from(0),
        Err(AppError::User(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::from(1)
        }
        Err(AppError::Runtime(e)) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

#[derive(Debug)]
enum AppError {
    User(String),
    Runtime(anyhow::Error),
}

fn unique_mosaic_count(cp: &texttv::parse::ColoredPage) -> usize {
    let mut seen = std::collections::HashSet::new();
    for line in &cp.lines {
        for cell in &line.cells {
            if let Some(url) = cell.mosaic_url.as_deref() {
                seen.insert(url);
            }
        }
    }
    seen.len()
}

fn run(args: Args) -> Result<(), AppError> {
    if args.list {
        let mut out = std::io::stdout().lock();
        print_sections(&mut out).map_err(|e| AppError::Runtime(e.into()))?;
        return Ok(());
    }

    let page = args
        .page
        .ok_or_else(|| AppError::User("PAGE is required".into()))?;

    // Load ~/.config/texttv/config.yaml; broken config is non-fatal — we warn
    // and fall back to defaults, so a typo doesn't lock the user out.
    let cfg = Config::load().unwrap_or_else(|e| {
        eprintln!("warning: ignoring config file: {e:#}");
        Config::default()
    });

    timing::set_enabled(
        args.verbose
            || cfg.verbose.unwrap_or(false)
            || std::env::var_os("TEXTTV_TIMINGS").is_some(),
    );

    let piped = !stdout_is_tty();
    // Precedence: CLI flag > config file > built-in default. NO_COLOR env and
    // a piped stdout always force color off regardless.
    let no_color = args.no_color
        || cfg.no_color.unwrap_or(false)
        || std::env::var_os("NO_COLOR").is_some()
        || piped;

    // Resolve --mode: CLI wins, then config, then terminal-based default.
    let resolved_mode = args
        .mode
        .or(cfg.mode)
        .unwrap_or_else(texttv::render::default_mode_for_terminal);
    // --mode auto on a piped stdout dumps escape codes, so degrade to text.
    let effective_mode = if piped && matches!(resolved_mode, Mode::Auto) {
        Mode::Teletext
    } else {
        resolved_mode
    };

    let resolved_size = args.size.or(cfg.size).unwrap_or(Size::Medium);

    // Source defaults: texttv.nu for the rich text render, svt.se for the GIF.
    let source = args
        .source
        .or(cfg.source)
        .unwrap_or(match effective_mode {
            Mode::Teletext => Source::TexttvNu,
            _ => Source::Svt,
        });

    match (effective_mode, source) {
        (Mode::Teletext, Source::TexttvNu) => {
            let json = timing::time(&format!("fetch texttv.nu/{page}"), || {
                fetch::fetch_texttv_nu(page)
            })
            .map_err(AppError::Runtime)?;
            let cp = timing::time("parse colored html", || {
                parse_texttv_nu(&json, page)
            })
            .map_err(AppError::Runtime)?;
            if !no_color {
                let n = unique_mosaic_count(&cp);
                if n > 0 {
                    timing::time(&format!("prefetch {n} mosaic GIFs"), || {
                        mosaic::prefetch_page(&cp)
                    });
                }
            }
            let mut out = std::io::stdout().lock();
            timing::time("render", || -> Result<(), anyhow::Error> {
                if no_color {
                    render_text(&cp.plain, &mut out)
                } else {
                    render_colored(&cp.lines, true, &mut out)
                }
            })
            .map_err(AppError::Runtime)?;
        }
        (Mode::Teletext, Source::Svt) => {
            let html = timing::time(&format!("fetch svt.se/{page}"), || {
                fetch::fetch_html(page)
            })
            .map_err(AppError::Runtime)?;
            let page_data = timing::time("parse svt html", || extract_page(&html, page))
                .map_err(AppError::Runtime)?;
            let mut out = std::io::stdout().lock();
            timing::time("render text", || render_text(&page_data.text, &mut out))
                .map_err(AppError::Runtime)?;
        }
        (image_mode, _) => {
            // Image rendering requires the GIF, which only svt.se serves.
            let html = timing::time(&format!("fetch svt.se/{page}"), || {
                fetch::fetch_html(page)
            })
            .map_err(AppError::Runtime)?;
            let page_data = timing::time("decode page GIF", || extract_page(&html, page))
                .map_err(AppError::Runtime)?;
            let opts = RenderOptions {
                mode: image_mode,
                size: resolved_size,
                debug_protocol: args.debug_protocol,
            };
            timing::time("render image", || render_images(&page_data.images, opts))
                .map_err(AppError::Runtime)?;
        }
    }
    Ok(())
}
