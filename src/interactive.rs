//! In-terminal interactive page browser. See
//! `docs/superpowers/specs/2026-05-15-interactive-mode-design.md`.

use std::io::{IsTerminal, Write, stdout};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use crossterm::{
    QueueableCommand,
    cursor::MoveTo,
    event::{Event, KeyCode, KeyEvent, KeyEventKind, poll, read},
    execute,
    style::Print,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use owo_colors::OwoColorize;

use crate::parse::{Cell, ColoredPage, Line, TtColor};

pub(crate) const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
/// Event-loop poll timeout (also the spinner cadence).
pub(crate) const SPINNER_INTERVAL: Duration = Duration::from_millis(80);

/// Background-fetch state. `Idle` between loads, `Fetching` while a worker
/// thread is running.
#[derive(Debug)]
pub enum FetchState {
    Idle,
    Fetching { target_page: u16, frame: usize },
}

#[derive(Debug)]
pub struct State {
    pub current_page: u16,
    pub input_buf: String,
    pub lines: Vec<Line>,
    pub links: Vec<Link>,
    pub selected: Option<usize>,
    pub fetch: FetchState,
    /// Channel where the worker thread sends its `Result<ColoredPage>`.
    pub pending_rx: Option<Receiver<anyhow::Result<ColoredPage>>>,
    /// One-shot bottom-bar message. Cleared by the next keystroke or load.
    pub status: Option<String>,
}

impl State {
    /// Build an initial state pointed at `page` with no rendered content
    /// yet. Caller is expected to immediately start a fetch for `page`.
    pub fn initial(page: u16) -> Self {
        Self {
            current_page: page,
            input_buf: String::new(),
            lines: Vec::new(),
            links: Vec::new(),
            selected: None,
            fetch: FetchState::Idle,
            pending_rx: None,
            status: None,
        }
    }

    /// Install a freshly-parsed page: replace `lines`, rescan links, reset
    /// selection to the first link (if any), clear fetch state + buffer.
    pub fn install_page(&mut self, page: ColoredPage) {
        self.current_page = page.page_no;
        self.lines = page.lines;
        self.links = scan_links(&self.lines);
        self.selected = if self.links.is_empty() { None } else { Some(0) };
        self.fetch = FetchState::Idle;
        self.pending_rx = None;
        self.input_buf.clear();
        self.status = None;
    }

    /// Install a "page doesn't exist" placeholder. Used when the worker
    /// thread reports a failed load — we still commit `current_page` to
    /// the target so neighbouring-page navigation works.
    pub fn install_placeholder(&mut self, page: u16) {
        self.current_page = page;
        self.lines = placeholder_lines();
        self.links = Vec::new();
        self.selected = None;
        self.fetch = FetchState::Idle;
        self.pending_rx = None;
        self.input_buf.clear();
        self.status = None;
    }
}

/// Build a 24-row, 40-col page containing `Sidan finns inte` centered.
/// All cells are white-on-black so the render path shows a uniform dark
/// page (matching the chrome rows).
fn placeholder_lines() -> Vec<Line> {
    const PAGE_WIDTH: usize = 40;
    const PAGE_HEIGHT: usize = 24;
    const MSG: &str = "Sidan finns inte";
    let msg_row = (PAGE_HEIGHT - 1) / 2;
    let msg_len = MSG.chars().count();
    let pad_left = (PAGE_WIDTH - msg_len) / 2;
    let pad_right = PAGE_WIDTH - pad_left - msg_len;

    let blank_cell = || Cell {
        text: " ".repeat(PAGE_WIDTH),
        fg: TtColor::White,
        bg: TtColor::Black,
        mosaic_url: None,
    };
    let msg_cell = || Cell {
        text: format!("{:pad_left$}{MSG}{:pad_right$}", "", "",),
        fg: TtColor::White,
        bg: TtColor::Black,
        mosaic_url: None,
    };

    (0..PAGE_HEIGHT)
        .map(|i| Line {
            cells: vec![if i == msg_row {
                msg_cell()
            } else {
                blank_cell()
            }],
            double_height: false,
        })
        .collect()
}

/// What `handle_key` tells the outer loop to do. Keeps `handle_key` pure:
/// no threads, no I/O, no global state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    /// Caller should kick off a fetch for the given page (already in 100..=999).
    StartFetch(u16),
    /// Caller should exit the event loop.
    Quit,
}

