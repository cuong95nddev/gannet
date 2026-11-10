//! The Settings window, laid out as macOS's own System Settings: grouped
//! rows, a shortcut recorder for each capture (click it, press the new
//! keys) and a pop-up button for the folder captures are saved to.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;

use gannet_core::settings::{Action, KEEP_CHOICES, Mods, Settings, Shortcut, ToolbarPlace};
use gannet_core::store;
use gpui_kit::component::Theme;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags, NSEventType};

use crate::icons::{self, IconName};
use crate::permissions::{self, Status};
use crate::popup_menu::{self, Item};
use crate::{capture, dock, hotkeys, prefs};

/// macOS's virtual key code for Escape.
const KEY_ESCAPE: u16 = 0x35;

const KEY_CONTEXT: &str = "GannetSettings";

/// The open window, if there is one.
#[derive(Default)]
struct Open(Option<AnyWindowHandle>);

impl Global for Open {}

/// Show the Settings window, opening it if it isn't open.
pub fn open(cx: &mut App) {
    // A regular app before it is active (see `library::show_window`).
    dock::set(dock::Window::Settings, true, cx);
    cx.activate(true);
    if let Some(handle) = cx.try_global::<Open>().and_then(|o| o.0)
        && handle.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(700.), px(460.)), cx))),
        titlebar: Some(TitlebarOptions { title: Some("Gannet Settings".into()), ..Default::default() }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| SettingsView::new(window, cx))) {
        Ok((handle, _)) => cx.set_global(Open(Some(handle))),
        Err(e) => log::error!("can't open the settings window: {e}"),
    }
}

/// The colours macOS draws its settings with, light or dark.
#[derive(Clone, Copy)]
pub(crate) struct Pal {
    pub(crate) window: Hsla,
    pub(crate) sidebar: Hsla,
    pub(crate) group: Hsla,
    pub(crate) group_border: Hsla,
    pub(crate) separator: Hsla,
    pub(crate) text: Hsla,
    pub(crate) secondary: Hsla,
    pub(crate) tertiary: Hsla,
    pub(crate) field: Hsla,
    pub(crate) field_border: Hsla,
    pub(crate) button: Hsla,
    pub(crate) button_border: Hsla,
    /// What is picked or focused: black, or white in the dark.
    pub(crate) strong: Hsla,
    /// Words on `strong`.
    pub(crate) on_strong: Hsla,
    /// The sidebar's page.
    pub(crate) selection: Hsla,
    /// A switch that is on.
    pub(crate) switch: Hsla,
    pub(crate) danger: Hsla,
    pub(crate) warning: Hsla,
    pub(crate) success: Hsla,
}

impl Pal {
    pub(crate) fn new(dark: bool) -> Pal {
        if dark {
            Pal {
                window: rgb(0x1e1e1e).into(),
                sidebar: rgb(0x262626).into(),
                group: rgba(0xffffff0c).into(),
                group_border: rgba(0xffffff0f).into(),
                separator: rgba(0xffffff14).into(),
                text: rgba(0xffffffd9).into(),
                secondary: rgba(0xffffff8c).into(),
                tertiary: rgba(0xffffff40).into(),
                field: rgba(0xffffff0d).into(),
                field_border: rgba(0xffffff1f).into(),
                button: rgba(0xffffff2b).into(),
                button_border: rgba(0xffffff14).into(),
                strong: rgb(0xf2f2f2).into(),
                on_strong: rgb(0x1e1e1e).into(),
                selection: rgba(0xffffff1a).into(),
                switch: rgb(0x9a9a9f).into(),
                danger: rgb(0xff453a).into(),
                warning: rgb(0xff9f0a).into(),
                success: rgb(0x30d158).into(),
            }
        } else {
            Pal {
                window: rgb(0xececec).into(),
                sidebar: rgb(0xe2e2e2).into(),
                group: rgba(0xffffff99).into(),
                group_border: rgba(0x0000000f).into(),
                separator: rgba(0x00000014).into(),
                text: rgba(0x000000d9).into(),
                secondary: rgba(0x00000080).into(),
                tertiary: rgba(0x00000040).into(),
                field: rgb(0xffffff).into(),
                field_border: rgba(0x00000026).into(),
                button: rgb(0xffffff).into(),
                button_border: rgba(0x00000026).into(),
                strong: rgb(0x1d1d1f).into(),
                on_strong: rgb(0xffffff).into(),
                selection: rgba(0x00000014).into(),
                switch: rgb(0x1d1d1f).into(),
                danger: rgb(0xff3b30).into(),
                warning: rgb(0xc77c02).into(),
                success: rgb(0x28a745).into(),
            }
        }
    }
}

