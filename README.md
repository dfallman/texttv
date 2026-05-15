# texttv

Render [SVT Text-TV](https://www.svt.se/text-tv/) pages in your terminal,
with full fidelity to the original 40-column teletext layout — real text,
real colors, and double-height headers.

```bash
texttv 300              # Sport
texttv 100              # News index
texttv 400              # Weather
texttv 300 --no-color   # Plain mono, grep-friendly
texttv 300 --mode auto  # Render the page bitmap via your terminal's graphics protocol
texttv --list           # Index of well-known section pages
```

`PAGE` is any integer in `100..=999`.

## Examples

```  ┌──────────────────────────────────┬───────────────────────────────┬─────────────────────┐  │             Terminal             │       How it's detected       │      Protocol       │  ├──────────────────────────────────┼───────────────────────────────┼─────────────────────┤  │ Kitty                            │ $TERM contains kitty          │ Kitty graphics      │  ├──────────────────────────────────┼───────────────────────────────┼─────────────────────┤  │ Ghostty                          │ $TERM contains ghostty        │ Kitty graphics      │  ├──────────────────────────────────┼───────────────────────────────┼─────────────────────┤  │ iTerm2                           │ $TERM_PROGRAM=iTerm.app       │ iTerm2 inline image │  ├──────────────────────────────────┼───────────────────────────────┼─────────────────────┤  │ WezTerm                          │ $TERM_PROGRAM=WezTerm         │ iTerm2 inline image │  ├──────────────────────────────────┼───────────────────────────────┼─────────────────────┤  │ mintty (Cygwin/MSYS2 on Windows) │ $TERM_PROGRAM contains mintty │ iTerm2 inline image │  ├──────────────────────────────────┼───────────────────────────────┼─────────────────────┤  │ Rio                              │ $TERM_PROGRAM contains rio    │ iTerm2 inline image │  ├──────────────────────────────────┼───────────────────────────────┼─────────────────────┤  │ Warp                             │ $TERM_PROGRAM=WarpTerminal    │ iTerm2 inline image │  └──────────────────────────────────┴───────────────────────────────┴─────────────────────┘
```
```
  cargo run -- 300                                    # text mode (default)  cargo run -- 300 --mode auto                        # render the GIF — should pick Kitty graphics  cargo run -- 300 --mode auto --debug-protocol       # confirms 'detected: kitty' on stderr  cargo run -- 300 --no-color                         # plain mono  cargo run -- 200 --mode auto                        # has double-height; also good for image diff  cargo run --release -- 300 --mode auto              # optimized build (avoids debug-mode slowness)
```

## How it works

`texttv` is teletext-first. By default it pulls the page from the
[`api.texttv.nu`](https://texttv.nu) JSON feed, which exposes per-cell
color attributes — the only public source faithful enough to reconstruct
teletext color and double-height. SVT's own HTML strips those attributes,
so it's used only for the bitmap render path (`--mode auto/kitty/iterm/blocks`),
which displays the original GIF SVT embeds.

## Install

```bash
cargo install --path .
```

Requires Rust 1.85+ (edition 2024).

## Flags

| Flag                                    | Meaning                                                                         |
| --------------------------------------- | ------------------------------------------------------------------------------- |
| `--mode {teletext,auto,kitty,iterm,blocks}` | Pick the rendering path. Defaults to `teletext`.                            |
| `--no-color`                            | Strip ANSI color and double-height escapes; plain mono.                         |
| `--list`                                | Print the section index and exit.                                               |
| `--source {svt,texttv-nu}`              | Override the data source. Default: `texttv-nu` for teletext mode, `svt` for image modes. |
| `--debug-protocol`                      | Print the detected graphics protocol on stderr before drawing.                  |
| `--help`, `--version`                   | Standard.                                                                       |

## Fidelity

- **Colors** — the 8 teletext primaries (black, red, green, yellow, blue,
  magenta, cyan, white) emitted as ANSI truecolor escapes via
  [`owo-colors`](https://crates.io/crates/owo-colors).
- **Double-height** — lines tagged `DH` render via the DEC private escapes
  `ESC # 3` (top half) / `ESC # 4` (bottom half). Supported by Kitty,
  Ghostty, WezTerm, iTerm2, xterm, and most VT-compatible terminals.
- **Mosaic graphics** — teletext mosaic block characters (small icons and
  borders) currently render as colored spaces. Layout is preserved; the
  block pattern is dropped. Mapping the mosaic GIFs to Unicode block
  characters is on the roadmap.

## Image mode

`--mode auto` (or `kitty` / `iterm` / `blocks`) renders the page GIF that
SVT embeds, capped at 60 terminal cells wide. The graphics protocol is
auto-detected via [`viuer`](https://crates.io/crates/viuer):

| Terminal       | Protocol used       | Status                                      |
| -------------- | ------------------- | ------------------------------------------- |
| Kitty          | Kitty graphics      | First-class — native pixel rendering        |
| Ghostty        | Kitty graphics      | First-class — native pixel rendering        |
| WezTerm        | iTerm2 inline image | First-class — native pixel rendering        |
| iTerm2         | iTerm2 inline image | First-class — native pixel rendering        |
| Apple Terminal | Unicode half-blocks | Fallback (no graphics protocol support)     |
| Alacritty      | Unicode half-blocks | Fallback                                    |
| Windows Term.  | Unicode half-blocks | Fallback                                    |
| foot / mlterm  | Unicode half-blocks | Falls back; Sixel support is on the roadmap |

Run `texttv 300 --mode auto --debug-protocol` to see which protocol your
terminal triggered.

### tmux caveat

Inside tmux, Kitty's graphics protocol requires passthrough. Add to
`~/.tmux.conf`:

```
set -g allow-passthrough on
set -g default-terminal "tmux-256color"
```

If `--debug-protocol` reports `halfblocks` inside tmux but your outer
terminal is Kitty/Ghostty/WezTerm/iTerm2, this is almost always the cause.
The default text mode is unaffected — DEC double-height escapes pass
through tmux without extra configuration.

## Auto-degrade

`texttv` strips ANSI escapes, the right-edge frame, and double-height
codes automatically when
stdout is piped or redirected, so `texttv 300 | grep` and
`texttv 300 > /tmp/page.txt` produce clean text. `NO_COLOR=1` and
`--no-color` do the same explicitly. To keep color through a pager,
use `texttv 300 | less -R`.

For image rendering, `--mode auto` piped degrades to teletext mode; the
forced graphics modes (`--mode kitty/iterm/blocks`) still emit escape
sequences, because that's what you asked for.

## Exit codes

| Code | Meaning                                                       |
| ---- | ------------------------------------------------------------- |
| `0`  | success                                                       |
| `1`  | bad arguments / page out of range                             |
| `2`  | network error, parse error, or "page not available"           |

## Data sources

- **Default (text):** [`api.texttv.nu`](https://texttv.nu) — the
  community-run JSON proxy that preserves teletext color attributes.
  `texttv` identifies itself with `app=texttv-rs` per their policy.
- **Image modes:** [`svt.se/text-tv/<PAGE>`](https://www.svt.se/text-tv/) —
  the original SVT page, used for the embedded GIF.

## License

MIT or Apache-2.0, at your option.
