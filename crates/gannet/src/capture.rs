//! A capture from start to end: freeze every display, put the overlay on
//! each, and deliver the result (clipboard, the captures folder).

use std::time::Duration;

use gannet_core::{capture as grab, store};
use gpui_kit::*;
use image::RgbaImage;

use crate::overlay::Overlay;

/// What a capture takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// A region dragged over the frozen screen.
    Region,
    /// A window, picked by macOS's own window picker.
    Window,
    /// Every display, whole.
    Screen,
}

/// Raw grabs are kept at most this long (they are deleted once read).
const GRABS_KEEP: Duration = Duration::from_secs(3600);

/// The capture on screen, if one is.
#[derive(Default)]
struct Capture {
    on: bool,
    overlays: Vec<AnyWindowHandle>,
}

impl Global for Capture {}

pub fn init(cx: &mut App) {
    cx.set_global(Capture::default());
    cx.background_executor().spawn(async { store::cleanup(&store::grabs_dir(), GRABS_KEEP) }).detach();
}

pub fn start(mode: Mode, cx: &mut App) {
    if cx.global::<Capture>().on {
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
    match mode {
        Mode::Window => pick_window(cx),
        Mode::Region | Mode::Screen => grab_displays(mode, cx),
    }
}

/// One display's frozen screen.
struct Grab {
    id: DisplayId,
    bounds: Bounds<Pixels>,
    image: Result<RgbaImage, String>,
}

fn grab_displays(mode: Mode, cx: &mut App) {
    let displays: Vec<_> = cx.displays().iter().map(|d| (d.id(), d.bounds())).collect();
    cx.spawn(async move |cx| {
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
                cx.global_mut::<Capture>().on = false;
                let images: Vec<_> =
                    grabs.into_iter().filter_map(|g| g.image.map_err(|e| log::error!("screen capture failed: {e}")).ok()).collect();
                if images.is_empty() {
                    notify("Couldn't capture the screen", "See the log for why.", cx);
                }
                for img in images {
                    deliver(img, cx);
                }
            },
            _ => open_overlays(grabs, cx),
        });
    })
    .detach();
}

fn pick_window(cx: &mut App) {
    cx.spawn(async move |cx| {
        let picked = cx.background_executor().spawn(async move { grab::pick_window(false) }).await;
        cx.update(|cx| {
            cx.global_mut::<Capture>().on = false;
            match picked {
                Ok(Some(img)) => deliver(img, cx),
                Ok(None) => {},
                Err(e) => notify("Couldn't capture the window", &e, cx),
            }
        });
    })
    .detach();
}

fn open_overlays(grabs: Vec<Grab>, cx: &mut App) {
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
            |window, cx| cx.new(|cx| Overlay::new(img, size.width.as_f32(), window, cx)),
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
        cx.global_mut::<Capture>().on = false;
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
fn ns_view(window: &Window) -> Option<std::ptr::NonNull<std::ffi::c_void>> {
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
        let capture = cx.global_mut::<Capture>();
        capture.on = false;
        for w in std::mem::take(&mut capture.overlays) {
            let _ = w.update(cx, |_, w, _| w.remove_window());
        }
    });
}

/// A finished capture: onto the clipboard and into the captures folder.
pub fn deliver(img: RgbaImage, cx: &mut App) {
    match store::png_bytes(&img) {
        Ok(bytes) => cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, bytes))),
        Err(e) => return notify("Couldn't copy the capture", &e, cx),
    }
    cx.spawn(async move |cx| {
        let saved = cx.background_executor().spawn(async move { store::save(&img, &store::captures_dir()) }).await;
        cx.update(|cx| match saved {
            Ok(path) => log::info!("saved {}", path.display()),
            Err(e) => notify("Couldn't save the capture", &e, cx),
        });
    })
    .detach();
}

/// `img` to a file the user picks.
pub fn save_as(img: RgbaImage, cx: &mut App) {
    let rx = cx.prompt_for_new_path(&store::captures_dir(), Some("Capture.png"));
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(path))) = rx.await else { return };
        if let Err(e) = img.save_with_format(&path, image::ImageFormat::Png) {
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
