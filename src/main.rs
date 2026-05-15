use clap::Parser;
use std::process::ExitCode;

use texttv::cli::{Args, Mode, Source, print_sections};
use texttv::fetch;
use texttv::parse::{extract_page, parse_texttv_nu};
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

fn run(args: Args) -> Result<(), AppError> {
    if args.list {
        let mut out = std::io::stdout().lock();
        print_sections(&mut out).map_err(|e| AppError::Runtime(e.into()))?;
        return Ok(());
    }

    let page = args
        .page
        .ok_or_else(|| AppError::User("PAGE is required".into()))?;

    let piped = !stdout_is_tty();
    // Piped stdout, NO_COLOR=1, or --no-color all disable ANSI escapes.
    // This matches the NO_COLOR informal spec and keeps `texttv 300 | grep`
    // working as plain text.
    let no_color = args.no_color || std::env::var_os("NO_COLOR").is_some() || piped;

    // --mode auto on a piped stdout dumps escape codes, so degrade to text.
    let effective_mode = if piped && matches!(args.mode, Mode::Auto) {
        Mode::Teletext
    } else {
        args.mode
    };

    // Source defaults: texttv.nu for the rich text render, svt.se for the GIF.
    let source = args.source.unwrap_or(match effective_mode {
        Mode::Teletext => Source::TexttvNu,
        _ => Source::Svt,
    });

    match (effective_mode, source) {
        (Mode::Teletext, Source::TexttvNu) => {
            let json = fetch::fetch_texttv_nu(page).map_err(AppError::Runtime)?;
            let cp = parse_texttv_nu(&json, page).map_err(AppError::Runtime)?;
            let mut out = std::io::stdout().lock();
            if no_color {
                // --no-color strips both color and double-height (DEC escapes
                // would render visually large even without color).
                render_text(&cp.plain, &mut out).map_err(AppError::Runtime)?;
            } else {
                render_colored(&cp.lines, true, true, &mut out)
                    .map_err(AppError::Runtime)?;
            }
        }
        (Mode::Teletext, Source::Svt) => {
            let html = fetch::fetch_html(page).map_err(AppError::Runtime)?;
            let page_data = extract_page(&html, page).map_err(AppError::Runtime)?;
            let mut out = std::io::stdout().lock();
            render_text(&page_data.text, &mut out).map_err(AppError::Runtime)?;
        }
        (image_mode, _) => {
            // Image rendering requires the GIF, which only svt.se serves.
            let html = fetch::fetch_html(page).map_err(AppError::Runtime)?;
            let page_data = extract_page(&html, page).map_err(AppError::Runtime)?;
            let opts = RenderOptions {
                mode: image_mode,
                size: args.size,
                debug_protocol: args.debug_protocol,
            };
            render_images(&page_data.images, opts).map_err(AppError::Runtime)?;
        }
    }
    Ok(())
}