/// Apply a key event to the state and return an action for the caller.
/// Pure: never touches I/O, never spawns threads.
pub fn handle_key(state: &mut State, ev: KeyEvent) -> Action {
    let was_fetching = matches!(state.fetch, FetchState::Fetching { .. });

    match ev.code {
        KeyCode::Esc => Action::Quit,
        _ if was_fetching => {
            // While a fetch is in flight, ignore everything except Esc.
            Action::None
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            state.status = None;
            state.input_buf.push(c);
            if state.input_buf.len() == 3 {
                let parsed: u16 = state.input_buf.parse().unwrap_or(0);
                state.input_buf.clear();
                if (100..=999).contains(&parsed) {
                    Action::StartFetch(parsed)
                } else {
                    state.status = Some(format!(
                        "Error: page must be in 100..=999 (got {parsed:03})"
                    ));
                    Action::None
                }
            } else {
                Action::None
            }
        }
        KeyCode::Backspace => {
            state.input_buf.pop();
            Action::None
        }
        KeyCode::Up => {
            if let Some(sel) = state.selected {
                state.selected = Some(sel.saturating_sub(1));
            }
            Action::None
        }
        KeyCode::Down => {
            if let Some(sel) = state.selected
                && sel + 1 < state.links.len()
            {
                state.selected = Some(sel + 1);
            }
            Action::None
        }
        KeyCode::Left => {
            if state.current_page > 100 {
                state.input_buf.clear();
                Action::StartFetch(state.current_page - 1)
            } else {
                state.status = Some("Already at first page (100)".into());
                Action::None
            }
        }
        KeyCode::Right => {
            if state.current_page < 999 {
                state.input_buf.clear();
                Action::StartFetch(state.current_page + 1)
            } else {
                state.status = Some("Already at last page (999)".into());
                Action::None
            }
        }
        KeyCode::Enter => {
            if let Some(sel) = state.selected
                && let Some(link) = state.links.get(sel)
            {
                if link.followable {
                    state.input_buf.clear();
                    Action::StartFetch(link.target)
                } else {
                    state.status = Some(format!("Error: page {} not in 100..=999", link.target));
                    Action::None
                }
            } else {
                Action::None
            }
        }
        _ => Action::None,
    }
}

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

/// Scan the rendered page for three-digit page references and emit them
/// as `Link`s. The scanner is intentionally permissive — SVT's pages use
/// several conventions for link decoration and missing real links is
/// worse than catching a few false positives:
///
/// - `" 300 "` — the bare case; surrounding spaces (or line edges) are
///   the canonical boundary.
/// - `" 328f "` — `f` is SVT's multi-page suffix; the link targets page
///   328, and the `f` is included in the highlight extent.
/// - `" 376- "` — trailing dash (often used when a page references the
///   first of a range without an explicit upper bound).
/// - `" 343-344 "` — range; both numbers are detected as independent
///   links to 343 and 344.
/// - `"100.000"` — *not* a link (digits adjacent to a `.`).
///
/// Mosaic cells count as space placeholders, so a mosaic adjacent to a
/// link doesn't disqualify it. Iteration is char-based (not byte-based)
/// so non-ASCII text earlier on the line doesn't shift link columns.
pub fn scan_links(lines: &[Line]) -> Vec<Link> {
    let mut out = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        // Flatten cells into a single char vector. Mosaic cells contribute
        // a single space placeholder so they form a link boundary.
        let mut chars: Vec<char> = Vec::new();
        for cell in &line.cells {
            if cell.is_mosaic() {
                chars.push(' ');
            } else {
                chars.extend(cell.text.chars());
            }
        }
        let mut i = 0;
        while i + 3 <= chars.len() {
            if !(chars[i].is_ascii_digit()
                && chars[i + 1].is_ascii_digit()
                && chars[i + 2].is_ascii_digit())
            {
                i += 1;
                continue;
            }
            let prev_is_digit = i > 0 && chars[i - 1].is_ascii_digit();
            let next_is_digit = i + 3 < chars.len() && chars[i + 3].is_ascii_digit();
            if prev_is_digit || next_is_digit {
                // Part of a longer digit run; skip past all the digits.
                let mut j = i;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                i = j;
                continue;
            }
            // Boundary characters. Left side: space, dash, or line edge.
            // Right side: space, dash, 'f' (multi-page indicator), or
            // line edge. Anything else (e.g. '.', ',') disqualifies the
            // run so `100.000` doesn't become a link.
            let left_ok = i == 0 || matches!(chars[i - 1], ' ' | '-');
            let right_ok = i + 3 == chars.len() || matches!(chars[i + 3], ' ' | '-' | 'f');
            if !(left_ok && right_ok) {
                i += 1;
                continue;
            }
            // The target is the 3-digit number; 'f' is decoration.
            let target_str: String = chars[i..i + 3].iter().collect();
            let target: u16 = target_str.parse().unwrap_or(0);

            // Visual extent: the 3 digits, plus an optional 'f' suffix,
            // plus up to one cell of flanking space/dash on each side.
            // Keeps highlight extents consistent (typically 4–6 cells).
            let mut core_end = i + 3;
            if core_end < chars.len() && chars[core_end] == 'f' {
                core_end += 1;
            }
            let left_pad = if i > 0 && matches!(chars[i - 1], ' ' | '-') {
                1
            } else {
                0
            };
            let right_pad = if core_end < chars.len() && matches!(chars[core_end], ' ' | '-') {
                1
            } else {
                0
            };
            let col_start = (i - left_pad) as u16;
            let col_len = (core_end + right_pad - i + left_pad) as u16;

            out.push(Link {
                row: row as u16,
                col_start,
                col_len,
                target,
                followable: target >= 100,
            });
            i = core_end;
        }
    }
    out
}

