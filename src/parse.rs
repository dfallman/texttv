#![allow(clippy::expect_used)] // static selectors below are infallible at compile time

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use image::{DynamicImage, ImageFormat};
use scraper::{Html, Selector};

#[derive(Debug)]
pub struct Page {
    pub page_no: u16,
    pub images: Vec<DynamicImage>,
    pub text: String,
}

pub fn extract_page(html: &str, page_no: u16) -> Result<Page> {
    let doc = Html::parse_document(html);

    let img_sel = Selector::parse("img[src^='data:image/gif;base64,']")
        .map_err(|e| anyhow!("invalid selector: {e:?}"))?;
    let images = doc
        .select(&img_sel)
        .filter_map(|el| el.value().attr("src"))
        .map(decode_data_uri)
        .collect::<Result<Vec<_>>>()?;

    if images.is_empty() {
        return Err(anyhow!(
            "page {page_no} not available (no subpage images in response)"
        ));
    }

    let text = extract_text(&doc);

    Ok(Page {
        page_no,
        images,
        text,
    })
}

fn decode_data_uri(src: &str) -> Result<DynamicImage> {
    let prefix = "data:image/gif;base64,";
    let payload = src
        .strip_prefix(prefix)
        .ok_or_else(|| anyhow!("img src does not start with {prefix}"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .context("base64 decode failed")?;
    image::load_from_memory_with_format(&bytes, ImageFormat::Gif).context("gif decode failed")
}

fn extract_text(doc: &Html) -> String {
    // SVT exposes the page body inside a screenreader-only container with a CSS
    // module class name like "Content_screenreaderOnly__3Cnkp". The hash suffix
    // changes between builds, so match on the stable prefix.
    let reader_sel = Selector::parse("div[class*=\"screenreaderOnly\"]").expect("static selector");
    if let Some(node) = doc.select(&reader_sel).next() {
        return collect_text(node);
    }
    // Fall back to <main> if SVT changes the layout.
    let main_sel = Selector::parse("main").expect("static selector");
    let body_sel = Selector::parse("body").expect("static selector");
    let root = doc
        .select(&main_sel)
        .next()
        .or_else(|| doc.select(&body_sel).next());
    match root {
        Some(node) => collect_text(node),
        None => String::new(),
    }
}

fn collect_text(node: scraper::ElementRef<'_>) -> String {
    let raw: String = node.text().collect();
    let mut out = String::new();
    let mut prev_blank = false;
    let mut any_seen = false;
    for line in raw.lines() {
        let stripped = line.trim_end();
        let is_blank = stripped.chars().all(char::is_whitespace);
        if is_blank {
            if any_seen && !prev_blank {
                out.push('\n');
                prev_blank = true;
            }
        } else {
            out.push_str(stripped);
            out.push('\n');
            prev_blank = false;
            any_seen = true;
        }
    }
    out.trim_end().to_string()
}

// ============================================================================
// Colored render path: parse texttv.nu's classed HTML into a structured page.
// ============================================================================

/// Teletext color palette (8 saturated primaries).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

impl TtColor {
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            Self::Black => (0, 0, 0),
            Self::Red => (255, 0, 0),
            Self::Green => (0, 255, 0),
            Self::Yellow => (255, 255, 0),
            Self::Blue => (0, 0, 255),
            Self::Magenta => (255, 0, 255),
            Self::Cyan => (0, 255, 255),
            Self::White => (255, 255, 255),
        }
    }
}

/// One contiguous run of cells sharing the same color attributes.
#[derive(Debug, Clone)]
pub struct Cell {
    /// Visible text (spaces preserved). For mosaic cells this is the width-equivalent run of spaces.
    pub text: String,
    pub fg: TtColor,
    pub bg: TtColor,
    /// Some(url) when this cell is a teletext mosaic block — the value is the
    /// `https://l.texttv.nu/storage/chars/<HASH>.gif` URL pointing at the
    /// 13×16-px GIF that encodes the 2×3 sub-cell pattern.
    pub mosaic_url: Option<String>,
}

