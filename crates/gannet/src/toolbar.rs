//! The capture toolbar (FastStone's capture panel): a small bar with a
//! button for each capture. Either it floats above every window, on every
//! Space, dragged by its grip to where AppKit remembers it; or it hangs
//! under the menu bar icon, shown by a click on the icon and gone at the
//! next click elsewhere. It steps aside while a capture is taken, so it
//! never shows up in one.
//!
//! Its tooltips open in popups of their own, so the bar's small window
//! doesn't cut them off. (AppKit's tooltips only show while the app is
//! active, which this bar never makes it.)

use std::time::Duration;

use gannet_core::settings::{Action, Shortcuts, ToolbarPlace};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::capture::{self, Mode};
use crate::icons::{IconName, icon};
use crate::popup::{self, Popup};
use crate::{Command, prefs, status_item};

/// How long the pointer rests on a button before its tooltip shows.
const TIP_DELAY: Duration = Duration::from_millis(500);

const BUTTON: f32 = 28.;
const HEIGHT: f32 = 36.;
const GRIP: f32 = 16.;
const GAP: f32 = 2.;
const BORDER: f32 = 1.;
const RIGHT_PAD: f32 = 6.;
/// A separator line with its margins.
const SEP: f32 = 9.;

/// What the bar holds, left to right after the grip.
#[derive(Clone, Copy)]
enum Item {
    Capture(Action, Mode, IconName),
    OpenFiles,
    OpenClipboard,
    Library,
    Settings,
    Hide,
    Separator,
}

const ITEMS: [Item; 11] = [
    Item::Capture(Action::Region, Mode::Region, IconName::Scan),
    Item::Capture(Action::Window, Mode::Window, IconName::AppWindow),
    Item::Capture(Action::Screen, Mode::Screen, IconName::Monitor),
    Item::Capture(Action::Scroll, Mode::Scroll, IconName::ScrollText),
    Item::Separator,
    Item::OpenFiles,
    Item::OpenClipboard,
    Item::Separator,
    Item::Library,
    Item::Settings,
    Item::Hide,
];

/// The bar's width: grip, items with the gaps between them, padding.
const WIDTH: f32 = {
    let mut w = 2. * BORDER + GRIP + RIGHT_PAD;
    let mut i = 0;
    while i < ITEMS.len() {
        w += GAP + if matches!(ITEMS[i], Item::Separator) { SEP } else { BUTTON };
        i += 1;
    }
    w
};

impl Item {
    fn hint(self, shortcuts: &Shortcuts) -> Option<String> {
        Some(match self {
            Item::Capture(a, _, _) => match shortcuts.get(a) {
                Some(s) => format!("{} ({})", a.title(), s.label()),
                None => a.title().to_owned(),
            },
            Item::OpenFiles => "Open image files…".to_owned(),
            Item::OpenClipboard => "New from the clipboard".to_owned(),
            Item::Library => "Open Library".to_owned(),
            Item::Settings => "Settings…".to_owned(),
            Item::Hide => "Hide".to_owned(),
            Item::Separator => return None,
        })
    }
}

/// Each item's left edge in the bar, in points.
fn lefts() -> [f32; ITEMS.len()] {
    let mut x = BORDER + GRIP;
    ITEMS.map(|item| {
        x += GAP;
        let left = x;
        x += if matches!(item, Item::Separator) { SEP } else { BUTTON };
        left
    })
}

/// The pointer came onto button `i` or left it.
fn hover(i: usize, on: bool, window: &mut Window, cx: &mut App) {
    if !on {
        if cx.default_global::<Toolbar>().hovered == Some(i) {
            cx.global_mut::<Toolbar>().hovered = None;
            hide_tip(cx);
        }
        return;
    }
    // Moving along the bar with a tooltip up shows the next one at once.
    let warm = cx.default_global::<Toolbar>().tip.is_some();
    hide_tip(cx);
    cx.global_mut::<Toolbar>().hovered = Some(i);
    let Some(hint) = ITEMS[i].hint(&prefs::get(cx).shortcuts) else { return };
    let handle = window.window_handle();
    let wait = cx.spawn(async move |cx| {
        if !warm {
            cx.background_executor().timer(TIP_DELAY).await;
        }
        cx.update(|cx| {
            if cx.global::<Toolbar>().hovered != Some(i) {
                return;
            }
            let _ = handle.update(cx, |_, window, cx| show_tip(i, hint, window, cx));
        });
    });
    cx.global_mut::<Toolbar>().tip_wait = Some(wait);
}