/// Advance the spinner frame by one (wrapping at `SPINNER.len()`) when
/// a fetch is in flight. No-op otherwise. Called by the event loop
/// every `SPINNER_INTERVAL` (80 ms).
pub fn tick(state: &mut State) {
    if let FetchState::Fetching { frame, .. } = &mut state.fetch {
        *frame = (*frame + 1) % SPINNER.len();
    }
}

/// Full width (in cells) the chrome rows paint with the page's black bg
/// so they read as part of the same visual surface. Matches the teletext
/// page's 40 data cells + 1 right-edge frame cell.
const CHROME_WIDTH: usize = 41;

/// Number of rows reserved for the page body (rows 0..PAGE_HEIGHT_MAX).
/// A standard SVT teletext page is 24 visible rows + 1 status row = 25.
const PAGE_HEIGHT_MAX: u16 = 25;

/// Compose the input-row content. Layout shifted one cell right so col 0
/// is a margin matching the visual breathing room around the page:
///
/// ```text
///   col: 0 1 2 3 4 5
///        _ ⠏ _ 1 0 0    (fetching)
///        _ _ _ 1 0 0    (idle)
/// ```
fn input_row(state: &State) -> String {
    let glyph = match state.fetch {
        FetchState::Fetching { frame, .. } => SPINNER[frame % SPINNER.len()],
        FetchState::Idle => ' ',
    };
    let digits: String = if state.input_buf.is_empty() {
        format!("{:03}", state.current_page)
    } else {
        let mut s = state.input_buf.clone();
        while s.len() < 3 {
            s.push('_');
        }
        s
    };
    format!(" {glyph} {digits}")
}

/// Pad `text` to `width` cells by centering it, with the surplus split
/// roughly evenly between left and right (left gets the lesser half on
/// odd remainders). Used for the hint bar.
fn center_padded(text: &str, width: usize) -> String {
    let visible: String = text.chars().take(width).collect();
    let vlen = visible.chars().count();
    if vlen >= width {
        return visible;
    }
    let total_pad = width - vlen;
    let left = total_pad / 2;
    let right = total_pad - left;
    format!("{:left$}{visible}{:right$}", "", "")
}

