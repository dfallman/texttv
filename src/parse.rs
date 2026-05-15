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
