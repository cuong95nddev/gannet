//! Grabbing the screen, and the Screen Recording permission it needs.
//!
//! On macOS 14 and later a display or a rectangle is grabbed in-process with
//! ScreenCaptureKit: no process to start, no PNG written to disk and read
//! back. Before that, or if ScreenCaptureKit fails, `/usr/sbin/screencapture`
//! does it, as it always did. The window picker is always `screencapture`'s.

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

/// Get ready to grab: what can be worked out before the first grab is
/// (blocks for some 60 ms; call it off the main thread).
pub fn prepare() {
    sck::prepare();
}

/// Display `id` (a `CGDirectDisplayID`) at its pixel resolution. Blocks
/// for a few hundred ms.
pub fn grab_display(id: u32) -> Result<RgbaImage, String> {
    // SAFETY: a stale id only gives an empty rectangle.
    let r = unsafe { CGDisplayBounds(id) };
    if r.w <= 0. || r.h <= 0. {
        return Err(format!("no display {id}"));
    }
    if let Some(img) = sck::grab(r.x, r.y, r.w, r.h) {
        return Ok(img);
    }
    let rect = format!("{},{},{},{}", r.x.round() as i64, r.y.round() as i64, r.w.round() as i64, r.h.round() as i64);
    run(&["-x", "-R", &rect])?.ok_or_else(|| "screencapture wrote nothing".to_owned())
}

/// The rectangle at `x, y` (points, global: from the top left of the main
/// display), `w` by `h` points, at the pixel resolution of its display.
/// Takes some 40 ms.
pub fn grab_rect(x: f64, y: f64, w: f64, h: f64) -> Result<RgbaImage, String> {
    if let Some(img) = sck::grab(x, y, w, h) {
        return Ok(img);
    }
    let rect = format!("{},{},{},{}", x.round() as i64, y.round() as i64, w.round() as i64, h.round() as i64);
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
    // The grabs are plain PNGs until read: the user's alone.
    let dir = store::grabs_dir();
    store::private_dir(&dir)?;
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
    store::open(file).map_err(|e| format!("can't read the capture: {e}"))
}

/// Grabbing with ScreenCaptureKit (macOS 14+).
mod sck {
    use std::ffi::c_void;
    use std::sync::Mutex;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use image::RgbaImage;
    use objc2::AnyThread;
    use objc2::rc::Retained;
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_core_graphics::{CGBitmapContextCreate, CGContext, CGDisplayCopyDisplayMode, CGDisplayMode, CGImage, CGImageAlphaInfo};
    use objc2_foundation::{NSArray, NSError, NSOperatingSystemVersion, NSProcessInfo};
    use objc2_screen_capture_kit::{
        SCCaptureResolutionType, SCContentFilter, SCDisplay, SCScreenshotManager, SCShareableContent, SCStreamConfiguration,
    };

    /// How long a list of the displays is trusted before it is asked for
    /// again (asking takes some 60 ms). It is forgotten at once when the
    /// displays change; this is only in case that is missed.
    const DISPLAYS_KEPT: Duration = Duration::from_secs(600);
    /// How long a grab may take before `screencapture` is tried instead.
    const WAIT: Duration = Duration::from_secs(3);

    /// The displays, as ScreenCaptureKit last listed them.
    struct Displays(Retained<NSArray<SCDisplay>>);
    // SAFETY: SCDisplay is an immutable description of a display; Apple's
    // own samples hand them between queues.
    unsafe impl Send for Displays {}

