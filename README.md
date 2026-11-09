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
`Enter` (or **Done**) copies the capture and saves it to `~/Pictures/Gannet` (or the folder
set in Settings);
`⌘C` only copies; `⌘S` saves to a file you pick; `Esc` cancels.

The same captures are in the menu bar item's menu, with **Settings…**:
**Shortcuts** (click one and press the new keys), **Captures** (the folder
they are saved to) and **Permissions** (Screen Recording and Notifications,
with a way to each in System Settings). Settings are kept in
`~/Library/Application Support/dev.gannet/settings.json`.

## Build and run

```sh
cargo run -p gannet                    # menu bar app
cargo run -p gannet -- --capture region   # start a capture right away (also window, screen)
cargo run -p gannet -- --settings         # open the Settings window
cargo test
```

macOS asks for **Screen Recording** access on the first capture (System
Settings ▸ Privacy & Security ▸ Screen Recording). Run from a terminal, the
permission belongs to the terminal app.

`scripts/bundle.sh` builds `dist/Gannet.app` (release, no Dock icon), which
macOS needs for notifications and to hold the permission itself.
`scripts/install.sh` builds it, installs it into `/Applications` (or
`$GANNET_INSTALL_DIR`) over a running copy and launches it; `--no-open` skips
the launch. It signs with the first "Apple Development" identity in the
keychain, so Screen Recording stays allowed across rebuilds (an ad hoc
signature is the binary's hash, which every build changes);
`GANNET_DEV_SIGN_ID` picks another. When the identity changes, clear the old
grant with `tccutil reset ScreenCapture dev.gannet` and allow it again.

## Release

Push a tag matching the version in `Cargo.toml` (`v0.1.0`): the Release
workflow builds a universal DMG with `scripts/release.sh` and publishes it as
a GitHub release (`v0.2.0-rc1` makes a prerelease). The CI workflow runs the
checks on the same tag.

The DMG is signed and notarized when these repository secrets are set (the
same ones as Tern's); without them it holds an ad hoc signed app that
Gatekeeper warns about:

| Secret | What |
| ------ | ---- |
| `MACOS_CERT_P12` | The Developer ID Application certificate and key, exported as .p12, base64 |
| `MACOS_CERT_PASSWORD` | That .p12's password |
| `MACOS_SIGN_ID` | The identity, e.g. `Developer ID Application: Name (TEAMID)` |
| `NOTARY_KEY` | An App Store Connect API key (.p8), base64 |
| `NOTARY_KEY_ID` | That key's ID |
| `NOTARY_ISSUER` | Its issuer ID |

Locally, `GANNET_SIGN_ID` and `GANNET_NOTARY_PROFILE` (a
`xcrun notarytool store-credentials` profile) do the same for
`scripts/release.sh`.

## Website

`site/` is the website, published to GitHub Pages by the Pages workflow on
each push to `main` that changes it (Settings ▸ Pages ▸ Source: GitHub
Actions): <https://cuong95nddev.github.io/gannet/>.

## Layout

- `crates/gannet-core`: no UI. Screen grabs (`capture`), the markup model
  (`model`), the exported image (`raster`), where captures go (`store`),
  the user's settings (`settings`).
- `crates/gannet`: the app. Menu bar item, global shortcuts, the capture
  overlay, the Settings window.
- `scripts/`: bundle, install and release. `site/`: the website.

## License

MIT
