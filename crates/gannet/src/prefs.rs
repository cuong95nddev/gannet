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
    /// Gannet is running, not still being set up (onboarding): only then
    /// are the shortcuts registered and the toolbar shown.
    live: bool,
}

impl Global for Prefs {}

impl Prefs {
    pub fn failure(&self, a: Action) -> Option<&str> {
        self.failed.iter().find(|(f, _)| *f == a).map(|(_, why)| why.as_str())
    }
}

/// Read the settings, and put them to work.
pub fn init(cx: &mut App) {
    load(cx);
    cx.global_mut::<Prefs>().live = true;
    reapply(cx);
}

/// Read the settings, if they haven't been, without putting them to work
/// (for onboarding, before Gannet runs).
pub fn load(cx: &mut App) {
    if cx.has_global::<Prefs>() {
        return;
    }
    let (settings, err) = Settings::load(&settings::settings_path());
    if let Some(err) = err {
        capture::notify("Couldn't read Gannet's settings", &format!("Using the defaults. {err}"), cx);
    }
    cx.set_global(Prefs { settings, failed: Vec::new(), live: false });
}

/// Whether Gannet has been set up here before: its settings were saved.
pub fn set_up_before() -> bool {
    settings::settings_path().exists()
}

/// Save the settings as they are, changed or not (the end of onboarding:
/// from then on Gannet has been set up).
pub fn save(cx: &mut App) {
    if let Err(e) = get(cx).save(&settings::settings_path()) {
        capture::notify("Couldn't save Gannet's settings", &e, cx);
    }
}

/// Save changed settings; not yet during the first onboarding, which saves
/// them at its end (a file there means Gannet has been set up).
fn keep(settings: &Settings, cx: &mut App) {
    if !cx.global::<Prefs>().live && !set_up_before() {
        return;
    }
    if let Err(e) = settings.save(&settings::settings_path()) {
        capture::notify("Couldn't save Gannet's settings", &e, cx);
    }
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
    keep(&settings, cx);
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
    keep(&settings, cx);
    cx.global_mut::<Prefs>().settings = settings;
}

/// Register the shortcuts again and refresh the menu (also after a
/// recording, which let go of them).
pub fn reapply(cx: &mut App) {
    if !cx.global::<Prefs>().live {
        return;
    }
    let shortcuts = get(cx).shortcuts.clone();
    let failed = hotkeys::apply(&shortcuts, cx);
    let s = get(cx);
    status_item::refresh(&shortcuts, s.toolbar, s.toolbar && s.toolbar_place == settings::ToolbarPlace::MenuBar);
    cx.global_mut::<Prefs>().failed = failed;
    toolbar::sync(cx);
}
