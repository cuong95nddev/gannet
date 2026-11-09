//! AppKit's Open panel, for image files only (GPUI's own can't be limited
//! to some kinds of file).

use std::path::PathBuf;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{NSArray, NSString, ns_string};

/// The image files that can be opened, by extension.
pub const IMAGE_TYPES: [&str; 8] = ["png", "jpg", "jpeg", "bmp", "gif", "webp", "tif", "tiff"];

/// Ask for image files; the ones picked go to `tx` (nothing does if the
/// panel is cancelled).
///
/// The panel runs a loop of its own, so this goes to the main queue rather
/// than running inside a GPUI update.
pub fn choose_images(tx: flume::Sender<Vec<PathBuf>>) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        let mtm = MainThreadMarker::new().expect("main queue");
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(true);
        panel.setPrompt(Some(ns_string!("Open")));
        let types: Vec<_> = IMAGE_TYPES.iter().map(|t| NSString::from_str(t)).collect();
        // `allowedContentTypes` needs UTType, from another framework; the
        // extensions do the same here.
        #[allow(deprecated)]
        panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&types)));
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        if panel.runModal() != NSModalResponseOK {
            return;
        }
        let paths: Vec<PathBuf> = panel.URLs().iter().filter_map(|url| url.path()).map(|p| PathBuf::from(p.to_string())).collect();
        if !paths.is_empty() {
            let _ = tx.send(paths);
        }
    });
}
