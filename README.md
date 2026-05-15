# texttv

Render SVT Text-TV pages in your terminal.

```bash
texttv 300              # Sport — prints the page text (default)
texttv 300 --mode auto  # Render the GIF page using the best graphics protocol
texttv --list           # Index of well-known section pages
```

By default `texttv` reconstructs the original 40-column teletext layout:
real text, real colors, and double-height headers. Data comes from the
`api.texttv.nu` JSON feed, which exposes per-cell color attributes that
SVT's official site doesn't publish. The page GIF render is still
available behind `--mode auto/kitty/iterm/blocks`.

## Fidelity

- **Colors** — the 8 teletext primaries (black/red/green/yellow/blue/
  magenta/cyan/white) emitted as ANSI truecolor escapes.
- **Double-height** — lines tagged DH are rendered using the DEC private
  escapes `ESC # 3` / `ESC # 4`, supported by Kitty, Ghostty, WezTerm,
  iTerm2, xterm, and most VT-compatible terminals.
- **Mosaic graphics** — teletext mosaic block characters (used for small
  icons and borders) are rendered as colored spaces in v1. Layout is
  preserved; the block pattern is lost. Mapping mosaic GIFs to Unicode
  block characters is on the roadmap.

## Install

```bash
cargo install --path .
```

Requires Rust 1.85+ (edition 2024).

## Terminal compatibility

| Terminal       | Protocol used       | Status                                       |
| -------------- | ------------------- | -------------------------------------------- |
| Kitty          | Kitty graphics      | First-class — native pixel rendering         |
| Ghostty        | Kitty graphics      | First-class — native pixel rendering         |
| WezTerm        | iTerm2 inline image | First-class — native pixel rendering         |
| iTerm2         | iTerm2 inline image | First-class — native pixel rendering         |
| Apple Terminal | Unicode half-blocks | Fallback (no graphics protocol support)      |
| Alacritty      | Unicode half-blocks | Fallback                                     |
| Windows Term.  | Unicode half-blocks | Fallback                                     |
| foot / mlterm  | Unicode half-blocks | Falls back; Sixel support is on the roadmap  |

Run `texttv 300 --debug-protocol` to print the detected protocol (`kitty`,
`iterm`, or `halfblocks`) on stderr before drawing.

## Flags

| Flag                              | Meaning                                                |
| --------------------------------- | ------------------------------------------------------ |
| `--mode {auto,kitty,iterm,blocks,text}` | Pick the rendering path. Defaults to `text`. |
| `--no-color`                      | Strip ANSI color and double-height escapes; plain mono. |
| `--list`                          | Print the section index and exit.                      |
| `--debug-protocol`                | Print the detected protocol on stderr before drawing.  |
| `--source {svt,texttv-nu}`        | Override the data source. Default: `texttv-nu` for text, `svt` for image modes. |

## tmux caveat

Inside tmux, Kitty's graphics protocol requires passthrough. Add to
`~/.tmux.conf`:

```
set -g allow-passthrough on
set -g default-terminal "tmux-256color"
```

If `texttv 300 --debug-protocol` reports `halfblocks` inside tmux but your
outer terminal is Kitty/Ghostty/WezTerm/iTerm2, this is almost always the
cause. With `--debug-protocol` set, `texttv` prints a hint on stderr when
it detects this situation.

## Auto-degrade

`texttv` is text-first. When stdout is piped or redirected, ANSI escapes
and double-height codes are stripped automatically so `texttv 300 | grep`
and `texttv 300 > /tmp/page.txt` produce clean text. `NO_COLOR=1` and
`--no-color` do the same explicitly. If you want color through a pager,
use `texttv 300 | less -R`.

If you explicitly ask for image rendering (`--mode auto/kitty/iterm/blocks`)
and stdout is piped, `--mode auto` still degrades to text; the forced
graphics modes write their escape sequences regardless.

## Exit codes

- `0` — success
- `1` — bad arguments / page out of range
- `2` — network or parse error, or "page not available"

## License

MIT or Apache-2.0, at your option.
