use clap::{Parser, ValueEnum};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Let viuer pick the best protocol available.
    Auto,
    /// Force Kitty graphics protocol.
    Kitty,
    /// Force iTerm2 inline-image protocol.
    Iterm,
    /// Force Unicode half-block fallback.
    Blocks,
    /// The default: reconstruct the original 40-col teletext layout
    /// with per-cell colors.
    Teletext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// Default: scrape svt.se/text-tv/<PAGE>.
    Svt,
    /// Use the third-party JSON proxy at api.texttv.nu.
    TexttvNu,
}

/// Render size for image modes (auto/kitty/iterm/blocks). Ignored in teletext
/// mode, where the page is always 41 cells wide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Size {
    /// 30 cells wide.
    Tiny,
    /// 45 cells wide.
    Small,
    /// 60 cells wide. The default.
    Medium,
    /// 90 cells wide.
    Large,
    /// 120 cells wide.
    Xl,
    /// Fill the terminal width.
    Full,
}

impl Size {
    /// Resolve to a cell-width using the current terminal size when needed.
    /// `term_cols` is the detected terminal width (0 if undetectable).
    pub fn to_width(self, term_cols: u32) -> u32 {
        match self {
            Self::Tiny => 30,
            Self::Small => 45,
            Self::Medium => 60,
            Self::Large => 90,
            Self::Xl => 120,
            Self::Full => {
                if term_cols == 0 {
                    200
                } else {
                    term_cols.clamp(1, 4000)
                }
            }
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "texttv", version, about = "Render SVT Text-TV pages in the terminal")]
pub struct Args {
    /// Page number in 100..=999. Omit only with --list.
    #[arg(value_parser = parse_page, required_unless_present = "list")]
    pub page: Option<u16>,

    /// Rendering mode. If unset, picks `auto` on terminals with high-quality
    /// graphics protocols (Kitty, Ghostty, WezTerm) and `teletext` everywhere
    /// else (iTerm2, Apple Terminal, Alacritty, etc.).
    #[arg(long, value_enum)]
    pub mode: Option<Mode>,

    /// Render size for image modes. Ignored in teletext mode.
    #[arg(long, value_enum)]
    pub size: Option<Size>,

    /// Data source. Defaults to texttv-nu for teletext mode (rich color),
    /// svt for image modes. Override only for debugging or to fall back when
    /// the preferred source is unreachable.
    #[arg(long, value_enum)]
    pub source: Option<Source>,

    /// Strip ANSI color and the right-edge frame; produces plain mono output.
    #[arg(long)]
    pub no_color: bool,

    /// Print the well-known section index and exit.
    #[arg(long)]
    pub list: bool,

    /// Print the detected rendering protocol to stderr before drawing.
    #[arg(long)]
    pub debug_protocol: bool,

    /// Emit per-phase timing traces on stderr. Useful for performance
    /// debugging. Can also be enabled via `verbose: true` in the config
    /// file or `TEXTTV_TIMINGS=1` in the environment.
    #[arg(short, long)]
    pub verbose: bool,
}

fn parse_page(s: &str) -> Result<u16, String> {
    let n: u16 = s
        .parse()
        .map_err(|_| format!("'{s}' is not a number; PAGE must be 100..=999"))?;
    if !(100..=999).contains(&n) {
        return Err(format!("PAGE must be in 100..=999, got {n}"));
    }
    Ok(n)
}

pub const SECTIONS: &[(u16, &str)] = &[
    (100, "Innehåll / Nyheter"),
    (104, "Inrikes"),
    (130, "Utrikes"),
    (200, "Ekonomi"),
    (300, "Sport"),
    (377, "Tippstips"),
    (400, "Vädret"),
    (500, "TV-tablå"),
    (600, "Kultur & Nöje"),
    (700, "Konsument"),
    (800, "Programinfo"),
    (888, "Text-TV-information"),
];

pub fn print_sections(out: &mut dyn std::io::Write) -> std::io::Result<()> {
    writeln!(out, "SVT Text-TV — well-known pages:")?;
    for (page, name) in SECTIONS {
        writeln!(out, "  {page:>3}  {name}")?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_valid_page() {
        let args = Args::try_parse_from(["texttv", "300"]).expect("should parse");
        assert_eq!(args.page, Some(300));
        assert_eq!(args.mode, None);
    }

    #[test]
    fn rejects_page_below_100() {
        let err = Args::try_parse_from(["texttv", "42"]).unwrap_err();
        assert!(err.to_string().contains("100"), "msg = {err}");
    }

    #[test]
    fn rejects_page_above_999() {
        let err = Args::try_parse_from(["texttv", "1000"]).unwrap_err();
        assert!(err.to_string().contains("999"), "msg = {err}");
    }

    #[test]
    fn parses_mode_flag() {
        let args = Args::try_parse_from(["texttv", "300", "--mode", "blocks"]).expect("parse");
        assert_eq!(args.mode, Some(Mode::Blocks));
    }

    #[test]
    fn list_flag_makes_page_optional() {
        let args = Args::try_parse_from(["texttv", "--list"]).expect("parse");
        assert!(args.list);
        assert!(args.page.is_none());
    }

    #[test]
    fn page_required_without_list() {
        let err = Args::try_parse_from(["texttv"]).unwrap_err();
        let s = err.to_string().to_lowercase();
        assert!(s.contains("required") || s.contains("page"), "msg = {err}");
    }

    #[test]
    fn print_sections_writes_known_entries() {
        let mut buf = Vec::new();
        print_sections(&mut buf).expect("write");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains("100  Innehåll"));
        assert!(out.contains("300  Sport"));
    }
}
