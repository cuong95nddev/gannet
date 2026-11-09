//! A scrolling capture, once its region is picked on the frozen screen: a
//! frame around the region and a bar by it, while the user scrolls what
//! is in the region. The region is grabbed again and again and the frames
//! are stitched (`gannet_core::stitch`); Done (or the shortcut again)
//! keeps the tall picture.
//!
//! The frame is drawn outside the region and the bar beside it, so
//! neither shows up in a grab. When the region leaves no room on its
//! display, the bar goes inside, at the bottom, and the region grabbed
//! stops above it. Both windows let the pointer through but for the bar's
//! buttons, so the scroll wheel reaches the app under the region.

use std::time::Duration;

use gannet_core::capture as grab;
use gannet_core::model::P;
use gannet_core::stitch::{Push, Stitcher};
use gpui_kit::*;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSStatusWindowLevel, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use crate::capture;
use crate::icons::IconName;
use crate::ui::{bar, icon_button, pill, tip};

/// The frame's width, outside the region.
const EDGE: f64 = 2.;
const BAR_W: f64 = 340.;
const BAR_H: f64 = 40.;
/// Room between the region and the bar, and the bar and the screen's edge.
const GAP: f64 = 8.;
/// The pause between two grabs (a grab takes some 40 ms).
const EVERY: Duration = Duration::from_millis(50);
/// How long the window server takes to drop the overlay.
const SETTLE: Duration = Duration::from_millis(150);

/// A rectangle in AppKit's screen coordinates: origin at the bottom left
/// of the main display, y up.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn ns(self) -> NSRect {
        NSRect::new(NSPoint::new(self.x, self.y), NSSize::new(self.w, self.h))
    }

    /// The four strips `d` wide around it, outside: top, bottom, left, right.
    fn edges(self, d: f64) -> [Rect; 4] {
        let Rect { x, y, w, h } = self;
        [
            Rect { x: x - d, y: y + h, w: w + 2. * d, h: d },
            Rect { x: x - d, y: y - d, w: w + 2. * d, h: d },
            Rect { x: x - d, y, w: d, h },
            Rect { x: x + w, y, w: d, h },
        ]
    }
}

/// The scrolling capture under way.
struct Session {
    /// `None` while it goes on; then whether to keep the picture.
    stop: Option<bool>,
    windows: Vec<AnyWindowHandle>,
    bar: Option<WeakEntity<Bar>>,
}

impl Global for Session {}

/// End the scrolling capture: keep the picture (`keep`) or not.
pub fn finish(keep: bool, cx: &mut App) {
    if let Some(s) = cx.try_global::<Session>()
        && s.stop.is_none()
    {
        cx.global_mut::<Session>().stop = Some(keep);
    }
}

/// Start one over `sel` (points, in `window`, the overlay that covers a
/// display), whose display has `scale` pixels per point. The overlays are
/// closed.
pub fn start(window: &Window, sel: (P, P), scale: f32, cx: &mut App) {
    let Some(places) = places(window, sel) else {
        capture::close(cx);
        return capture::notify("Couldn't start the scrolling capture", "The display the region is on wasn't found.", cx);
    };
    capture::hand_over(cx);
    cx.defer(move |cx| open(places, scale, cx));
}

/// Where the region, its frame and the bar are on screen, and the
/// rectangle to grab in global points (from the top left of the main
/// display).
#[derive(Clone, Copy, Debug)]
struct Places {
    region: Rect,
    bar: Rect,
    grab: (f64, f64, f64, f64),
}

fn places(window: &Window, (lo, hi): (P, P)) -> Option<Places> {
    let view = capture::ns_view(window)?;
    // SAFETY: GPUI's AppKit handle is its NSView, alive while the window is.
    let view: &NSView = unsafe { view.cast().as_ref() };
    let screen = view.window()?.screen()?.frame();
    let mtm = MainThreadMarker::new()?;
    let main_h = NSScreen::screens(mtm).firstObject()?.frame().size.height;
    let screen = Rect { x: screen.origin.x, y: screen.origin.y, w: screen.size.width, h: screen.size.height };
    let region = Rect {
        x: screen.x + f64::from(lo.x),
        y: screen.y + screen.h - f64::from(hi.y),
        w: f64::from(hi.x - lo.x),
        h: f64::from(hi.y - lo.y),
    };
    let (region, bar) = layout(region, screen);
    let grab = (region.x, main_h - region.y - region.h, region.w, region.h);
    Some(Places { region, bar, grab })
}