/// Paint a uniform black background covering the page body before any
/// content is drawn on top. Eliminates the flicker that would otherwise
/// show during the initial fetch (when `state.lines` is still empty) and
/// fills the area between a short page and the hint bar with the same
/// dark surface that `render_colored`'s cells emit.
fn paint_black_canvas<W: Write>(out: &mut W) -> anyhow::Result<()> {
    let row: String = " ".repeat(CHROME_WIDTH);
    let styled = row.on_truecolor(0, 0, 0).to_string();
    for row in 0..PAGE_HEIGHT_MAX {
        out.queue(MoveTo(0, row))?;
        out.queue(Print(&styled))?;
    }
    Ok(())
}

/// Full redraw of the interactive screen. `out` is typically stdout in
/// raw mode wrapped in a BufWriter; tests pass a `Vec<u8>`.
pub fn draw<W: Write>(state: &State, out: &mut W) -> anyhow::Result<()> {
    // Black canvas under everything. Short pages and initial-load
    // (no-page-yet) states then have a uniform dark surface instead of
    // terminal-default bg flickering through.
    paint_black_canvas(out)?;

    // Page body starts at row 0. The page's own top row carries the page
    // number — we overwrite the leftmost cells with our input overlay so
    // the page-number area doubles as the editor.
    for (i, line) in state.lines.iter().enumerate() {
        out.queue(MoveTo(0, i as u16))?;
        let slice = std::slice::from_ref(line);
        crate::render::render_colored(slice, true, out)?;
    }

    // Selected-link highlight: reverse-video overlay. Row is 0-based
    // within the page body, which now matches absolute terminal rows
    // since the page starts at row 0.
    if let Some(sel) = state.selected
        && let Some(link) = state.links.get(sel)
    {
        let row_idx = link.row as usize;
        if let Some(line) = state.lines.get(row_idx) {
            out.queue(MoveTo(link.col_start, link.row))?;
            let visible = visible_chars_at(line, link.col_start, link.col_len);
            out.queue(Print("\x1b[7m"))?;
            out.queue(Print(visible))?;
            out.queue(Print("\x1b[27m"))?;
        }
    }

    // Input overlay on row 0, cols 0..6. White-on-black so it visually
    // replaces the page's existing page-number cells with our editor.
    out.queue(MoveTo(0, 0))?;
    let input = input_row(state);
    let styled = input
        .truecolor(255, 255, 255)
        .on_truecolor(0, 0, 0)
        .to_string();
    out.queue(Print(styled))?;

    // Hint at row `PAGE_HEIGHT_MAX` (just below the page area), centered
    // within `CHROME_WIDTH`. Sits on the black canvas so short pages get
    // a clean dark band between the page content and the hint.
    out.queue(MoveTo(0, PAGE_HEIGHT_MAX))?;
    let hint_text = state.status.as_deref().unwrap_or("↑↓ · Enter · Esc quit");
    let centered = center_padded(hint_text, CHROME_WIDTH);
    let styled = centered
        .truecolor(255, 255, 255)
        .on_truecolor(0, 0, 0)
        .to_string();
    out.queue(Print(styled))?;

    // Park the system cursor at the next typing position in the input
    // zone. No Hide/Show pair — the terminal's native cursor stays visible
    // throughout.
    let cursor_col = 3 + (state.input_buf.len() as u16);
    out.queue(MoveTo(cursor_col, 0))?;
    out.flush()?;
    Ok(())
}

/// Walk `line`'s cells character by character and return the substring
/// covering columns `[col_start, col_start + col_len)`. Mosaic cells
/// contribute a single space (matching `scan_links`).
fn visible_chars_at(line: &Line, col_start: u16, col_len: u16) -> String {
    let start = col_start as usize;
    let end = start + col_len as usize;
    let mut out = String::new();
    let mut col = 0usize;
    for cell in &line.cells {
        if col >= end {
            break;
        }
        let text = if cell.is_mosaic() {
            " ".to_string()
        } else {
            cell.text.clone()
        };
        for ch in text.chars() {
            if col >= start && col < end {
                out.push(ch);
            }
            col += 1;
            if col >= end {
                break;
            }
        }
    }
    out
}

