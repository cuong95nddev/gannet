//! A capture from start to end: freeze every display, put the overlay on
//! each, and deliver the result (clipboard, the library).

use std::sync::Arc;
use std::time::Duration;

use gannet_core::{capture as grab, store};
use gpui_kit::*;
use image::RgbaImage;

use crate::overlay::Overlay;
use crate::{library, paint, prefs, scroll, toolbar};

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

/// The capture on screen, if one is.
#[derive(Default)]
struct Capture {
    on: bool,
    /// The library window steps aside for this capture (it was asked for
    /// from the library).
    hide_library: bool,
    overlays: Vec<AnyWindowHandle>,
    /// The app in front when the capture started (not Gannet): it gets
    /// the focus back after it, rather than the library window.
    before: Option<objc2::rc::Retained<objc2_app_kit::NSRunningApplication>>,
}

impl Global for Capture {}

pub fn init(cx: &mut App) {
    cx.set_global(Capture::default());
    // Raw grabs are deleted once read; any still there were left by a
    // crash, and are plain PNGs.
    cx.background_executor()
        .spawn(async {
            store::cleanup(&store::grabs_dir(), Duration::ZERO);
            grab::prepare();
        })
        .detach();
    cx.on_app_quit(|_| {
        store::cleanup(&store::grabs_dir(), Duration::ZERO);
        async {}
    })
    .detach();
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
    cx.global_mut::<Capture>().before = app_in_front();
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
    if let Some(app) = cx.global_mut::<Capture>().before.take() {
        app.activateWithOptions(objc2_app_kit::NSApplicationActivationOptions::empty());
    }
    toolbar::come_back(cx);
    library::come_back(cx);
}

/// The app in front, unless it is Gannet.
fn app_in_front() -> Option<objc2::rc::Retained<objc2_app_kit::NSRunningApplication>> {
    use objc2_app_kit::{NSRunningApplication, NSWorkspace};
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    (app != NSRunningApplication::currentApplication()).then_some(app)
}

/// One display's frozen screen.
struct Grab {
    id: DisplayId,
    bounds: Bounds<Pixels>,
    image: Result<RgbaImage, String>,
    /// The image as the overlay shows it, made beside the grab rather
    /// than on the main thread.
    picture: Option<Arc<RenderImage>>,
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
                        .map(|(id, bounds)| {
                            s.spawn(move || {
                                let image = grab::grab_display(u64::from(id) as u32);
                                let picture = (mode != Mode::Screen).then(|| image.as_ref().ok().map(paint::picture)).flatten();
                                Grab { id, bounds, image, picture }
                            })
                        })
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
                // The window was clicked: it is on the display under the pointer.
                Ok(Some(img)) => deliver(img, pointer_screen_scale(), true, cx),
                Ok(None) => {},
                Err(e) => notify("Couldn't capture the window", &e, cx),
            }
        });
    })
    .detach();
}

fn open_overlays(grabs: Vec<Grab>, mode: Mode, cx: &mut App) {
    let mut overlays: Vec<AnyWindowHandle> = Vec::new();
    for Grab { id, bounds, image, picture } in grabs {
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
            |window, cx| cx.new(|cx| Overlay::new(img, picture, size.width.as_f32(), mode == Mode::Scroll, window, cx)),
        );
        match opened {
            Ok((handle, _)) => {
                // AppKit calls back into GPUI while the window changes, so
                // after this update, from the main queue.
                if let Ok(Some(view)) = handle.update(cx, |_, window, _| ViewRef::of(window)) {
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

/// A window's NSView, kept alive until dropped, to use on the main queue
/// (as `exec_async` wants, sent there, though only the main thread may
/// touch it).
pub(crate) struct ViewRef(dispatch2::MainThreadBound<objc2::rc::Retained<objc2_app_kit::NSView>>);

impl ViewRef {
    /// `window`'s NSView. Main thread only.
    pub(crate) fn of(window: &Window) -> Option<Self> {
        let mtm = objc2::MainThreadMarker::new()?;
        let view = ns_view(window)?;
        // SAFETY: GPUI's AppKit handle is its NSView, alive while the
        // window is (now): retained here, it stays alive after.
        let view = unsafe { objc2::rc::Retained::retain(view.cast::<objc2_app_kit::NSView>().as_ptr()) }?;
        Some(ViewRef(dispatch2::MainThreadBound::new(view, mtm)))
    }

    /// The view; on the main thread, as the main queue's blocks are.
    pub(crate) fn get(&self) -> &objc2_app_kit::NSView {
        self.0.get(objc2::MainThreadMarker::new().expect("on the main thread"))
    }
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
fn cover_display(view: ViewRef) {
    use objc2_app_kit::NSScreenSaverWindowLevel;

    // Closed already (Esc at once): nothing to do.
    let Some(ns_window) = view.get().window() else { return };
    let Some(screen) = ns_window.screen() else { return };
    crate::capture::borderless(&ns_window);
    ns_window.setLevel(NSScreenSaverWindowLevel);
    ns_window.setFrame_display(screen.frame(), true);
}

/// Make a GPUI window borderless and non-activating, keeping it see-through
/// where nothing is drawn: a new style mask leaves AppKit's window opaque,
/// which fills the corners of a rounded bar.
pub(crate) fn borderless(window: &objc2_app_kit::NSWindow) {
    use objc2_app_kit::{NSColor, NSWindowStyleMask};
    window.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
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
    cx.defer(|cx| {
        remove_overlays(cx);
        // Gannet came to the front for the overlays: give the focus back
        // to the app being scrolled, or AppKit brings up the library.
        if let Some(app) = &cx.global::<Capture>().before {
            app.activateWithOptions(objc2_app_kit::NSApplicationActivationOptions::empty());
        }
    });
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
    deliver_with(move || img, scale, edit, cx);
}

/// Like [`deliver`], with the capture still to be made by `make`: it is
/// made and encoded off the main thread (a big one takes a while), and
/// encoded once, for both the clipboard and the library.
pub fn deliver_with(make: impl FnOnce() -> RgbaImage + Send + 'static, scale: f32, edit: bool, cx: &mut App) {
    cx.spawn(async move |cx| {
        let (img, png) = cx
            .background_executor()
            .spawn(async move {
                let img = make();
                let png = store::png_bytes(&img);
                (img, png)
            })
            .await;
        cx.update(|cx| match png {
            Ok(bytes) => {
                cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, bytes.clone())));
                library::keep_encoded(img, Some(bytes), scale, edit, cx);
            },
            Err(e) => {
                notify("Couldn't copy the capture", &e, cx);
                library::keep(img, scale, edit, cx);
            },
        });
    })
    .detach();
}

/// The main display's pixels per point.
pub(crate) fn main_screen_scale() -> f32 {
    let Some(mtm) = objc2::MainThreadMarker::new() else { return 2. };
    objc2_app_kit::NSScreen::mainScreen(mtm).map_or(2., |s| s.backingScaleFactor() as f32)
}

/// The pixels per point of the display under the pointer (the main one's
/// if none is).
fn pointer_screen_scale() -> f32 {
    use objc2_app_kit::{NSEvent, NSScreen};
    let Some(mtm) = objc2::MainThreadMarker::new() else { return main_screen_scale() };
    let at = NSEvent::mouseLocation();
    NSScreen::screens(mtm)
        .iter()
        .find(|s| {
            let f = s.frame();
            (f.origin.x..f.origin.x + f.size.width).contains(&at.x) && (f.origin.y..=f.origin.y + f.size.height).contains(&at.y)
        })
        .map_or_else(main_screen_scale, |s| s.backingScaleFactor() as f32)
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
