# Gannet

A screen capture tool for macOS that lives in the menu bar, written in Rust
with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).

Press a shortcut, pick what you want, mark it up, done.

**Website:** <https://cuong95nddev.github.io/gannet/>

## Features

| Shortcut | Capture |
| -------- | ------- |
| `⌃⇧4` | A region, dragged over the frozen screen, then marked up |
| `⌃⇧5` | A window, picked by clicking it |
| `⌃⇧3` | Every display, whole |

- Mark up a region with rectangles, ellipses, lines, arrows, pen, text,
  numbered markers and mosaic, in 8 colours and 3 sizes, with undo and redo.
- `Enter` copies the capture and saves it as a PNG; `⌘C` only copies; `⌘S`
  saves to a file you pick; `Esc` cancels.
- **Settings…** in the menu bar item: your own shortcuts, the folder captures
  are saved to (`~/Pictures/Gannet` by default) and the permissions Gannet
  needs.

See [ROADMAP.md](ROADMAP.md) for what's next.

## Install

Download the DMG from the [latest release](https://github.com/cuong95nddev/gannet/releases/latest)
and drag **Gannet** onto **Applications**. macOS 12.3 or later, Apple Silicon
and Intel.

On the first capture, allow Gannet under System Settings ▸ Privacy & Security ▸
Screen Recording, then reopen it (Settings ▸ Permissions ▸ Reopen Gannet).

## Build

```sh
scripts/install.sh                        # build and install into /Applications
cargo run -p gannet -- --capture region   # try a capture (also window, screen)
cargo test
```

## License

MIT
