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
    let reader_sel =
        Selector::parse("div[class*=\"screenreaderOnly\"]").expect("static selector");
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
    /// True for teletext mosaic block-graphics — rendered as a colored space until we map them.
    pub mosaic: bool,
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
    pub lines: Vec<Line>,
    /// content_plain from the JSON, used when colors are disabled.
    pub plain: String,
}

pub fn parse_texttv_nu(json: &str, page_no: u16) -> Result<ColoredPage> {
    let v: serde_json::Value =
        serde_json::from_str(json).context("texttv.nu response is not valid JSON")?;
    let entry = v
        .get(0)
        .ok_or_else(|| anyhow!("page {page_no} not available (empty texttv.nu response)"))?;
    let content_html = entry
        .get("content")
        .and_then(|c| c.get(0))
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow!("page {page_no}: missing content[0] in texttv.nu JSON"))?;
    let lines = parse_colored_html(content_html);
    if lines.is_empty() {
        return Err(anyhow!("page {page_no} not available (no lines parsed)"));
    }
    // texttv.nu sometimes omits content_plain — derive it from the parsed lines
    // so the --no-color path always has something to print.
    let plain = derive_plain(&lines);
    Ok(ColoredPage {
        page_no,
        lines,
        plain,
    })
}

fn derive_plain(lines: &[Line]) -> String {
    let mut out = String::new();
    for line in lines {
        for cell in &line.cells {
            out.push_str(&cell.text);
        }
        let trimmed_len = out.trim_end_matches(' ').len();
        out.truncate(trimmed_len);
        out.push('\n');
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
            let (fg, bg, mosaic) = parse_cell_classes(cls);
            let text: String = cell_el.text().collect();
            // Mosaic spans have no inner text — preserve a single-space placeholder
            // so the line keeps its width.
            let text = if mosaic && text.is_empty() {
                " ".to_string()
            } else {
                text
            };
            if text.is_empty() {
                continue;
            }
            // Merge consecutive cells with identical attributes to keep escapes minimal.
            if let Some(last) = cells.last_mut()
                && last.fg == fg
                && last.bg == bg
                && last.mosaic == mosaic
            {
                last.text.push_str(&text);
            } else {
                cells.push(Cell {
                    text,
                    fg,
                    bg,
                    mosaic,
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
        }
        // 'line', 'toprow', 'DH', 'root' fall through; handled elsewhere.
    }
    (fg, bg, mosaic)
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
