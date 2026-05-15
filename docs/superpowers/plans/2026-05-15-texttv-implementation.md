# texttv Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `texttv`, a Rust CLI (edition 2024) that fetches an SVT Text-TV page by number and renders the embedded GIF sub-pages in the terminal with native graphics protocols on Kitty/Ghostty/WezTerm/iTerm2 and a half-block fallback elsewhere.

**Architecture:** Single binary crate. Synchronous I/O (no async runtime). Flat module layout: `cli` (clap parser) → `fetch` (blocking HTTP via `ureq`) → `parse` (HTML → `Vec<DynamicImage>` + plain text, via `scraper` + `image`) → `render` (image via `viuer`, or colorized text). `main.rs` wires them and translates errors into exit codes 0/1/2. Fixture-based unit tests for `parse` and `cli` keep the test suite deterministic and offline; rendering is verified manually per the terminal matrix.

**Tech Stack:** Rust edition 2024 (toolchain 1.94+), `clap` 4 (derive), `ureq` 2 (blocking HTTP+TLS), `scraper` 0.20 (HTML), `image` 0.25 (gif decode only), `base64` 0.22, `viuer` 0.9 (terminal image protocols), `anyhow`, `thiserror`.

---

## File Map

| Path | Responsibility |
| --- | --- |
| `Cargo.toml` | Package metadata, deps from §3 of the spec, edition 2024, lints config |
| `src/main.rs` | Entry point. Parse args, dispatch, map errors to exit codes, write to stdout/stderr |
| `src/lib.rs` | Re-exports the modules so they can be unit-tested |
| `src/cli.rs` | `#[derive(Parser)] Args`, the `Mode` enum, `parse_page()` validator, `--list` table |
| `src/fetch.rs` | `fetch_html(page: u16) -> anyhow::Result<String>` with UA + 10s timeout; typed error for non-2xx |
| `src/parse.rs` | `extract_page(html: &str, page: u16) -> anyhow::Result<Page>` → `Page { page_no, images: Vec<DynamicImage>, text: String }` |
| `src/render.rs` | `render_images(&[DynamicImage], Mode) -> Result<DetectedProtocol>` and `render_text(&str, color: bool) -> Result<()>` |
| `tests/fixtures/page-300.html` | Captured-once HTML response from `svt.se/text-tv/300` for parser tests |
| `tests/fixtures/page-empty.html` | A near-empty page (no `<img data:image/gif…>`) for the "page not available" path |
| `tests/parse.rs` | Integration tests against the fixtures |
| `README.md` | Install, terminal matrix, `--mode` table, tmux passthrough caveat |

The `lib.rs` + `main.rs` split is so `tests/parse.rs` can `use texttv::parse::extract_page` without going through the binary.

---

## Task 1: Project scaffolding

**Files:**
- Create: `/home/dfallman/dev/texttv/Cargo.toml`
- Create: `/home/dfallman/dev/texttv/src/main.rs`
- Create: `/home/dfallman/dev/texttv/src/lib.rs`
- Create: `/home/dfallman/dev/texttv/.gitignore`

- [ ] **Step 1: `cargo init` the binary crate**

Run from `/home/dfallman/dev/texttv`:
```bash
cargo init --name texttv --vcs git
```
Expected: creates `Cargo.toml`, `src/main.rs`, `.gitignore`, and `git init`s the directory.

- [ ] **Step 2: Replace `Cargo.toml` with the verified dependency set**

Write `/home/dfallman/dev/texttv/Cargo.toml`:
```toml
[package]
name = "texttv"
version = "0.1.0"
edition = "2024"
description = "Render SVT Text-TV pages in the terminal"
license = "MIT OR Apache-2.0"

[dependencies]
clap = { version = "4", features = ["derive"] }
ureq = { version = "2", features = ["tls"] }
viuer = "0.9"
image = { version = "0.25", default-features = false, features = ["gif"] }
base64 = "0.22"
scraper = "0.20"
anyhow = "1"
thiserror = "1"

[profile.release]
lto = "thin"
strip = "symbols"

[lints.rust]
unsafe_code = "forbid"
unused_must_use = "deny"

[lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
```

The `unwrap_used`/`expect_used` clippy lints enforce acceptance criterion §7.8.

