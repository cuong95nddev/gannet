//! The menu bar item and its menu. Gannet has no Dock icon and no windows
//! of its own until a capture starts.

use std::cell::RefCell;
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem};
use objc2_foundation::{NSString, ns_string};

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

        #[unsafe(method(settings:))]
        fn settings(&self, _sender: &NSMenuItem) {
            send(Command::OpenSettings);
        }

        #[unsafe(method(openCaptures:))]
        fn open_captures(&self, _sender: &NSMenuItem) {
            send(Command::OpenCaptures);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &NSMenuItem) {
            send(Command::Quit);
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
    _item: Retained<NSStatusItem>,
    _target: Retained<Target>,
    /// The capture items, retitled when their shortcuts change.
    captures: Vec<(Action, Retained<NSMenuItem>)>,
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
        let image =
            NSImage::imageWithSystemSymbolName_accessibilityDescription(ns_string!("camera.viewfinder"), Some(ns_string!("Gannet")));
        if let Some(image) = image {
            image.setTemplate(true);
            button.setImage(Some(&image));
        }
    }
    let target = Target::new(mtm);
    let menu = NSMenu::new(mtm);
    let entries: [(Action, Sel); 3] = [(Action::Region, sel!(region:)), (Action::Window, sel!(window:)), (Action::Screen, sel!(screen:))];
    let mut captures = Vec::new();
    for (a, action) in entries {
        let item = menu_item(a.title(), action, &target, mtm);
        menu.addItem(&item);
        captures.push((a, item));
    }
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&menu_item("Open Captures Folder", sel!(openCaptures:), &target, mtm));
    menu.addItem(&menu_item("Settings…", sel!(settings:), &target, mtm));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&menu_item("Quit Gannet", sel!(quit:), &target, mtm));
    item.setMenu(Some(&menu));
    UI.with(|u| *u.borrow_mut() = Some(Ui { _item: item, _target: target, captures }));
}

/// Show `shortcuts` next to their captures. Main thread only.
pub fn refresh(shortcuts: &Shortcuts) {
    UI.with(|u| {
        let Some(ui) = &*u.borrow() else { return };
        for (a, item) in &ui.captures {
            let title = match shortcuts.get(*a) {
                Some(s) => format!("{}\t{}", a.title(), s.label()),
                None => a.title().to_owned(),
            };
            item.setTitle(&NSString::from_str(&title));
        }
    });
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