fn show_tip(i: usize, hint: String, window: &mut Window, cx: &mut App) {
    const FONT: f32 = 12.;
    let run = TextRun {
        len: hint.len(),
        font: font(".SystemUIFont"),
        color: white(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let text_w = window.text_system().shape_line(hint.clone().into(), px(FONT), &[run], None).width.as_f32();
    let size = size(px(text_w.ceil() + 18.), px(24.));
    let anchor = Bounds::new(point(px(lefts()[i]), px(0.)), gpui_kit::size(px(BUTTON), px(HEIGHT)));
    let tip = popup::open(window, anchor, size, popup::Kind::Passive, cx, |_, _| TipView(hint.into()));
    cx.global_mut::<Toolbar>().tip = tip;
}

fn hide_tip(cx: &mut App) {
    let t = cx.default_global::<Toolbar>();
    t.tip_wait = None;
    if let Some(tip) = t.tip.take() {
        tip.close(cx);
    }
}

struct TipView(SharedString);

impl Render for TipView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .px(px(8.))
            .bg(rgba(0x1e1f22f2))
            .border_1()
            .border_color(rgba(0xffffff26))
            .text_color(rgb(0xe8e8ea))
            .text_size(px(12.))
            .child(self.0.clone())
    }
}

/// Where AppKit keeps the toolbar's frame (in the app's user defaults).
const AUTOSAVE: &str = "GannetCaptureToolbar";

/// An AppKit monitor of clicks in other apps, removed when dropped.
struct ClickMonitor(objc2::rc::Retained<objc2::runtime::AnyObject>);

impl Drop for ClickMonitor {
    fn drop(&mut self) {
        // SAFETY: the object `addGlobalMonitor…` returned, removed once.
        unsafe { objc2_app_kit::NSEvent::removeMonitor(&self.0) };
    }
}

#[derive(Default)]
struct Toolbar {
    window: Option<AnyWindowHandle>,
    /// The bar open is the one under the menu bar icon.
    attached: bool,
    /// Close the bar under the icon at a click elsewhere: in other apps,
    /// and in Gannet's own windows.
    monitors: Vec<ClickMonitor>,
    /// Ordered out for a capture, to come back after it.
    stepped_aside: bool,
    /// The library window is open: the floating bar is ordered out (the
    /// bar under the icon, asked for by a click, shows over it).
    library_open: bool,
    /// The button under the pointer (an index into `ITEMS`).
    hovered: Option<usize>,
    /// The tooltip on screen, and the wait before showing one.
    tip: Option<Popup>,
    tip_wait: Option<Task<()>>,
}

impl Global for Toolbar {}

/// Show or close the toolbar as the settings say.
pub fn sync(cx: &mut App) {
    let s = prefs::get(cx);
    let place = s.toolbar.then_some(s.toolbar_place);
    let (open, attached) = cx.try_global::<Toolbar>().map_or((false, false), |t| (t.window.is_some(), t.attached));
    match (open, attached, place) {
        // Open as it should be; the bar under the icon waits for a click.
        (true, false, Some(ToolbarPlace::Floating))
        | (true, true, Some(ToolbarPlace::MenuBar))
        | (false, _, None | Some(ToolbarPlace::MenuBar)) => {},
        (false, _, Some(ToolbarPlace::Floating)) => show(false, cx),
        (true, _, _) => {
            close(cx);
            sync(cx);
        },
    }
}

/// The menu bar icon was clicked: show the bar under it, or close it.
pub fn toggle_attached(cx: &mut App) {
    let t = cx.default_global::<Toolbar>();
    match (t.window.is_some(), t.attached) {
        (true, true) => close(cx),
        (false, _) => show(true, cx),
        (true, false) => {},
    }
}

/// A click elsewhere: the bar under the icon goes.
pub fn close_attached(cx: &mut App) {
    if cx.try_global::<Toolbar>().is_some_and(|t| t.attached && t.window.is_some()) {
        close(cx);
    }
}

fn close(cx: &mut App) {
    hide_tip(cx);
    let Some(w) = cx.default_global::<Toolbar>().window else { return };
    // Forget the bar only once it is closed: dropped while still open, it
    // would stay on screen (and in captures) out of reach.
    match w.update(cx, |_, w, _| w.remove_window()) {
        Ok(()) => {
            let t = cx.global_mut::<Toolbar>();
            *t = Toolbar { library_open: t.library_open, ..Toolbar::default() };
        },
        Err(e) => log::error!("can't close the capture toolbar: {e}"),
    }
}

