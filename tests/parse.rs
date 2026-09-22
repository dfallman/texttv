#![allow(clippy::unwrap_used, clippy::expect_used)]

use texttv::parse::{TtColor, extract_page, parse_texttv_nu};

const PAGE_300: &str = include_str!("fixtures/page-300.html");
const PAGE_EMPTY: &str = include_str!("fixtures/page-empty.html");
const PAGE_300_NU: &str = include_str!("fixtures/page-300.texttv-nu.json");
const PAGE_200_NU: &str = include_str!("fixtures/page-200.texttv-nu.json");

#[test]
fn extracts_at_least_one_subpage_image() {
    let page = extract_page(PAGE_300, 300).expect("should parse");
    assert_eq!(page.page_no, 300);
    assert!(!page.images.is_empty(), "expected at least one subpage GIF");
}

#[test]
fn each_image_has_nonzero_dimensions() {
    let page = extract_page(PAGE_300, 300).expect("parse");
    for (i, img) in page.images.iter().enumerate() {
        assert!(
            img.width() > 0 && img.height() > 0,
            "subpage {i} has zero dims"
        );
    }
}

#[test]
fn extracts_non_empty_swedish_text() {
    let page = extract_page(PAGE_300, 300).expect("parse");
    assert!(page.text.len() > 50, "text body too short: {:?}", page.text);
    let lower = page.text.to_lowercase();
    assert!(
        lower.contains('å')
            || lower.contains('ä')
            || lower.contains('ö')
            || lower.contains("sport")
            || lower.contains("svt"),
        "no Swedish/sport marker in extracted text: {lower:?}"
    );
}

#[test]
fn empty_page_is_an_error() {
    let err = extract_page(PAGE_EMPTY, 404).unwrap_err();
    let msg = format!("{err:#}");
    let lower = msg.to_lowercase();
    assert!(
        lower.contains("not available") || lower.contains("no subpage"),
        "unexpected error: {msg}"
    );
}

// -------- texttv.nu colored parser tests --------

#[test]
fn texttv_nu_parses_lines() {
    let cp = parse_texttv_nu(PAGE_300_NU, 300).expect("parse");
    assert_eq!(cp.page_no, 300);
    assert!(
        cp.lines.len() >= 20,
        "expected ~24 teletext rows, got {}",
        cp.lines.len()
    );
    assert!(
        !cp.plain.is_empty(),
        "plain text fallback should be populated"
    );
}

#[test]
fn texttv_nu_top_row_has_yellow_svt_text() {
    let cp = parse_texttv_nu(PAGE_300_NU, 300).expect("parse");
    // The header row contains a cell with yellow foreground reading "SVT Text".
    let top = &cp.lines[0];
    let yellow_cell = top
        .cells
        .iter()
        .find(|c| c.fg == TtColor::Yellow && c.text.contains("SVT"))
        .expect("expected a yellow SVT Text cell on the top row");
    assert!(yellow_cell.text.contains("SVT Text"));
}

#[test]
fn texttv_nu_swedish_characters_pass_through() {
    let cp = parse_texttv_nu(PAGE_300_NU, 300).expect("parse");
    let joined: String = cp
        .lines
        .iter()
        .flat_map(|l| l.cells.iter())
        .map(|c| c.text.as_str())
        .collect();
    assert!(joined.to_lowercase().contains('å') || joined.contains("Åberg"));
}

#[test]
fn texttv_nu_detects_double_height() {
    let cp = parse_texttv_nu(PAGE_200_NU, 200).expect("parse");
    assert!(
        cp.lines.iter().any(|l| l.double_height),
        "page 200 fixture should contain at least one double-height line"
    );
}

// ---------------------------------------------------------------------------
// Hardening: untrusted response content must not reach the terminal raw.
// ---------------------------------------------------------------------------

fn nu_json(html: &str) -> String {
    serde_json::json!([{ "content": [html] }]).to_string()
}

#[test]
fn texttv_nu_strips_control_characters_from_cell_text() {
    // `&#27;` decodes to ESC; `&#7;` to BEL — an OSC title-set sequence.
    let html = r#"<span class="root"><span class="line"><span class="W">100 &#27;]0;PWNED&#7; hej</span></span></span>"#;
    let cp = parse_texttv_nu(&nu_json(html), 100).expect("parse");
    let text: String = cp.lines[0].cells.iter().map(|c| c.text.as_str()).collect();
    assert!(
        !text.chars().any(char::is_control),
        "control char leaked into cell text: {text:?}"
    );
    assert!(text.contains("hej"), "visible text must survive: {text:?}");
    assert!(
        !cp.plain.chars().any(|c| c.is_control() && c != '\n'),
        "control char leaked into plain text: {:?}",
        cp.plain
    );
}

#[test]
fn svt_html_strips_control_characters_from_text() {
    let html = r#"<html><body><img src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"><div class="Content_screenreaderOnly__x">100 &#27;]0;PWNED&#7; hej</div></body></html>"#;
    let page = extract_page(html, 100).expect("parse");
    assert!(
        !page.text.chars().any(|c| c.is_control() && c != '\n'),
        "control char leaked: {:?}",
        page.text
    );
    assert!(page.text.contains("hej"));
}

#[test]
fn texttv_nu_rejects_mosaic_urls_outside_texttv_nu_cdn() {
    let html = r#"<span class="root"><span class="line"><span class="bgImg" style="background-image: url(http://127.0.0.1:9/123.gif)"> </span><span class="bgImg" style="background-image: url(https://evil.example/storage/chars/123.gif)"> </span><span class="bgImg" style="background-image: url(https://l.texttv.nu/storage/chars/123.gif)"> </span></span></span>"#;
    let cp = parse_texttv_nu(&nu_json(html), 100).expect("parse");
    let urls: Vec<&str> = cp.lines[0]
        .cells
        .iter()
        .filter_map(|c| c.mosaic_url.as_deref())
        .collect();
    assert_eq!(
        urls,
        vec!["https://l.texttv.nu/storage/chars/123.gif"],
        "only the texttv.nu CDN may be fetched for mosaics"
    );
    // Rejected mosaics still occupy their columns so layout is preserved
    // (the two rejected cells merge into one plain two-space run).
    let width: usize = cp.lines[0]
        .cells
        .iter()
        .map(|c| c.text.chars().count())
        .sum();
    assert_eq!(width, 3);
}

#[test]
fn svt_html_rejects_oversized_page_gif() {
    // A valid GIF one pixel wider than the page limit. SVT's real pages
    // are 520×400; the decoder must refuse anything past the cap rather
    // than allocating hundreds of megabytes on a hostile response.
    use base64::Engine;
    use image::codecs::gif::GifEncoder;
    let (w, h) = (texttv::parse::MAX_PAGE_GIF_DIM + 1, 1u32);
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));
    let mut gif = Vec::new();
    GifEncoder::new(&mut gif)
        .encode(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .expect("encode");
    let b64 = base64::engine::general_purpose::STANDARD.encode(&gif);
    let html = format!(
        r#"<html><body><img src="data:image/gif;base64,{b64}"><div class="Content_screenreaderOnly__x">x</div></body></html>"#
    );
    let err = extract_page(&html, 100).expect_err("oversized page GIF must be rejected");
    let msg = format!("{err:#}").to_lowercase();
    assert!(msg.contains("limit"), "unexpected error: {err:#}");
}