impl Cell {
    pub fn is_mosaic(&self) -> bool {
        self.mosaic_url.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct Line {
    pub cells: Vec<Cell>,
    /// Whether this line is double-height (teletext DH attribute).
    pub double_height: bool,
}

#[derive(Debug)]
pub struct ColoredPage {
    pub page_no: u16,
    /// First subpage's lines. Same as `subpages[0]` — kept for batch-CLI
    /// callers that don't care about multi-page rotation.
    pub lines: Vec<Line>,
    /// All subpages parsed from texttv.nu's `content[]` array. Most pages
    /// have a single entry; multi-page pages (the `XXXf` indicator) have
    /// 2+ entries representing the rotating subpages.
    pub subpages: Vec<Vec<Line>>,
    /// content_plain from the JSON, used when colors are disabled.
    pub plain: String,
}

pub fn parse_texttv_nu(json: &str, page_no: u16) -> Result<ColoredPage> {
    let v: serde_json::Value =
        serde_json::from_str(json).context("texttv.nu response is not valid JSON")?;
    let entry = v
        .get(0)
        .ok_or_else(|| anyhow!("page {page_no} not available (empty texttv.nu response)"))?;
    let content_array = entry
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| anyhow!("page {page_no}: missing content[] in texttv.nu JSON"))?;

    let mut subpages: Vec<Vec<Line>> = Vec::with_capacity(content_array.len());
    for item in content_array {
        if let Some(html) = item.as_str() {
            let lines = dedupe_overflow(parse_colored_html(html));
            if !lines.is_empty() {
                subpages.push(lines);
            }
        }
    }
    if subpages.is_empty() {
        return Err(anyhow!("page {page_no} not available (no lines parsed)"));
    }
    // texttv.nu sometimes omits content_plain — derive it from the first
    // subpage so the --no-color path always has something to print.
    let plain = derive_plain(&subpages[0]);
    let lines = subpages[0].clone();
    Ok(ColoredPage {
        page_no,
        lines,
        subpages,
        plain,
    })
}

/// Teletext is 24 visible rows + a 25th status row. If the parser ever
/// produces more than that — e.g. if texttv.nu starts stuttering DH lines or
/// duplicates a header/footer — collapse consecutive identical text-bearing
/// lines so we don't ship visible duplicates. Blank or mosaic-only lines are
/// left alone; legitimate teletext frequently has runs of blank rows.
const EXPECTED_TELETEXT_ROWS: usize = 25;

fn dedupe_overflow(lines: Vec<Line>) -> Vec<Line> {
    if lines.len() <= EXPECTED_TELETEXT_ROWS {
        return lines;
    }
    let mut out: Vec<Line> = Vec::with_capacity(lines.len());
    for line in lines {
        let drop = out
            .last()
            .is_some_and(|prev| has_text_content(prev) && lines_equivalent(&line, prev));
        if !drop {
            out.push(line);
        }
    }
    out
}

fn has_text_content(line: &Line) -> bool {
    line.cells
        .iter()
        .any(|c| !c.is_mosaic() && c.text.chars().any(|ch| !ch.is_whitespace()))
}

fn lines_equivalent(a: &Line, b: &Line) -> bool {
    if a.cells.len() != b.cells.len() {
        return false;
    }
    a.cells.iter().zip(&b.cells).all(|(x, y)| {
        x.text == y.text && x.fg == y.fg && x.bg == y.bg && x.mosaic_url == y.mosaic_url
    })
}

fn derive_plain(lines: &[Line]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        for cell in &line.cells {
            out.push_str(&cell.text);
        }
        let trimmed_len = out.trim_end_matches(' ').len();
        out.truncate(trimmed_len);
        out.push('\n');
        // Match the colored renderer's behaviour: swallow the always-blank row
        // that texttv.nu emits below each DH heading.
        if line.double_height
            && lines.get(i + 1).is_some_and(|n| {
                n.cells
                    .iter()
                    .all(|c| c.text.chars().all(char::is_whitespace))
            })
        {
            i += 2;
        } else {
            i += 1;
        }
    }
    out.trim_end().to_string()
}

fn parse_colored_html(html: &str) -> Vec<Line> {
    let frag = Html::parse_fragment(html);
    let line_sel = Selector::parse("span.line").expect("static selector");
    let span_sel = Selector::parse(":scope > span").expect("static selector");

    let mut lines = Vec::new();
    for line_el in frag.select(&line_sel) {
        let classes = line_el.value().attr("class").unwrap_or("");
        let double_height = classes.split_whitespace().any(|c| c == "DH");
        let mut cells: Vec<Cell> = Vec::new();
        for cell_el in line_el.select(&span_sel) {
            let cls = cell_el.value().attr("class").unwrap_or("");
            let style = cell_el.value().attr("style").unwrap_or("");
            let (fg, bg, mosaic_flag) = parse_cell_classes(cls);
            let mosaic_url = if mosaic_flag {
                extract_url_from_style(style)
            } else {
                None
            };
            let text: String = cell_el.text().collect();
            // Mosaic spans have no inner text — preserve a single-space placeholder
            // so the line keeps its width.
            let text = if mosaic_flag && text.is_empty() {
                " ".to_string()
            } else {
                text
            };
            if text.is_empty() {
                continue;
            }
            // Merge consecutive cells with identical attributes to keep escapes
            // minimal. Mosaic cells never merge — each has its own URL.
            let mergeable = mosaic_url.is_none();
            if mergeable
                && let Some(last) = cells.last_mut()
                && last.fg == fg
                && last.bg == bg
                && last.mosaic_url.is_none()
            {
                last.text.push_str(&text);
            } else {
                cells.push(Cell {
                    text,
                    fg,
                    bg,
                    mosaic_url,
                });
            }
        }
        if !cells.is_empty() {
            lines.push(Line {
                cells,
                double_height,
            });
        }
    }
    lines
}

