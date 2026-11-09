//! The global shortcuts, as the settings have them (⌃⇧4 region, ⌃⇧5 window,
//! ⌃⇧3 full screen by default). Registered again whenever they change.

use std::sync::{Arc, Mutex};

use gannet_core::settings::{Action, Shortcuts};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui_kit::*;

use crate::Command;
use crate::capture::Mode;

/// The registered shortcuts, kept for the app's life.
struct Registered {
    manager: GlobalHotKeyManager,
    keys: Vec<HotKey>,
    /// Hot key id to capture, shared with the event handler.
    routes: Arc<Mutex<Vec<(u32, Mode)>>>,
}

impl Global for Registered {}

pub fn mode(a: Action) -> Mode {
    match a {
        Action::Region => Mode::Region,
        Action::Window => Mode::Window,
        Action::Screen => Mode::Screen,
    }
}

/// Start listening; nothing is registered until [`apply`].
pub fn install(tx: flume::Sender<Command>, cx: &mut App) {
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(e) => return log::warn!("no global shortcuts: {e}"),
    };
    let routes: Arc<Mutex<Vec<(u32, Mode)>>> = Arc::default();
    let shared = routes.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |ev: GlobalHotKeyEvent| {
        if ev.state != HotKeyState::Pressed {
            return;
        }
        let mode = shared.lock().ok().and_then(|r| r.iter().find(|(id, _)| *id == ev.id).map(|(_, m)| *m));
        if let Some(mode) = mode {
            let _ = tx.send(Command::Capture(mode));
        }
    }));
    cx.set_global(Registered { manager, keys: Vec::new(), routes });
}

/// Register `shortcuts` in place of the ones before. Returns the captures
/// whose shortcut couldn't be registered, and why.
pub fn apply(shortcuts: &Shortcuts, cx: &mut App) -> Vec<(Action, String)> {
    let mut failed = Vec::new();
    if !cx.has_global::<Registered>() {
        return failed;
    }
    let reg = cx.global_mut::<Registered>();
    unregister(reg);
    let mut routes = Vec::new();
    for a in Action::ALL {
        let Some(s) = shortcuts.get(a) else { continue };
        let key = match s.to_string().parse::<HotKey>() {
            Ok(k) => k,
            Err(e) => {
                failed.push((a, e.to_string()));
                continue;
            },
        };
        match reg.manager.register(key) {
            Ok(()) => {
                reg.keys.push(key);
                routes.push((key.id(), mode(a)));
            },
            Err(e) => {
                log::warn!("can't register {}: {e}", s.label());
                failed.push((a, "Another app already uses this shortcut".to_owned()));
            },
        }
    }
    if let Ok(mut r) = reg.routes.lock() {
        *r = routes;
    }
    failed
}

/// Let go of every shortcut (while one is being recorded), until the next
/// [`apply`].
pub fn suspend(cx: &mut App) {
    if cx.has_global::<Registered>() {
        unregister(cx.global_mut::<Registered>());
    }
}

fn unregister(reg: &mut Registered) {
    for key in reg.keys.drain(..) {
        let _ = reg.manager.unregister(key);
    }
    if let Ok(mut r) = reg.routes.lock() {
        r.clear();
    }
}
