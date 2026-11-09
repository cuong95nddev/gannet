//! The global shortcuts: ⌃⇧4 region, ⌃⇧5 window, ⌃⇧3 full screen (the
//! system's ⌘⇧ ones, with ⌃ in place of ⌘, so they don't clash).

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui_kit::*;

use crate::Command;
use crate::capture::Mode;

const MODS: Modifiers = Modifiers::CONTROL.union(Modifiers::SHIFT);

/// Each shortcut, its command and how menus show it.
pub const KEYS: [(Code, Mode, &str); 3] =
    [(Code::Digit4, Mode::Region, "⌃⇧4"), (Code::Digit5, Mode::Window, "⌃⇧5"), (Code::Digit3, Mode::Screen, "⌃⇧3")];

/// Keeps the shortcuts registered for the app's life.
struct Registered(#[allow(dead_code)] GlobalHotKeyManager);

impl Global for Registered {}

pub fn install(tx: flume::Sender<Command>, cx: &mut App) {
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(e) => return log::warn!("no global shortcuts: {e}"),
    };
    let mut ids = Vec::new();
    for (code, mode, label) in KEYS {
        let key = HotKey::new(Some(MODS), code);
        match manager.register(key) {
            Ok(()) => ids.push((key.id(), mode)),
            Err(e) => log::warn!("can't register {label}: {e}"),
        }
    }
    GlobalHotKeyEvent::set_event_handler(Some(move |ev: GlobalHotKeyEvent| {
        if ev.state != HotKeyState::Pressed {
            return;
        }
        if let Some((_, mode)) = ids.iter().find(|(id, _)| *id == ev.id) {
            let _ = tx.send(Command::Capture(*mode));
        }
    }));
    cx.set_global(Registered(manager));
}

/// The shortcut of `mode`, as menus show it.
pub fn label(mode: Mode) -> &'static str {
    KEYS.iter().find(|(_, m, _)| *m == mode).map_or("", |(_, _, l)| l)
}