/// Entry point for interactive mode. Sets up raw mode + the alt screen,
/// renders the initial page, and runs the event loop until the user
/// presses Esc.
pub fn run(initial_page: u16) -> Result<()> {
    // Refuse if stdout isn't a terminal — interactive emits escape codes
    // that don't belong in pipes or files.
    if !stdout().is_terminal() {
        return Err(anyhow!("--interactive requires a terminal"));
    }

    // The layout needs 41 cols (40-cell page + right-edge frame) and
    // 26 rows (25-row page area + 1 hint row). The input field overlays
    // the page's own top row, so there's no dedicated chrome row above.
    let (cols, rows) = crossterm::terminal::size().context("reading terminal size")?;
    if cols < 41 || rows < 26 {
        return Err(anyhow!(
            "terminal too small ({cols}x{rows}); need at least 41x26"
        ));
    }

    install_panic_hook();

    let mut stdout = stdout();
    enable_raw_mode().context("entering raw mode")?;
    execute!(stdout, EnterAlternateScreen).context("entering alt screen")?;

    let result = run_inner(initial_page, &mut stdout);

    // Always restore terminal state, even on error.
    let _ = execute!(stdout, LeaveAlternateScreen);
    let _ = disable_raw_mode();

    result
}

/// Restore the terminal before the default panic handler runs. Without
/// this, a panic mid-render leaves the user in a terminal with raw mode
/// on and the alt-screen active.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut stdout = stdout();
        let _ = execute!(stdout, LeaveAlternateScreen);
        let _ = disable_raw_mode();
        original(info);
    }));
}

fn run_inner<W: Write>(initial_page: u16, out: &mut W) -> Result<()> {
    let mut state = State::initial(initial_page);

    // Kick off the initial load on a worker thread; the main loop polls
    // for events on `SPINNER_INTERVAL` and animates the spinner in the
    // meantime.
    start_fetch(&mut state, initial_page);
    draw(&state, out)?;

    loop {
        drain_fetch(&mut state);

        let timed_out = !poll(SPINNER_INTERVAL).context("polling for events")?;

        if timed_out {
            tick(&mut state);
        } else {
            match read().context("reading terminal event")? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match handle_key(&mut state, k) {
                    Action::None => {}
                    Action::Quit => break,
                    Action::StartFetch(page) => start_fetch(&mut state, page),
                },
                Event::Resize(_, _) => {} // just redraw below
                _ => {}
            }
        }

        draw(&state, out)?;
    }
    Ok(())
}

/// Kick off a background fetch. Flips `state.fetch` to `Fetching` and
/// returns immediately. The main loop drains `state.pending_rx` to pick
/// up the result.
fn start_fetch(state: &mut State, page: u16) {
    state.fetch = FetchState::Fetching {
        target_page: page,
        frame: 0,
    };
    state.status = None;
    let (tx, rx) = channel::<anyhow::Result<ColoredPage>>();
    state.pending_rx = Some(rx);
    thread::spawn(move || {
        let result = crate::fetch::fetch_texttv_nu(page)
            .and_then(|json| crate::parse::parse_texttv_nu(&json, page))
            .inspect(|cp| {
                crate::mosaic::prefetch_page(cp);
            });
        let _ = tx.send(result);
    });
}

