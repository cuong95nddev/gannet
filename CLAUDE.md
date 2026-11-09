Discuss with me in Vietnamese.
Code and comments always in English.

macOS only for now. FastStone Capture is the reference for features: see
ROADMAP.md.

Keep `gannet-core` free of UI so it stays testable; anything that needs a
window goes in `crates/gannet`.

Checks before a commit: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
`cargo run -p gannet -- --capture region` starts a capture without a shortcut,
`--settings` opens the Settings window, `--library` the library. Debug
builds keep their own library (`library-dev`, key in a file) so the
Keychain doesn't ask on every rebuild. `scripts/install.sh` installs the
app; releases come from a `v*` tag (`.github/workflows/release.yml`
lists the signing secrets).
