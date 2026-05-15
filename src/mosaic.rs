//! Mosaic-glyph decoding for teletext block characters.
//!
//! texttv.nu encodes each unique 2×3 mosaic pattern as a tiny GIF (~13×16 px)
//! hosted at `https://l.texttv.nu/storage/chars/<hash>.gif`. We fetch each
//! unique URL once, sample the six sub-cell centres, classify each as fg-or-bg
//! by Euclidean distance to the cell's two known teletext colors, and pack
//! the result into a 6-bit pattern. The pattern maps to a Unicode sextant
//! character in the "Symbols for Legacy Computing" block (U+1FB00 .. U+1FB3B),
//! plus four special cases (space / left half / right half / full block).

use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::parse::{ColoredPage, TtColor};

const FETCH_TIMEOUT_SECS: u64 = 5;
const USER_AGENT: &str = concat!(
    "texttv/",
    env!("CARGO_PKG_VERSION"),
    " (+mosaic-fetch)"
);

/// Shared HTTP agent. Building one fresh per call meant a new TLS handshake
/// every fetch — death by latency on a page with a dozen unique mosaics.
/// A single Agent pools the underlying TCP/TLS connection across all
/// requests in this process.
fn agent() -> &'static ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(3))
            .timeout_read(Duration::from_secs(FETCH_TIMEOUT_SECS))
            .user_agent(USER_AGENT)
            .build()
    })
}

/// In-process cache of mosaic patterns. Keyed by URL because the URL hash is
/// what texttv.nu serves; same hash → same pattern, regardless of which
/// page or cell it appeared in.
fn cache() -> &'static Mutex<HashMap<String, u8>> {
    static CELL: OnceLock<Mutex<HashMap<String, u8>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve a mosaic URL to its 6-bit pattern, fetching + decoding once and
/// caching for the rest of the process. Three-tier lookup:
///
/// 1. Process-memory cache (fastest).
/// 2. On-disk cache under `$XDG_CACHE_HOME/texttv/mosaics/<hash>.pat`.
///    Patterns are stable across runs because the GIF hash is content-addressed:
///    same hash → same image bytes → same pattern, forever.
/// 3. Network fetch + decode + write through both caches.
pub fn resolve_pattern(url: &str, fg: TtColor, bg: TtColor) -> Result<u8> {
    if let Ok(guard) = cache().lock()
        && let Some(p) = guard.get(url).copied()
    {
        return Ok(p);
    }
    if let Some(p) = read_disk_cache(url) {
        if let Ok(mut guard) = cache().lock() {
            guard.insert(url.to_string(), p);
        }
        return Ok(p);
    }
    let bytes = fetch(url)?;
    let pattern = decode_pattern(&bytes, fg, bg)?;
    if let Ok(mut guard) = cache().lock() {
        guard.insert(url.to_string(), pattern);
    }
    write_disk_cache(url, pattern);
    Ok(pattern)
}

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("texttv").join("mosaics"))
}

/// File name = the bare GIF hash from the URL (the digits before `.gif`).
/// Falling back to a stable hash of the whole URL means an off-spec URL still
/// caches, just under a less-recognisable name.
fn cache_key(url: &str) -> String {
    if let Some(name) = url.rsplit('/').next()
        && let Some(stem) = name.strip_suffix(".gif")
        && stem.chars().all(|c| c.is_ascii_digit())
    {
        return stem.to_string();
    }
    // Fallback: hash the URL deterministically.
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write(url.as_bytes());
    format!("u{:016x}", h.finish())
}

fn read_disk_cache(url: &str) -> Option<u8> {
    let path = cache_dir()?.join(format!("{}.pat", cache_key(url)));
    let bytes = std::fs::read(&path).ok()?;
    // File holds exactly one byte: the 6-bit pattern (high two bits ignored).
    bytes.first().copied().map(|b| b & 0b00111111)
}

