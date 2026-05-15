# texttv

A small and fast, yet decisively over-engineered command-line parser and reader for 
[SVT Text-TV](https://www.svt.se/text-tv/) — Sweden's public-service teletext. 
While 1979 _is_ calling, it's still a properly excellent source of news. Now available
in a terminal near you.

`texttv` is written entirely in [Rust](https://rustup.rs/) and works on most platforms 
(macOS, Windows, Linux, etc.) and most terminal emulators (such as iTerm2, Kitty, WezTerm, 
Ghostty, etc.) 

## Usage
`texttv` is simple to use out of the box and doesn't require any particular configuration
in most terminals to get started. To use, simply type `textv <PAGE>` (such as `texttv 100`). 

```bash
texttv 300              # sport
texttv 100              # news index
texttv 400              # weather
texttv --list           # show some named sections
```

`PAGE` can be any integer between `100..=999`. Note that all pages aren't available at all
times. If you enter a page that's not avaialable, `texttv` will tell you so.

## What it shows you

`texttv` has two different ways to render a page. The right one for your 
terminal defaults automatically, but you can always override it. 
Both render types have their pros and cons, pick the one you like the most:

1. **Teletext mode** (the default and fallback option) reconstructs the original
40-column page as real terminal text: the 8 teletext colors as ANSI
truecolor, bold double-height headings, Unicode sextants for the mosaic
block-characters (the SVT logo, sport icons, weather symbols, navigation
borders). The benefits here is that the text you see is real text: copyable, 
grep-able, and feels right at home in the terminal. The downside is that we lose some
minor details on the page, most notably the double-height characters that most terminal
emulators can't render in a predictable fashion.

3. **Image mode**, on the other hand, asks your terminal to draw the original page GIF that SVT
itself serves. Here, you'll get pixel-perfect rendering, but you lose selectable text.
This is the default pick for terminals with a native graphics protocol (such as Kitty,
Ghostty, WezTerm). On other terminal emulators, you can try it via `--mode auto`. On
half-block-only terminals (these include Apple Terminal, Alacritty, Windows Terminal,
and others) image mode falls back to a Unicode-half-block rendering, which is
considerably worse than the teletext text path. Unless you have a very large terminal
window, it'll look quite blurry (still pretty cool though). So for these terminals, 
the default stays text.

Pick explicitly with `--mode`:

```bash
texttv 300 --mode teletext       # force the text render
texttv 300 --mode auto           # render the GIF via whichever graphics protocol your terminal supports
texttv 300 --mode kitty          # force Kitty graphics protocol
texttv 300 --mode iterm          # force iTerm2 inline-image protocol
texttv 300 --mode blocks         # force the half-block fallback
```

## Install

Make sure you have the latest version of Rust installed:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then clone the repo and install:
```bash
git clone https://github.com/dfallman/texttv
cd texttv
cargo install --path .
```

Requires Rust 1.85+ (edition 2024).

## Options

| Flag | Meaning |
| --- | --- |
| `PAGE` | The page number. `100..=999`. |
| `--mode {teletext,auto,kitty,iterm,blocks}` | Rendering path. Default depends on your terminal (see the matrix below). |
| `--size {tiny,small,medium,large,xl,full}` | Width for image modes. Default `medium` (60 cells). Ignored in teletext mode. |
| `--source {texttv-nu,svt}` | Data source. Defaults to `texttv-nu` for teletext (it exposes the color attributes we need to reconstruct the page), `svt` for image modes. |
| `--no-color` | Strip ANSI color and the right-edge frame. Plain mono. |
| `--no-padding` | Drop the blank rows above and below the page. On by default. |
| `--list` | Print the named-section index and exit. |
| `--debug-protocol` | Echo the detected graphics protocol on stderr before drawing. |
| `--verbose` / `-v` | Per-phase timing traces on stderr. Also enabled by `TEXTTV_TIMINGS=1`. |
| `--help` / `--version` | Standard. |

Image-mode sizing (`--size`):

| value | cells |
| --- | --- |
| `tiny` | 30 |
| `small` | 45 |
| **`medium`** | **60 (default)** |
| `large` | 90 |
| `xl` | 120 |
| `full` | terminal width, capped at 4000 |

## Terminal matrix

Defaults vary by terminal. You can always override with `--mode`.

| Terminal | Default | Image-mode protocol |
| --- | --- | --- |
| Kitty | image | Kitty graphics (true pixels) |
| Ghostty | image | Kitty graphics (true pixels) |
| WezTerm | image | iTerm2 inline (true pixels) |
| iTerm2 | image | iTerm2 inline (true pixels) |
| Apple Terminal | teletext | half-blocks fallback |
| Alacritty | teletext | half-blocks fallback |
| Windows Terminal | teletext | half-blocks fallback |
| foot / mlterm | teletext | half-blocks fallback (Sixel is on the roadmap) |

`texttv 300 --debug-protocol` reports the detected protocol on stderr if
you're curious.

### tmux

Kitty's graphics protocol needs tmux passthrough:

```
set -g allow-passthrough on
set -g default-terminal "tmux-256color"
```

If `--debug-protocol` says `halfblocks` inside tmux but your outer terminal
is Kitty / Ghostty / WezTerm, this is almost certainly why. Teletext mode is
unaffected.

## Auto-degrade

When `stdout` is piped or redirected, ANSI escapes are stripped automatically
— so `texttv 300 | grep` and `texttv 300 > page.txt` produce clean text.
`NO_COLOR=1` and `--no-color` do the same explicitly. Image mode is replaced
by teletext mode when the output isn't a TTY (you don't want raw image-
protocol escape codes ending up in a pipe).

## Config file

Optional. Read from `$XDG_CONFIG_HOME/texttv/config.yaml` (defaults to
`~/.config/texttv/config.yaml`). CLI flags always win; missing keys fall back
to built-in defaults; unknown keys are an error.

```yaml
# All fields optional. Lines below show the built-in defaults.

# mode: teletext               # teletext | auto | kitty | iterm | blocks
                               # (terminal-dependent default, see matrix)
# size: medium                 # tiny | small | medium | large | xl | full
# source: texttv-nu            # texttv-nu | svt
# no_color: false              # also honoured: NO_COLOR=1 env, piped stdout
# padding: true                # blank row above and below the page
# verbose: false               # per-phase timing traces on stderr
```

## Exit codes

| code | meaning |
| --- | --- |
| `0` | success |
| `1` | bad arguments / page out of range |
| `2` | network error, parse error, or "page not available" |

## Data sources

- **[`api.texttv.nu`](https://texttv.nu)** — community-run JSON proxy that
  preserves per-cell color attributes. The teletext mode depends on this;
  the official SVT pages strip the color information. We identify
  ourselves as `app=texttvcliv<version>` per
  [their policy](https://texttv.nu/blogg/texttv-api).
- **[`svt.se/text-tv/<PAGE>`](https://www.svt.se/text-tv/)** — the official
  page, used for the embedded GIF when an image mode is active.

Decoded mosaic patterns persist to `$XDG_CACHE_HOME/texttv/mosaics/` so a
given pattern is fetched once across all runs.

## License

MIT or Apache-2.0, at your option.
