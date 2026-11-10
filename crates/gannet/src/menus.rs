//! The app's menus in the menu bar, shown while Gannet is a regular app
//! (a window of its own open, see [`crate::dock`]).

use gannet_core::settings::Action;
use gpui_kit::*;

use crate::Command;
use crate::capture::Mode;

gpui_kit::actions!(
    gannet,
    [About, Settings, Hide, HideOthers, ShowAll, Quit, CloseWindow, Minimize, Region, Window, Screen, Scroll, Library]
);

/// Set the menus and their shortcuts, and what the app-wide items do.
/// (A window closes itself on [`CloseWindow`].)
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-,", Settings, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-m", Minimize, None),
    ]);
    cx.on_action(|_: &About, _| about());
    cx.on_action(|_: &Settings, cx| crate::send(cx, Command::OpenSettings));
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Quit, cx| crate::send(cx, Command::Quit));
    cx.on_action(|_: &Minimize, cx| {
        if let Some(w) = cx.active_window() {
            let _ = w.update(cx, |_, window, _| window.minimize_window());
        }
    });
    cx.on_action(|_: &Region, cx| crate::send(cx, Command::Capture(Mode::Region)));
    cx.on_action(|_: &Window, cx| crate::send(cx, Command::Capture(Mode::Window)));
    cx.on_action(|_: &Screen, cx| crate::send(cx, Command::Capture(Mode::Screen)));
    cx.on_action(|_: &Scroll, cx| crate::send(cx, Command::Capture(Mode::Scroll)));
    cx.on_action(|_: &Library, cx| crate::send(cx, Command::OpenLibrary));
    cx.set_menus([
        Menu::new("Gannet").items([
            MenuItem::action("About Gannet", About),
            MenuItem::separator(),
            MenuItem::action("Settings…", Settings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Gannet", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Gannet", Quit),
        ]),
        Menu::new("Capture").items([
            MenuItem::action(Action::Region.title(), Region),
            MenuItem::action(Action::Window.title(), Window),
            MenuItem::action(Action::Screen.title(), Screen),
            MenuItem::action(Action::Scroll.title(), Scroll),
            MenuItem::separator(),
            MenuItem::action("Open Library", Library),
        ]),
        Menu::new("Window").items([MenuItem::action("Minimize", Minimize), MenuItem::action("Close", CloseWindow)]),
    ]);
}

/// AppKit's standard About panel (name, version and icon from the bundle).
fn about() {
    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    app.orderFrontStandardAboutPanel(None);
}
