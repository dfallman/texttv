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
