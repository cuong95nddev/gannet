//! Grabbing the screen with `/usr/sbin/screencapture`, which every macOS
//! has, and the Screen Recording permission it needs.

use std::path::Path;
use std::process::Command;

use image::RgbaImage;

use crate::store;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGDisplayBounds(display: u32) -> CGRect;
}

/// Whether Gannet may record the screen (other apps' windows show up).
pub fn has_permission() -> bool {
    // SAFETY: no arguments; only reads the TCC state.
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// Ask for Screen Recording access; macOS shows its prompt once.
pub fn request_permission() {
    // SAFETY: no arguments.
    unsafe { CGRequestScreenCaptureAccess() };
}

/// System Settings at Privacy & Security ▸ Screen Recording.
pub fn open_permission_settings() {
    let _ = Command::new("/usr/bin/open").arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture").spawn();
}

/// Display `id` (a `CGDirectDisplayID`) at its pixel resolution. Blocks
/// for a few hundred ms.
pub fn grab_display(id: u32) -> Result<RgbaImage, String> {
    // SAFETY: a stale id only gives an empty rectangle.
    let r = unsafe { CGDisplayBounds(id) };
    if r.w <= 0. || r.h <= 0. {
        return Err(format!("no display {id}"));
    }
    let rect = format!("{},{},{},{}", r.x.round() as i64, r.y.round() as i64, r.w.round() as i64, r.h.round() as i64);
    run(&["-x", "-R", &rect])?.ok_or_else(|| "screencapture wrote nothing".to_owned())
}

/// A window the user clicks, by `screencapture`'s own picker; `None` when
/// they cancel (Esc). Blocks until then.
pub fn pick_window(shadow: bool) -> Result<Option<RgbaImage>, String> {
    // Interactive, window mode, no sound; `-o` leaves the shadow out.
    let args: &[&str] = if shadow { &["-i", "-W", "-x"] } else { &["-i", "-W", "-o", "-x"] };
    run(args)
}

/// Run `screencapture` with `args` into a fresh file and read it back.
/// `None` when it wrote nothing but exited fine (the user cancelled).
fn run(args: &[&str]) -> Result<Option<RgbaImage>, String> {
    let dir = store::grabs_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(store::unique_name("grab"));
    let status = Command::new("/usr/sbin/screencapture").args(args).arg(&file).status().map_err(|e| e.to_string())?;
    if !file.exists() {
        return if status.success() { Ok(None) } else { Err(format!("screencapture failed ({status})")) };
    }
    let img = read(&file);
    let _ = std::fs::remove_file(&file);
    img.map(Some)
}

fn read(file: &Path) -> Result<RgbaImage, String> {
    image::open(file).map(|i| i.to_rgba8()).map_err(|e| format!("can't read the capture: {e}"))
}