pub(crate) fn is_dark(window: &Window) -> bool {
    matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

/// What a key press while recording says.
enum Rec {
    /// The modifiers held changed.
    Flags(Mods),
    /// A key went down: its virtual key code, with the modifiers held.
    Key(u16, Mods),
}

fn mods(flags: NSEventModifierFlags) -> Mods {
    Mods {
        control: flags.contains(NSEventModifierFlags::Control),
        alt: flags.contains(NSEventModifierFlags::Option),
        shift: flags.contains(NSEventModifierFlags::Shift),
        command: flags.contains(NSEventModifierFlags::Command),
    }
}

/// An AppKit monitor that takes the app's key presses (so ⌘W and the
/// like don't act while a shortcut is recorded). Removed when dropped.
struct Monitor(Retained<AnyObject>);

impl Monitor {
    fn start(tx: flume::Sender<Rec>) -> Option<Monitor> {
        let block = block2::RcBlock::new(move |ev: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit hands the monitor a live event.
            let event = unsafe { ev.as_ref() };
            let m = mods(event.modifierFlags());
            if event.r#type() == NSEventType::FlagsChanged {
                let _ = tx.send(Rec::Flags(m));
                return ev.as_ptr();
            }
            let _ = tx.send(Rec::Key(event.keyCode(), m));
            std::ptr::null_mut()
        });
        // SAFETY: the block returns the event or null, as AppKit expects.
        let monitor =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown | NSEventMask::FlagsChanged, &block) };
        monitor.map(Monitor)
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        // SAFETY: the object `addLocalMonitor…` returned, removed once.
        unsafe { NSEvent::removeMonitor(&self.0) };
    }
}

struct Recording {
    action: Action,
    /// The modifiers held right now, shown while recording.
    held: Mods,
    _monitor: Monitor,
    _task: Task<()>,
}

/// The save folder pop-up's choices, in menu order.
#[derive(Clone, Copy)]
enum FolderChoice {
    Current,
    Default,
    Other,
    Reveal,
}

/// The sidebar's pages.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Shortcuts,
    Captures,
    Permissions,
}

impl Page {
    const ALL: [Page; 3] = [Page::Shortcuts, Page::Captures, Page::Permissions];

    fn title(self) -> &'static str {
        match self {
            Page::Shortcuts => "Shortcuts",
            Page::Captures => "Captures",
            Page::Permissions => "Permissions",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Page::Shortcuts => IconName::Keyboard,
            Page::Captures => IconName::FolderDown,
            Page::Permissions => IconName::ShieldCheck,
        }
    }
}

/// A part of the settings shown in another window (onboarding).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Embedded {
    Shortcuts,
    /// Where exported captures go.
    Folder,
}

pub struct SettingsView {
    page: Page,
    /// Shown inside another window: only this part, no sidebar or title.
    embedded: Option<Embedded>,
    screen: Status,
    notifications: Status,
    recording: Option<Recording>,
    /// Why the last keys pressed for a capture weren't taken.
    hint: Option<(Action, String)>,
    /// One per capture, so Tab moves between the recorders.
    fields: Vec<FocusHandle>,
    /// Where the folder pop-up button was last drawn.
    folder_button: Rc<Cell<Bounds<Pixels>>>,
    _subs: Vec<Subscription>,
}