    static DISPLAYS: Mutex<Option<(Instant, Displays)>> = Mutex::new(None);

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGDisplayRegisterReconfigurationCallback(
            callback: extern "C" fn(display: u32, flags: u32, user_info: *mut c_void),
            user_info: *mut c_void,
        ) -> i32;
    }

    /// A display was added, removed, moved or changed resolution: the
    /// displays are listed again at the next grab.
    extern "C" fn displays_changed(_display: u32, _flags: u32, _user_info: *mut c_void) {
        *DISPLAYS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// List the displays now, so the first grab doesn't wait for it. Only
    /// with the permission: without it, listing them would ask for it.
    pub fn prepare() {
        if supported() && super::has_permission() {
            let _ = displays(false);
        }
    }

    fn supported() -> bool {
        static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *OK.get_or_init(|| {
            NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
                majorVersion: 14,
                minorVersion: 0,
                patchVersion: 0,
            })
        })
    }

    /// The rectangle at `x, y` (points, global), `w` by `h` points, at its
    /// display's pixel resolution; `None` if ScreenCaptureKit can't (an
    /// older macOS, no permission, the rectangle on no display), for the
    /// caller to grab another way.
    pub fn grab(x: f64, y: f64, w: f64, h: f64) -> Option<RgbaImage> {
        if !supported() || w <= 0. || h <= 0. {
            return None;
        }
        let started = Instant::now();
        let img = grab_on(x, y, w, h, false).or_else(|| grab_on(x, y, w, h, true));
        match &img {
            Some(i) => log::debug!("ScreenCaptureKit grabbed {}x{} in {:?}", i.width(), i.height(), started.elapsed()),
            None => log::info!("ScreenCaptureKit couldn't grab; trying screencapture"),
        }
        img
    }

    /// [`grab`], with the displays listed afresh if `fresh`.
    fn grab_on(x: f64, y: f64, w: f64, h: f64, fresh: bool) -> Option<RgbaImage> {
        let displays = displays(fresh)?;
        let (cx, cy) = (x + w / 2., y + h / 2.);
        let display = displays.0.iter().find(|d| {
            // SAFETY: a plain property read.
            let f = unsafe { d.frame() };
            cx >= f.origin.x && cx < f.origin.x + f.size.width && cy >= f.origin.y && cy < f.origin.y + f.size.height
        })?;
        // SAFETY: plain property reads.
        let (frame, id) = unsafe { (display.frame(), display.displayID()) };
        // Pixels per point of the display, as `screencapture` grabs it.
        let mode = CGDisplayCopyDisplayMode(id)?;
        let scale = CGDisplayMode::pixel_width(Some(&mode)) as f64 / frame.size.width.max(1.);
        // The rectangle on the display, in its own points.
        let lx = (x - frame.origin.x).max(0.);
        let ly = (y - frame.origin.y).max(0.);
        let lw = w.min(frame.size.width - lx);
        let lh = h.min(frame.size.height - ly);
        if lw <= 0. || lh <= 0. {
            return None;
        }
        let (pw, ph) = ((lw * scale).round().max(1.) as usize, (lh * scale).round().max(1.) as usize);
        // SAFETY: a filter on one display with no windows left out, and a
        // configuration of plain values.
        let (filter, config) = unsafe {
            let filter = SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &display, &NSArray::new());
            let config = SCStreamConfiguration::new();
            config.setSourceRect(CGRect::new(CGPoint::new(lx, ly), CGSize::new(lw, lh)));
            config.setWidth(pw);
            config.setHeight(ph);
            config.setShowsCursor(false);
            config.setCaptureResolution(SCCaptureResolutionType::Best);
            (filter, config)
        };
        let (tx, rx) = mpsc::sync_channel::<Option<RgbaImage>>(1);
        let done = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
            // SAFETY: the image, if any, lives at least as long as this call.
            let img = unsafe { image.as_ref() }.and_then(to_rgba);
            if img.is_none()
                && let Some(e) = unsafe { error.as_ref() }
            {
                log::warn!("ScreenCaptureKit: {}", e.localizedDescription());
            }
            let _ = tx.try_send(img);
        });
        // SAFETY: the filter and configuration are alive for the call; the
        // block is copied by the framework.
        unsafe { SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(&filter, &config, Some(&done)) };
        rx.recv_timeout(WAIT).ok().flatten()
    }

    /// The displays, from the last listing while it is recent, unless
    /// `fresh`.
    fn displays(fresh: bool) -> Option<Displays> {
        static WATCHED: std::sync::Once = std::sync::Once::new();
        // SAFETY: the callback is a plain function, valid for the process.
        WATCHED.call_once(|| unsafe {
            CGDisplayRegisterReconfigurationCallback(displays_changed, std::ptr::null_mut());
        });
        // Not held while listing: the main thread takes it when the
        // displays change, and mustn't wait on ScreenCaptureKit.
        if !fresh
            && let Some((at, d)) = DISPLAYS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
            && at.elapsed() < DISPLAYS_KEPT
        {
            return Some(Displays(d.0.clone()));
        }
        let (tx, rx) = mpsc::sync_channel::<Option<Displays>>(1);
        let done = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: the content, if any, lives at least as long as this call.
            let d = unsafe { content.as_ref() }.map(|c| Displays(unsafe { c.displays() }));
            if d.is_none()
                && let Some(e) = unsafe { error.as_ref() }
            {
                log::warn!("ScreenCaptureKit can't list the displays: {}", e.localizedDescription());
            }
            let _ = tx.try_send(d);
        });
        // SAFETY: the block is copied by the framework.
        unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&done) };
        let d = rx.recv_timeout(WAIT).ok().flatten()?;
        *DISPLAYS.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), Displays(d.0.clone())));
        Some(d)
    }

    /// `image`'s pixels as RGBA, in its own colour space (no colours
    /// changed: what `screencapture` writes too).
    fn to_rgba(image: &CGImage) -> Option<RgbaImage> {
        let (w, h) = (CGImage::width(Some(image)), CGImage::height(Some(image)));
        if w == 0 || h == 0 {
            return None;
        }
        let space = CGImage::color_space(Some(image))?;
        let mut buf = vec![0u8; w * h * 4];
        // SAFETY: `buf` is w × h × 4 bytes, rows of w × 4, and outlives the
        // context, which is dropped before it is read.
        let ctx = unsafe {
            CGBitmapContextCreate(buf.as_mut_ptr().cast::<c_void>(), w, h, 8, w * 4, Some(&space), CGImageAlphaInfo::PremultipliedLast.0)
        }?;
        CGContext::draw_image(Some(&ctx), CGRect::new(CGPoint::new(0., 0.), CGSize::new(w as f64, h as f64)), Some(image));
        drop(ctx);
        // Screens are opaque; any pixel that isn't, back from premultiplied.
        for px in buf.as_chunks_mut::<4>().0 {
            let a = u16::from(px[3]);
            if a != 255 && a != 0 {
                for c in px.iter_mut().take(3) {
                    *c = ((u16::from(*c) * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        RgbaImage::from_raw(w as u32, h as u32, buf)
    }
}