- [ ] **Step 3: Create `src/lib.rs` exposing the modules**

Write `/home/dfallman/dev/texttv/src/lib.rs`:
```rust
pub mod cli;
pub mod fetch;
pub mod parse;
pub mod render;
```

- [ ] **Step 4: Stub the four modules so the crate compiles**

Write `/home/dfallman/dev/texttv/src/cli.rs`:
```rust
// Filled in by Task 2.
```
Write `/home/dfallman/dev/texttv/src/fetch.rs`:
```rust
// Filled in by Task 4.
```
Write `/home/dfallman/dev/texttv/src/parse.rs`:
```rust
// Filled in by Task 3.
```
Write `/home/dfallman/dev/texttv/src/render.rs`:
```rust
// Filled in by Task 5.
```

- [ ] **Step 5: Replace `src/main.rs` with a trivial entry point that compiles**

Write `/home/dfallman/dev/texttv/src/main.rs`:
```rust
fn main() -> anyhow::Result<()> {
    Ok(())
}
```

- [ ] **Step 6: Verify the crate builds clean**

Run: `cargo build`
Expected: success, no warnings. If a dependency version is unavailable on crates.io, log the resolved version that `cargo` selected and proceed — the spec's versions are minimums.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/ .gitignore
git commit -m "feat: scaffold texttv binary crate"
```

---

## Task 2: CLI parsing with validation

**Files:**
- Modify: `/home/dfallman/dev/texttv/src/cli.rs`
- Create: `/home/dfallman/dev/texttv/src/cli_tests.rs` (inline `#[cfg(test)] mod tests` is also fine — pick one and stay consistent)

- [ ] **Step 1: Write the failing tests for CLI parsing**

Append at the bottom of `src/cli.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_valid_page() {
        let args = Args::try_parse_from(["texttv", "300"]).expect("should parse");
        assert_eq!(args.page, Some(300));
        assert_eq!(args.mode, Mode::Auto);
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
        assert_eq!(args.mode, Mode::Blocks);
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
        // clap's "missing required" error class
        assert!(err.to_string().to_lowercase().contains("required") || err.to_string().contains("PAGE"));
    }
}
```

The `unwrap`/`expect` here are inside `#[cfg(test)]` — that's fine; the clippy lints from Task 1 are intended for production code paths. If the lints fire under `cargo test`, allow them at the test module: `#![allow(clippy::unwrap_used, clippy::expect_used)]`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib cli::tests -- --nocapture`
Expected: compile error — `Args`, `Mode` not defined.

- [ ] **Step 3: Implement the `Args` struct and `Mode` enum**

Replace `src/cli.rs` with:
```rust
use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// Let viuer pick the best protocol available.
    Auto,
    /// Force Kitty graphics protocol.
    Kitty,
    /// Force iTerm2 inline-image protocol.
    Iterm,
    /// Force Unicode half-block fallback.
    Blocks,
    /// Skip image rendering; print extracted plain text.
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Source {
    /// Default: scrape svt.se/text-tv/<PAGE>.
    Svt,
    /// Use the third-party JSON proxy at api.texttv.nu.
    TexttvNu,
}

#[derive(Debug, Parser)]
#[command(name = "texttv", version, about = "Render SVT Text-TV pages in the terminal")]
pub struct Args {
    /// Page number in 100..=999. Omit only with --list.
    #[arg(value_parser = parse_page, required_unless_present = "list")]
    pub page: Option<u16>,

    /// Rendering mode.
    #[arg(long, value_enum, default_value_t = Mode::Auto)]
    pub mode: Mode,

    /// Data source.
    #[arg(long, value_enum, default_value_t = Source::Svt)]
    pub source: Source,

    /// Disable ANSI color in text mode.
    #[arg(long)]
    pub no_color: bool,

    /// Print the well-known section index and exit.
    #[arg(long)]
    pub list: bool,

    /// Print the detected rendering protocol to stderr before drawing.
    #[arg(long)]
    pub debug_protocol: bool,
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
```

