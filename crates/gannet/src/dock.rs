//! Gannet lives in the menu bar, with no Dock icon, except while one of
//! its own windows (the library, Settings) is open: then it is in the Dock
//! and in ⌘Tab like any app, so the window can be found again.

use std::time::Duration;

use gpui_kit::{App, Global};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};
use objc2_foundation::ns_string;

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
        if regular {
            // The menu bar stays the app's before until Gannet is activated
            // after the switch, not in the same turn of the run loop.
            dispatch2::DispatchQueue::main().exec_async(|| claim_menu_bar(true));
        }
    }
}

/// How long activating takes to show in the menu bar before it is tried
/// the other way.
const SETTLE: Duration = Duration::from_millis(150);

/// Make Gannet the app whose menus the menu bar shows. If activating it
/// isn't enough (`first` try), the Dock is activated for a moment and
/// Gannet again: macOS then hands the menu bar over.
fn claim_menu_bar(first: bool) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let me = NSRunningApplication::currentApplication();
    let owns = || NSWorkspace::sharedWorkspace().menuBarOwningApplication().is_some_and(|a| a == me);
    // Back to a menu bar app meanwhile, or already done.
    if NSApplication::sharedApplication(mtm).activationPolicy() != NSApplicationActivationPolicy::Regular || owns() {
        return;
    }
    if first {
        me.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
        after(SETTLE, || claim_menu_bar(false));
        return;
    }
    log::debug!("the menu bar kept another app's menus; going through the Dock");
    if let Some(dock) = NSRunningApplication::runningApplicationsWithBundleIdentifier(ns_string!("com.apple.dock")).firstObject() {
        dock.activateWithOptions(NSApplicationActivationOptions::empty());
    }
    after(Duration::from_millis(50), || {
        NSRunningApplication::currentApplication().activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
    });
}

/// Run `f` on the main queue after `delay`.
fn after(delay: Duration, f: impl FnOnce() + Send + 'static) {
    let when = dispatch2::DispatchTime::try_from(delay).unwrap_or(dispatch2::DispatchTime::NOW);
    let _ = dispatch2::DispatchQueue::main().after(when, f);
}
