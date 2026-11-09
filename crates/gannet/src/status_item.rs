//! The menu bar item and its menu. Gannet has no Dock icon and no windows
//! of its own until a capture starts.

use std::cell::RefCell;
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSControlStateValueOff, NSControlStateValueOn, NSEventMask, NSEventModifierFlags,
    NSEventType, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem,
};
use objc2_foundation::{NSData, NSRect, NSSize, NSString, ns_string};

use gannet_core::settings::{Action, Shortcuts};

use crate::Command;
use crate::capture::Mode;

static TX: OnceLock<flume::Sender<Command>> = OnceLock::new();

fn send(cmd: Command) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(cmd);
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GannetMenuTarget"]
    struct Target;

    impl Target {
        #[unsafe(method(region:))]
        fn region(&self, _sender: &NSMenuItem) {
            send(Command::Capture(Mode::Region));
        }

        #[unsafe(method(window:))]
        fn window(&self, _sender: &NSMenuItem) {
            send(Command::Capture(Mode::Window));
        }

        #[unsafe(method(screen:))]
        fn screen(&self, _sender: &NSMenuItem) {
            send(Command::Capture(Mode::Screen));
        }

        #[unsafe(method(scroll:))]
        fn scroll(&self, _sender: &NSMenuItem) {
            send(Command::Capture(Mode::Scroll));
        }

        #[unsafe(method(toolbar:))]
        fn toolbar(&self, _sender: &NSMenuItem) {
            send(Command::ToggleToolbar);
        }

        #[unsafe(method(settings:))]
        fn settings(&self, _sender: &NSMenuItem) {
            send(Command::OpenSettings);
        }

        #[unsafe(method(openLibrary:))]
        fn open_library(&self, _sender: &NSMenuItem) {
            send(Command::OpenLibrary);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &NSMenuItem) {
            send(Command::Quit);
        }

        /// The icon clicked while the toolbar lives under it: a click
        /// shows the toolbar, a right-click (or ⌃-click) the menu.
        #[unsafe(method(iconClicked:))]
        fn icon_clicked(&self, _sender: &AnyObject) {
            let mtm = MainThreadMarker::from(self);
            let event = NSApplication::sharedApplication(mtm).currentEvent();
            let menu = event.is_some_and(|e| {
                e.r#type() == NSEventType::RightMouseUp || e.modifierFlags().contains(NSEventModifierFlags::Control)
            });
            if menu {
                show_menu();
            } else {
                send(Command::ToggleAttachedToolbar);
            }
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's init.
        unsafe { msg_send![super(this), init] }
    }
}

struct Ui {
    item: Retained<NSStatusItem>,
    menu: Retained<NSMenu>,
    target: Retained<Target>,
    /// The capture items, retitled when their shortcuts change.
    captures: Vec<(Action, Retained<NSMenuItem>)>,
    /// "Show Capture Toolbar", checked while it is on.
    toolbar: Retained<NSMenuItem>,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

/// Put the item in the menu bar and hide the Dock icon. Main thread only.
pub fn install(tx: flume::Sender<Command>) {
    let _ = TX.set(tx);
    let mtm = MainThreadMarker::new().expect("main thread");
    NSApplication::sharedApplication(mtm).setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0); // NSVariableStatusItemLength
    if let Some(button) = item.button(mtm) {
        // Gannet's own icon, drawn as a template (scripts/icon.sh).
        let data = NSData::with_bytes(include_bytes!("../resources/MenuBarIcon.pdf"));
        let image = NSImage::initWithData(<NSImage as objc2::AnyThread>::alloc(), &data);
        if let Some(image) = image {
            image.setSize(NSSize::new(18., 18.));
            image.setAccessibilityDescription(Some(ns_string!("Gannet")));
            image.setTemplate(true);
            button.setImage(Some(&image));
        }
    }
    let target = Target::new(mtm);
    let menu = NSMenu::new(mtm);
    let entries: [(Action, Sel); 4] = [
        (Action::Region, sel!(region:)),
        (Action::Window, sel!(window:)),
        (Action::Screen, sel!(screen:)),
        (Action::Scroll, sel!(scroll:)),
    ];
    let mut captures = Vec::new();
    for (a, action) in entries {
        let item = menu_item(a.title(), action, &target, mtm);
        menu.addItem(&item);
        captures.push((a, item));
    }
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let toolbar = menu_item("Show Capture Toolbar", sel!(toolbar:), &target, mtm);
    menu.addItem(&toolbar);
    menu.addItem(&menu_item("Open Library", sel!(openLibrary:), &target, mtm));
    menu.addItem(&menu_item("Settings…", sel!(settings:), &target, mtm));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&menu_item("Quit Gannet", sel!(quit:), &target, mtm));
    item.setMenu(Some(&menu));
    UI.with(|u| *u.borrow_mut() = Some(Ui { item, menu, target, captures, toolbar }));
}

