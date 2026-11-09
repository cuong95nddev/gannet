//! The app's icons: Lucide, as bundled with GPUI Kit. Only the icons
//! listed here are embedded, on top of the ones the components use.

use std::borrow::Cow;

pub use gpui_kit::assets::IconName;
use gpui_kit::assets::{Assets as ComponentAssets, icon_assets};
use gpui_kit::component::Icon;
use gpui_kit::{AssetSource, Pixels, Result, SharedString, Styled};

icon_assets!(
    AppIcons,
    [
        AppWindow,
        ArrowDown,
        ArrowUp,
        Check,
        ChevronDown,
        ChevronLeft,
        ChevronRight,
        Circle,
        ClipboardPaste,
        Copy,
        Crop,
        Download,
        Eraser,
        Eye,
        EyeOff,
        Folder,
        FolderDown,
        FolderOpen,
        FolderPlus,
        Grid3x3,
        GripVertical,
        Hash,
        Image,
        Images,
        Keyboard,
        Layers,
        Maximize2,
        Minimize2,
        Minus,
        Monitor,
        MoveUpRight,
        Pencil,
        Redo2,
        Scan,
        ScrollText,
        ShieldCheck,
        Square,
        SquareDashed,
        SquarePen,
        Trash,
        Type,
        Undo2,
        X,
        ZoomIn,
        ZoomOut,
    ]
);

/// The app's icons, then the components' own.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match AppIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => ComponentAssets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = ComponentAssets.list(path)?;
        paths.extend(AppIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// `name` drawn `size` points square, in the current text colour.
pub fn icon(name: IconName, size: f32) -> Icon {
    Icon::new(name).size(Pixels::from(size))
}

#[cfg(test)]
mod tests {
    use super::*;

    // `gpui_kit::*` brings a `test` macro of its own.
    #[::core::prelude::v1::test]
    fn icons_load() {
        for name in [IconName::SquareDashed, IconName::Images, IconName::X, IconName::Check] {
            assert!(Assets.load(&name.path()).unwrap().is_some(), "{}", name.path());
        }
    }
}