fn show(attached: bool, cx: &mut App) {
    let primary = cx.primary_display().map(|d| d.bounds());
    let icon = attached.then(status_item::icon_frame).flatten();
    let origin = match (primary, icon) {
        // Under the icon, centred on it, on the icon's own display (AppKit
        // counts from the bottom of the primary one).
        (Some(b), Some((f, screen))) => {
            let x = (f.origin.x + f.size.width / 2.) as f32 - WIDTH / 2.;
            let (left, right) = (screen.origin.x as f32 + 4., (screen.origin.x + screen.size.width) as f32 - WIDTH - 4.);
            let x = x.min(right).max(left);
            point(px(x), b.size.height - px(f.origin.y as f32) + px(6.))
        },
        (Some(b), None) => point(b.origin.x + (b.size.width - px(WIDTH)) / 2., b.origin.y + px(40.)),
        (None, _) => point(px(200.), px(40.)),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, size(px(WIDTH), px(HEIGHT))))),
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: true,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| ToolbarView { attached })) {
        Ok((handle, _)) => {
            // AppKit calls back into GPUI while the window changes, so
            // after this update, from the main queue.
            let away = !attached && cx.default_global::<Toolbar>().library_open;
            if let Ok(Some(view)) = handle.update(cx, |_, window, _| capture::ViewRef::of(window)) {
                dispatch2::DispatchQueue::main().exec_async(move || float(view, away, attached));
            }
            let monitors = if attached {
                let own = ns_window(handle, cx).map(|w| w.windowNumber());
                watch_clicks(own, crate::sender(cx))
            } else {
                Vec::new()
            };
            let t = cx.default_global::<Toolbar>();
            t.window = Some(handle);
            t.attached = attached;
            t.monitors = monitors;
        },
        Err(e) => log::error!("can't open the capture toolbar: {e}"),
    }
}

/// Close the bar under the icon at a click elsewhere: in another app (a
/// global monitor), or in one of Gannet's own windows other than the bar
/// itself (window number `own`) and the menu bar icon, which toggles it.
fn watch_clicks(own: Option<isize>, tx: flume::Sender<Command>) -> Vec<ClickMonitor> {
    use objc2_app_kit::{NSEvent, NSEventMask};
    let mask = NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown;
    let global = {
        let tx = tx.clone();
        let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSEvent>| {
            let _ = tx.send(Command::CloseAttachedToolbar);
        });
        NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &block)
    };
    let local = {
        let block = block2::RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit hands the monitor a live event.
            let number = unsafe { event.as_ref() }.windowNumber();
            if Some(number) != own && Some(number) != status_item::icon_window_number() {
                let _ = tx.send(Command::CloseAttachedToolbar);
            }
            event.as_ptr()
        });
        // SAFETY: the block passes every event on, unchanged.
        unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) }
    };
    [global, local].into_iter().flatten().map(ClickMonitor).collect()
}

/// Square (borderless: macOS rounds a titled window's corners), above
/// other apps' windows but below menus; floating, back where it was left;
/// off screen if `away` (the library is open).
fn float(view: capture::ViewRef, away: bool, attached: bool) {
    use objc2_app_kit::NSFloatingWindowLevel;
    use objc2_foundation::NSString;

    let Some(window) = view.get().window() else { return };
    let frame = window.frame();
    crate::capture::borderless(&window);
    window.setFrame_display(frame, true);
    window.setLevel(NSFloatingWindowLevel);
    // AppKit's shadow follows the window's square frame (with a light rim
    // in dark mode), which would show around the rounded bar's corners.
    window.setHasShadow(!attached);
    if attached {
        // Clip to the bar's rounded corners, whatever is drawn under them.
        if let Some(content) = window.contentView() {
            content.setWantsLayer(true);
            if let Some(layer) = content.layer() {
                // SAFETY: CALayer's `cornerRadius` setter, with a CGFloat.
                let () = unsafe { objc2::msg_send![&*layer, setCornerRadius: 8.0f64] };
                layer.setMasksToBounds(true);
            }
        }
    }
    if !attached {
        let name = NSString::from_str(AUTOSAVE);
        window.setFrameUsingName(&name);
        window.setFrameAutosaveName(&name);
    }
    if away {
        window.orderOut(None);
    }
}

pub(crate) fn ns_window(handle: AnyWindowHandle, cx: &mut App) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    let view = handle.update(cx, |_, window, _| capture::ns_view(window)).ok()??;
    // SAFETY: GPUI's AppKit handle is its NSView, alive while the window is.
    let view: &objc2_app_kit::NSView = unsafe { view.cast().as_ref() };
    view.window()
}

/// Take the toolbar off screen for a capture. Whether it was on screen
/// (the window server may need a moment before it is gone from a grab).
pub fn step_aside(cx: &mut App) -> bool {
    hide_tip(cx);
    let t = cx.default_global::<Toolbar>();
    let was_shown = t.window.is_some() && !t.stepped_aside && !t.library_open;
    t.stepped_aside = true;
    place(cx);
    was_shown
}

/// Bring the toolbar back after a capture (unless the library is open);
/// the bar under the icon is done with, and goes.
pub fn come_back(cx: &mut App) {
    if !cx.has_global::<Toolbar>() || !std::mem::take(&mut cx.global_mut::<Toolbar>().stepped_aside) {
        return;
    }
    if cx.global::<Toolbar>().attached {
        return close(cx);
    }
    place(cx);
}