/// Show `shortcuts` next to their captures, and whether the toolbar is
/// on; with the toolbar `attached` under the icon, a click on the icon
/// shows the toolbar instead of the menu. Main thread only.
pub fn refresh(shortcuts: &Shortcuts, toolbar: bool, attached: bool) {
    UI.with(|u| {
        let Some(ui) = &*u.borrow() else { return };
        ui.toolbar.setState(if toolbar { NSControlStateValueOn } else { NSControlStateValueOff });
        let mtm = MainThreadMarker::new().expect("main thread");
        if let Some(button) = ui.item.button(mtm) {
            if attached {
                ui.item.setMenu(None);
                // SAFETY: `iconClicked:` is a method of `Target`, which lives
                // as long as the UI.
                unsafe {
                    button.setTarget(Some(&ui.target as &AnyObject));
                    button.setAction(Some(sel!(iconClicked:)));
                }
                button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
            } else {
                // SAFETY: clearing the action set above.
                unsafe {
                    button.setTarget(None);
                    button.setAction(None);
                }
                ui.item.setMenu(Some(&ui.menu));
            }
        }
        for (a, item) in &ui.captures {
            let title = match shortcuts.get(*a) {
                Some(s) => format!("{}\t{}", a.title(), s.label()),
                None => a.title().to_owned(),
            };
            item.setTitle(&NSString::from_str(&title));
        }
    });
}

/// Drop the menu down from the icon, as a click does without the toolbar.
fn show_menu() {
    UI.with(|u| {
        let Some(ui) = &*u.borrow() else { return };
        let mtm = MainThreadMarker::new().expect("main thread");
        let Some(button) = ui.item.button(mtm) else { return };
        // The menu, set for this click only (and the icon's own action
        // off meanwhile, so the click can't come back here).
        // SAFETY: setting the action off and back to `Target`'s method,
        // and the button's own click, with no sender.
        unsafe {
            button.setAction(None);
            ui.item.setMenu(Some(&ui.menu));
            button.performClick(None);
            ui.item.setMenu(None);
            button.setAction(Some(sel!(iconClicked:)));
        }
    });
}

/// The icon's frame on screen (AppKit's coordinates, from the bottom left
/// of the main display). Main thread only.
pub fn icon_frame() -> Option<NSRect> {
    UI.with(|u| {
        let ui = u.borrow();
        let ui = ui.as_ref()?;
        let mtm = MainThreadMarker::new()?;
        let button = ui.item.button(mtm)?;
        Some(button.window()?.frame())
    })
}

fn menu_item(title: &str, action: Sel, target: &Target, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    // SAFETY: the selectors are methods of `Target`.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), Some(action), ns_string!(""))
    };
    // SAFETY: `target` lives as long as the UI (thread-local, never dropped).
    unsafe { item.setTarget(Some(target as &AnyObject)) };
    item
}