/// The region to grab and where the bar goes: centred under the region,
/// or over it, or else inside it at the bottom, the region cut short above.
fn layout(region: Rect, screen: Rect) -> (Rect, Rect) {
    let x = (region.x + (region.w - BAR_W) / 2.).clamp(screen.x + GAP, (screen.x + screen.w - BAR_W - GAP).max(screen.x + GAP));
    let below = region.y - EDGE - GAP - BAR_H;
    if below >= screen.y + GAP {
        return (region, Rect { x, y: below, w: BAR_W, h: BAR_H });
    }
    let above = region.y + region.h + EDGE + GAP;
    if above + BAR_H <= screen.y + screen.h - GAP {
        return (region, Rect { x, y: above, w: BAR_W, h: BAR_H });
    }
    let cut = BAR_H + 2. * GAP;
    let bar = Rect { x, y: region.y + GAP, w: BAR_W, h: BAR_H };
    (Rect { y: region.y + cut, h: (region.h - cut).max(1.), ..region }, bar)
}

fn open(places: Places, scale: f32, cx: &mut App) {
    let mut windows = Vec::new();
    // Four solid strips just outside the region: a see-through window
    // over it comes out filled, and would be grabbed.
    for strip in places.region.edges(EDGE) {
        let frame = open_window(strip, true, cx, |_, cx| cx.new(|_| Frame));
        windows.extend(frame.map(|(w, _)| w));
    }
    let bar = open_window(places.bar, false, cx, |_, cx| cx.new(|_| Bar { height: 0, lost: false }));
    let bar_view = bar.as_ref().map(|(_, v)| v.downgrade());
    windows.extend(bar.map(|(w, _)| w));
    cx.set_global(Session { stop: None, windows, bar: bar_view });
    run(places.grab, scale, cx);
}

/// A window of the session at `rect`; `passive`, the pointer goes through
/// it.
fn open_window<V: Render + 'static>(
    rect: Rect,
    passive: bool,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Option<(AnyWindowHandle, Entity<V>)> {
    let options = WindowOptions {
        // Placed for real below, in AppKit's coordinates.
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size(px(rect.w as f32), px(rect.h as f32))))),
        titlebar: None,
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let (handle, view) = gpui_kit::open_window(options, cx, build).map_err(|e| log::error!("can't open a window: {e}")).ok()?;
    let ns = handle.update(cx, |_, w, _| capture::ns_view(w)).ok()??.as_ptr() as usize;
    // AppKit calls back into GPUI while the window changes, so after this
    // update, from the main queue.
    dispatch2::DispatchQueue::main().exec_async(move || place(ns, rect, passive));
    Some((handle, view))
}

fn place(view: usize, rect: Rect, passive: bool) {
    // SAFETY: GPUI's AppKit handle is its NSView; this runs on the main
    // queue right after the window opened, before anything closed it.
    let view: &NSView = unsafe { &*(view as *const NSView) };
    let Some(window) = view.window() else { return };
    crate::capture::borderless(&window);
    window.setFrame_display(rect.ns(), true);
    window.setHasShadow(!passive);
    window.setIgnoresMouseEvents(passive);
    window.setLevel(NSStatusWindowLevel);
    window.orderFrontRegardless();
}

/// Grab, stitch, show the height so far, until the session is stopped.
fn run((x, y, w, h): (f64, f64, f64, f64), scale: f32, cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(SETTLE).await;
        let mut stitcher: Option<Stitcher> = None;
        let mut failed = None;
        loop {
            let stop = cx.update(|cx| cx.try_global::<Session>().map_or(Some(false), |s| s.stop));
            if stop == Some(false) {
                break;
            }
            let (st, pushed) = cx
                .background_executor()
                .spawn(async move {
                    let mut st = stitcher;
                    let pushed = grab::grab_rect(x, y, w, h).map(|frame| match &mut st {
                        Some(s) => s.push(frame),
                        None => {
                            st = Some(Stitcher::new(frame));
                            Push::Same
                        },
                    });
                    (st, pushed)
                })
                .await;
            stitcher = st;
            let height = stitcher.as_ref().map_or(0, Stitcher::height);
            match pushed {
                Err(e) => {
                    failed = Some(e);
                    break;
                },
                Ok(Push::Full) => break,
                Ok(push) => cx.update(|cx| show(height, scale, push == Push::Lost, cx)),
            }
            if stop == Some(true) {
                break;
            }
            cx.background_executor().timer(EVERY).await;
        }
        cx.update(|cx| {
            let keep = cx.try_global::<Session>().and_then(|s| s.stop) != Some(false);
            close(cx);
            if let Some(e) = failed {
                return capture::notify("Couldn't capture the region", &e, cx);
            }
            if keep && let Some(st) = stitcher {
                capture::deliver(st.finish(), scale, true, cx);
            }
        });
    })
    .detach();
}

