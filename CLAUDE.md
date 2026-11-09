Discuss with me in Vietnamese.
Code and comments always in English.

macOS only for now. Gannet came out of Tern's screenshot tool
(`tern/crates/tern-app/src/shot/`); look there first when porting a feature
(the editor and the tray are not ported yet). FastStone Capture is the
reference for features: see ROADMAP.md.

Keep `gannet-core` free of UI so it stays testable; anything that needs a
window goes in `crates/gannet`.

Checks before a commit: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
`cargo run -p gannet -- --capture region` starts a capture without a shortcut,
`--settings` opens the Settings window. `scripts/install.sh` installs the
app; releases come from a `v*` tag (see README).
