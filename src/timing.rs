//! Lightweight timing instrumentation for development.
//!
//! Enabled automatically in debug builds (`cargo run --`) so iterating is
//! fast to observe. Disabled in release builds (`cargo install`) unless
//! `TEXTTV_TIMINGS=1` is set in the environment — handy when chasing a
//! perf regression on an installed binary without rebuilding.
//!
//! Output goes to stderr so it doesn't pollute the rendered page on stdout.

use std::sync::OnceLock;
use std::time::Instant;

pub fn enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| {
        cfg!(debug_assertions) || std::env::var_os("TEXTTV_TIMINGS").is_some()
    })
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
