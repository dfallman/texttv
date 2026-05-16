# texttv

A small and fast, yet decidedly over-engineered command-line viewer for [SVT Text-TV](https://www.svt.se/text-tv/), Swedish Television's public-service teletext. The UI might be [1979 calling](https://www.theguardian.com/world/2024/jan/11/teletext-lives-on-in-sweden-thanks-to-nostalgia-and-trusted-content), but it's still a properly excellent source of news. Now available in a terminal near you.

<p align="center">

  <img width="800" alt="texttv in use in the terminal" src="https://github.com/user-attachments/assets/e5f20f29-5f19-438d-9c6b-f31e51885e0a" />

</p>

`texttv` is written entirely in [Rust](https://rustup.rs/) and runs on most platforms 
(macOS, Windows, Linux, etc.) in most terminal emulators (such as [iTerm2](https://iterm2.com/), [Ghostty](https://ghostty.org/), [Terminal.app](https://support.apple.com/guide/terminal/welcome/mac), [Windows Terminal](https://github.com/microsoft/terminal), [GNOME Terminal](https://help.gnome.org/users/gnome-terminal/stable/), [Konsole](https://konsole.kde.org/), [Foot](https://codeberg.org/dnkl/foot), [Terminator](https://gnome-terminator.org/), [Kitty](https://sw.kovidgoyal.net/kitty/), [WezTerm](https://wezterm.org/), [Alacritty](https://alacritty.org/), [Tabby](https://tabby.sh/), and others). 

## Installation

Make sure you have the latest version of Rust installed. Use [Rustup](https://rustup.rs/) rather than your package manager to install, as this ensures you'll get the latest version.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then clone the repo and install:

```bash
git clone https://github.com/dfallman/texttv
cd texttv
cargo install --path .
```

## Usage
`texttv` is simple to use out of the box and doesn't require any particular configuration
in most terminals to get started. 

The app has two different modes, **interactive mode** (where you browse the pages) and **single page view** (where you output a single page to the terminal).

### Interactive mode 
This opens a Text-TV browser in the terminal and allows you to navigate the teletext pages using familiar inputs. Type the page number to go to that page, use your arrow keys left and right to go to next/previous page, use up and down to select links on the page, and enter to select. Press `Esc` to quit.

Use the commands `texttv` or `texttv --interactive`.

<p align="center">

  <img width="400" alt="texttv interactive mode" src="https://github.com/user-attachments/assets/cf0a95bd-c155-4ef1-b985-2b2d781d8d4d" />
  <br>  
  <em>Interactive mode</em>

</p>

### Single page view mode
Outputs a single page to the terminal. Type `texttv 360` to view page 360, for example (I wonder how those Birch Leaves are going...) How the single page is rendered depends on your terminal emulator's capabilities and your settings (see below).

## Rendering modes

`texttv` has two different ways to render a page. The right one for your 
terminal defaults automatically, but you can always override it. 
Both render types have their pros and cons, pick the one you like the most:

1. **Teletext mode** (the default and fallback option) reconstructs the original
40-column page as real terminal text: the 8 teletext colors as ANSI
truecolor, bold double-height headings, Unicode sextants for the mosaic
block-characters (the SVT logo, sport icons, weather symbols, navigation
borders). The benefit here is that the text you see is real text: copyable, 
grep-able, and feels right at home in the terminal. The downside is that we lose some
minor details on the page, most notably the double-height characters that most terminal
emulators can't render in a predictable fashion.

<p align="center">
    <img width="800" alt="texttv in use in the terminal" src="https://github.com/user-attachments/assets/1dfe1df2-e52f-4ec9-b617-e467254157e6" />
    <br>
    <em>Teletext mode: pages are rendered as text</em>
</p>

3. **Image mode**, on the other hand, asks your terminal to draw the original page GIF that SVT
itself serves. Here, you'll get pixel-perfect rendering, but you lose selectable text.
This is the default pick for terminals with a native graphics protocol (Kitty,
Ghostty, WezTerm, iTerm2). On other terminal emulators, you can try it via `--mode auto`. On
half-block-only terminals (these include Apple Terminal, Alacritty, Windows Terminal,
and others) image mode falls back to a Unicode-half-block rendering, which is
considerably worse than the teletext text path. Unless you have a very large terminal
window, it'll look quite blurry (still pretty cool though!) So for these terminals, 
the default is `--mode teletext`.

<p align="center">
    <img width="800" alt="texttv rednered as image" src="https://github.com/user-attachments/assets/563e2d42-a1ad-48d1-b7b1-401c4cd761a1" />
    <br>
    <em>Image mode: pages are rendered as images in supported terminals</em>
</p>


Pick your mode explicitly with `--mode`:

```bash
texttv 300 --mode teletext       # force the text render
texttv 300 --mode auto           # render the GIF via whichever graphics protocol your terminal supports
texttv 300 --mode kitty          # force Kitty graphics protocol
texttv 300 --mode iterm          # force iTerm2 inline-image protocol
texttv 300 --mode blocks         # force the half-block fallback
```

**Pro tip:** if you prefer a rendering mode or a size that isn't the default for your terminal emulator, consider creating a configuration file (see below). That way your preferred settings will load automatically and you'll only have to type `texttv <PAGE>`.

### A note on double-height headlines

Teletext's headline rows are *double-height*, where each character is one column
wide but two rows tall. SVT Text-TV tends to frequently use double-height characters for headings, typically on page `100`.

The terminal equivalent is called `DECDHL` (DEC Double-Height
Line), a VT100-era escape sequence that does exactly the same thing.

In theory `texttv` could emit `DECDHL` and let your terminal handle it. In practice
though, support is a mess at best: xterm, Kitty, WezTerm, and Konsole render it correctly (more or less);
Apple Terminal, Alacritty, Windows Terminal, and Ghostty either ignore it,
half-render it, or break in interesting ways around scrolling and cursor
positioning. Many emulators also disagree about how copy-paste, resize, and reflow should behave.

Rather than ship a feature that looks good on a third of terminals and
visibly broken on the rest, `texttv` renders headlines at normal height in
teletext mode. You still get the bold weight and the color, just not the
2x vertical. Image mode preserves them faithfully however. Optional support for DECDHL might be added in future releases.

## Usage examples
`texttv` is easy to use out of the box but also highly configurable, so the above is only the basic operation. Here are some
examples of a few typical options (see below for details):

```
texttv
```

The first example starts `texttv` in interactive mode, which allows you to navigate through pages using number input, arrow keys, and `Enter` to select, `Esc` to quit.

```
texttv 300 --mode teletext
```

The second example shows page 300 but overrides the default rendering mode (see below) to `teletext` mode, which draws the page as text+ANSI.

```
texttv 101 --mode iterm --size small --source svt --verbose 
```

The third example enforces iTerm2's default rendering mode (image), outputs it in small size, enforces the source to be SVT, and shows verbose logging output in the console.

## Options

`texttv <PAGE> [options]`

| Flag | Meaning |
| --- | --- |
| `<PAGE>` | The page number between 100 and 999 |
| `--mode {teletext,auto,kitty,iterm,blocks}` | Rendering path. Default depends on your terminal (see the matrix below). |
| `--size {tiny,small,medium,large,xl,full}` | Width for image modes. Default `medium` (60 cells). Ignored in teletext mode. |
| `--source {texttv-nu,svt}` | Data source. Defaults to `texttv-nu` for teletext (it exposes the color attributes we need to reconstruct the page), `svt` for image modes. |
| `--no-color` | Strip ANSI color and the right-edge frame. Plain mono. |
| `--no-padding` | Drop the blank rows above and below the page. (Padding is on by default.) |
| `--list` | Print the named-section index and exit. |
| `--debug-protocol` | Echo the detected graphics protocol on stderr before drawing. |
| `--verbose` / `-v` | Per-phase timing traces on stderr. Also enabled by `TEXTTV_TIMINGS=1`. |
| `--help` / `--version` | Standard. |

### Image-mode sizing (`--size`):

| value | cells |
| --- | --- |
| `tiny` | 30 |
| `small` | 45 |
| **`medium`** | **60 (default)** |
| `large` | 90 |
| `xl` | 120 |
| `full` | terminal width, capped at 4000 |

## Terminal image mode matrix

Defaults vary by terminal. You can always override with `--mode`. Note however that your terminal can only draw what it can draw, so even if you override the image-mode protocol with something your terminal can't handle, the fallback will be shown (usually half-blocks). If this happens, `--mode teletext` is the safest option.

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

## Interactive mode

Running `texttv` with no arguments — or with `-i` / `--interactive` — opens
an in-terminal page browser starting at page 100. Pass an explicit page to
start elsewhere, e.g. `texttv -i 300`.

**The input field** sits in the top-left, overlaying the page's own
header. It shows a triangle pointer (`⏵`) when idle and a braille spinner
during loads, followed by the current page number. Type three digits to
jump to a page — no Enter required, the page loads as soon as the third
digit lands. Partial input shows as middle-dot placeholders (`3··`).
Backspace removes the last typed digit. Out-of-range pages (`000`–`099`)
flash an error in the hint bar.

**Links** are three-digit page references found on the rendered page.
The detector is permissive on purpose:

- `" 300 "` — the bare case.
- `" 328f "` — `f` is the multi-page suffix; the link targets 328.
- `" 376- "` — trailing dash (range start with no upper bound).
- `" 343-344 "` — range; both numbers become links.
- `"100.000"` is *not* a link (digits adjacent to a `.`).

When the cursor is on a link the three digits are highlighted in
white-on-magenta. Press Enter or Space to follow it.

**Multi-page** pages (the `XXXf` indicator) carry multiple subpages.
The bottom bar replaces the standard hint with a subpage selector:
`Page: >1< 2 3 4 …` with `>active<` marking the currently rendered
subpage. When the cursor is parked on a subpage selector, `←/→` cycle
through the subpages instead of stepping page numbers.

**Keys:**

| Key | Action |
| --- | --- |
| `0`–`9` | Type a page number (3 digits → load immediately) |
| Backspace | Remove last typed digit |
| `↑` `↓` | Move between input field, links, and subpage selectors |
| `←` `→` | Page ±1 (or cycle subpages when on a subpage selector) |
| Enter / Space | Follow the selected link, or switch subpage |
| `q` / Esc | Quit |

A freshly-loaded page has no active link until you press an arrow key.
The input field is reachable via `↑` from the topmost page link.

Interactive mode always uses teletext rendering — image modes are not
compatible with link scanning. `--mode` and `--source` flags are ignored
(with a one-line stderr warning) if passed alongside `-i`. If a page
doesn't exist, a centered "Sidan finns inte" placeholder renders in
place of content and `←/→` keep working so you can step past gaps in
SVT's numbering. The terminal must be at least 41 columns × 26 rows.

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

Decoded mosaic patterns persist to the OS cache directory under
`texttv/mosaics/` (Linux: `$XDG_CACHE_HOME/...` or `~/.cache/...`; macOS:
`~/Library/Caches/...`; Windows: `%LOCALAPPDATA%\...`) so a given pattern is
fetched once across all runs.

This project is not affiliated with, endorsed by, or sponsored by SVT or
texttv.nu. It's an independent reader for publicly broadcast teletext data.
Please respect the upstream terms of service when using it — keep your
requests reasonable, don't strip the User-Agent or the `app=` parameter, and
don't use this tool or any part of this tool to overload either service.

# Installation

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

`texttv` requires at least Rust 1.85+, but the latest version is suggested. On most machines, Rust is very easy to install. Use [Rustup](https://rustup.rs/) rather than your package manager to install it though, as this ensures you'll get the latest version.

# Misc

## Help! I don't know any Swedish!

Obvious perhaps, but SVT Text-TV is in Swedish. While a built-in translation engine is
out of scope for this application, there are ways around this if you
don't speak Swedish but are keen to read what's happening:

1. Learn Swedish. It's not the easiest language to learn, to be honest, but there are many [online resources available](https://studyinsweden.se/moving-to-sweden/learn-swedish/).

2. Pipe the output of `texttv` into a translation engine or LLM, or you can use terminal-based translation engines such as [translate-shell](https://github.com/soimort/translate-shell). With it installed, pipe the output of `texttv` into the translation engine:

```
texttv 100 | trans -b sv:en
```

It's not going to be perfect, but you'll get the gist of it.

## Note on AI use

The author of this application has been writing code for over 30 years. Lately, 
LLM agent-enhanced coding practices have rekindled my sense of awe at what's 
possible. This project has been built using a range of tools, including Anthropic's Claude Code (using Opus 4.7).

Unlike some who dismiss anything touched by a coding agent as "slop," I don't 
see it that way. To me, these tools are a way to move much faster, explore 
many more ideas, and test those ideas and implementations more rigorously 
than I ever could on my own.

## License

MIT
