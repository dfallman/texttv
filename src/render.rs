use anyhow::{Context, Result};
use image::DynamicImage;
use std::io::{IsTerminal, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedProtocol {
    Kitty,
    Iterm,
    Halfblocks,
}

impl std::fmt::Display for DetectedProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Kitty => f.write_str("kitty"),
            Self::Iterm => f.write_str("iterm"),
            Self::Halfblocks => f.write_str("halfblocks"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub mode: crate::cli::Mode,
    pub debug_protocol: bool,
}

pub fn render_images(images: &[DynamicImage], opts: RenderOptions) -> Result<DetectedProtocol> {
    use crate::cli::Mode;

    let (cfg, protocol) = match opts.mode {
        Mode::Auto => {
            let p = detect_protocol();
            (config_for(p), p)
        }
        Mode::Kitty => (force_kitty_config(), DetectedProtocol::Kitty),
        Mode::Iterm => (force_iterm_config(), DetectedProtocol::Iterm),
        Mode::Blocks => (force_blocks_config(), DetectedProtocol::Halfblocks),
        Mode::Text => unreachable!("Mode::Text is handled before render_images"),
    };

    if opts.debug_protocol {
        eprintln!("detected: {protocol}");
        if protocol == DetectedProtocol::Halfblocks && std::env::var_os("TMUX").is_some() {
            eprintln!(
                "hint: inside tmux. For native graphics, set `set -g allow-passthrough on` \
                 (Kitty) or use `set -g default-terminal \"tmux-256color\"` plus the appropriate \
                 terminal-features overrides for your outer terminal."
            );
        }
    }

    let mut stdout = std::io::stdout().lock();
    for (i, img) in images.iter().enumerate() {
        if i > 0 {
            writeln!(stdout)?;
        }
        viuer::print(img, &cfg).context("viuer failed to print image")?;
    }
    writeln!(stdout)?;
    Ok(protocol)
}

fn config_for(p: DetectedProtocol) -> viuer::Config {
    match p {
        DetectedProtocol::Kitty | DetectedProtocol::Iterm => viuer::Config {
            absolute_offset: false,
            ..viuer::Config::default()
        },
        DetectedProtocol::Halfblocks => viuer::Config {
            absolute_offset: false,
            width: Some(u32::from(terminal_cols())),
            use_kitty: false,
            use_iterm: false,
            ..viuer::Config::default()
        },
    }
}

fn force_kitty_config() -> viuer::Config {
    viuer::Config {
        absolute_offset: false,
        use_kitty: true,
        use_iterm: false,
        ..viuer::Config::default()
    }
}

fn force_iterm_config() -> viuer::Config {
    viuer::Config {
        absolute_offset: false,
        use_kitty: false,
        use_iterm: true,
        ..viuer::Config::default()
    }
}

fn force_blocks_config() -> viuer::Config {
    viuer::Config {
        absolute_offset: false,
        width: Some(u32::from(terminal_cols())),
        use_kitty: false,
        use_iterm: false,
        ..viuer::Config::default()
    }
}

fn detect_protocol() -> DetectedProtocol {
    match viuer::get_kitty_support() {
        viuer::KittySupport::Local | viuer::KittySupport::Remote => DetectedProtocol::Kitty,
        viuer::KittySupport::None => {
            if viuer::is_iterm_supported() {
                DetectedProtocol::Iterm
            } else {
                DetectedProtocol::Halfblocks
            }
        }
    }
}

fn terminal_cols() -> u16 {
    let (cols, _rows) = viuer::terminal_size();
    if cols == 0 {
        std::env::var("COLUMNS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(80)
    } else {
        cols
    }
}

pub fn render_text(text: &str, color: bool, out: &mut dyn Write) -> Result<()> {
    if !color {
        writeln!(out, "{text}")?;
        return Ok(());
    }
    for line in text.lines() {
        if is_heading(line) {
            writeln!(out, "\x1b[1;33m{line}\x1b[0m")?;
        } else {
            writeln!(out, "{line}")?;
        }
    }
    Ok(())
}

fn is_heading(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.len() > 40 {
        return false;
    }
    let mut saw_alpha = false;
    for c in trimmed.chars() {
        if c.is_alphabetic() {
            saw_alpha = true;
            if !c.is_uppercase() {
                return false;
            }
        }
    }
    saw_alpha
}

pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}
