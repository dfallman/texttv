use anyhow::{Context, Result, anyhow};
use std::sync::OnceLock;
use std::time::Duration;

const USER_AGENT: &str = concat!(
    "texttv/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/dfallman/texttv)"
);

/// Soft cap on response body size. Both endpoints we hit return well under
/// 1 MB in practice; this is the ceiling before we treat the body as hostile
/// and stop reading rather than buffering unbounded bytes.
const MAX_BODY_BYTES: u64 = 5 * 1024 * 1024;

/// Shared HTTP agent. Reusing a single agent across requests pools the
/// underlying TCP/TLS connection — building one fresh per call eats a TLS
/// handshake every time.
fn agent() -> &'static ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(Duration::from_secs(10))
            .timeout_write(Duration::from_secs(10))
            .user_agent(USER_AGENT)
            .build()
    })
}

pub fn fetch_html(page: u16) -> Result<String> {
    let url = format!("https://www.svt.se/text-tv/{page}");
    do_get(&url)
}

/// api.texttv.nu returns JSON whose `content[0]` is the page rendered as HTML
/// with per-cell color classes — that's what `parse_texttv_nu` consumes.
///
/// The `app` query parameter is what texttv.nu's documentation
/// (<https://texttv.nu/blogg/texttv-api>) asks API users to set so they can
/// identify which client is fetching pages and curb abuse. We embed our own
/// version so the value rolls forward automatically with each release.
pub fn fetch_texttv_nu(page: u16) -> Result<String> {
    let url = format!(
        "https://api.texttv.nu/api/get/{page}?app=texttvcliv{version}",
        version = env!("CARGO_PKG_VERSION"),
    );
    do_get(&url)
}

fn do_get(url: &str) -> Result<String> {
    match agent().get(url).call() {
        Ok(resp) => {
            use std::io::Read;
            let mut buf = String::new();
            // Cap at MAX_BODY_BYTES so a hostile or runaway endpoint can't
            // exhaust memory by streaming gigabytes. take() reads at most the
            // cap; if the body is exactly the cap there is no signal to the
            // caller that it was truncated, but for our two endpoints that
            // can't happen in practice.
            resp.into_reader()
                .take(MAX_BODY_BYTES)
                .read_to_string(&mut buf)
                .context("read response body")?;
            Ok(buf)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let status_text = resp.status_text().to_string();
            Err(anyhow!("HTTP {code} {status_text} for {url}"))
        }
        Err(ureq::Error::Transport(t)) => Err(anyhow!("network error: {t}")),
    }
}
