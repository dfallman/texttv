//! Lightweight timing instrumentation.
//!
//! Off by default. Enable with `--verbose` (or `-v`), `verbose: true` in
//! `~/.config/texttv/config.yaml`, or `TEXTTV_TIMINGS=1` in the environment
//! — the env var is kept for scripts that want to flip timings on without
//! rewriting their argv.
//!
//! Output goes to stderr so it doesn't pollute the rendered page on stdout.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Call once at startup from main.rs to wire up the flag/env/config result.
pub fn set_enabled(b: bool) {
    ENABLED.store(b, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Wrap a closure with a stderr "[texttv] <label> (<n>ms)" trace line when
/// timing is enabled. Zero overhead when disabled.
pub fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    if !enabled() {
        return f();
    }
    let start = Instant::now();
    let result = f();
    let ms = start.elapsed().as_millis();
    eprintln!("[texttv] {label} ({ms}ms)");
    result
}

/// Convenience: log a single one-line trace if timing is on. Use for
/// "summary" or "context" lines (e.g. "prefetched 12 unique mosaics") that
/// don't naturally wrap a closure.
pub fn note(msg: &str) {
    if enabled() {
        eprintln!("[texttv] {msg}");
    }
}
