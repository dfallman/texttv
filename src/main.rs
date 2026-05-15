use clap::Parser;
use std::process::ExitCode;

use texttv::cli::{Args, Mode, Source, print_sections};
use texttv::fetch;
use texttv::parse::extract_page;
use texttv::render::{RenderOptions, render_images, render_text, stdout_is_tty};

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(a) => a,
        Err(e) => {
            // clap defaults to exit code 2 for parse errors; --help/--version are also Err
            // with ErrorKind::DisplayHelp/DisplayVersion. Route those to exit 0.
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

    let html = match args.source {
        Source::Svt => fetch::fetch_html(page),
        Source::TexttvNu => fetch::fetch_html_texttv_nu(page),
    }
    .map_err(AppError::Runtime)?;

    let page_data = extract_page(&html, page).map_err(AppError::Runtime)?;

    let no_color = args.no_color || std::env::var_os("NO_COLOR").is_some();
    let piped = !stdout_is_tty();

    let effective_mode = if piped && matches!(args.mode, Mode::Auto) {
        Mode::Text
    } else {
        args.mode
    };

    match effective_mode {
        Mode::Text => {
            let mut out = std::io::stdout().lock();
            render_text(&page_data.text, !no_color, &mut out).map_err(AppError::Runtime)?;
        }
        _ => {
            let opts = RenderOptions {
                mode: effective_mode,
                debug_protocol: args.debug_protocol,
            };
            render_images(&page_data.images, opts).map_err(AppError::Runtime)?;
        }
    }
    Ok(())
}