/// Pull the URL out of a CSS `background-image: url(...)` declaration.
///
/// Handles both CSS url() forms:
/// - `url("...")` / `url('...')` — quoted; ends at the matching quote
/// - `url(...)` — unquoted; ends at the first `)` (CSS spec disallows
///   unescaped `)` or whitespace inside unquoted url() tokens, so the next
///   `)` is unambiguous)
fn extract_url_from_style(style: &str) -> Option<String> {
    let after_open = &style[style.find("url(")? + 4..];
    let trimmed = after_open.trim_start();
    let first = trimmed.chars().next()?;
    if first == '"' || first == '\'' {
        let after_quote = &trimmed[1..];
        let close = after_quote.find(first)?;
        return Some(after_quote[..close].to_string());
    }
    let end = trimmed.find(')')?;
    Some(trimmed[..end].trim().to_string())
}

fn parse_cell_classes(class_attr: &str) -> (TtColor, TtColor, bool) {
    let mut fg = TtColor::White;
    let mut bg = TtColor::Black;
    let mut mosaic = false;
    for tok in class_attr.split_whitespace() {
        if tok == "bgImg" {
            mosaic = true;
        } else if let Some(c) = bg_from_token(tok) {
            bg = c;
        } else if let Some(c) = fg_from_token(tok) {
            fg = c;
        } else if !is_known_structural_token(tok) {
            // Under --verbose, log unfamiliar tokens once. They're harmless
            // (we just ignore them and render with the default fg/bg), but
            // they suggest texttv.nu's class vocabulary has shifted and the
            // parser may need updating.
            note_unknown_class(tok);
        }
    }
    (fg, bg, mosaic)
}

/// Class tokens we know about but don't act on inside `parse_cell_classes`;
/// they're handled by the line-level loop (`line`, `DH`) or are structural
/// wrappers (`toprow`, `root`).
fn is_known_structural_token(tok: &str) -> bool {
    matches!(tok, "line" | "toprow" | "DH" | "root")
}

/// Emit a one-time `[texttv]` note for an unknown class token. De-duplicated
/// so a class appearing in every cell logs once, not hundreds of times.
/// No-op when `--verbose` is off.
fn note_unknown_class(tok: &str) {
    if !crate::timing::enabled() {
        return;
    }
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let set = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    if let Ok(mut s) = set.lock()
        && s.insert(tok.to_string())
    {
        crate::timing::note(&format!("unknown teletext class token: {tok}"));
    }
}

fn fg_from_token(tok: &str) -> Option<TtColor> {
    match tok {
        "W" => Some(TtColor::White),
        "Y" => Some(TtColor::Yellow),
        "R" => Some(TtColor::Red),
        "G" => Some(TtColor::Green),
        "B" => Some(TtColor::Blue),
        "C" => Some(TtColor::Cyan),
        "M" => Some(TtColor::Magenta),
        "bl" => Some(TtColor::Black),
        _ => None,
    }
}

fn bg_from_token(tok: &str) -> Option<TtColor> {
    let suf = tok.strip_prefix("bg")?;
    match suf {
        "W" => Some(TtColor::White),
        "Y" => Some(TtColor::Yellow),
        "R" => Some(TtColor::Red),
        "G" => Some(TtColor::Green),
        "B" => Some(TtColor::Blue),
        "C" => Some(TtColor::Cyan),
        "M" => Some(TtColor::Magenta),
        "Bl" => Some(TtColor::Black),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn extract_url_unquoted() {
        let style = "background-image: url(https://l.texttv.nu/storage/chars/abc.gif); other: foo;";
        assert_eq!(
            extract_url_from_style(style).as_deref(),
            Some("https://l.texttv.nu/storage/chars/abc.gif")
        );
    }

    #[test]
    fn extract_url_double_quoted() {
        let style = "background-image: url(\"https://example.com/has)paren.gif\");";
        assert_eq!(
            extract_url_from_style(style).as_deref(),
            Some("https://example.com/has)paren.gif")
        );
    }

    #[test]
    fn extract_url_single_quoted() {
        let style = "background-image: url('https://example.com/img.gif')";
        assert_eq!(
            extract_url_from_style(style).as_deref(),
            Some("https://example.com/img.gif")
        );
    }

    #[test]
    fn extract_url_missing_returns_none() {
        assert_eq!(extract_url_from_style("color: red;"), None);
    }

    #[test]
    fn parse_cell_classes_defaults() {
        let (fg, bg, mosaic) = parse_cell_classes("");
        assert_eq!(fg, TtColor::White);
        assert_eq!(bg, TtColor::Black);
        assert!(!mosaic);
    }

    #[test]
    fn parse_cell_classes_picks_fg_and_bg() {
        let (fg, bg, mosaic) = parse_cell_classes("Y bgR");
        assert_eq!(fg, TtColor::Yellow);
        assert_eq!(bg, TtColor::Red);
        assert!(!mosaic);
    }

    #[test]
    fn parse_cell_classes_detects_mosaic() {
        let (_, _, mosaic) = parse_cell_classes("bgImg G bgB");
        assert!(mosaic);
    }

    #[test]
    fn parse_cell_classes_ignores_structural_tokens() {
        // Should not warn or affect colors; defaults stay.
        let (fg, bg, _) = parse_cell_classes("line DH toprow root");
        assert_eq!(fg, TtColor::White);
        assert_eq!(bg, TtColor::Black);
    }
}
