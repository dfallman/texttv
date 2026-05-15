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
        let framed = add_right_frame(img, RIGHT_FRAME_PX);
        viuer::print(&framed, &cfg).context("viuer failed to print image")?;
    }
    writeln!(stdout)?;
    Ok(protocol)
}

/// Width in source pixels of the black frame added to the right edge of each
/// rendered subpage. SVT's teletext glyphs are ~13 px wide in the served GIF,
/// so 16 px gives a visible ~one-cell black margin after viuer scales the
/// padded image down to 60 terminal cells.
const RIGHT_FRAME_PX: u32 = 16;

/// Pad the right side of an image with `pad_px` columns of black pixels.
/// Returns a fresh RGB8 image; the original is left untouched.
fn add_right_frame(img: &image::DynamicImage, pad_px: u32) -> image::DynamicImage {
    if pad_px == 0 {
        return img.clone();
    }
    let src = img.to_rgb8();
    let (w, h) = (img.width(), img.height());
    let mut framed: image::ImageBuffer<image::Rgb<u8>, Vec<u8>> =
        image::ImageBuffer::new(w + pad_px, h);
    // ImageBuffer::new zero-fills; for Rgb<u8> that's (0,0,0) = black.
    image::imageops::replace(&mut framed, &src, 0, 0);
    image::DynamicImage::ImageRgb8(framed)
}

/// Target image width in terminal cells. SVT teletext is 40 cols natively;
/// 60 leaves a little headroom for letterforms in graphics protocols while
/// keeping the rendering compact.
const TARGET_IMG_WIDTH: u32 = 60;

fn capped_width() -> u32 {
    let term = u32::from(terminal_cols());
    if term == 0 {
        TARGET_IMG_WIDTH
    } else {
        term.clamp(1, TARGET_IMG_WIDTH)
    }
}

fn config_for(p: DetectedProtocol) -> viuer::Config {
    match p {
        DetectedProtocol::Kitty | DetectedProtocol::Iterm => viuer::Config {
            absolute_offset: false,
            width: Some(capped_width()),
            ..viuer::Config::default()
        },
        DetectedProtocol::Halfblocks => viuer::Config {
            absolute_offset: false,
            width: Some(capped_width()),
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
        width: Some(capped_width()),
        ..viuer::Config::default()
    }
}

fn force_iterm_config() -> viuer::Config {
    viuer::Config {
        absolute_offset: false,
        use_kitty: false,
        use_iterm: true,
        width: Some(capped_width()),
        ..viuer::Config::default()
    }
}

fn force_blocks_config() -> viuer::Config {
    viuer::Config {
        absolute_offset: false,
        width: Some(capped_width()),
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

pub fn render_text(text: &str, out: &mut dyn Write) -> Result<()> {
    writeln!(out, "{text}")?;
    Ok(())
}

/// Render the colored, optionally double-height teletext line stream.
///
/// `color` enables ANSI truecolor escapes. `double_height` enables DEC private
/// escapes (`ESC # 3` top half, `ESC # 4` bottom half) for lines marked DH. The
/// two flags travel together when the caller wants a "plain" render — see
/// main.rs.
pub fn render_colored(
    lines: &[crate::parse::Line],
    color: bool,
    double_height: bool,
    out: &mut dyn Write,
) -> Result<()> {
    for line in lines {
        if double_height && line.double_height {
            // DEC double-height: the same character row is emitted twice, once
            // with ESC#3 (top half), once with ESC#4 (bottom half). Conforming
            // terminals (Kitty, Ghostty, WezTerm, iTerm2, xterm) render the
            // pair as a single visually-tall row.
            write_line(line, color, "\x1b#3", out)?;
            write_line(line, color, "\x1b#4", out)?;
        } else {
            write_line(line, color, "", out)?;
        }
    }
    Ok(())
}

fn write_line(
    line: &crate::parse::Line,
    color: bool,
    prefix: &str,
    out: &mut dyn Write,
) -> Result<()> {
    use owo_colors::OwoColorize;

    if !prefix.is_empty() {
        out.write_all(prefix.as_bytes())?;
    }
    for cell in &line.cells {
        let render_text: String = if cell.mosaic {
            // Replace mosaic placeholder with a space of equal width; we lose the
            // block pattern but keep the background fill so layout survives.
            " ".repeat(cell.text.chars().count().max(1))
        } else {
            cell.text.clone()
        };
        if color {
            let (fr, fg, fb) = cell.fg.rgb();
            let (br, bg, bb) = cell.bg.rgb();
            write!(
                out,
                "{}",
                render_text
                    .truecolor(fr, fg, fb)
                    .on_truecolor(br, bg, bb)
            )?;
        } else {
            out.write_all(render_text.as_bytes())?;
        }
    }
    if color {
        out.write_all(b"\x1b[0m")?;
    }
    out.write_all(b"\n")?;
    Ok(())
}

pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}