/// The height so far on the bar, and whether the last frame was lost.
fn show(height: u32, scale: f32, lost: bool, cx: &mut App) {
    let Some(bar) = cx.try_global::<Session>().and_then(|s| s.bar.clone()) else { return };
    let height = (height as f32 / scale.max(1.)).round() as u32;
    let _ = bar.update(cx, |b, cx| {
        if (b.height, b.lost) != (height, lost) {
            (b.height, b.lost) = (height, lost);
            cx.notify();
        }
    });
}

fn close(cx: &mut App) {
    let Some(session) = cx.try_global::<Session>() else { return };
    let windows = session.windows.clone();
    cx.remove_global::<Session>();
    for w in windows {
        let _ = w.update(cx, |_, w, _| w.remove_window());
    }
    capture::ended(cx);
}

/// A strip of the frame around the region.
struct Frame;

impl Render for Frame {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(white())
    }
}

/// The bar by the region: how tall the picture is so far, Cancel, Done.
struct Bar {
    /// In points.
    height: u32,
    lost: bool,
}

impl Render for Bar {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let status = if self.lost {
            "Lost the place: scroll back up a little".to_owned()
        } else if self.height == 0 {
            "Scroll down slowly…".to_owned()
        } else {
            format!("Scroll down slowly · {} pt", self.height)
        };
        bar()
            .top_0()
            .left_0()
            .size_full()
            .pl(px(12.))
            .child(div().flex_1().min_w_0().truncate().text_color(if self.lost { rgb(0xffb340) } else { rgb(0xe8e8ea) }).child(status))
            .child(icon_button("cancel", IconName::X, false, true).tooltip(tip("Cancel")).on_click(|_, _, cx| finish(false, cx)))
            .child(div().w(px(4.)))
            .child(
                pill("done", "Done", true).tooltip(tip("Keep the picture and open it in the editor")).on_click(|_, _, cx| finish(true, cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0., y: 0., w: 1440., h: 900. };

    #[::core::prelude::v1::test]
    fn the_frame_is_outside_the_region() {
        let r = Rect { x: 100., y: 300., w: 600., h: 400. };
        let [top, bottom, left, right] = r.edges(2.);
        assert_eq!(top, Rect { x: 98., y: 700., w: 604., h: 2. });
        assert_eq!(bottom, Rect { x: 98., y: 298., w: 604., h: 2. });
        assert_eq!(left, Rect { x: 98., y: 300., w: 2., h: 400. });
        assert_eq!(right, Rect { x: 700., y: 300., w: 2., h: 400. });
    }

    #[::core::prelude::v1::test]
    fn the_bar_goes_under_the_region() {
        let region = Rect { x: 100., y: 300., w: 600., h: 400. };
        let (r, b) = layout(region, SCREEN);
        assert_eq!(r, region);
        assert_eq!(b.y, 300. - EDGE - GAP - BAR_H);
        assert_eq!(b.x, 100. + (600. - BAR_W) / 2.);
    }

    #[::core::prelude::v1::test]
    fn over_it_at_the_bottom_of_the_screen() {
        let region = Rect { x: 0., y: 5., w: 600., h: 400. };
        let (r, b) = layout(region, SCREEN);
        assert_eq!(r, region);
        assert_eq!(b.y, 405. + EDGE + GAP);
        assert_eq!(b.x, (600. - BAR_W) / 2.);
    }

    #[::core::prelude::v1::test]
    fn inside_it_when_it_is_the_whole_screen() {
        let (r, b) = layout(SCREEN, SCREEN);
        assert_eq!(b.y, GAP);
        assert!(r.y >= b.y + b.h);
        assert_eq!(r.y + r.h, SCREEN.h);
    }
}
