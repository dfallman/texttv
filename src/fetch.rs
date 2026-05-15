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

/// Number of *additional* attempts after the first one for transient errors.
/// A value of 1 means up to 2 total tries.
const RETRY_COUNT: u32 = 1;
const RETRY_BACKOFF: Duration = Duration::from_millis(300);

fn do_get(url: &str) -> Result<String> {
    let mut attempts = 0u32;
    let resp = loop {
        match agent().get(url).call() {
            Ok(resp) => break resp,
            Err(e) if is_transient(&e) && attempts < RETRY_COUNT => {
                attempts += 1;
                std::thread::sleep(RETRY_BACKOFF);
                continue;
            }
            Err(ureq::Error::Status(code, resp)) => {
                let status_text = resp.status_text().to_string();
                return Err(anyhow!("HTTP {code} {status_text} for {url}"));
            }
            Err(ureq::Error::Transport(t)) => {
                return Err(anyhow!("network error: {t}"));
            }
        }
    };
    use std::io::Read;
    let mut buf = String::new();
    // Cap at MAX_BODY_BYTES so a hostile or runaway endpoint can't exhaust
    // memory by streaming gigabytes. take() reads at most the cap; for our
    // two endpoints exceeding it can't happen in practice.
    resp.into_reader()
        .take(MAX_BODY_BYTES)
        .read_to_string(&mut buf)
        .context("read response body")?;
    Ok(buf)
}

/// Classify a `ureq::Error` as transient (worth retrying) or terminal.
/// Transport errors (DNS, connect, read-timeout, TLS) are transient; HTTP
/// 5xx is transient (server-side hiccup); 4xx is terminal (the request
/// itself won't succeed on retry).
fn is_transient(e: &ureq::Error) -> bool {
    match e {
        ureq::Error::Transport(_) => true,
        ureq::Error::Status(code, _) => *code >= 500,
    }
}
