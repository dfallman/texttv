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
    pub size: crate::cli::Size,
    pub debug_protocol: bool,
}

pub fn render_images(images: &[DynamicImage], opts: RenderOptions) -> Result<DetectedProtocol> {
    use crate::cli::Mode;

    let target_width = opts.size.to_width(u32::from(terminal_cols()));

    let (cfg, protocol) = match opts.mode {
        Mode::Auto => {
            let p = detect_protocol();
            (config_for(p, target_width), p)
        }
        Mode::Kitty => (
            force_config(true, false, target_width),
            DetectedProtocol::Kitty,
        ),
        Mode::Iterm => (
            force_config(false, true, target_width),
            DetectedProtocol::Iterm,
        ),
        Mode::Blocks => (
            force_config(false, false, target_width),
            DetectedProtocol::Halfblocks,
        ),
        Mode::Teletext => unreachable!("Mode::Teletext is handled before render_images"),
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

    // Forced image-protocol modes inside tmux silently produce garbled escapes
    // when passthrough is off. The user opted into the override; warn them
    // unconditionally so they don't waste time guessing why the page is
    // broken. (Auto mode goes through detect_protocol and already lands on
    // halfblocks in that situation, which the debug_protocol hint above
    // already covers.)
    if matches!(opts.mode, Mode::Kitty | Mode::Iterm) && std::env::var_os("TMUX").is_some() {
        let proto = if opts.mode == Mode::Kitty {
            "kitty"
        } else {
            "iterm"
        };
        eprintln!(
            "hint: forcing {proto} graphics inside tmux requires `set -g allow-passthrough on` \
             in tmux.conf. If the output looks garbled, that's why."
        );
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

fn config_for(p: DetectedProtocol, width: u32) -> viuer::Config {
    match p {
        DetectedProtocol::Kitty | DetectedProtocol::Iterm => viuer::Config {
            absolute_offset: false,
            width: Some(width),
            ..viuer::Config::default()
        },
        DetectedProtocol::Halfblocks => viuer::Config {
            absolute_offset: false,
            width: Some(width),
            use_kitty: false,
            use_iterm: false,
            ..viuer::Config::default()
        },
    }
}

fn force_config(use_kitty: bool, use_iterm: bool, width: u32) -> viuer::Config {
    viuer::Config {
        absolute_offset: false,
        use_kitty,
        use_iterm,
        width: Some(width),
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

/// Pick the default rendering mode for the current terminal. Terminals with
/// a native graphics protocol (Kitty or iTerm2 inline-image) get `auto` (image
/// render); half-block-only terminals get `teletext`. Piped/redirected stdout
/// always defaults to teletext regardless of terminal.
pub fn default_mode_for_terminal() -> crate::cli::Mode {
    use crate::cli::Mode;
    if !stdout_is_tty() {
        return Mode::Teletext;
    }
    match detect_protocol() {
        DetectedProtocol::Kitty | DetectedProtocol::Iterm => Mode::Auto,
        DetectedProtocol::Halfblocks => Mode::Teletext,
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

/// Render the colored teletext line stream.
///
/// `color` enables ANSI truecolor escapes. Each line is emitted exactly once.
/// Lines flagged `double_height` in the source are rendered as bold single-
/// height — DECDHL was unreliable, the inline colored-bar approximation got
/// busy, and bold is the lowest-friction signal that still distinguishes a
/// heading. The always-blank row that texttv.nu pads with after a DH line
/// (the bottom half of the would-be tall character) is swallowed.
pub fn render_colored(
    lines: &[crate::parse::Line],
    color: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        write_line(line, color, "", out)?;
        if line.double_height && lines.get(i + 1).is_some_and(is_blank_line) {
            // Skip the next-row blank padding so the page stays the original
            // teletext row count rather than picking up an extra empty row
            // beneath every heading.
            i += 2;
        } else {
            i += 1;
        }
    }
    Ok(())
}

pub(crate) fn is_blank_line(line: &crate::parse::Line) -> bool {
    line.cells
        .iter()
        .all(|c| !c.is_mosaic() && c.text.chars().all(char::is_whitespace))
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
    let bold = line.double_height;
    for cell in &line.cells {
        let render_text: String = if let Some(url) = cell.mosaic_url.as_deref() {
            // Resolve the mosaic to a Unicode sextant (or block-special) glyph.
            // On any failure (network down, GIF corrupt, …) fall back to a
            // colored space so the page still renders cleanly.
            match crate::mosaic::resolve_pattern(url, cell.fg, cell.bg) {
                Ok(pat) => crate::mosaic::pattern_to_glyph(pat).to_string(),
                Err(_) => " ".repeat(cell.text.chars().count().max(1)),
            }
        } else {
            cell.text.clone()
        };
        if color {
            let (fr, fg, fb) = cell.fg.rgb();
            let (br, bg, bb) = cell.bg.rgb();
            if bold {
                write!(
                    out,
                    "{}",
                    render_text
                        .truecolor(fr, fg, fb)
                        .on_truecolor(br, bg, bb)
                        .bold()
                )?;
            } else {
                write!(
                    out,
                    "{}",
                    render_text.truecolor(fr, fg, fb).on_truecolor(br, bg, bb)
                )?;
            }
        } else {
            out.write_all(render_text.as_bytes())?;
        }
    }
    if color {
        // Right-edge frame: one cell of solid black, mirroring the bitmap
        // render's right-side padding. Inherits DEC double-height on DH lines.
        out.write_all(b"\x1b[48;2;0;0;0m \x1b[0m")?;
    }
    out.write_all(b"\n")?;
    Ok(())
}

pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}