impl SettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.set_window_title(Page::Shortcuts.title());
        dock::set(dock::Window::Settings, true, cx);
        Self::with(None, window, cx)
    }

    /// Part `part` of the settings, for another window to show.
    pub(crate) fn embedded(part: Embedded, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with(Some(part), window, cx)
    }

    fn with(embedded: Option<Embedded>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Light or dark, as macOS is (the component theme starts light).
        Theme::sync_system_appearance(Some(window), cx);
        let subs = vec![
            cx.observe_window_appearance(window, |_, window, cx| Theme::sync_system_appearance(Some(window), cx)),
            cx.observe_global::<prefs::Prefs>(|_, cx| cx.notify()),
            // Clicking away ends a recording, so the shortcuts come back.
            // Coming back (from System Settings, say) reads the permissions again.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.refresh_permissions(cx);
                } else {
                    this.stop(cx);
                }
                cx.notify();
            }),
            cx.on_release(|this, cx| {
                if this.embedded.is_none() {
                    dock::set(dock::Window::Settings, false, cx);
                }
                if this.recording.take().is_some() {
                    prefs::reapply(cx);
                }
            }),
        ];
        let fields = Action::ALL.iter().enumerate().map(|(i, _)| cx.focus_handle().tab_index(i as isize).tab_stop(true)).collect();
        let mut this = Self {
            page: Page::Shortcuts,
            embedded,
            screen: permissions::screen_recording(),
            notifications: Status::NotAsked,
            recording: None,
            hint: None,
            fields,
            folder_button: Rc::default(),
            _subs: subs,
        };
        this.refresh_permissions(cx);
        this
    }

    fn show(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.stop(cx);
        self.page = page;
        window.set_window_title(page.title());
        if page == Page::Permissions {
            self.refresh_permissions(cx);
        }
        cx.notify();
    }

    fn refresh_permissions(&mut self, cx: &mut Context<Self>) {
        self.screen = permissions::screen_recording();
        let (tx, rx) = flume::bounded(1);
        permissions::notifications(tx);
        cx.spawn(async move |this, cx| {
            if let Ok(status) = rx.recv_async().await {
                let _ = this.update(cx, |this, cx| {
                    this.notifications = status;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn ask_notifications(&mut self, cx: &mut Context<Self>) {
        let (tx, rx) = flume::bounded(1);
        permissions::ask_notifications(tx);
        cx.spawn(async move |this, cx| {
            if rx.recv_async().await.is_ok() {
                let _ = this.update(cx, |this, cx| this.refresh_permissions(cx));
            }
        })
        .detach();
    }

    fn record(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.recording.as_ref().is_some_and(|r| r.action == action) {
            return self.stop(cx);
        }
        self.recording = None;
        self.hint = None;
        if let Some(i) = Action::ALL.iter().position(|&a| a == action) {
            window.focus(&self.fields[i], cx);
        }
        // Let go of the shortcuts, or pressing one would capture instead.
        hotkeys::suspend(cx);
        let (tx, rx) = flume::unbounded();
        let Some(monitor) = Monitor::start(tx) else {
            log::error!("can't watch the keyboard to record a shortcut");
            return prefs::reapply(cx);
        };
        let task = cx.spawn(async move |this, cx| {
            while let Ok(rec) = rx.recv_async().await {
                if this.update(cx, |this, cx| this.on_rec(rec, cx)).is_err() {
                    break;
                }
            }
        });
        self.recording = Some(Recording { action, held: Mods::default(), _monitor: monitor, _task: task });
        cx.notify();
    }

    /// End a recording, keeping the shortcut as it was.
    fn stop(&mut self, cx: &mut Context<Self>) {
        if self.recording.take().is_some() {
            prefs::reapply(cx);
            cx.notify();
        }
    }

    fn on_rec(&mut self, rec: Rec, cx: &mut Context<Self>) {
        let Some(r) = &mut self.recording else { return };
        let action = r.action;
        match rec {
            Rec::Flags(m) => r.held = m,
            Rec::Key(KEY_ESCAPE, m) if m == Mods::default() => {
                self.hint = None;
                return self.stop(cx);
            },
            Rec::Key(code, m) => match Shortcut::from_mac(m, code) {
                None => self.hint = Some((action, "That key can't be a shortcut.".to_owned())),
                Some(s) => {
                    if let Some(why) = s.problem() {
                        self.hint = Some((action, format!("{why}.")));
                    } else if let Some(other) = prefs::get(cx).shortcuts.taken_by(s, action) {
                        self.hint = Some((action, format!("{} already uses {}.", other.title(), s.label())));
                    } else {
                        self.hint = None;
                        // Dropping the recording first: `update` registers again.
                        self.recording = None;
                        prefs::update(cx, |st| st.shortcuts.set(action, Some(s)));
                    }
                },
            },
        }
        cx.notify();
    }

    fn set(&mut self, action: Action, s: Option<Shortcut>, cx: &mut Context<Self>) {
        self.recording = None;
        self.hint = None;
        prefs::update(cx, |st| st.shortcuts.set(action, s));
        cx.notify();
    }

    fn restore_defaults(&mut self, cx: &mut Context<Self>) {
        self.recording = None;
        self.hint = None;
        prefs::update(cx, |st| st.shortcuts = Default::default());
        cx.notify();
    }

    fn folder_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stop(cx);
        let Some(view) = capture::ns_view(window) else { return };
        let settings = prefs::get(cx);
        let custom = settings.save_dir.clone();
        let default = store::captures_dir();
        let mut choices = Vec::new();
        let mut items = Vec::new();
        if let Some(dir) = &custom {
            choices.push(FolderChoice::Current);
            items.push(Item::Entry { title: folder_title(dir), checked: true, enabled: true });
        }
        choices.push(FolderChoice::Default);
        items.push(Item::Entry { title: folder_title(&default), checked: custom.is_none(), enabled: true });
        for (choice, item) in [
            (None, Item::Separator),
            (Some(FolderChoice::Other), Item::Entry { title: "Other…".into(), checked: false, enabled: true }),
            (None, Item::Separator),
            (Some(FolderChoice::Reveal), Item::Entry { title: "Show in Finder".into(), checked: false, enabled: true }),
        ] {
            // Separators take an index too, so items and choices line up.
            choices.push(choice.unwrap_or(FolderChoice::Current));
            items.push(item);
        }
        let b = self.folder_button.get();
        let at = (f64::from(b.origin.x.as_f32()) + 4., f64::from(b.origin.y.as_f32()) + 2.);
        let (tx, rx) = flume::bounded(1);
        popup_menu::show(view.as_ptr() as usize, at, items, tx);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(i) = rx.recv_async().await else { return };
            let Some(&choice) = choices.get(i) else { return };
            let _ = this.update_in(cx, |this, window, cx| this.pick_folder(choice, window, cx));
        })
        .detach();
    }

    fn pick_folder(&mut self, choice: FolderChoice, window: &mut Window, cx: &mut Context<Self>) {
        match choice {
            FolderChoice::Current => {},
            FolderChoice::Default => prefs::update(cx, |st| st.save_dir = None),
            FolderChoice::Reveal => {
                let dir = prefs::captures_dir(cx);
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("/usr/bin/open").arg(&dir).spawn();
            },
            FolderChoice::Other => {
                let rx = cx.prompt_for_paths(PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: Some("Choose".into()),
                });
                cx.spawn_in(window, async move |_, cx| {
                    let Ok(Ok(Some(paths))) = rx.await else { return };
                    if let Some(dir) = paths.into_iter().next() {
                        let _ = cx.update(|_, cx| prefs::update(cx, |st| st.save_dir = Some(dir)));
                    }
                })
                .detach();
            },
        }
    }

    fn on_field_key(&mut self, action: Action, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.recording.is_some() {
            return;
        }
        let m = ev.keystroke.modifiers;
        if m.modified() {
            return;
        }
        match ev.keystroke.key.as_str() {
            "space" | "enter" => {
                cx.stop_propagation();
                self.record(action, window, cx);
            },
            "backspace" | "delete" => {
                cx.stop_propagation();
                self.set(action, None, cx);
            },
            _ => {},
        }
    }

    fn shortcut_row(&self, i: usize, action: Action, pal: Pal, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let current = prefs::get(cx).shortcuts.get(action);
        let recording = self.recording.as_ref().filter(|r| r.action == action);
        let note = if let Some((_, why)) = self.hint.as_ref().filter(|(a, _)| *a == action) {
            Some((why.clone(), pal.danger))
        } else if let Some(why) = cx.global::<prefs::Prefs>().failure(action).filter(|_| recording.is_none()) {
            Some((format!("{why}."), pal.danger))
        } else {
            current.filter(|_| recording.is_none()).and_then(|s| s.system_clash()).map(|why| (format!("{why}."), pal.warning))
        };
        let focused = self.fields[i].is_focused(window) && window.is_window_active();
        let (text, color) = match (recording, current) {
            (Some(r), _) if r.held != Mods::default() => (held_label(r.held), pal.text),
            (Some(_), _) => ("Type Shortcut".to_owned(), pal.secondary),
            (None, Some(s)) => (s.label(), pal.text),
            (None, None) => ("Record Shortcut".to_owned(), pal.secondary),
        };
        let clear = (recording.is_none() && current.is_some()).then(|| {
            div()
                .id(("clear", i))
                .absolute()
                .right(px(5.))
                .size(px(14.))
                .rounded_full()
                .bg(pal.tertiary)
                .text_color(pal.window)
                .text_size(px(11.))
                .line_height(px(14.))
                .flex()
                .justify_center()
                .cursor_default()
                .child("×")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.set(action, None, cx);
                }))
        });
        let field = div()
            .id(("shortcut", i))
            .track_focus(&self.fields[i])
            .relative()
            .w(px(160.))
            .h(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .bg(pal.field)
            .border_1()
            .border_color(pal.field_border)
            .when(recording.is_some() || focused, |d| d.border_color(pal.strong).shadow(vec![ring(pal.strong)]))
            .text_size(px(13.))
            .text_color(color)
            .cursor_default()
            .child(text)
            .children(clear)
            .on_key_down(cx.listener(move |this, ev, window, cx| this.on_field_key(action, ev, window, cx)))
            .on_click(cx.listener(move |this, _, window, cx| this.record(action, window, cx)));
        row(pal, i > 0)
            .child(
                v_stack()
                    .flex_1()
                    .gap(px(2.))
                    .child(div().text_size(px(13.)).text_color(pal.text).child(action.title()))
                    .when_some(note, |d, (why, color)| d.child(div().text_size(px(11.)).text_color(color).child(why))),
            )
            .child(field)
            .into_any_element()
    }

    fn folder_row(&self, settings: &Settings, pal: Pal, cx: &mut Context<Self>) -> AnyElement {
        let dir = settings.captures_dir();
        let bounds = self.folder_button.clone();
        let button = push_button("folder", pal)
            .gap(px(6.))
            .pr(px(3.))
            .child(div().max_w(px(180.)).truncate().child(folder_name(&dir)))
            .child(chevrons(pal))
            .child(canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {}).absolute().size_full())
            .on_click(cx.listener(|this, _, window, cx| this.folder_menu(window, cx)));
        row(pal, false)
            .child(
                v_stack()
                    .flex_1()
                    .gap(px(2.))
                    .min_w_0()
                    .child(div().text_size(px(13.)).text_color(pal.text).child("Export to"))
                    .child(div().text_size(px(11.)).text_color(pal.secondary).truncate().child(tilde(&dir))),
            )
            .child(button)
            .into_any_element()
    }
}

