use texttv::parse::extract_page;

const PAGE_300: &str = include_str!("fixtures/page-300.html");
const PAGE_EMPTY: &str = include_str!("fixtures/page-empty.html");

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
    assert!(
        page.text.len() > 50,
        "text body too short: {:?}",
        page.text
    );
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
