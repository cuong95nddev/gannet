//! Gannet lives in the menu bar, with no Dock icon, except while one of
//! its own windows (the library, Settings) is open: then it is in the Dock
//! and in ⌘Tab like any app, so the window can be found again.

use gpui_kit::{App, Global};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

/// The windows that bring the Dock icon.
#[derive(Clone, Copy)]
pub enum Window {
    Library,
    Settings,
}

#[derive(Default)]
struct Open {
    library: bool,
    settings: bool,
}

impl Global for Open {}

/// `window` opened or closed: in the Dock while any of them is open.
pub fn set(window: Window, open: bool, cx: &mut App) {
    let o = cx.default_global::<Open>();
    match window {
        Window::Library => o.library = open,
        Window::Settings => o.settings = open,
    }
    let regular = o.library || o.settings;
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let policy = if regular { NSApplicationActivationPolicy::Regular } else { NSApplicationActivationPolicy::Accessory };
    if app.activationPolicy() != policy {
        app.setActivationPolicy(policy);
    }
}
