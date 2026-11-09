//! A capture from start to end: freeze every display, put the overlay on
//! each, and deliver the result (clipboard, the library).

use std::time::Duration;

use gannet_core::{capture as grab, store};
use gpui_kit::*;
use image::RgbaImage;

use crate::overlay::Overlay;
use crate::{library, prefs, scroll, toolbar};

/// What a capture takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// A region dragged over the frozen screen.
    Region,
    /// A window, picked by macOS's own window picker.
    Window,
    /// Every display, whole.
    Screen,
    /// A region picked like [`Mode::Region`], then grabbed again and again
    /// while what is in it scrolls, stitched into one tall picture.
    Scroll,
}

/// Raw grabs are kept at most this long (they are deleted once read).
const GRABS_KEEP: Duration = Duration::from_secs(3600);

/// The capture on screen, if one is.
#[derive(Default)]
struct Capture {
    on: bool,
    /// The library window steps aside for this capture (it was asked for
    /// from the library).
    hide_library: bool,
    overlays: Vec<AnyWindowHandle>,
}

impl Global for Capture {}

pub fn init(cx: &mut App) {
    cx.set_global(Capture::default());
    cx.background_executor().spawn(async { store::cleanup(&store::grabs_dir(), GRABS_KEEP) }).detach();
}

/// A capture asked for from the library, whose window is put away until
/// it is over.
pub fn start_from_library(mode: Mode, cx: &mut App) {
    cx.global_mut::<Capture>().hide_library = true;
    start(mode, cx);
}

pub fn start(mode: Mode, cx: &mut App) {
    let hide_library = std::mem::take(&mut cx.global_mut::<Capture>().hide_library);
    if cx.global::<Capture>().on {
        // The shortcut again ends a scrolling capture, keeping it.
        if mode == Mode::Scroll {
            scroll::finish(true, cx);
        }
        return;
    }
    if !grab::has_permission() {
        grab::request_permission();
        notify(
            "Gannet needs Screen Recording access",
            "Allow Gannet in System Settings ▸ Privacy & Security ▸ Screen Recording, then try again.",
            cx,
        );
        grab::open_permission_settings();
        return;
    }
    cx.global_mut::<Capture>().on = true;
    // Out of the way of the grab and of the window picker.
    let toolbar_away = toolbar::step_aside(cx);
    let library_away = hide_library && library::step_aside(cx);
    let settle = if toolbar_away || library_away { SETTLE } else { Duration::ZERO };
    match mode {
        Mode::Window => pick_window(settle, cx),
        Mode::Region | Mode::Screen | Mode::Scroll => grab_displays(mode, settle, cx),
    }
}

/// How long the window server takes to drop a window ordered out.
const SETTLE: Duration = Duration::from_millis(120);

/// The capture is over: another may start, and the toolbar (and the
/// library, if it stepped aside) come back.
pub(crate) fn ended(cx: &mut App) {
    cx.global_mut::<Capture>().on = false;
    toolbar::come_back(cx);
    library::come_back(cx);
}

/// One display's frozen screen.
struct Grab {
    id: DisplayId,
    bounds: Bounds<Pixels>,
    image: Result<RgbaImage, String>,
}

fn grab_displays(mode: Mode, settle: Duration, cx: &mut App) {
    let displays: Vec<_> = cx.displays().iter().map(|d| (d.id(), d.bounds())).collect();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(settle).await;
        let grabs = cx
            .background_executor()
            .spawn(async move {
                std::thread::scope(|s| {
                    let jobs: Vec<_> = displays
                        .into_iter()
                        .map(|(id, bounds)| s.spawn(move || Grab { id, bounds, image: grab::grab_display(u64::from(id) as u32) }))
                        .collect();
                    jobs.into_iter().filter_map(|j| j.join().ok()).collect::<Vec<_>>()
                })
            })
            .await;
        cx.update(|cx| match mode {
            Mode::Screen => {
                ended(cx);
                let images: Vec<_> = grabs
                    .into_iter()
                    .filter_map(|g| {
                        let img = g.image.map_err(|e| log::error!("screen capture failed: {e}")).ok()?;
                        let width = g.bounds.size.width.as_f32();
                        let scale = if width > 0. { img.width() as f32 / width } else { 1. };
                        Some((img, scale))
                    })
                    .collect();
                if images.is_empty() {
                    notify("Couldn't capture the screen", "See the log for why.", cx);
                }
                for (img, scale) in images {
                    deliver(img, scale, true, cx);
                }
            },
            _ => open_overlays(grabs, mode, cx),
        });
    })
    .detach();
}

fn pick_window(settle: Duration, cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(settle).await;
        let picked = cx.background_executor().spawn(async move { grab::pick_window(false) }).await;
        cx.update(|cx| {
            ended(cx);
            match picked {
                Ok(Some(img)) => deliver(img, main_screen_scale(), true, cx),
                Ok(None) => {},
                Err(e) => notify("Couldn't capture the window", &e, cx),
            }
        });
    })
    .detach();
}

