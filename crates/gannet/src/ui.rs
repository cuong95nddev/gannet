//! The look the capture overlay, the editor and the capture toolbar share:
//! dark, black and white. What is picked or does the main thing is drawn
//! inverted (white, with black on it); colour is left to the markup.

use gannet_core::model::{Rgba, Tool};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{IconName, icon};
use crate::paint::hsla;

/// The library's background.
pub const BG: u32 = 0x161618;
/// Bars and panels.
pub const PANEL: u32 = 0x1e1f22;
pub const TEXT: u32 = 0xe8e8ea;
pub const DIM: u32 = 0x8e8e93;
/// A floating bar's background, a little see-through, and its edge.
pub const BAR_BG: u32 = 0x1e1f22f2;
pub const BAR_LINE: u32 = 0xffffff26;
pub const LINE: u32 = 0xffffff1a;
const HOVER: u32 = 0xffffff1f;
const PRESSED: u32 = 0xffffff33;
/// A quiet button's fill.
const QUIET: u32 = 0xffffff14;
/// A control's height.
pub const CONTROL: f32 = 26.;

/// A tool's icon and tooltip.
pub fn tool_label(tool: Tool) -> (IconName, &'static str) {
    match tool {
        Tool::Rect => (IconName::Square, "Rectangle (R)"),
        Tool::Ellipse => (IconName::Circle, "Ellipse (E)"),
        Tool::Line => (IconName::Minus, "Line (L)"),
        Tool::Arrow => (IconName::MoveUpRight, "Arrow (A)"),
        Tool::Pen => (IconName::Pencil, "Pen (P)"),
        Tool::Text => (IconName::Type, "Text (T)"),
        Tool::Marker => (IconName::Hash, "Numbered marker (N)"),
        Tool::Mosaic => (IconName::Grid3x3, "Mosaic (M)"),
    }
}

/// A floating bar, placed by the caller.
pub fn bar() -> Div {
    div()
        .absolute()
        .flex()
        .items_center()
        .gap(px(2.))
        .px(px(6.))
        .rounded(px(8.))
        .bg(rgba(BAR_BG))
        .border_1()
        .border_color(rgba(BAR_LINE))
        .text_color(rgb(TEXT))
        .text_size(px(13.))
}

/// A flat button: picked, it is inverted; off, it is faded.
pub fn button(id: impl Into<ElementId>, label: impl IntoElement, selected: bool, enabled: bool) -> Stateful<Div> {
    let b = div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(CONTROL))
        .min_w(px(CONTROL))
        .px(px(5.))
        .rounded(px(6.))
        .child(label);
    if !enabled {
        b.opacity(0.3)
    } else if selected {
        b.bg(white()).text_color(black()).cursor_pointer()
    } else {
        b.cursor_pointer().hover(|s| s.bg(rgba(HOVER))).active(|s| s.bg(rgba(PRESSED)))
    }
}

/// A button showing `name`.
pub fn icon_button(id: impl Into<ElementId>, name: IconName, selected: bool, enabled: bool) -> Stateful<Div> {
    button(id, icon(name, 16.), selected, enabled).px_0().w(px(CONTROL))
}

/// A button with words; `strong` is the window's main action (inverted).
pub fn pill(id: impl Into<ElementId>, label: impl Into<SharedString>, strong: bool) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .h(px(CONTROL))
        .px(px(10.))
        .rounded(px(6.))
        .text_size(px(13.))
        .cursor_pointer()
        .child(label.into())
        .map(|d| {
            if strong {
                d.bg(white()).text_color(black()).font_weight(FontWeight::MEDIUM).hover(|s| s.opacity(0.88))
            } else {
                d.bg(rgba(QUIET)).hover(|s| s.bg(rgba(HOVER))).active(|s| s.bg(rgba(PRESSED)))
            }
        })
}

pub fn tip(text: &'static str) -> impl Fn(&mut Window, &mut App) -> AnyView {
    move |window, cx| Tooltip::new(text).build(window, cx)
}

/// A thin upright line between groups of buttons.
pub fn sep() -> Div {
    div().flex_none().w(px(1.)).h(px(16.)).mx(px(4.)).bg(rgba(0xffffff2e))
}

/// A stroke size: a white dot, bigger for each size.
pub fn size_dot(size: u8, selected: bool) -> Stateful<Div> {
    let dot = 4. + 3. * f32::from(size);
    div()
        .id(("size", usize::from(size)))
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|s| s.bg(rgba(HOVER)))
        .when(selected, |d| d.bg(rgba(PRESSED)))
        .child(div().size(px(dot)).rounded_full().bg(white()))
}

/// A colour to draw with, ringed in white once picked.
pub fn swatch(i: usize, color: Rgba, selected: bool) -> Stateful<Div> {
    div()
        .id(("color", i))
        .size(px(22.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .border_2()
        .border_color(if selected { white() } else { transparent_black() })
        .child(div().size(px(14.)).rounded_full().bg(hsla(color)).border_1().border_color(rgba(0xffffff40)))
}
