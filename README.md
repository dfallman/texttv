# texttv

Render SVT Text-TV pages in your terminal.

```bash
texttv 300              # Sport — prints the page text (default)
texttv 300 --mode auto  # Render the GIF page using the best graphics protocol
texttv --list           # Index of well-known section pages
```

By default `texttv` prints the actual text content extracted from SVT's
accessibility data — faithful to the 40-column teletext layout and grep-
friendly. The original page GIF is also available as a bitmap render
(`--mode auto/kitty/iterm/blocks`), capped at 60 terminal cells wide.

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
| `--no-color`                      | Disable ANSI color in text mode.                       |
| `--list`                          | Print the section index and exit.                      |
| `--debug-protocol`                | Print the detected protocol on stderr before drawing.  |
| `--source {svt,texttv-nu}`        | Data source. Defaults to the official SVT site.        |

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

`texttv` is text-first; piping just works (`texttv 300 | grep`). If you
explicitly ask for image rendering (`--mode auto/kitty/iterm/blocks`) and
stdout is piped, `--mode auto` still degrades to text — but the forced
graphics modes write their escape sequences regardless, since that's what
you asked for. `NO_COLOR=1` disables ANSI color in text mode.

## Exit codes

- `0` — success
- `1` — bad arguments / page out of range
- `2` — network or parse error, or "page not available"

## License

MIT or Apache-2.0, at your option.
