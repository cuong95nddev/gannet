//! The settings while the app runs: loaded at start, and on each change
//! saved, the shortcuts registered again and the menu brought up to date.

use std::path::PathBuf;

use gannet_core::settings::{self, Action, Settings};
use gpui_kit::*;

use crate::{capture, hotkeys, status_item, toolbar};

pub struct Prefs {
    pub settings: Settings,
    /// Shortcuts that couldn't be registered, and why.
    pub failed: Vec<(Action, String)>,
}

impl Global for Prefs {}

impl Prefs {
    pub fn failure(&self, a: Action) -> Option<&str> {
        self.failed.iter().find(|(f, _)| *f == a).map(|(_, why)| why.as_str())
    }
}

pub fn init(cx: &mut App) {
    let (settings, err) = Settings::load(&settings::settings_path());
    if let Some(err) = err {
        capture::notify("Couldn't read Gannet's settings", &format!("Using the defaults. {err}"), cx);
    }
    cx.set_global(Prefs { settings, failed: Vec::new() });
    reapply(cx);
}

pub fn get(cx: &App) -> &Settings {
    &cx.global::<Prefs>().settings
}

/// Where captures are saved.
pub fn captures_dir(cx: &App) -> PathBuf {
    get(cx).captures_dir()
}

/// Change the settings, save them and put them to work.
pub fn update(cx: &mut App, f: impl FnOnce(&mut Settings)) {
    let mut settings = get(cx).clone();
    f(&mut settings);
    if settings == *get(cx) {
        return reapply(cx);
    }
    if let Err(e) = settings.save(&settings::settings_path()) {
        capture::notify("Couldn't save Gannet's settings", &e, cx);
    }
    cx.global_mut::<Prefs>().settings = settings;
    reapply(cx);
}

/// Change a setting that only the window it belongs to reads, and save
/// it, without putting the rest to work again.
pub fn remember(cx: &mut App, f: impl FnOnce(&mut Settings)) {
    let mut settings = get(cx).clone();
    f(&mut settings);
    if settings == *get(cx) {
        return;
    }
    if let Err(e) = settings.save(&settings::settings_path()) {
        capture::notify("Couldn't save Gannet's settings", &e, cx);
    }
    cx.global_mut::<Prefs>().settings = settings;
}

/// Register the shortcuts again and refresh the menu (also after a
/// recording, which let go of them).
pub fn reapply(cx: &mut App) {
    let shortcuts = get(cx).shortcuts.clone();
    let failed = hotkeys::apply(&shortcuts, cx);
    let s = get(cx);
    status_item::refresh(&shortcuts, s.toolbar, s.toolbar && s.toolbar_place == settings::ToolbarPlace::MenuBar);
    cx.global_mut::<Prefs>().failed = failed;
    toolbar::sync(cx);
}