Then keep the `#[cfg(test)] mod tests { … }` block from Step 1 at the bottom.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib cli`
Expected: all 6 tests pass.

- [ ] **Step 5: Add the section-index table for `--list`**

Append to `src/cli.rs`:
```rust
/// The well-known SVT Text-TV section index. Hand-curated; SVT does not expose this as data.
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
```

- [ ] **Step 6: Add a test for `print_sections`**

Append to the `#[cfg(test)] mod tests` block:
```rust
#[test]
fn print_sections_writes_known_entries() {
    let mut buf = Vec::new();
    print_sections(&mut buf).expect("write");
    let out = String::from_utf8(buf).expect("utf8");
    assert!(out.contains("100  Innehåll"));
    assert!(out.contains("300  Sport"));
}
```

- [ ] **Step 7: Run all CLI tests**

Run: `cargo test --lib cli`
Expected: 7 tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/cli.rs
git commit -m "feat(cli): parse args, validate page range, --list table"
```

---

## Task 3: Fixture and parser

**Files:**
- Create: `/home/dfallman/dev/texttv/tests/fixtures/page-300.html`
- Create: `/home/dfallman/dev/texttv/tests/fixtures/page-empty.html`
- Modify: `/home/dfallman/dev/texttv/src/parse.rs`
- Create: `/home/dfallman/dev/texttv/tests/parse.rs`

- [ ] **Step 1: Capture a real page as a fixture**

Run:
```bash
curl --silent --show-error --fail \
  -H 'User-Agent: texttv/0.1 (+https://github.com/example/texttv)' \
  'https://www.svt.se/text-tv/300' \
  -o tests/fixtures/page-300.html
```
Expected: file exists, ≥ 50 KB (it contains base64 GIFs).
Verify: `grep -c 'data:image/gif;base64' tests/fixtures/page-300.html` → at least 1.

- [ ] **Step 2: Create the "empty" fixture**

Write `tests/fixtures/page-empty.html`:
```html
<!doctype html><html><head><title>Sida saknas</title></head>
<body><main class="page"><p>Sidan finns inte.</p></main></body></html>
```

- [ ] **Step 3: Write failing tests in `tests/parse.rs`**

```rust
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
        assert!(img.width() > 0 && img.height() > 0, "subpage {i} has zero dims");
    }
}

#[test]
fn extracts_non_empty_swedish_text() {
    let page = extract_page(PAGE_300, 300).expect("parse");
    assert!(page.text.len() > 50, "text body too short: {:?}", page.text);
    // Some Swedish character or word almost certainly present on the sport page.
    let lower = page.text.to_lowercase();
    assert!(
        lower.contains("å") || lower.contains("ä") || lower.contains("ö")
            || lower.contains("sport") || lower.contains("svt"),
        "no Swedish/sport marker in extracted text"
    );
}

#[test]
fn empty_page_is_an_error() {
    let err = extract_page(PAGE_EMPTY, 404).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.to_lowercase().contains("not available") || msg.to_lowercase().contains("no subpage"),
        "unexpected error: {msg}");
}
```

- [ ] **Step 4: Run tests to confirm they fail**

Run: `cargo test --test parse`
Expected: compile error — `texttv::parse::extract_page` not defined.

- [ ] **Step 5: Implement `extract_page`**

Replace `src/parse.rs` with:
```rust
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

    Ok(Page { page_no, images, text })
}