/// Drain a completed fetch result if one is waiting. Updates state in
/// place.
fn drain_fetch(state: &mut State) {
    let Some(rx) = state.pending_rx.as_ref() else {
        return;
    };
    match rx.try_recv() {
        Ok(Ok(cp)) => {
            state.install_page(cp);
        }
        Ok(Err(_)) => {
            // The worker reported a load failure. Commit the target page
            // as the new current and render the "Sidan finns inte"
            // placeholder so neighbouring-page navigation continues to
            // work even past holes in SVT's numbering.
            let target = match state.fetch {
                FetchState::Fetching { target_page, .. } => target_page,
                FetchState::Idle => state.current_page,
            };
            state.install_placeholder(target);
        }
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => {
            let target = match state.fetch {
                FetchState::Fetching { target_page, .. } => target_page,
                FetchState::Idle => state.current_page,
            };
            state.install_placeholder(target);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::parse::{Cell, ColoredPage, Line, TtColor};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn page_with_links() -> ColoredPage {
        ColoredPage {
            page_no: 100,
            lines: vec![line(" 300  400 ")],
            plain: String::new(),
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
    fn scan_finds_link_with_f_suffix() {
        let lines = vec![line(" 328f ")];
        let links = scan_links(&lines);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, 328);
        // ' 328f ' → 6 cells; col_start=0; extent covers ' ', '328', 'f', ' '
        assert_eq!(links[0].col_start, 0);
        assert_eq!(links[0].col_len, 6);
        assert!(links[0].followable);
    }

    #[test]
    fn scan_finds_link_with_f_at_line_end() {
        let lines = vec![line(" 328f")];
        let links = scan_links(&lines);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, 328);
        // No trailing space pad → col_len = 5 (' 328f')
        assert_eq!(links[0].col_len, 5);
    }

    #[test]
    fn scan_finds_link_with_trailing_dash() {
        let lines = vec![line(" 376- ")];
        let links = scan_links(&lines);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, 376);
        assert_eq!(links[0].col_start, 0);
        assert_eq!(links[0].col_len, 5);
    }

    #[test]
    fn scan_finds_both_pages_in_range_link() {
        let lines = vec![line(" 343-344 ")];
        let links = scan_links(&lines);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].target, 343);
        assert_eq!(links[1].target, 344);
    }

    #[test]
    fn scan_finds_range_link_after_label() {
        // Real-world example: "Herrallsv 343-344"
        let lines = vec![line("Herrallsv 343-344")];
        let links = scan_links(&lines);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].target, 343);
        assert_eq!(links[1].target, 344);
    }

    #[test]
    fn scan_finds_dash_link_after_label() {
        // Real-world example: "Målservice 376-"
        let lines = vec![line("Målservice 376-")];
        let links = scan_links(&lines);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, 376);
    }

    #[test]
    fn scan_still_rejects_decimal_numbers() {
        // Regression: `.` is not a valid right boundary.
        let lines = vec![line(" 100.000 ")];
        let links = scan_links(&lines);
        assert!(links.is_empty());
    }

    #[test]
    fn scan_links_columns_are_char_positions_not_byte_positions() {
        // Regression: Swedish 'ö' is 2 bytes in UTF-8. A byte-indexed
        // scanner reports col_start = 4 (the leading space byte position)
        // for the link " 300 ", which paints the reverse-video highlight
        // one cell to the right of where the digits actually render.
        let lines = vec![line("Höj 300 hi")];
        let links = scan_links(&lines);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, 300);
        // 'H'=col 0, 'ö'=col 1, 'j'=col 2, ' '=col 3, '3'=col 4 → col_start = 3
        assert_eq!(links[0].col_start, 3);
        assert_eq!(links[0].col_len, 5);
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

    #[test]
    fn typing_three_digits_emits_start_fetch() {
        let mut s = State::initial(100);
        s.install_page(page_with_links());

        assert_eq!(handle_key(&mut s, key(KeyCode::Char('3'))), Action::None);
        assert_eq!(s.input_buf, "3");

        assert_eq!(handle_key(&mut s, key(KeyCode::Char('0'))), Action::None);
        assert_eq!(s.input_buf, "30");

        assert_eq!(
            handle_key(&mut s, key(KeyCode::Char('5'))),
            Action::StartFetch(305)
        );
        assert_eq!(s.input_buf, ""); // cleared so the user can type again
    }

    #[test]
    fn typing_out_of_range_page_sets_status_no_fetch() {
        let mut s = State::initial(100);
        s.install_page(page_with_links());
        handle_key(&mut s, key(KeyCode::Char('0')));
        handle_key(&mut s, key(KeyCode::Char('9')));
        let action = handle_key(&mut s, key(KeyCode::Char('9')));
        assert_eq!(action, Action::None);
        assert!(s.status.as_deref().unwrap_or("").contains("100..=999"));
        assert_eq!(s.input_buf, "");
    }

    #[test]
    fn backspace_pops_input_buf() {
        let mut s = State::initial(100);
        s.install_page(page_with_links());
        handle_key(&mut s, key(KeyCode::Char('3')));
        handle_key(&mut s, key(KeyCode::Char('0')));
        assert_eq!(s.input_buf, "30");
        handle_key(&mut s, key(KeyCode::Backspace));
        assert_eq!(s.input_buf, "3");
        handle_key(&mut s, key(KeyCode::Backspace));
        assert_eq!(s.input_buf, "");
        // Backspace on empty is a no-op.
        let action = handle_key(&mut s, key(KeyCode::Backspace));
        assert_eq!(action, Action::None);
        assert_eq!(s.input_buf, "");
    }

    #[test]
    fn down_arrow_moves_selection_within_bounds() {
        let mut s = State::initial(100);
        s.install_page(page_with_links());
        assert_eq!(s.selected, Some(0));
        handle_key(&mut s, key(KeyCode::Down));
        assert_eq!(s.selected, Some(1));
        // Saturating at last.
        handle_key(&mut s, key(KeyCode::Down));
        assert_eq!(s.selected, Some(1));
    }

    #[test]
    fn up_arrow_moves_selection_within_bounds() {
        let mut s = State::initial(100);
        s.install_page(page_with_links());
        handle_key(&mut s, key(KeyCode::Down));
        assert_eq!(s.selected, Some(1));
        handle_key(&mut s, key(KeyCode::Up));
        assert_eq!(s.selected, Some(0));
        // Saturating at first.
        handle_key(&mut s, key(KeyCode::Up));
        assert_eq!(s.selected, Some(0));
    }

    #[test]
    fn enter_on_followable_link_emits_start_fetch() {
        let mut s = State::initial(100);
        s.install_page(page_with_links());
        let action = handle_key(&mut s, key(KeyCode::Enter));
        assert_eq!(action, Action::StartFetch(300));
    }

    #[test]
    fn enter_on_unfollowable_link_is_noop_with_status() {
        let mut s = State::initial(100);
        s.install_page(ColoredPage {
            page_no: 100,
            lines: vec![line(" 099 ")],
            plain: String::new(),
        });
        let action = handle_key(&mut s, key(KeyCode::Enter));
        assert_eq!(action, Action::None);
        assert!(s.status.is_some());
    }

    #[test]
    fn esc_emits_quit() {
        let mut s = State::initial(100);
        assert_eq!(handle_key(&mut s, key(KeyCode::Esc)), Action::Quit);
    }

    #[test]
    fn digit_during_fetch_is_ignored() {
        let mut s = State::initial(100);
        s.fetch = FetchState::Fetching {
            target_page: 200,
            frame: 0,
        };
        let action = handle_key(&mut s, key(KeyCode::Char('3')));
        assert_eq!(action, Action::None);
        assert_eq!(s.input_buf, "");
    }

    #[test]
    fn enter_during_fetch_is_ignored() {
        let mut s = State::initial(100);
        s.install_page(page_with_links());
        s.fetch = FetchState::Fetching {
            target_page: 200,
            frame: 0,
        };
        let action = handle_key(&mut s, key(KeyCode::Enter));
        assert_eq!(action, Action::None);
    }

    #[test]
    fn left_arrow_starts_fetch_for_previous_page() {
        let mut s = State::initial(305);
        let action = handle_key(&mut s, key(KeyCode::Left));
        assert_eq!(action, Action::StartFetch(304));
    }

    #[test]
    fn left_arrow_at_first_page_is_noop_with_status() {
        let mut s = State::initial(100);
        let action = handle_key(&mut s, key(KeyCode::Left));
        assert_eq!(action, Action::None);
        assert!(s.status.is_some());
    }

    #[test]
    fn right_arrow_starts_fetch_for_next_page() {
        let mut s = State::initial(305);
        let action = handle_key(&mut s, key(KeyCode::Right));
        assert_eq!(action, Action::StartFetch(306));
    }

    #[test]
    fn right_arrow_at_last_page_is_noop_with_status() {
        let mut s = State::initial(999);
        let action = handle_key(&mut s, key(KeyCode::Right));
        assert_eq!(action, Action::None);
        assert!(s.status.is_some());
    }

    #[test]
    fn install_placeholder_sets_current_page_and_renders_message() {
        let mut s = State::initial(100);
        s.install_placeholder(420);
        assert_eq!(s.current_page, 420);
        assert!(s.links.is_empty());
        assert_eq!(s.selected, None);
        let combined: String = s
            .lines
            .iter()
            .flat_map(|l| l.cells.iter().map(|c| c.text.clone()))
            .collect();
        assert!(
            combined.contains("Sidan finns inte"),
            "placeholder text missing: {combined}"
        );
    }

    #[test]
    fn center_padded_centers_text_within_even_width() {
        assert_eq!(center_padded("hi", 6), "  hi  ");
    }

    #[test]
    fn center_padded_uses_extra_on_right_when_odd() {
        // width 4, content 1 char → 3 pad → 1 left + 2 right
        assert_eq!(center_padded("x", 4), " x  ");
    }

    #[test]
    fn center_padded_truncates_when_too_long() {
        assert_eq!(center_padded("abcdef", 3), "abc");
    }

    #[test]
    fn install_placeholder_clears_input_and_fetch_state() {
        let mut s = State::initial(100);
        s.input_buf = "1".to_string();
        s.fetch = FetchState::Fetching {
            target_page: 420,
            frame: 5,
        };
        s.install_placeholder(420);
        assert_eq!(s.input_buf, "");
        assert!(matches!(s.fetch, FetchState::Idle));
    }

    #[test]
    fn tick_advances_frame_while_fetching() {
        let mut s = State::initial(100);
        s.fetch = FetchState::Fetching {
            target_page: 200,
            frame: 0,
        };
        tick(&mut s);
        if let FetchState::Fetching { frame, .. } = s.fetch {
            assert_eq!(frame, 1);
        } else {
            panic!("expected Fetching");
        }
    }

    #[test]
    fn tick_wraps_frame_at_spinner_len() {
        let mut s = State::initial(100);
        s.fetch = FetchState::Fetching {
            target_page: 200,
            frame: SPINNER.len() - 1,
        };
        tick(&mut s);
        if let FetchState::Fetching { frame, .. } = s.fetch {
            assert_eq!(frame, 0);
        } else {
            panic!("expected Fetching");
        }
    }

    #[test]
    fn tick_is_noop_while_idle() {
        let mut s = State::initial(100);
        tick(&mut s);
        assert!(matches!(s.fetch, FetchState::Idle));
    }

    #[test]
    fn draw_emits_idle_input_row_with_current_page() {
        let mut s = State::initial(100);
        s.install_page(page_with_links());
        let mut buf: Vec<u8> = Vec::new();
        draw(&s, &mut buf).expect("draw");
        let out = String::from_utf8_lossy(&buf);
        assert!(out.contains("  100"), "idle input row missing: {out:?}");
    }

    #[test]
    fn draw_emits_spinner_glyph_while_fetching() {
        let mut s = State::initial(100);
        s.install_page(page_with_links());
        s.fetch = FetchState::Fetching {
            target_page: 200,
            frame: 0,
        };
        let mut buf: Vec<u8> = Vec::new();
        draw(&s, &mut buf).expect("draw");
        let out = String::from_utf8_lossy(&buf);
        assert!(out.contains(SPINNER[0]), "spinner glyph missing: {out:?}");
    }

    #[test]
    fn draw_shows_input_buf_padded_with_underscores() {
        let mut s = State::initial(100);
        s.install_page(page_with_links());
        s.input_buf = "3".to_string();
        let mut buf: Vec<u8> = Vec::new();
        draw(&s, &mut buf).expect("draw");
        let out = String::from_utf8_lossy(&buf);
        assert!(out.contains("3__"), "padded input missing: {out:?}");
    }

    #[test]
    fn draw_emits_reverse_video_escape_for_selected_link() {
        let mut s = State::initial(100);
        s.install_page(page_with_links());
        assert_eq!(s.selected, Some(0));
        let mut buf: Vec<u8> = Vec::new();
        draw(&s, &mut buf).expect("draw");
        let out = String::from_utf8_lossy(&buf);
        assert!(out.contains("\x1b[7m"), "no reverse-on escape: {out:?}");
        assert!(out.contains("\x1b[27m"), "no reverse-off escape: {out:?}");
    }

    #[test]
    fn draw_skips_link_highlight_when_no_selection() {
        let mut s = State::initial(100);
        s.install_page(ColoredPage {
            page_no: 100,
            lines: vec![line("hello world")],
            plain: String::new(),
        });
        assert_eq!(s.selected, None);
        let mut buf: Vec<u8> = Vec::new();
        draw(&s, &mut buf).expect("draw");
        let out = String::from_utf8_lossy(&buf);
        assert!(!out.contains("\x1b[7m"), "spurious reverse-on: {out:?}");
    }
}
