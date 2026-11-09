# Gannet

A screen capture tool for macOS that lives in the menu bar, written in Rust
with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).

The gannet is a seabird that spots its catch from high up and dives straight
for it: press a shortcut, pick what you want, done.

Gannet started as the screenshot tool of [Tern](https://github.com/cuong95nddev/tern),
taken out into an app of its own. Its feature set is modelled on
[FastStone Capture](https://www.faststone.org/FSCaptureDetail.htm) (see
[ROADMAP.md](ROADMAP.md)).

## What it does today

| Shortcut | Capture |
| -------- | ------- |
| `⌃⇧4` | A region, dragged over the frozen screen, then marked up |
| `⌃⇧5` | A window, picked by clicking it |
| `⌃⇧3` | Every display, whole |

Over a region: rectangle, ellipse, line, arrow, pen, text, numbered markers
and mosaic (pixelate), in 8 colours and 3 sizes, with undo and redo.
`Enter` (or **Done**) copies the capture and saves it to `~/Pictures/Gannet`;
`⌘C` only copies; `⌘S` saves to a file you pick; `Esc` cancels.

The same captures are in the menu bar item's menu.

## Build and run

```sh
cargo run -p gannet                    # menu bar app
cargo run -p gannet -- --capture region   # start a capture right away (also window, screen)
cargo test
```

macOS asks for **Screen Recording** access on the first capture (System
Settings ▸ Privacy & Security ▸ Screen Recording). Run from a terminal, the
permission belongs to the terminal app.

`scripts/bundle.sh` builds `dist/Gannet.app` (release, no Dock icon), which
macOS needs for notifications and to hold the permission itself.

## Layout

- `crates/gannet-core`: no UI. Screen grabs (`capture`), the markup model
  (`model`), the exported image (`raster`), where captures go (`store`).
- `crates/gannet`: the app. Menu bar item, global shortcuts, the capture
  overlay.

## License

MIT