fn open_overlays(grabs: Vec<Grab>, mode: Mode, cx: &mut App) {
    let mut overlays: Vec<AnyWindowHandle> = Vec::new();
    for Grab { id, bounds, image } in grabs {
        let img = match image {
            Ok(img) => img,
            Err(e) => {
                log::error!("screen capture failed: {e}");
                continue;
            },
        };
        let size = bounds.size;
        let opened = gpui_kit::open_window(
            WindowOptions {
                // macOS places windows relative to their display.
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size))),
                titlebar: None,
                focus: true,
                show: true,
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                display_id: Some(id),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Overlay::new(img, size.width.as_f32(), mode == Mode::Scroll, window, cx)),
        );
        match opened {
            Ok((handle, _)) => {
                // AppKit calls back into GPUI while the window changes, so
                // after this update, from the main queue.
                if let Ok(Some(view)) = handle.update(cx, |_, window, _| ns_view(window)) {
                    let view = view.as_ptr() as usize;
                    dispatch2::DispatchQueue::main().exec_async(move || cover_display(view));
                }
                overlays.push(handle);
            },
            Err(e) => log::error!("can't open the capture overlay: {e}"),
        }
    }
    if overlays.is_empty() {
        ended(cx);
        notify("Couldn't capture the screen", "See the log for why.", cx);
        return;
    }
    // An accessory app's windows come up behind the app in front.
    cx.activate(true);
    if let Some(first) = overlays.first() {
        let _ = first.update(cx, |_, w, _| w.activate_window());
    }
    cx.global_mut::<Capture>().overlays = overlays;
}

/// The window's NSView (AppKit's raw handle).
pub(crate) fn ns_view(window: &Window) -> Option<std::ptr::NonNull<std::ffi::c_void>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::AppKit(appkit) => Some(appkit.ns_view),
        _ => None,
    }
}

/// Put the overlay over its whole display, menu bar and Dock included.
/// GPUI's pop-up is a titled panel, and AppKit keeps a titled window's
/// frame below the menu bar whatever its level. Borderless, it isn't
/// (GPUI's panel class can still become key, so keys reach the overlay).
fn cover_display(view: usize) {
    use objc2_app_kit::{NSScreenSaverWindowLevel, NSView, NSWindowStyleMask};

    // SAFETY: GPUI's AppKit handle is its NSView. This runs on the main
    // queue right after the window opened, before anything can close it.
    let view: &NSView = unsafe { &*(view as *const NSView) };
    let Some(ns_window) = view.window() else { return };
    let Some(screen) = ns_window.screen() else { return };
    ns_window.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
    ns_window.setLevel(NSScreenSaverWindowLevel);
    ns_window.setFrame_display(screen.frame(), true);
}

/// Close the overlays (after the current update, which may be one of theirs).
pub fn close(cx: &mut App) {
    cx.defer(|cx| {
        remove_overlays(cx);
        ended(cx);
    });
}

/// Close the overlays, the capture going on without them (a scrolling
/// capture, which calls [`ended`] when it is over).
pub fn hand_over(cx: &mut App) {
    cx.defer(remove_overlays);
}

fn remove_overlays(cx: &mut App) {
    for w in std::mem::take(&mut cx.global_mut::<Capture>().overlays) {
        let _ = w.update(cx, |_, w, _| w.remove_window());
    }
}

/// A finished capture (`scale` pixels per point): onto the clipboard and
/// into the library, encrypted, until the user exports it. With `edit`, it
/// opens in the editor whatever the setting (a capture taken without the
/// overlay hasn't been marked up yet).
pub fn deliver(img: RgbaImage, scale: f32, edit: bool, cx: &mut App) {
    match store::png_bytes(&img) {
        Ok(bytes) => cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, bytes))),
        Err(e) => notify("Couldn't copy the capture", &e, cx),
    }
    library::keep(img, scale, edit, cx);
}

/// The main display's pixels per point.
pub(crate) fn main_screen_scale() -> f32 {
    let Some(mtm) = objc2::MainThreadMarker::new() else { return 2. };
    objc2_app_kit::NSScreen::mainScreen(mtm).map_or(2., |s| s.backingScaleFactor() as f32)
}

/// The file types a capture can be saved as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Png,
    Jpeg,
    Bmp,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Png, Format::Jpeg, Format::Bmp];

    pub fn label(self) -> &'static str {
        match self {
            Format::Png => "PNG",
            Format::Jpeg => "JPEG",
            Format::Bmp => "BMP",
        }
    }

    fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpg",
            Format::Bmp => "bmp",
        }
    }

    /// Write `img` to `path` in this format (JPEG has no alpha).
    fn write(self, img: &RgbaImage, path: &std::path::Path) -> image::ImageResult<()> {
        match self {
            Format::Png => img.save_with_format(path, image::ImageFormat::Png),
            Format::Jpeg => image::DynamicImage::ImageRgba8(img.clone()).to_rgb8().save_with_format(path, image::ImageFormat::Jpeg),
            Format::Bmp => img.save_with_format(path, image::ImageFormat::Bmp),
        }
    }
}

/// `img` to a file the user picks, as `format`.
pub fn save_as(img: RgbaImage, format: Format, cx: &mut App) {
    let name = format!("Capture.{}", format.ext());
    let rx = cx.prompt_for_new_path(&prefs::captures_dir(cx), Some(&name));
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(path))) = rx.await else { return };
        if let Err(e) = format.write(&img, &path) {
            cx.update(|cx| notify("Couldn't save the capture", &e.to_string(), cx));
        }
    })
    .detach();
}

pub fn notify(title: &str, body: &str, cx: &mut App) {
    log::warn!("{title}: {body}");
    cx.show_system_notification(SystemNotification {
        tag: "gannet".into(),
        title: title.to_owned().into(),
        body: body.to_owned().into(),
        actions: vec![],
    });
}