fn write_disk_cache(url: &str, pattern: u8) {
    let Some(dir) = cache_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join(format!("{}.pat", cache_key(url)));
    // Write atomically via tmp+rename so concurrent writers don't tear.
    let tmp = path.with_extension("pat.tmp");
    if std::fs::write(&tmp, [pattern]).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Pre-fetch every unique mosaic URL in the page in parallel so the render
/// loop can hit the cache for every cell. Failed fetches are recorded as
/// cache misses (the render path falls back to a colored space). Caller
/// invokes this once between parse and render.
pub fn prefetch_page(page: &ColoredPage) {
    let mut seen: HashMap<String, (TtColor, TtColor)> = HashMap::new();
    for line in &page.lines {
        for cell in &line.cells {
            if let Some(url) = cell.mosaic_url.as_deref()
                && !seen.contains_key(url)
            {
                if let Ok(guard) = cache().lock()
                    && guard.contains_key(url)
                {
                    continue;
                }
                seen.insert(url.to_string(), (cell.fg, cell.bg));
            }
        }
    }
    if seen.is_empty() {
        return;
    }
    let items: Vec<_> = seen.into_iter().collect();
    std::thread::scope(|s| {
        for (url, (fg, bg)) in &items {
            s.spawn(move || {
                let _ = resolve_pattern(url, *fg, *bg);
            });
        }
    });
}

fn fetch(url: &str) -> Result<Vec<u8>> {
    let t0 = std::time::Instant::now();
    let resp = agent()
        .get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let t_resp = t0.elapsed().as_millis();
    let mut buf = Vec::new();
    use std::io::Read;
    resp.into_reader()
        .take(64 * 1024)
        .read_to_end(&mut buf)
        .with_context(|| format!("reading {url}"))?;
    let t_total = t0.elapsed().as_millis();
    if crate::timing::enabled() {
        let short = url.rsplit('/').next().unwrap_or(url);
        eprintln!("[texttv]   fetch {short}: resp={t_resp}ms total={t_total}ms");
    }
    Ok(buf)
}

/// Decode the GIF, sample 6 sub-cells at their geometric centres, and
/// classify each as fg (bit set) or bg (bit clear) by closer Euclidean
/// distance to the two expected teletext colors.
pub fn decode_pattern(gif_bytes: &[u8], fg: TtColor, bg: TtColor) -> Result<u8> {
    let img = image::load_from_memory_with_format(gif_bytes, image::ImageFormat::Gif)
        .context("decoding mosaic GIF")?;
    let rgb = img.to_rgb8();
    let (w, h) = (rgb.width(), rgb.height());
    if w == 0 || h == 0 {
        return Err(anyhow!("mosaic GIF has zero dimensions"));
    }
    let fg_rgb = fg.rgb();
    let bg_rgb = bg.rgb();
    let mut pattern: u8 = 0;
    // Sub-cell order: top-left, top-right, mid-left, mid-right, bot-left, bot-right.
    // Bit index follows the same order — bit 0 = top-left.
    for sy in 0..3u32 {
        for sx in 0..2u32 {
            let cx = (w * (2 * sx + 1)) / 4;
            let cy = (h * (2 * sy + 1)) / 6;
            let p = rgb.get_pixel(cx.min(w - 1), cy.min(h - 1));
            if is_closer_to(p.0, fg_rgb, bg_rgb) {
                let bit = sy * 2 + sx;
                pattern |= 1 << bit;
            }
        }
    }
    Ok(pattern)
}

fn is_closer_to(pixel: [u8; 3], target: (u8, u8, u8), other: (u8, u8, u8)) -> bool {
    fn d2(a: [u8; 3], b: (u8, u8, u8)) -> u32 {
        let dr = a[0] as i32 - b.0 as i32;
        let dg = a[1] as i32 - b.1 as i32;
        let db = a[2] as i32 - b.2 as i32;
        (dr * dr + dg * dg + db * db) as u32
    }
    d2(pixel, target) < d2(pixel, other)
}

/// Map a 6-bit sub-cell pattern to its Unicode glyph.
///
/// Sub-cell bit order (matches `decode_pattern`):
/// ```text
///   bit 0 (top-left)    bit 1 (top-right)
///   bit 2 (mid-left)    bit 3 (mid-right)
///   bit 4 (bot-left)    bit 5 (bot-right)
/// ```
///
/// The "Symbols for Legacy Computing" block (Unicode 13) covers 60 of the 64
/// possible patterns. The four special cases (empty / left half / right half
/// / full) map to pre-existing characters.
pub fn pattern_to_glyph(pattern: u8) -> char {
    match pattern {
        0 => ' ',
        // bits 0, 2, 4 = left column = bits 0b010101 = 21 → LEFT HALF BLOCK
        0b010101 => '\u{258C}',
        // bits 1, 3, 5 = right column = bits 0b101010 = 42 → RIGHT HALF BLOCK
        0b101010 => '\u{2590}',
        0b111111 => '\u{2588}', // FULL BLOCK
        p => {
            // U+1FB00 corresponds to pattern 1 (top-left only). Codepoints
            // count up sequentially, skipping the three "in-existing-block"
            // patterns (21, 42, 63).
            let mut offset = p as u32 - 1;
            if p > 21 {
                offset -= 1;
            }
            if p > 42 {
                offset -= 1;
            }
            char::from_u32(0x1FB00 + offset).unwrap_or('?')
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_pattern_is_space() {
        assert_eq!(pattern_to_glyph(0), ' ');
    }

    #[test]
    fn left_column_is_left_half_block() {
        // bits 0 (top-left) + 2 (mid-left) + 4 (bot-left) = 0b010101 = 21
        assert_eq!(pattern_to_glyph(0b010101), '\u{258C}');
    }

    #[test]
    fn right_column_is_right_half_block() {
        // bits 1 (top-right) + 3 (mid-right) + 5 (bot-right) = 0b101010 = 42
        assert_eq!(pattern_to_glyph(0b101010), '\u{2590}');
    }

    #[test]
    fn full_pattern_is_full_block() {
        assert_eq!(pattern_to_glyph(0b111111), '\u{2588}');
    }

    #[test]
    fn top_left_only_is_first_sextant() {
        assert_eq!(pattern_to_glyph(1), '\u{1FB00}');
    }

    #[test]
    fn top_right_only_is_second_sextant() {
        assert_eq!(pattern_to_glyph(2), '\u{1FB01}');
    }

    #[test]
    fn pattern_20_codepoint_offset() {
        // pattern 20 — sextant; offset = 20-1 = 19 → U+1FB13
        assert_eq!(pattern_to_glyph(20), '\u{1FB13}');
    }

    #[test]
    fn pattern_22_skips_left_half() {
        // pattern 22 — sextant; offset = 22-1-1 = 20 → U+1FB14
        assert_eq!(pattern_to_glyph(22), '\u{1FB14}');
    }

    #[test]
    fn last_sextant_is_fb3b() {
        // pattern 62 — offset = 62 - 1 - 1 - 1 = 59 → U+1FB3B
        assert_eq!(pattern_to_glyph(62), '\u{1FB3B}');
    }

    #[test]
    fn is_closer_to_picks_closer_color() {
        assert!(is_closer_to([255, 255, 255], (255, 255, 255), (0, 0, 0)));
        assert!(!is_closer_to([0, 0, 0], (255, 255, 255), (0, 0, 0)));
        assert!(is_closer_to([200, 200, 200], (255, 255, 255), (0, 0, 0)));
    }
}