/// A segmented control for where the capture toolbar is, if anywhere.
fn toolbar_choices(current: Option<ToolbarPlace>, pal: Pal) -> Div {
    let mut seg = div().flex().p(px(2.)).gap(px(2.)).rounded(px(6.)).bg(pal.field).border_1().border_color(pal.field_border);
    for (i, (place, label)) in
        [(None, "Off"), (Some(ToolbarPlace::Floating), "Floating"), (Some(ToolbarPlace::MenuBar), "Menu Bar")].into_iter().enumerate()
    {
        let on = place == current;
        seg = seg.child(
            div()
                .id(("toolbar", i))
                .px(px(8.))
                .h(px(20.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .text_size(px(12.))
                .text_color(if on { pal.on_strong } else { pal.text })
                .when(on, |d| d.bg(pal.strong))
                .cursor_default()
                .child(label)
                .on_click(move |_, _, cx| {
                    prefs::update(cx, |s| match place {
                        None => s.toolbar = false,
                        Some(p) => {
                            s.toolbar = true;
                            s.toolbar_place = p;
                        },
                    })
                }),
        );
    }
    seg
}

/// A segmented control for how long captures are kept.
/// How many days to keep something, `set` in the settings when picked.
fn keep_choices(id: &'static str, current: Option<u32>, pal: Pal, set: fn(&mut Settings, Option<u32>)) -> Div {
    let mut seg = div().flex().p(px(2.)).gap(px(2.)).rounded(px(6.)).bg(pal.field).border_1().border_color(pal.field_border);
    for (i, days) in KEEP_CHOICES.into_iter().enumerate() {
        let label = match days {
            None => "Forever".to_owned(),
            Some(1) => "1 day".to_owned(),
            Some(d) => format!("{d} days"),
        };
        let on = days == current;
        seg = seg.child(
            div()
                .id((id, i))
                .px(px(8.))
                .h(px(20.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .text_size(px(12.))
                .text_color(if on { pal.on_strong } else { pal.text })
                .when(on, |d| d.bg(pal.strong))
                .cursor_default()
                .child(label)
                .on_click(move |_, _, cx| prefs::update(cx, |s| set(s, days))),
        );
    }
    seg
}

fn v_stack() -> Div {
    div().flex().flex_col()
}

/// A row of a settings group, with a hairline above it unless it's first.
fn row(pal: Pal, rule: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .min_h(px(44.))
        .px(px(10.))
        .py(px(6.))
        .when(rule, |d| d.border_t_1().border_color(pal.separator))
}

fn group(pal: Pal) -> Div {
    div().flex().flex_col().px(px(2.)).rounded(px(10.)).bg(pal.group).border_1().border_color(pal.group_border)
}

/// A macOS push button (rounded, regular size).
pub(crate) fn push_button(id: &'static str, pal: Pal) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .h(px(22.))
        .px(px(10.))
        .rounded(px(5.))
        .bg(pal.button)
        .border_1()
        .border_color(pal.button_border)
        .shadow(vec![BoxShadow {
            color: hsla(0., 0., 0., 0.12),
            offset: point(px(0.), px(0.5)),
            blur_radius: px(1.),
            spread_radius: px(0.),
            inset: false,
        }])
        .text_size(px(13.))
        .text_color(pal.text)
        .cursor_default()
        .active(|s| s.opacity(0.8))
}

/// A pop-up button's arrows: up and down chevrons.
fn chevrons(pal: Pal) -> Div {
    let color = pal.secondary;
    let arrows = canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let (cx, cy) = (b.origin.x + b.size.width / 2., b.origin.y + b.size.height / 2.);
            let (w, h, gap) = (px(3.), px(2.5), px(1.5));
            for dir in [-1., 1.] {
                let tip = cy + (gap + h) * dir;
                let base = cy + gap * dir;
                let mut path = PathBuilder::stroke(px(1.5));
                path.move_to(point(cx - w, base));
                path.line_to(point(cx, tip));
                path.line_to(point(cx + w, base));
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
        },
    )
    .size_full();
    div().size(px(16.)).child(arrows)
}

/// The focus ring macOS draws round a focused control.
fn ring(color: Hsla) -> BoxShadow {
    BoxShadow { color: color.opacity(0.2), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(3.), inset: false }
}

/// The modifiers held, waiting for a key: `⌃⇧…`.
fn held_label(m: Mods) -> String {
    let mut s = String::new();
    for (on, sym) in [(m.control, '⌃'), (m.alt, '⌥'), (m.shift, '⇧'), (m.command, '⌘')] {
        if on {
            s.push(sym);
        }
    }
    s.push('…');
    s
}

fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// A folder as a menu shows it: its name, with the folder it's in.
fn folder_title(path: &Path) -> String {
    match path.parent().and_then(Path::file_name) {
        Some(parent) => format!("{} — {}", folder_name(path), parent.to_string_lossy()),
        None => folder_name(path),
    }
}

/// `path` with the home folder as `~`.
fn tilde(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

impl SettingsView {
    fn sidebar(&self, pal: Pal, cx: &mut Context<Self>) -> AnyElement {
        let items = Page::ALL.into_iter().map(|page| {
            let selected = page == self.page;

            div()
                .id(page.title())
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(28.))
                .px(px(6.))
                .rounded(px(6.))
                .text_size(px(13.))
                .cursor_default()
                .when(selected, |d| d.bg(pal.selection).font_weight(FontWeight::MEDIUM))
                .child(
                    div()
                        .size(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(if selected { pal.text } else { pal.secondary })
                        .child(icons::icon(page.icon(), 15.)),
                )
                .child(page.title())
                .on_click(cx.listener(move |this, _, window, cx| this.show(page, window, cx)))
        });
        v_stack()
            .w(px(200.))
            .h_full()
            .flex_none()
            .px(px(10.))
            .pt(px(12.))
            .gap(px(2.))
            .bg(pal.sidebar)
            .border_r_1()
            .border_color(pal.separator)
            .children(items)
            .into_any_element()
    }

    fn shortcuts_page(&self, pal: Pal, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let rows: Vec<_> = Action::ALL.into_iter().enumerate().map(|(i, a)| self.shortcut_row(i, a, pal, window, cx)).collect();
        let is_default = prefs::get(cx).shortcuts == Default::default();
        vec![
            group(pal).children(rows).into_any_element(),
            note("Click a shortcut and press the new keys. Esc cancels, Delete removes it.", pal),
            div().flex_1().into_any_element(),
            div()
                .flex()
                .justify_end()
                .child(
                    push_button("restore", pal)
                        .child("Restore Defaults")
                        .when(is_default, |d| d.opacity(0.45))
                        .when(!is_default, |d| d.on_click(cx.listener(|this, _, _, cx| this.restore_defaults(cx)))),
                )
                .into_any_element(),
        ]
    }

    fn captures_page(&self, pal: Pal, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let settings = prefs::get(cx).clone();
        vec![
            group(pal).child(self.folder_row(&settings, pal, cx)).into_any_element(),
            note("Captures are kept encrypted in Gannet's library; Export saves one here as a PNG file.", pal),
            group(pal)
                .child(
                    row(pal, false)
                        .child(described(
                            "Capture toolbar",
                            match settings.toolbar.then_some(settings.toolbar_place) {
                                None => "Off. Captures start from the menu bar icon or a shortcut.",
                                Some(ToolbarPlace::Floating) => "Floats above every window; drag it by its grip.",
                                Some(ToolbarPlace::MenuBar) => {
                                    "Opens under the menu bar icon when it's clicked. Right-click the icon for the menu."
                                },
                            },
                            pal,
                        ))
                        .child(toolbar_choices(settings.toolbar.then_some(settings.toolbar_place), pal)),
                )
                .child(
                    row(pal, true)
                        .child(described("Open the editor after a capture", "Show each new capture in the library, ready to mark up.", pal))
                        .child(
                            Switch::new("open-editor")
                                .color(pal.switch)
                                .checked(settings.open_editor)
                                .on_click(|on, _, cx| prefs::update(cx, |s| s.open_editor = *on)),
                        ),
                )
                .child(
                    row(pal, true)
                        .child(described("Keep captures", "Older captures are deleted from the library. Exported files stay.", pal))
                        .child(keep_choices("keep", settings.keep_days, pal, |s, d| s.keep_days = d)),
                )
                .child(
                    row(pal, true)
                        .child(described("Empty the trash", "Deleted captures stay in the library's trash this long.", pal))
                        .child(keep_choices("trash", settings.trash_days, pal, |s, d| s.trash_days = d)),
                )
                .into_any_element(),
        ]
    }

    fn permissions_page(&self, pal: Pal, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let screen_button = (self.screen != Status::Allowed)
            .then(|| push_button("screen", pal).child("Open Settings…").on_click(|_, _, _| permissions::ask_screen_recording()));
        let notif_button = match self.notifications {
            Status::NotAsked => {
                Some(push_button("notif", pal).child("Allow…").on_click(cx.listener(|this, _, _, cx| this.ask_notifications(cx))))
            },
            Status::Denied => {
                Some(push_button("notif", pal).child("Open Settings…").on_click(|_, _, _| permissions::open_notification_settings()))
            },
            _ => None,
        };
        let mut out = vec![
            group(pal)
                .child(
                    row(pal, false)
                        .child(described("Screen Recording", "Needed to capture the screen and other apps' windows.", pal))
                        .child(status_label(self.screen, pal))
                        .children(screen_button),
                )
                .child(
                    row(pal, true)
                        .child(described("Notifications", "Tells you when a capture can't be taken or saved.", pal))
                        .child(status_label(self.notifications, pal))
                        .children(notif_button),
                )
                .into_any_element(),
        ];
        if self.screen != Status::Allowed && permissions::can_relaunch() {
            out.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(pal.secondary)
                            .child("macOS applies Screen Recording when Gannet starts: after allowing it, reopen Gannet."),
                    )
                    .child(push_button("relaunch", pal).child("Reopen Gannet").on_click(|_, _, cx| permissions::relaunch(true, cx)))
                    .into_any_element(),
            );
        }
        out
    }
}

/// A row's title, with a line of explanation under it.
fn described(title: &str, detail: &str, pal: Pal) -> Div {
    v_stack()
        .flex_1()
        .min_w_0()
        .gap(px(2.))
        .child(div().text_size(px(13.)).text_color(pal.text).child(title.to_owned()))
        .child(div().text_size(px(11.)).text_color(pal.secondary).child(detail.to_owned()))
}

/// A permission's state: a coloured dot and a word.
fn status_label(status: Status, pal: Pal) -> Div {
    let (text, color) = match status {
        Status::Allowed => ("Allowed", pal.success),
        Status::Denied => ("Not allowed", pal.warning),
        Status::NotAsked => ("Not asked", pal.secondary),
        Status::Unavailable => ("Only in Gannet.app", pal.secondary),
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(pal.secondary)
        .child(div().size(px(8.)).rounded_full().bg(color))
        .child(text)
}

fn note(text: &str, pal: Pal) -> AnyElement {
    div().px(px(10.)).text_size(px(11.)).text_color(pal.secondary).child(text.to_owned()).into_any_element()
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Pal::new(is_dark(window));
        if let Some(part) = self.embedded {
            let content = match part {
                // The shortcuts and how to change them; not Restore Defaults,
                // which they are on a first start.
                Embedded::Shortcuts => self.shortcuts_page(pal, window, cx).into_iter().take(2).collect(),
                Embedded::Folder => {
                    let settings = prefs::get(cx).clone();
                    vec![group(pal).child(self.folder_row(&settings, pal, cx)).into_any_element()]
                },
            };
            return v_stack().key_context(KEY_CONTEXT).w_full().gap(px(8.)).text_color(pal.text).children(content).into_any_element();
        }
        let sidebar = self.sidebar(pal, cx);
        let content = match self.page {
            Page::Shortcuts => self.shortcuts_page(pal, window, cx),
            Page::Captures => self.captures_page(pal, cx),
            Page::Permissions => self.permissions_page(pal, cx),
        };
        div()
            .key_context(KEY_CONTEXT)
            .on_action(|_: &crate::menus::CloseWindow, window, _| window.remove_window())
            .size_full()
            .flex()
            .bg(pal.window)
            .text_color(pal.text)
            .font_family(".SystemUIFont")
            .child(sidebar)
            // The page scrolls when it is taller than the window (which
            // keeps its size), with a scroll bar to show there is more.
            .child(
                div().id("page").flex_1().min_w_0().h_full().overflow_y_scrollbar().child(
                    // At least the window's height, so a short page's last
                    // row still sits at the bottom.
                    v_stack()
                        .min_h_full()
                        .px(px(20.))
                        .pt(px(14.))
                        .pb(px(20.))
                        .gap(px(8.))
                        .child(div().px(px(10.)).pb(px(4.)).text_size(px(15.)).font_weight(FontWeight::BOLD).child(self.page.title()))
                        .children(content),
                ),
            )
            .into_any_element()
    }
}
