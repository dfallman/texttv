//! In-terminal interactive page browser. See
//! `docs/superpowers/specs/2026-05-15-interactive-mode-design.md`.

use std::io::{IsTerminal, Write, stdout};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use crossterm::{
    QueueableCommand, execute,
    cursor::{Hide, MoveTo, Show},
    event::{Event, KeyCode, KeyEvent, KeyEventKind, read},
    style::Print,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};

use crate::parse::{ColoredPage, Line};

pub(crate) const SPINNER: &[char] = &[
    '⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏',
];
/// Event-loop poll timeout (also the spinner cadence). Used by the
/// event loop landing in Task 10.
#[allow(dead_code)]
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
        KeyCode::Enter => {
            if let Some(sel) = state.selected
                && let Some(link) = state.links.get(sel)
            {
                if link.followable {
                    Action::StartFetch(link.target)
                } else {
                    state.status =
                        Some(format!("Error: page {} not in 100..=999", link.target));
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

/// Advance the spinner frame by one (wrapping at `SPINNER.len()`) when
/// a fetch is in flight. No-op otherwise. Called by the event loop
/// every `SPINNER_INTERVAL` (80 ms).
pub fn tick(state: &mut State) {
    if let FetchState::Fetching { frame, .. } = &mut state.fetch {
        *frame = (*frame + 1) % SPINNER.len();
    }
}

/// Compose the input-row string. `frame` is `Some(glyph)` while fetching.
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
    format!("{glyph} {digits}")
}

/// Full redraw of the interactive screen. `out` is typically stdout in
/// raw mode wrapped in a BufWriter; tests pass a `Vec<u8>`.
pub fn draw<W: Write>(state: &State, out: &mut W) -> anyhow::Result<()> {
    out.queue(Hide)?;
    out.queue(MoveTo(0, 0))?;
    out.queue(Clear(ClearType::CurrentLine))?;
    out.queue(Print(input_row(state)))?;

    // Page body starts at row 1. Render each line individually with an
    // explicit MoveTo so raw mode's missing carriage-return doesn't leave
    // the cursor drifting right.
    for (i, line) in state.lines.iter().enumerate() {
        out.queue(MoveTo(0, (i as u16) + 1))?;
        out.queue(Clear(ClearType::CurrentLine))?;
        let slice = std::slice::from_ref(line);
        crate::render::render_colored(slice, true, out)?;
    }

    // Selected-link highlight: overlay reverse video on the link's run
    // of cells. Run after the body render so we paint over the original
    // colors with `\x1b[7m … \x1b[27m` around the visible characters.
    if let Some(sel) = state.selected
        && let Some(link) = state.links.get(sel)
    {
        let row_idx = link.row as usize;
        if let Some(line) = state.lines.get(row_idx) {
            out.queue(MoveTo(link.col_start, link.row + 1))?;
            let visible = visible_chars_at(line, link.col_start, link.col_len);
            out.queue(Print("\x1b[7m"))?;
            out.queue(Print(visible))?;
            out.queue(Print("\x1b[27m"))?;
        }
    }

    // Hint bar on row 26.
    out.queue(MoveTo(0, 26))?;
    out.queue(Clear(ClearType::CurrentLine))?;
    let hint = state
        .status
        .as_deref()
        .unwrap_or("↑↓ links · Enter open · 0-9 jump · Esc quit");
    out.queue(Print(hint))?;

    // Put the cursor back at the next typing position in the input zone.
    let cursor_col = 2 + (state.input_buf.len() as u16);
    out.queue(MoveTo(cursor_col, 0))?;
    out.queue(Show)?;
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

    let mut stdout = stdout();
    enable_raw_mode().context("entering raw mode")?;
    execute!(stdout, EnterAlternateScreen).context("entering alt screen")?;

    let result = run_inner(initial_page, &mut stdout);

    // Always restore terminal state, even on error.
    let _ = execute!(stdout, LeaveAlternateScreen);
    let _ = disable_raw_mode();

    result
}

fn run_inner<W: Write>(initial_page: u16, out: &mut W) -> Result<()> {
    let mut state = State::initial(initial_page);

    // Initial load is synchronous (we don't have the spinner yet).
    load_into_state(&mut state, initial_page);
    draw(&state, out)?;

    loop {
        let ev = read().context("reading terminal event")?;
        match ev {
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                match handle_key(&mut state, k) {
                    Action::None => {}
                    Action::Quit => break,
                    Action::StartFetch(page) => {
                        load_into_state(&mut state, page);
                    }
                }
            }
            Event::Resize(_, _) => {} // just redraw below
            _ => {}
        }
        draw(&state, out)?;
    }
    Ok(())
}

/// Synchronous fetch + parse + mosaic prefetch on the main thread. Updates
/// `state.lines` / `state.links` / `state.selected` on success; sets a
/// status message on failure. Replaced by an off-thread version in
/// the next task.
fn load_into_state(state: &mut State, page: u16) {
    let result = crate::fetch::fetch_texttv_nu(page)
        .and_then(|json| crate::parse::parse_texttv_nu(&json, page));
    match result {
        Ok(cp) => {
            crate::mosaic::prefetch_page(&cp);
            state.install_page(cp);
        }
        Err(e) => {
            state.status = Some(format!("Error: {e:#}"));
            state.input_buf.clear();
            state.fetch = FetchState::Idle;
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
