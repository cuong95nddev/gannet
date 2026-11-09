//! Popups anchored to a spot in another window, in a window of their own,
//! so they can reach past their parent's edges: tooltips now, dropdowns
//! later. This is GPUI's `WindowKind::AnchoredPopup`, which its macOS
//! backend doesn't have yet (zed-industries/zed#64353 adds it); switch to
//! that once it lands.
//!
//! A popup is a GPUI pop-up window made borderless and a child of its
//! parent, so it moves with it. It goes below its anchor, or above when
//! there's no room, and is kept on the parent's screen.

use gpui_kit::*;
use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindow, NSWindowOrderingMode, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use crate::capture;

/// Room between the anchor and the popup.
const GAP: f64 = 4.;

/// An open popup. Closed with [`Popup::close`].
pub struct Popup {
    window: AnyWindowHandle,
}

/// What a popup does with the pointer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Shown only: clicks go through it (a tooltip).
    Passive,
}

/// Open a popup of `size` by `anchor`, a rectangle in `parent`'s content
/// (GPUI's coordinates: points, from the top left).
pub fn open<V: Render + 'static>(
    parent: &Window,
    anchor: Bounds<Pixels>,
    size: Size<Pixels>,
    kind: Kind,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut Context<V>) -> V,
) -> Option<Popup> {
    let parent_view = capture::ns_view(parent)?.as_ptr() as usize;
    let display = parent.display(cx).map(|d| d.id());
    let options = WindowOptions {
        // Placed for real below, from AppKit's frames.
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size))),
        titlebar: None,
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: display,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let (window, _) = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| build(window, cx)))
        .map_err(|e| log::error!("can't open a popup: {e}"))
        .ok()?;
    let view = window.update(cx, |_, w, _| capture::ns_view(w)).ok()??.as_ptr() as usize;
    let anchor = (
        f64::from(anchor.origin.x.as_f32()),
        f64::from(anchor.origin.y.as_f32()),
        f64::from(anchor.size.width.as_f32()),
        f64::from(anchor.size.height.as_f32()),
    );
    let size = (f64::from(size.width.as_f32()), f64::from(size.height.as_f32()));
    // AppKit calls back into GPUI while windows change, so after this
    // update, from the main queue.
    dispatch2::DispatchQueue::main().exec_async(move || attach(parent_view, view, anchor, size, kind));
    Some(Popup { window })
}

impl Popup {
    pub fn close(self, cx: &mut App) {
        let Some(view) = self.window.update(cx, |_, w, _| capture::ns_view(w)).ok().flatten() else { return };
        // SAFETY: GPUI's AppKit handle is its NSView, alive while the window is.
        let ns: Option<Retained<NSWindow>> = unsafe { view.cast::<NSView>().as_ref() }.window();
        if let Some(ns) = ns
            && let Some(parent) = ns.parentWindow()
        {
            parent.removeChildWindow(&ns);
        }
        let _ = self.window.update(cx, |_, w, _| w.remove_window());
    }
}

/// Make the popup borderless, place it by its anchor and show it as a
/// child of the parent.
fn attach(parent_view: usize, view: usize, anchor: (f64, f64, f64, f64), size: (f64, f64), kind: Kind) {
    // SAFETY: GPUI's AppKit handles are their NSViews; this runs on the
    // main queue right after the popup opened, before anything closed it.
    let (parent_view, view): (&NSView, &NSView) = unsafe { (&*(parent_view as *const NSView), &*(view as *const NSView)) };
    let (Some(parent), Some(popup)) = (parent_view.window(), view.window()) else { return };
    let Some(screen) = parent.screen() else { return };
    let pf = parent.frame();
    // The anchor's top left in AppKit's screen coordinates (y up).
    let (ax, ay, aw, ah) = anchor;
    let content_h = parent.contentRectForFrameRect(pf).size.height;
    let rect = Rect { x: pf.origin.x + ax, y: pf.origin.y + content_h - ay - ah, w: aw, h: ah };
    let vf = screen.visibleFrame();
    let visible = Rect { x: vf.origin.x, y: vf.origin.y, w: vf.size.width, h: vf.size.height };
    let (x, y) = place(rect, size, visible);
    popup.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
    popup.setFrame_display(NSRect::new(NSPoint::new(x, y), NSSize::new(size.0, size.1)), true);
    popup.setHasShadow(true);
    popup.setIgnoresMouseEvents(kind == Kind::Passive);
    popup.setLevel(parent.level());
    // SAFETY: both windows are open; the child is removed before it closes.
    unsafe { parent.addChildWindow_ordered(&popup, NSWindowOrderingMode::Above) };
}

/// A rectangle in AppKit's screen coordinates: origin at the bottom left.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// Where a popup of `size` goes by `anchor`: its bottom left. Below the
/// anchor, lined up with its left edge; above it when there's no room
/// below; always inside `visible`.
fn place(anchor: Rect, (w, h): (f64, f64), visible: Rect) -> (f64, f64) {
    let below = anchor.y - GAP - h;
    let above = anchor.y + anchor.h + GAP;
    let y = if below >= visible.y || above + h > visible.y + visible.h { below } else { above };
    let x = anchor.x.min(visible.x + visible.w - w).max(visible.x);
    let y = y.min(visible.y + visible.h - h).max(visible.y);
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0., y: 0., w: 1000., h: 800. };

    // `gpui_kit::*` brings a `test` macro of its own.
    #[::core::prelude::v1::test]
    fn goes_below_its_anchor() {
        let anchor = Rect { x: 100., y: 700., w: 28., h: 36. };
        assert_eq!(place(anchor, (80., 24.), SCREEN), (100., 700. - GAP - 24.));
    }

    #[::core::prelude::v1::test]
    fn flips_above_at_the_bottom() {
        let anchor = Rect { x: 100., y: 10., w: 28., h: 36. };
        assert_eq!(place(anchor, (80., 24.), SCREEN), (100., 46. + GAP));
    }

    #[::core::prelude::v1::test]
    fn stays_on_screen() {
        let anchor = Rect { x: 980., y: 400., w: 28., h: 36. };
        assert_eq!(place(anchor, (80., 24.), SCREEN).0, 920.);
        let anchor = Rect { x: -50., y: 400., w: 28., h: 36. };
        assert_eq!(place(anchor, (80., 24.), SCREEN).0, 0.);
    }
}
