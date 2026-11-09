//! A native pop-up menu (AppKit's NSMenu) over a GPUI window, as a macOS
//! pop-up button shows its choices.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSMenu, NSMenuItem, NSView};
use objc2_foundation::{NSPoint, NSString, ns_string};

pub enum Item {
    /// A choice: its title, whether it has a check mark, whether it can be
    /// picked.
    Entry {
        title: String,
        checked: bool,
        enabled: bool,
    },
    Separator,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GannetPopupTarget"]
    #[ivars = flume::Sender<usize>]
    struct Target;

    impl Target {
        #[unsafe(method(pick:))]
        fn pick(&self, sender: &NSMenuItem) {
            let _ = self.ivars().send(sender.tag() as usize);
        }
    }
);

impl Target {
    fn new(tx: flume::Sender<usize>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(tx);
        // SAFETY: NSObject's init.
        unsafe { msg_send![super(this), init] }
    }
}

/// Pop `items` up in `view` (a GPUI window's NSView), the checked one over
/// `at`, a point in the window from its top left. The index of the item
/// picked goes to `tx`; nothing does if the menu is dismissed.
///
/// AppKit runs the menu in a loop of its own, so this goes to the main
/// queue rather than running inside a GPUI update.
pub fn show(view: usize, at: (f64, f64), items: Vec<Item>, tx: flume::Sender<usize>) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        let mtm = MainThreadMarker::new().expect("main queue");
        // SAFETY: GPUI's AppKit handle is its NSView, alive while its window
        // is; the window asked for this menu a moment ago.
        let view: &NSView = unsafe { &*(view as *const NSView) };
        if view.window().is_none() {
            return;
        }
        let target = Target::new(tx, mtm);
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        let mut over = None;
        for (i, item) in items.into_iter().enumerate() {
            match item {
                Item::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
                Item::Entry { title, checked, enabled } => {
                    // SAFETY: `pick:` is a method of `Target`.
                    let entry = unsafe {
                        NSMenuItem::initWithTitle_action_keyEquivalent(
                            NSMenuItem::alloc(mtm),
                            &NSString::from_str(&title),
                            Some(sel!(pick:)),
                            ns_string!(""),
                        )
                    };
                    // SAFETY: `target` outlives the menu, which is modal here.
                    unsafe { entry.setTarget(Some(&*target as &AnyObject)) };
                    entry.setTag(i as isize);
                    entry.setEnabled(enabled);
                    entry.setState(if checked { 1 } else { 0 }); // NSControlStateValueOn / Off
                    if checked && over.is_none() {
                        over = Some(entry.clone());
                    }
                    menu.addItem(&entry);
                },
            }
        }
        let height = view.frame().size.height;
        let y = if view.isFlipped() { at.1 } else { height - at.1 };
        menu.popUpMenuPositioningItem_atLocation_inView(over.as_deref(), NSPoint::new(at.0, y), Some(view));
        drop(target);
    });
}
