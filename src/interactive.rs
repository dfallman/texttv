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
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
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

/// Scan the rendered page for three-digit page references that are
/// flanked by spaces (or line edges) on both sides. Mosaic cells count
/// as spaces — they never participate in a link.
///
/// Iterates by char (not byte) so non-ASCII text earlier on the line
/// — Swedish å/ö/ä, copyright sigils, etc. — doesn't shift the link's
/// `col_start` relative to the rendered grid. (A byte index over a UTF-8
/// string overcounts the column for every multi-byte char that came
/// before, which is what was painting the reverse-video highlight off
/// by one or more cells.)
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
            let left_boundary = i == 0 || chars[i - 1] == ' ';
            let right_boundary = i + 3 == chars.len() || chars[i + 3] == ' ';
            if !(left_boundary && right_boundary) {
                i += 1;
                continue;
            }
            // Compose the highlight extent: include the surrounding space
            // cells when they exist.
            let col_start = if i > 0 { (i as u16) - 1 } else { 0 };
            let after_digits = i + 3;
            let col_end = if after_digits < chars.len() {
                (after_digits as u16) + 1
            } else {
                after_digits as u16
            };
            let target_str: String = chars[i..i + 3].iter().collect();
            let target: u16 = target_str.parse().unwrap_or(0);
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

/// Full width (in cells) the chrome rows paint with the page's black bg
/// so they read as part of the same visual surface. Matches the teletext
/// page's 40 data cells + 1 right-edge frame cell.
const CHROME_WIDTH: usize = 41;

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

/// Paint a single chrome row (input or hint) with the page's white-on-black
/// colors, padded out to `CHROME_WIDTH` so the row visually blends with
/// the page below it.
fn paint_chrome_row<W: Write>(out: &mut W, row: u16, content: &str) -> anyhow::Result<()> {
    out.queue(MoveTo(0, row))?;
    out.queue(Clear(ClearType::CurrentLine))?;
    let visible: String = content.chars().take(CHROME_WIDTH).collect();
    let visible_width = visible.chars().count();
    let pad = CHROME_WIDTH.saturating_sub(visible_width);
    let line = format!("{visible}{:pad$}", "", pad = pad);
    let styled = line
        .truecolor(255, 255, 255)
        .on_truecolor(0, 0, 0)
        .to_string();
    out.queue(Print(styled))?;
    Ok(())
}

/// Full redraw of the interactive screen. `out` is typically stdout in
/// raw mode wrapped in a BufWriter; tests pass a `Vec<u8>`.
pub fn draw<W: Write>(state: &State, out: &mut W) -> anyhow::Result<()> {
    // Row 0: input field. Chrome row, white-on-black to match the page.
    paint_chrome_row(out, 0, &input_row(state))?;

    // Rows 1..=N: page body. Each row gets an explicit MoveTo so raw
    // mode's missing carriage-return doesn't leave the cursor drifting.
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

    // Hint bar: placed immediately below the last page row so there's no
    // dead space for short pages. Also a chrome row.
    let hint_row = 1 + state.lines.len() as u16;
    let hint = state.status.as_deref().unwrap_or("↑↓ · Enter · Esc quit");
    paint_chrome_row(out, hint_row, hint)?;

    // Park the system cursor at the next typing position in the input
    // zone. No Hide/Show pair — the terminal's native cursor (block, bar,
    // underline, whatever the user has configured) stays visible
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
    // 27 rows (input + 25 page + hint). Smaller windows would render
    // chrome on top of the page or vice versa.
    let (cols, rows) = crossterm::terminal::size().context("reading terminal size")?;
    if cols < 41 || rows < 27 {
        return Err(anyhow!(
            "terminal too small ({cols}x{rows}); need at least 41x27"
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
