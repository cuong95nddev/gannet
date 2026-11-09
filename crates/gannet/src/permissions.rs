//! The macOS permissions Gannet uses: Screen Recording (to capture at all)
//! and Notifications (to say when a capture fails), where they stand and
//! where to change them.

use std::ptr::NonNull;

use gannet_core::capture as grab;
use gpui_kit::*;
use objc2::runtime::Bool;
use objc2_foundation::{NSBundle, NSError};
use objc2_user_notifications::{UNAuthorizationOptions, UNAuthorizationStatus, UNNotificationSettings, UNUserNotificationCenter};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Allowed,
    Denied,
    /// macOS hasn't asked yet.
    NotAsked,
    /// Can't be had here: notifications outside an app bundle.
    Unavailable,
}

pub fn screen_recording() -> Status {
    if grab::has_permission() { Status::Allowed } else { Status::Denied }
}

/// Ask for Screen Recording (macOS prompts once) and show its settings.
pub fn ask_screen_recording() {
    grab::request_permission();
    grab::open_permission_settings();
}

/// `UNUserNotificationCenter` aborts the process outside an app bundle
/// (a `cargo run` build).
fn in_bundle() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

/// Where notifications stand; the answer comes later, from AppKit.
pub fn notifications(tx: flume::Sender<Status>) {
    if !in_bundle() {
        let _ = tx.send(Status::Unavailable);
        return;
    }
    let completion = block2::RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: the settings object is live for the callback.
        let status = unsafe { settings.as_ref() }.authorizationStatus();
        let _ = tx.send(match status {
            UNAuthorizationStatus::NotDetermined => Status::NotAsked,
            UNAuthorizationStatus::Denied => Status::Denied,
            _ => Status::Allowed,
        });
    });
    UNUserNotificationCenter::currentNotificationCenter().getNotificationSettingsWithCompletionHandler(&completion);
}

/// Ask for notifications (macOS prompts once); `done` follows the answer.
pub fn ask_notifications(done: flume::Sender<()>) {
    if !in_bundle() {
        return;
    }
    let completion = block2::RcBlock::new(move |_granted: Bool, error: *mut NSError| {
        // SAFETY: when non-null, `error` is live for the callback.
        if let Some(error) = unsafe { error.as_ref() } {
            log::warn!("notification authorization failed: {}", error.localizedDescription());
        }
        let _ = done.send(());
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &completion);
}

/// System Settings at Notifications ▸ Gannet.
pub fn open_notification_settings() {
    let _ = std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension?id=dev.gannet")
        .spawn();
}

/// The `.app` Gannet runs from, if it runs from one.
fn bundle_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.parent()?.parent()?.parent()?; // Gannet.app/Contents/MacOS/gannet
    app.extension().is_some_and(|e| e == "app").then(|| app.to_path_buf())
}

pub fn can_relaunch() -> bool {
    bundle_path().is_some()
}

/// Quit and open again, so a permission granted while running takes.
pub fn relaunch(cx: &mut App) {
    let Some(app) = bundle_path() else { return };
    let spawned = std::process::Command::new("/bin/sh").args(["-c", "sleep 1; /usr/bin/open \"$0\" --args --settings"]).arg(&app).spawn();
    match spawned {
        Ok(_) => cx.quit(),
        Err(e) => log::error!("can't reopen Gannet: {e}"),
    }
}