fn decode_data_uri(src: &str) -> Result<DynamicImage> {
    let prefix = "data:image/gif;base64,";
    let payload = src
        .strip_prefix(prefix)
        .ok_or_else(|| anyhow!("img src does not start with {prefix}"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .context("base64 decode failed")?;
    image::load_from_memory_with_format(&bytes, ImageFormat::Gif)
        .context("gif decode failed")
}

fn extract_text(doc: &Html) -> String {
    // SVT wraps the page content in <main>. If that selector ever changes, fall back to <body>.
    let main_sel = Selector::parse("main").expect("static selector");
    let body_sel = Selector::parse("body").expect("static selector");
    let root = doc.select(&main_sel).next().or_else(|| doc.select(&body_sel).next());
    let Some(root) = root else { return String::new(); };

    let mut out = String::new();
    for node in root.text() {
        let trimmed = node.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(trimmed);
    }
    out
}
```

Note: this file uses `.expect("static selector")` on hand-written selectors that cannot fail at runtime. Allow it at module scope: add `#![allow(clippy::expect_used)]` at the top of `src/parse.rs` and document why in a one-line comment, OR rewrite to `.unwrap_or_else(|e| panic!(…))` — same outcome. The clippy lint should not block reading static-string selectors that are part of the program's structure, only fallible input. Pick one approach.

- [ ] **Step 6: Run tests to confirm they pass**

Run: `cargo test --test parse`
Expected: 4 tests pass.

If the "text" test fails because SVT changed its layout and `<main>` no longer wraps the content, inspect `tests/fixtures/page-300.html` to find the correct selector (e.g. `[class*='TextTv']`, `article`, `.svt-text-tv`) and update `extract_text`. Update the fixture comment accordingly.

- [ ] **Step 7: Commit**

```bash
git add tests/fixtures/ tests/parse.rs src/parse.rs
git commit -m "feat(parse): extract subpage GIFs and body text from SVT HTML"
```

---

## Task 4: HTTP fetcher

**Files:**
- Modify: `/home/dfallman/dev/texttv/src/fetch.rs`

- [ ] **Step 1: Implement `fetch_html`**

Replace `src/fetch.rs` with:
```rust
use anyhow::{Context, Result, anyhow};
use std::time::Duration;

const USER_AGENT: &str = concat!(
    "texttv/", env!("CARGO_PKG_VERSION"),
    " (+https://github.com/example/texttv)"
);

pub fn fetch_html(page: u16) -> Result<String> {
    let url = format!("https://www.svt.se/text-tv/{page}");
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(10))
        .timeout_write(Duration::from_secs(10))
        .user_agent(USER_AGENT)
        .build();

    match agent.get(&url).call() {
        Ok(resp) => resp.into_string().context("read response body"),
        Err(ureq::Error::Status(code, resp)) => {
            let status_text = resp.status_text().to_string();
            Err(anyhow!("HTTP {code} {status_text} for {url}"))
        }
        Err(ureq::Error::Transport(t)) => Err(anyhow!("network error: {t}")),
    }
}

/// Optional fallback source: api.texttv.nu returns JSON with the raw text body
/// but no GIFs. Used only behind --source texttv-nu.
pub fn fetch_html_texttv_nu(page: u16) -> Result<String> {
    let url = format!("https://api.texttv.nu/api/get/{page}?app=texttv-rs");
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(10))
        .user_agent(USER_AGENT)
        .build();
    match agent.get(&url).call() {
        Ok(resp) => resp.into_string().context("read response body"),
        Err(ureq::Error::Status(code, resp)) => Err(anyhow!(
            "HTTP {code} {} for {url}",
            resp.status_text()
        )),
        Err(ureq::Error::Transport(t)) => Err(anyhow!("network error: {t}")),
    }
}
```

No automated tests for the network path — the fixture-based parser tests cover the format, and a live test would be flaky in CI. The acceptance pass in Task 9 exercises real fetches end-to-end.

- [ ] **Step 2: Verify compile**

Run: `cargo build`
Expected: success, no warnings.

If `ureq::AgentBuilder` API doesn't match (the API stabilized differently across 2.x versions), inspect `cargo doc --open -p ureq` or `cargo tree -p ureq` for the resolved version and adjust to its `Agent::config_builder()` form. Keep the same observable behavior: 10s read timeout, set UA, follow 3xx.

- [ ] **Step 3: Commit**

```bash
git add src/fetch.rs
git commit -m "feat(fetch): blocking HTTP with UA and timeouts"
```

---

## Task 5: Image rendering via viuer

**Files:**
- Modify: `/home/dfallman/dev/texttv/src/render.rs`

- [ ] **Step 1: Implement render module**

Replace `src/render.rs` with:
```rust
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
    pub debug_protocol: bool,
}

pub fn render_images(images: &[DynamicImage], opts: RenderOptions) -> Result<DetectedProtocol> {
    use crate::cli::Mode;

    let (cfg, protocol) = match opts.mode {
        Mode::Auto => {
            let p = detect_protocol();
            (config_for(p), p)
        }
        Mode::Kitty => (force_kitty_config(), DetectedProtocol::Kitty),
        Mode::Iterm => (force_iterm_config(), DetectedProtocol::Iterm),
        Mode::Blocks => (force_blocks_config(), DetectedProtocol::Halfblocks),
        Mode::Text => unreachable!("Mode::Text is handled before render_images"),
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

    let mut stdout = std::io::stdout().lock();
    for (i, img) in images.iter().enumerate() {
        if i > 0 {
            writeln!(stdout)?;
        }
        viuer::print(img, &cfg).context("viuer failed to print image")?;
    }
    writeln!(stdout)?;
    Ok(protocol)
}

fn config_for(p: DetectedProtocol) -> viuer::Config {
    match p {
        DetectedProtocol::Kitty | DetectedProtocol::Iterm => viuer::Config {
            absolute_offset: false,
            // Native protocols: let the image render at natural size.
            ..Default::default()
        },
        DetectedProtocol::Halfblocks => {
            let cols = terminal_cols().unwrap_or(80);
            viuer::Config {
                absolute_offset: false,
                width: Some(cols as u32),
                use_kitty: false,
                use_iterm: false,
                ..Default::default()
            }
        }
    }
}

fn force_kitty_config() -> viuer::Config {
    viuer::Config { absolute_offset: false, use_kitty: true, use_iterm: false, ..Default::default() }
}

fn force_iterm_config() -> viuer::Config {
    viuer::Config { absolute_offset: false, use_kitty: false, use_iterm: true, ..Default::default() }
}

fn force_blocks_config() -> viuer::Config {
    let cols = terminal_cols().unwrap_or(80);
    viuer::Config {
        absolute_offset: false,
        width: Some(cols as u32),
        use_kitty: false,
        use_iterm: false,
        ..Default::default()
    }
}

fn detect_protocol() -> DetectedProtocol {
    // viuer exposes KittySupport/iTerm checks. Reproduce the same precedence here so
    // we can report it via --debug-protocol.
    if viuer::KittySupport::Local == viuer::get_kitty_support()
        || viuer::KittySupport::LocalTmux == viuer::get_kitty_support()
    {
        DetectedProtocol::Kitty
    } else if viuer::is_iterm_supported() {
        DetectedProtocol::Iterm
    } else {
        DetectedProtocol::Halfblocks
    }
}

fn terminal_cols() -> Option<u16> {
    viuer::terminal_size().map(|(c, _)| c).ok().or_else(|| {
        // Fallback if viuer can't probe (e.g. piped stdout). Use crossterm via env COLUMNS.
        std::env::var("COLUMNS").ok().and_then(|s| s.parse().ok())
    })
}

pub fn render_text(text: &str, color: bool, out: &mut dyn Write) -> Result<()> {
    if !color {
        writeln!(out, "{text}")?;
        return Ok(());
    }
    // Lightweight ANSI colorization: bold-yellow on lines that look like headings
    // (uppercase + short), default fg elsewhere. Avoid a full theme — the heuristic
    // is good enough for the text fallback.
    for line in text.lines() {
        let is_heading = !line.is_empty()
            && line.len() <= 40
            && line.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase());
        if is_heading {
            writeln!(out, "\x1b[1;33m{line}\x1b[0m")?;
        } else {
            writeln!(out, "{line}")?;
        }
    }
    Ok(())
}

/// True if stdout is a TTY. Used by main.rs to decide whether to auto-degrade to text mode.
pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}
```

The `viuer` API surface varies slightly across patch releases. If `KittySupport`, `get_kitty_support`, or `is_iterm_supported` don't exist under those exact names in the resolved 0.9.x, run `cargo doc -p viuer --open` and adjust to the actual symbol. The contract to preserve: a function that returns one of the three protocols based on env detection.

- [ ] **Step 2: Verify compile**

Run: `cargo build`
Expected: success, no warnings.

- [ ] **Step 3: Commit**

```bash
git add src/render.rs
git commit -m "feat(render): viuer-backed image rendering and text colorizer"
```

---

## Task 6: Wire main.rs

**Files:**
- Modify: `/home/dfallman/dev/texttv/src/main.rs`

- [ ] **Step 1: Implement the entry point**

Replace `src/main.rs` with:
```rust
use anyhow::Result;
use clap::Parser;
use std::io::Write;
use std::process::ExitCode;

use texttv::cli::{Args, Mode, Source, print_sections};
use texttv::fetch;
use texttv::parse::extract_page;
use texttv::render::{RenderOptions, render_images, render_text, stdout_is_tty};

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok(()) => ExitCode::from(0),
        Err(AppError::User(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::from(1)
        }
        Err(AppError::Runtime(e)) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

#[derive(Debug)]
enum AppError {
    User(String),
    Runtime(anyhow::Error),
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        Self::Runtime(e)
    }
}

fn run(args: Args) -> Result<(), AppError> {
    if args.list {
        let mut out = std::io::stdout().lock();
        print_sections(&mut out).map_err(anyhow::Error::from)?;
        return Ok(());
    }

    let page = args
        .page
        .ok_or_else(|| AppError::User("PAGE is required".into()))?;

    let html = match args.source {
        Source::Svt => fetch::fetch_html(page),
        Source::TexttvNu => fetch::fetch_html_texttv_nu(page),
    }
    .map_err(AppError::Runtime)?;

    let page_data = extract_page(&html, page).map_err(AppError::Runtime)?;

    // Auto-degrade: if stdout is piped or NO_COLOR=1, force --mode text unless the
    // user explicitly chose a graphics mode.
    let no_color = args.no_color || std::env::var_os("NO_COLOR").is_some();
    let piped = !stdout_is_tty();

    let effective_mode = if piped && matches!(args.mode, Mode::Auto) {
        Mode::Text
    } else {
        args.mode
    };

    match effective_mode {
        Mode::Text => {
            let mut out = std::io::stdout().lock();
            render_text(&page_data.text, !no_color, &mut out).map_err(AppError::Runtime)?;
        }
        _ => {
            let opts = RenderOptions {
                mode: effective_mode,
                debug_protocol: args.debug_protocol,
            };
            render_images(&page_data.images, opts).map_err(AppError::Runtime)?;
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Verify compile, all warnings clean**

Run: `cargo build --release 2>&1 | tee /tmp/texttv-build.log`
Expected: success. If any warnings appear, fix them — `acceptance §7.1` requires zero warnings.

- [ ] **Step 3: Run unit tests + parser fixture tests**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 4: Smoke-test validation errors**

Run (in the dev shell — these don't need network):
```bash
./target/release/texttv 42 2>&1 | head -1
./target/release/texttv 1000 2>&1 | head -1
./target/release/texttv abc 2>&1 | head -1
```
Expected: each prints a "PAGE must be 100..=999" or "is not a number" message and exits with code 1. Verify with `echo $?` after each.

- [ ] **Step 5: Live fetch smoke test (text mode)**

Run: `./target/release/texttv 300 --mode text | head -20`
Expected: ~20 lines of readable Swedish text from page 300, no mojibake. If this fails with HTTP or parse errors, debug here before continuing.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat(main): wire fetch/parse/render with exit codes and auto-degrade"
```

---

## Task 7: Acceptance pass — per-terminal verification

This task is manual. Do it in this order; do not skip terminals you don't have access to — install them via OrbStack / your platform's package manager. If a terminal is unavailable, leave that checkbox unchecked and document why in the README.

**Files:** none modified. Output recorded in the commit message of Task 8.

- [ ] **Step 1: Kitty**

In Kitty: `texttv 300 --debug-protocol 2>/dev/null`
Then: `texttv 300 --debug-protocol >/dev/null`
Expected: stderr prints `detected: kitty`, image renders at native size.

- [ ] **Step 2: Ghostty**

Same as Step 1 inside Ghostty.
Expected: `detected: kitty`.

- [ ] **Step 3: WezTerm**

Same inside WezTerm.
Expected: `detected: iterm`.

- [ ] **Step 4: iTerm2** (macOS only)

Same inside iTerm2.
Expected: `detected: iterm`.

- [ ] **Step 5: Apple Terminal**

`texttv 300 --mode blocks --debug-protocol`
Expected: `detected: halfblocks`, recognizable blocky rendering.

- [ ] **Step 6: Pipe degradation**

`texttv 300 | head -5`
Expected: 5 lines of text body (no escape sequences).

- [ ] **Step 7: NO_COLOR**

`NO_COLOR=1 texttv 300 --mode text | head -5`
Expected: plain text, no `\x1b[` sequences in output. Verify with `NO_COLOR=1 texttv 300 --mode text | cat -v | head -5`.

- [ ] **Step 8: Rare/empty page handling**

Pick a page known to be sparsely used. Try 999, 998, 042.
Expected: 042 → exit 1 with validation error; 999 → either renders, or exit 2 with "page not available". No panics.

- [ ] **Step 9: Tmux hint**

Inside tmux without `allow-passthrough`: `texttv 300 --debug-protocol`
Expected: if protocol falls back to `halfblocks`, the tmux hint appears on stderr.

---

## Task 8: README and final commit

**Files:**
- Create: `/home/dfallman/dev/texttv/README.md`

- [ ] **Step 1: Write the README**

```markdown
# texttv

Render SVT Text-TV pages in your terminal.

```bash
texttv 300            # Sport — auto-detect best protocol
texttv 100 --mode text # Plain text, no graphics
texttv --list          # Well-known section pages
```

## Install

```bash
cargo install --path .
```

Requires a Rust 1.85+ toolchain (edition 2024).

## Terminal compatibility

| Terminal       | Protocol used         | Status              |
| -------------- | --------------------- | ------------------- |
| Kitty          | Kitty graphics        | ✅ first-class      |
| Ghostty        | Kitty graphics        | ✅ first-class      |
| WezTerm        | iTerm2 inline image   | ✅ first-class      |
| iTerm2         | iTerm2 inline image   | ✅ first-class      |
| Apple Terminal | Unicode half-blocks   | ✅ fallback         |
| Alacritty      | Unicode half-blocks   | ✅ fallback         |
| Windows Term.  | Unicode half-blocks   | ✅ fallback         |
| foot / mlterm  | Unicode half-blocks   | ⚠️ Sixel-only terminals fall back to blocks (see roadmap) |

## Flags

- `--mode {auto,kitty,iterm,blocks,text}` — force a rendering path. Defaults to `auto`.
- `--no-color` — disable ANSI color in text mode.
- `--list` — print the section index.
- `--debug-protocol` — print which protocol was detected, on stderr.
- `--source {svt,texttv-nu}` — pick the data source. Defaults to the official SVT site.

## tmux caveat

Inside tmux, Kitty's graphics protocol requires passthrough. Add to `~/.tmux.conf`:

```
set -g allow-passthrough on
set -g default-terminal "tmux-256color"
```

If `texttv --debug-protocol` says `halfblocks` inside tmux but the outer terminal is Kitty/Ghostty/WezTerm, this is almost always the cause.

## Exit codes

- `0` — success
- `1` — bad arguments / page out of range
- `2` — network or parse error / page not available

## License

MIT or Apache-2.0, at your option.
```

- [ ] **Step 2: Verify everything still builds and tests pass**

Run: `cargo build --release && cargo test`
Expected: both clean.

- [ ] **Step 3: Final commit**

```bash
git add README.md
git commit -m "docs: README with terminal matrix and tmux caveat"
```

---

## Self-Review notes

- §7.1 (clean build): covered by Task 1 lints config + Task 6 Step 2.
- §7.2 (per-terminal verification): Task 7 steps 1-4 plus the `--debug-protocol` flag implemented in Task 5/6.
- §7.3 (Apple Terminal blocks): Task 7 step 5.
- §7.4 (text mode, Swedish): Task 6 step 5 + Task 3 step 5.
- §7.5 (validation errors): Task 6 step 4.
- §7.6 (empty page → exit 2, no panic): Task 3's `extract_page` returns Err for the empty case; Task 6 maps it to exit 2. Task 7 step 8 exercises it.
- §7.7 (pipe auto-degrade): Task 6 step 1 logic + Task 7 step 6.
- §7.8 (no unwrap/expect on fallible inputs): Cargo.toml clippy lints from Task 1. The two `.expect("static selector")` in `parse.rs` are on infallible compile-time inputs and are explicitly allowed.
- §7.9 (README): Task 8.

No placeholders. Types are consistent: `Page { page_no, images, text }` used in Tasks 3 and 6; `Mode` enum used in Tasks 2, 5, 6; `DetectedProtocol` introduced in Task 5 and surfaced in `--debug-protocol`.
