//! In-terminal interactive page browser. See
//! `docs/superpowers/specs/2026-05-15-interactive-mode-design.md`.

use anyhow::Result;

use crate::parse::Line;

/// A three-digit page reference scanned out of a rendered teletext page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// 0-based row within the page body (i.e. excluding the input row).
    pub row: u16,
    /// 0-based column of the leading space cell.
    pub col_start: u16,
    /// 4 (no leading space at row start/end) or 5 (full ` XXX ` run).
    pub col_len: u16,
    /// The 3-digit number. 0..=999.
    pub target: u16,
    /// `false` when `target < 100`. ↑↓ still land on this link; Enter
    /// is a no-op + status flash.
    pub followable: bool,
}

/// Scan the rendered page for three-digit page references that are
/// flanked by spaces (or line edges) on both sides. Mosaic cells count
/// as spaces — they never participate in a link.
pub fn scan_links(lines: &[Line]) -> Vec<Link> {
    let mut out = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        // Flatten cells into a single string. Mosaic cells contribute a
        // single space placeholder so they form a link boundary.
        let mut flat = String::new();
        for cell in &line.cells {
            if cell.is_mosaic() {
                flat.push(' ');
            } else {
                flat.push_str(&cell.text);
            }
        }
        let bytes = flat.as_bytes();
        let mut i = 0;
        while i + 3 <= bytes.len() {
            if !(bytes[i].is_ascii_digit()
                && bytes[i + 1].is_ascii_digit()
                && bytes[i + 2].is_ascii_digit())
            {
                i += 1;
                continue;
            }
            let prev_is_digit = i > 0 && bytes[i - 1].is_ascii_digit();
            let next_is_digit = i + 3 < bytes.len() && bytes[i + 3].is_ascii_digit();
            if prev_is_digit || next_is_digit {
                // Part of a longer digit run; skip past all the digits.
                let mut j = i;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                i = j;
                continue;
            }
            let left_boundary = i == 0 || bytes[i - 1] == b' ';
            let right_boundary = i + 3 == bytes.len() || bytes[i + 3] == b' ';
            if !(left_boundary && right_boundary) {
                i += 1;
                continue;
            }
            // Compose the highlight extent: include the surrounding space
            // cells when they exist.
            let col_start = if i > 0 { (i as u16) - 1 } else { 0 };
            let after_digits = i + 3;
            let col_end = if after_digits < bytes.len() {
                (after_digits as u16) + 1
            } else {
                after_digits as u16
            };
            let target: u16 = std::str::from_utf8(&bytes[i..i + 3])
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            out.push(Link {
                row: row as u16,
                col_start,
                col_len: col_end - col_start,
                target,
                followable: target >= 100,
            });
            i = after_digits;
        }
    }
    out
}

/// Entry point for interactive mode. Renders the initial page and runs the
/// event loop until the user presses Esc.
pub fn run(_initial_page: u16) -> Result<()> {
    // Filled in over the following tasks.
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::parse::{Cell, Line, TtColor};

    /// Build a one-cell line containing the given text. Tests don't need
    /// per-cell color attributes — the scanner ignores them.
    fn line(text: &str) -> Line {
        Line {
            cells: vec![Cell {
                text: text.to_string(),
                fg: TtColor::White,
                bg: TtColor::Black,
                mosaic_url: None,
            }],
            double_height: false,
        }
    }

    #[test]
    fn scan_finds_single_link() {
        let lines = vec![line(" 300 ")];
        let links = scan_links(&lines);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, 300);
        assert_eq!(links[0].row, 0);
        assert_eq!(links[0].col_start, 0);
        assert_eq!(links[0].col_len, 5);
        assert!(links[0].followable);
    }

    #[test]
    fn scan_ignores_decimal_numbers() {
        let lines = vec![line("100.000")];
        let links = scan_links(&lines);
        assert!(links.is_empty());
    }

    #[test]
    fn scan_ignores_four_digit_runs() {
        let lines = vec![line(" 1234 ")];
        let links = scan_links(&lines);
        assert!(links.is_empty());
    }

    #[test]
    fn scan_marks_low_pages_unfollowable() {
        let lines = vec![line(" 099 ")];
        let links = scan_links(&lines);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, 99);
        assert!(!links[0].followable);
    }

    #[test]
    fn scan_finds_multiple_links_per_row() {
        let lines = vec![line(" 300  400 ")];
        let links = scan_links(&lines);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].target, 300);
        assert_eq!(links[1].target, 400);
        assert_eq!(links[0].col_start, 0);
        assert_eq!(links[1].col_start, 5);
    }

    #[test]
    fn scan_finds_link_at_line_start() {
        let lines = vec![line("300 foo")];
        let links = scan_links(&lines);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].col_start, 0);
        assert_eq!(links[0].col_len, 4);
    }

    #[test]
    fn scan_finds_link_at_line_end() {
        let lines = vec![line("foo 300")];
        let links = scan_links(&lines);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].col_start, 3);
        assert_eq!(links[0].col_len, 4);
    }

    #[test]
    fn scan_treats_mosaic_cell_as_space() {
        let lines = vec![Line {
            cells: vec![
                Cell {
                    text: " 300 ".to_string(),
                    fg: TtColor::White,
                    bg: TtColor::Black,
                    mosaic_url: None,
                },
                Cell {
                    text: " ".to_string(),
                    fg: TtColor::White,
                    bg: TtColor::Black,
                    mosaic_url: Some("https://example.com/m.gif".to_string()),
                },
                Cell {
                    text: " 400 ".to_string(),
                    fg: TtColor::White,
                    bg: TtColor::Black,
                    mosaic_url: None,
                },
            ],
            double_height: false,
        }];
        let links = scan_links(&lines);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].target, 300);
        assert_eq!(links[1].target, 400);
    }
}