/// The library window opened (the toolbar goes away) or closed (it is back).
pub fn library_open(open: bool, cx: &mut App) {
    hide_tip(cx);
    let t = cx.default_global::<Toolbar>();
    if t.library_open == open {
        return;
    }
    t.library_open = open;
    place(cx);
}

/// On screen or off, as the capture and the library want.
fn place(cx: &mut App) {
    let Some(t) = cx.try_global::<Toolbar>() else { return };
    let away = t.stepped_aside || (t.library_open && !t.attached);
    let (Some(handle), away) = (t.window, away) else { return };
    let Some(window) = ns_window(handle, cx) else { return };
    if away {
        window.orderOut(None);
    } else {
        window.orderFrontRegardless();
    }
}
struct ToolbarView {
    /// Under the menu bar icon: no grip, and gone once a button is used.
    attached: bool,
}

/// After this click (closing the bar from inside its own event would fail,
/// as GPUI is updating the window): the bar under the icon goes.
fn done(attached: bool, cx: &mut App) {
    if attached {
        cx.defer(close_attached);
    }
}

impl Render for ToolbarView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let attached = self.attached;
        let grip = div().id("grip").h_full().w(px(GRIP)).flex().items_center().justify_center().text_color(rgba(0xffffff59));
        let grip = if attached {
            grip
        } else {
            grip.cursor(CursorStyle::OpenHand)
                .child(icon(IconName::GripVertical, 14.))
                .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
        };
        let mut bar = div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(GAP))
            .pr(px(RIGHT_PAD))
            .bg(rgba(0x1e1f22f2))
            .border(px(BORDER))
            .border_color(rgba(0xffffff26))
            .text_color(rgb(0xe8e8ea))
            .when(attached, |d| d.rounded(px(8.)))
            .child(grip);
        for (i, item) in ITEMS.into_iter().enumerate() {
            bar = bar.child(match item {
                Item::Separator => div().w(px(1.)).h(px(18.)).mx(px((SEP - 1.) / 2.)).bg(rgba(0xffffff2e)).into_any_element(),
                Item::Capture(_, mode, name) => {
                    button(i, name).on_click(move |_, _, cx| crate::send(cx, Command::Capture(mode))).into_any_element()
                },
                Item::OpenFiles => button(i, IconName::FolderOpen)
                    .on_click(move |_, _, cx| {
                        crate::send(cx, Command::OpenFiles);
                        done(attached, cx);
                    })
                    .into_any_element(),
                Item::OpenClipboard => button(i, IconName::ClipboardPaste)
                    .on_click(move |_, _, cx| {
                        crate::send(cx, Command::OpenClipboard);
                        done(attached, cx);
                    })
                    .into_any_element(),
                Item::Library => button(i, IconName::Images)
                    .on_click(move |_, _, cx| {
                        crate::send(cx, Command::OpenLibrary);
                        done(attached, cx);
                    })
                    .into_any_element(),
                Item::Settings => button(i, IconName::Settings)
                    .on_click(move |_, _, cx| {
                        crate::send(cx, Command::OpenSettings);
                        done(attached, cx);
                    })
                    .into_any_element(),
                // Under the icon, it just closes the bar; floating, it
                // turns the bar off. After this click, as above.
                Item::Hide if attached => button(i, IconName::X).on_click(|_, _, cx| done(true, cx)).into_any_element(),
                Item::Hide => {
                    button(i, IconName::X).on_click(|_, _, cx| cx.defer(|cx| prefs::update(cx, |s| s.toolbar = false))).into_any_element()
                },
            });
        }
        bar
    }
}

fn button(i: usize, name: IconName) -> Stateful<Div> {
    div()
        .id(("item", i))
        .on_hover(move |on, window, cx| hover(i, *on, window, cx))
        .on_mouse_down(MouseButton::Left, |_, _, cx| hide_tip(cx))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(BUTTON))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(0xffffff1f)))
        .active(|s| s.bg(rgba(0xffffff33)))
        .child(icon(name, 17.))
}

#[cfg(test)]
mod tests {
    use super::*;

    // `gpui_kit::*` brings a `test` macro of its own.
    #[::core::prelude::v1::test]
    fn items_fill_the_bar() {
        let lefts = lefts();
        assert_eq!(lefts[0], BORDER + GRIP + GAP);
        // After the separator, the first button of the next group.
        assert_eq!(lefts[5], lefts[3] + BUTTON + GAP + SEP + GAP);
        // The last button ends at the padding.
        assert_eq!(lefts[ITEMS.len() - 1] + BUTTON + RIGHT_PAD + BORDER, WIDTH);
        assert!(ITEMS[0].hint(&Shortcuts::default()).unwrap().starts_with(Action::Region.title()));
        assert!(ITEMS[4].hint(&Shortcuts::default()).is_none());
    }
}
