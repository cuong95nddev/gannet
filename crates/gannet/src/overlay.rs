//! The capture overlay: one borderless window per display showing its
//! frozen screen, where a region is picked and marked up.

use std::sync::Arc;

use gannet_core::model::{self, COLORS, Doc, P, Shape, Style, Tool, p};
use gannet_core::raster::{self, Crop};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::*;
use image::RgbaImage;

use crate::icons::IconName;
use crate::paint::{self, View, from_point, hsla, picture, to_point};
use crate::ui::{bar, icon_button, pill, sep, size_dot, swatch, tip, tool_label};
use crate::{capture, library, scroll};

const HANDLE: f32 = 8.;
const BAR_H: f32 = 36.;
const STYLE_H: f32 = 30.;
/// The toolbar's width, to place it before it is laid out.
const BAR_W: f32 = 470.;
/// The same, for a scrolling capture's.
const SCROLL_BAR_W: f32 = 210.;

enum Drag {
    Select {
        start: P,
    },
    Move {
        grab: P,
        orig: (P, P),
    },
    /// Handle index: 0 top left, then clockwise.
    Resize {
        handle: usize,
        orig: (P, P),
    },
    /// A shape in image pixels.
    Draw(Shape),
}

struct TextEdit {
    /// Top left, in points.
    at: P,
    input: Entity<InputState>,
    _sub: Subscription,
}

pub struct Overlay {
    focus: FocusHandle,
    base: Arc<RgbaImage>,
    picture: Arc<RenderImage>,
    /// Image pixels per point.
    scale: f32,
    /// The region, in points, normalised.
    sel: Option<(P, P)>,
    drag: Option<Drag>,
    tool: Option<Tool>,
    style: Style,
    doc: Doc,
    text: Option<TextEdit>,
    /// The Save menu, open: where it was asked for, in points.
    save_menu: Option<P>,
    /// The region is for a scrolling capture: no markup, Start goes on to it.
    scroll: bool,
}

impl Overlay {
    /// The overlay over `base`, a display `width_pt` points wide; with
    /// `scroll`, the region picked is for a scrolling capture. `picture`
    /// is `base` as GPUI shows it, if it was made already.
    pub fn new(
        base: RgbaImage,
        picture: Option<Arc<RenderImage>>,
        width_pt: f32,
        scroll: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let scale = if width_pt > 0. { base.width() as f32 / width_pt } else { 1. };
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            focus,
            picture: picture.unwrap_or_else(|| self::picture(&base)),
            base: Arc::new(base),
            scale,
            sel: None,
            drag: None,
            tool: None,
            style: Style::default(),
            doc: Doc::new(scale),
            text: None,
            save_menu: None,
            scroll,
        }
    }

    fn to_px(&self, q: P) -> P {
        q.scale(self.scale)
    }

    fn handles(sel: (P, P)) -> [P; 8] {
        let (lo, hi) = sel;
        let mid = (lo + hi).scale(0.5);
        [lo, p(mid.x, lo.y), p(hi.x, lo.y), p(hi.x, mid.y), hi, p(mid.x, hi.y), p(lo.x, hi.y), p(lo.x, mid.y)]
    }

    fn handle_at(&self, q: P) -> Option<usize> {
        let sel = self.sel?;
        Self::handles(sel).iter().position(|h| (q.x - h.x).abs() <= HANDLE && (q.y - h.y).abs() <= HANDLE)
    }

    fn inside(&self, q: P) -> bool {
        self.sel.is_some_and(|(lo, hi)| q.x >= lo.x && q.x <= hi.x && q.y >= lo.y && q.y <= hi.y)
    }

    fn on_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let q = from_point(ev.position);
        if self.save_menu.take().is_some() {
            cx.notify();
            return;
        }
        if self.text.is_some() {
            self.commit_text(window, cx);
            return;
        }
        if ev.click_count == 2 && self.inside(q) && self.tool.is_none() {
            return self.done(window, cx);
        }
        self.drag = match (self.sel, self.handle_at(q), self.tool) {
            (None, _, _) => Some(Drag::Select { start: q }),
            (Some(sel), Some(handle), _) => Some(Drag::Resize { handle, orig: sel }),
            (Some(_), None, Some(tool)) if self.inside(q) => self.start_tool(tool, q, window, cx),
            (Some(sel), None, None) if self.inside(q) => Some(Drag::Move { grab: q, orig: sel }),
            // Outside the region: pick a new one, unless something is drawn.
            (Some(_), None, _) if self.doc.shapes.is_empty() => Some(Drag::Select { start: q }),
            _ => None,
        };
        cx.notify();
    }

    fn start_tool(&mut self, tool: Tool, q: P, window: &mut Window, cx: &mut Context<Self>) -> Option<Drag> {
        let at = self.to_px(q);
        let style = self.style;
        Some(Drag::Draw(match tool {
            Tool::Rect => Shape::Rect { a: at, b: at, style },
            Tool::Ellipse => Shape::Ellipse { a: at, b: at, style },
            Tool::Line => Shape::Line { a: at, b: at, style },
            Tool::Arrow => Shape::Arrow { a: at, b: at, style },
            Tool::Pen => Shape::Pen { points: vec![at], style },
            Tool::Mosaic => Shape::Mosaic { a: at, b: at },
            Tool::Marker => {
                self.doc.push(Shape::Marker { at, n: self.doc.next_marker(), style });
                return None;
            },
            Tool::Text => {
                self.start_text(q, window, cx);
                return None;
            },
        }))
    }

    fn on_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let q = from_point(ev.position);
        let at = self.to_px(q);
        let Some(drag) = &mut self.drag else { return };
        match drag {
            Drag::Select { start } => self.sel = Some(model::norm(*start, q)),
            Drag::Move { grab, orig } => {
                let d = q - *grab;
                self.sel = Some((orig.0 + d, orig.1 + d));
            },
            Drag::Resize { handle, orig } => {
                let (mut lo, mut hi) = *orig;
                match handle {
                    0 => lo = q,
                    1 => lo.y = q.y,
                    2 => (hi.x, lo.y) = (q.x, q.y),
                    3 => hi.x = q.x,
                    4 => hi = q,
                    5 => hi.y = q.y,
                    6 => (lo.x, hi.y) = (q.x, q.y),
                    _ => lo.x = q.x,
                }
                self.sel = Some(model::norm(lo, hi));
            },
            Drag::Draw(shape) => match shape {
                Shape::Rect { b, .. }
                | Shape::Ellipse { b, .. }
                | Shape::Line { b, .. }
                | Shape::Arrow { b, .. }
                | Shape::Mosaic { b, .. } => *b = at,
                Shape::Pen { points, .. } => {
                    if points.last().is_none_or(|l| (*l - at).len() >= 1.) {
                        points.push(at)
                    }
                },
                Shape::Text { .. } | Shape::Marker { .. } | Shape::Image { .. } | Shape::Erase { .. } => {},
            },
        }
        cx.notify();
    }

    fn on_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        match self.drag.take() {
            Some(Drag::Select { .. }) => {
                if self.sel.is_some_and(|(lo, hi)| hi.x - lo.x < 4. || hi.y - lo.y < 4.) {
                    self.sel = None;
                }
            },
            Some(Drag::Draw(shape)) => {
                let keep = match &shape {
                    Shape::Rect { a, b, .. }
                    | Shape::Ellipse { a, b, .. }
                    | Shape::Line { a, b, .. }
                    | Shape::Arrow { a, b, .. }
                    | Shape::Mosaic { a, b } => !model::too_small(*a, *b, self.scale),
                    _ => true,
                };
                if keep {
                    self.doc.push(shape);
                }
            },
            _ => {},
        }
        cx.notify();
    }

    fn start_text(&mut self, q: P, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_text(window, cx),
            _ => {},
        });
        // After this click, which focuses the overlay when it ends.
        let handle = input.focus_handle(cx);
        window.defer(cx, move |window, cx| handle.focus(window, cx));
        self.text = Some(TextEdit { at: q, input, _sub: sub });
        cx.notify();
    }

    fn commit_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.text.take() else { return };
        let text = edit.input.read(cx).value().trim_end().to_string();
        if !text.is_empty() {
            let at = self.to_px(edit.at);
            self.doc.push(Shape::Text { at, text, style: self.style });
        }
        // Later, so the Enter that committed doesn't also reach the overlay.
        cx.defer_in(window, |this, window, cx| window.focus(&this.focus, cx));
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let ks = &ev.keystroke;
        let m = ks.modifiers;
        if self.scroll {
            match ks.key.as_str() {
                "escape" if self.drag.is_some() => self.drag = None,
                "escape" => return capture::close(cx),
                "enter" if self.sel.is_some() => return self.done(window, cx),
                _ => return,
            }
            return cx.notify();
        }
        match ks.key.as_str() {
            "escape" if self.save_menu.is_some() => self.save_menu = None,
            "escape" if self.drag.is_some() => self.drag = None,
            "escape" => return capture::close(cx),
            "enter" if self.sel.is_some() => return self.done(window, cx),
            "z" if m.secondary() && m.shift => {
                self.doc.redo();
            },
            "z" if m.secondary() => {
                self.doc.undo();
            },
            "c" if m.secondary() && self.sel.is_some() => return self.done(window, cx),
            "e" if m.secondary() && self.sel.is_some() => return self.edit(cx),
            "s" if m.secondary() => return self.save_as(capture::Format::Png, cx),
            key if !m.platform && !m.control && !m.alt && self.sel.is_some() => {
                let tool = match key {
                    "r" => Tool::Rect,
                    "e" => Tool::Ellipse,
                    "l" => Tool::Line,
                    "a" => Tool::Arrow,
                    "p" => Tool::Pen,
                    "t" => Tool::Text,
                    "n" => Tool::Marker,
                    "m" => Tool::Mosaic,
                    _ => return,
                };
                self.tool = Some(tool);
            },
            _ => return,
        }
        cx.notify();
    }

    /// The region with its markup, at full resolution.
    fn export(&self) -> Option<RgbaImage> {
        self.exporter().map(|make| make())
    }

    /// What makes [`Self::export`]'s image, to run off the main thread.
    fn exporter(&self) -> Option<impl FnOnce() -> RgbaImage + Send + 'static> {
        let (lo, hi) = self.sel?;
        let (lo, hi) = (self.to_px(lo), self.to_px(hi));
        let crop = Crop {
            x: lo.x.max(0.).round() as u32,
            y: lo.y.max(0.).round() as u32,
            w: (hi.x - lo.x).round().max(1.) as u32,
            h: (hi.y - lo.y).round().max(1.) as u32,
        };
        let (base, doc) = (self.base.clone(), self.doc.clone());
        Some(move || raster::render(&base, crop, &doc, raster::font()))
    }

    fn finish_text(&mut self, cx: &mut Context<Self>) {
        if let Some(edit) = self.text.take() {
            let text = edit.input.read(cx).value().trim_end().to_string();
            if !text.is_empty() {
                let at = self.to_px(edit.at);
                self.doc.push(Shape::Text { at, text, style: self.style });
            }
        }
    }

    /// Enter's action: copy the region with its markup, and keep it in the
    /// library.
    /// For a scrolling capture, go on to it.
    fn done(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.scroll {
            if let Some(sel) = self.sel {
                scroll::start(window, sel, self.scale, cx);
            }
            return;
        }
        self.finish_text(cx);
        let Some(make) = self.exporter() else { return };
        capture::close(cx);
        capture::deliver_with(make, self.scale, false, cx);
    }

    /// Copy and keep it like `done`, then open it in the library's editor.
    fn edit(&mut self, cx: &mut Context<Self>) {
        self.finish_text(cx);
        let Some(make) = self.exporter() else { return };
        capture::close(cx);
        capture::deliver_with(make, self.scale, true, cx);
    }

    fn save_as(&mut self, format: capture::Format, cx: &mut Context<Self>) {
        self.finish_text(cx);
        let Some(img) = self.export() else { return };
        // The save panel would open under the overlay.
        capture::close(cx);
        library::keep(img.clone(), self.scale, false, cx);
        capture::save_as(img, format, cx);
    }

    fn scene(&self) -> Scene {
        let mut shapes = self.doc.shapes.clone();
        if let Some(Drag::Draw(s)) = &self.drag {
            shapes.push(s.clone());
        }
        Scene { shapes, sel: self.sel, scale: self.scale, base: self.base.clone(), accent: white() }
    }
}

impl Focusable for Overlay {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Overlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let (w, h) = (size.width.as_f32(), size.height.as_f32());
        let scene = self.scene();
        let mut root = div()
            .id("overlay")
            .size_full()
            .relative()
            .track_focus(&self.focus)
            .cursor(CursorStyle::Crosshair)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
            .child(img(ImageSource::Render(self.picture.clone())).absolute().top_0().left_0().w(size.width).h(size.height))
            .child(canvas(|_, _, _| {}, move |_, _, window, cx| scene.paint(window, cx)).absolute().top_0().left_0().size_full());
        let Some((lo, hi)) = self.sel else {
            return root.child(div().absolute().top(px(h / 2. - 20.)).w_full().flex().justify_center().child(
                div().px_3().py_2().rounded_md().bg(rgba(0x000000a0)).text_color(white()).text_size(px(14.)).child(if self.scroll {
                    "Drag over what scrolls, then scroll it · Esc to cancel"
                } else {
                    "Drag to capture a region · Esc to cancel"
                }),
            ));
        };
        // Size label above the region (inside it at the top of the screen).
        let label_top = if lo.y >= 30. { lo.y - 28. } else { lo.y + 6. };
        root = root.child(
            div()
                .absolute()
                .left(px(lo.x))
                .top(px(label_top))
                .px_2()
                .py(px(2.))
                .rounded_sm()
                .bg(rgba(0x000000b0))
                .text_color(white())
                .text_size(px(12.))
                .child(format!("{} × {}", (hi.x - lo.x).round(), (hi.y - lo.y).round())),
        );
        if self.drag.as_ref().is_some_and(|d| matches!(d, Drag::Select { .. })) {
            return root;
        }
        root.children(self.bars((lo, hi), (w, h), cx))
    }
}

impl Overlay {
    /// The toolbar under the region `(lo, hi)` (above it, or inside, when
    /// there is no room), the style bar by it while a drawing tool is
    /// picked, and the text box being typed in.
    fn bars(&self, (lo, hi): (P, P), (w, h): (f32, f32), cx: &mut Context<Self>) -> Vec<AnyElement> {
        let styled = self.tool.is_some_and(|t| t != Tool::Mosaic);
        let need = BAR_H + if styled { STYLE_H + 6. } else { 0. } + 10.;
        let below = hi.y + need <= h;
        let bar_y = if below {
            hi.y + 8.
        } else if lo.y - need >= 0. {
            lo.y - 8. - BAR_H
        } else {
            hi.y - 8. - BAR_H
        };
        let right = (w - hi.x).min(w - BAR_W - 8.).max(8.);
        let stop = |_: &MouseDownEvent, _: &mut Window, cx: &mut App| cx.stop_propagation();
        if self.scroll {
            let toolbar = bar()
                .top(px(bar_y))
                .right(px((w - hi.x).min(w - SCROLL_BAR_W - 8.).max(8.)))
                .h(px(BAR_H))
                .occlude()
                .on_mouse_down(MouseButton::Left, stop)
                .child(div().px(px(6.)).text_color(rgb(crate::ui::DIM)).child("Scrolling capture"))
                .child(sep())
                .child(icon_button("cancel", IconName::X, false, true).tooltip(tip("Cancel (Esc)")).on_click(|_, _, cx| capture::close(cx)))
                .child(div().w(px(4.)))
                .child(
                    pill("start", "Start", true)
                        .tooltip(tip("Start, then scroll what is in the region (Enter)"))
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.done(window, cx))),
                );
            return vec![toolbar.into_any_element()];
        }

        let mut toolbar = bar().top(px(bar_y)).right(px(right)).h(px(BAR_H)).occlude().on_mouse_down(MouseButton::Left, stop);
        for (i, tool) in Tool::ALL.into_iter().enumerate() {
            let (name, hint) = tool_label(tool);
            toolbar = toolbar.child(icon_button(("tool", i), name, self.tool == Some(tool), true).tooltip(tip(hint)).on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.commit_text(window, cx);
                    this.tool = if this.tool == Some(tool) { None } else { Some(tool) };
                    cx.notify();
                }),
            ));
        }
        toolbar = toolbar
            .child(sep())
            .child(icon_button("undo", IconName::Undo2, false, self.doc.can_undo()).tooltip(tip("Undo (⌘Z)")).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.doc.undo();
                    cx.notify();
                },
            )))
            .child(icon_button("redo", IconName::Redo2, false, self.doc.can_redo()).tooltip(tip("Redo (⌘⇧Z)")).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.doc.redo();
                    cx.notify();
                },
            )))
            .child(sep())
            .child(icon_button("cancel", IconName::X, false, true).tooltip(tip("Cancel (Esc)")).on_click(|_, _, cx| capture::close(cx)))
            .child(
                icon_button("save", IconName::Download, self.save_menu.is_some(), true)
                    .tooltip(tip("Save to a file: PNG, JPEG or BMP (⌘S: PNG)"))
                    .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                        this.finish_text(cx);
                        this.save_menu = if this.save_menu.is_some() { None } else { Some(from_point(ev.position())) };
                        cx.notify();
                    })),
            )
            .child(
                icon_button("edit", IconName::SquarePen, false, true)
                    .tooltip(tip("Copy, and open in the editor (⌘E)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.edit(cx))),
            )
            .child(div().w(px(4.)))
            .child(
                pill("copy", "Copy", true)
                    .tooltip(tip("Copy, and keep in the library (Enter or ⌘C)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.done(window, cx))),
            );
        let mut out = vec![toolbar.into_any_element()];

        if let Some(at) = self.save_menu {
            const ROW_H: f32 = 26.;
            const MENU_W: f32 = 150.;
            let menu_h = ROW_H * capture::Format::ALL.len() as f32 + 8.;
            // Away from the region: under the bar if it sits below it.
            let top = if bar_y > lo.y { bar_y + BAR_H + 4. } else { bar_y - 4. - menu_h };
            let top = top.clamp(0., (h - menu_h).max(0.));
            let left = (at.x - MENU_W / 2.).clamp(4., (w - MENU_W - 4.).max(4.));
            let mut menu = bar()
                .flex_col()
                .items_stretch()
                .gap_0()
                .py_1()
                .px_1()
                .top(px(top))
                .left(px(left))
                .w(px(MENU_W))
                .occlude()
                .on_mouse_down(MouseButton::Left, stop);
            for (i, format) in capture::Format::ALL.into_iter().enumerate() {
                menu = menu.child(
                    div()
                        .id(("format", i))
                        .h(px(ROW_H))
                        .px_2()
                        .flex()
                        .items_center()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|s| s.bg(white()).text_color(black()))
                        .child(format.label())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.save_menu = None;
                            this.save_as(format, cx);
                        })),
                );
            }
            out.push(menu.into_any_element());
        }

        if styled {
            let style_y = if below || bar_y > lo.y { bar_y + BAR_H + 6. } else { bar_y - 6. - STYLE_H };
            let mut style_bar = bar().top(px(style_y)).right(px(right)).h(px(STYLE_H)).occlude().on_mouse_down(MouseButton::Left, stop);
            for size in 0..3u8 {
                style_bar =
                    style_bar.child(size_dot(size, self.style.size == size).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.style.size = size;
                        cx.notify();
                    })));
            }
            style_bar = style_bar.child(sep());
            for (i, color) in COLORS.into_iter().enumerate() {
                style_bar = style_bar.child(swatch(i, color, self.style.color == color).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.style.color = color;
                        cx.notify();
                    },
                )));
            }
            out.push(style_bar.into_any_element());
        }

        if let Some(edit) = &self.text {
            let font = self.style.font();
            let input = Input::new(&edit.input).appearance(false).text_color(hsla(self.style.color)).text_size(px(font));
            out.push(
                div()
                    .absolute()
                    .left(px(edit.at.x))
                    .top(px(edit.at.y - 4.))
                    .w(px((hi.x - edit.at.x).max(120.)))
                    .occlude()
                    .on_mouse_down(MouseButton::Left, stop)
                    .child(input)
                    .into_any_element(),
            );
        }
        out
    }
}

/// What the canvas paints: the dimmed surroundings, the shapes clipped to
/// the region, and the region's frame.
struct Scene {
    shapes: Vec<Shape>,
    sel: Option<(P, P)>,
    scale: f32,
    base: Arc<RgbaImage>,
    accent: Hsla,
}

impl Scene {
    fn paint(&self, window: &mut Window, cx: &mut App) {
        let size = window.viewport_size();
        let dim = hsla([0, 0, 0, 0x73]);
        let Some((lo, hi)) = self.sel else {
            window.paint_quad(fill(Bounds::new(point(px(0.), px(0.)), size), dim));
            return;
        };
        let (w, h) = (size.width, size.height);
        let rect = |x0: f32, y0: f32, x1: Pixels, y1: Pixels| Bounds::from_corners(point(px(x0), px(y0)), point(x1, y1));
        window.paint_quad(fill(rect(0., 0., w, px(lo.y)), dim));
        window.paint_quad(fill(rect(0., hi.y, w, h), dim));
        window.paint_quad(fill(rect(0., lo.y, px(lo.x), px(hi.y)), dim));
        window.paint_quad(fill(rect(hi.x, lo.y, w, px(hi.y)), dim));

        let region = Bounds::from_corners(to_point(lo), to_point(hi));
        window.with_content_mask(Some(ContentMask { bounds: region }), |window| {
            let view = View { origin: P::default(), k: 1. / self.scale, scale: self.scale };
            for shape in &self.shapes {
                paint::shape(view, shape, &self.base, window, cx);
            }
        });
        window.paint_quad(outline(region, self.accent, BorderStyle::Solid));
        for c in Overlay::handles((lo, hi)) {
            let b = Bounds::new(point(px(c.x - 4.), px(c.y - 4.)), gpui_kit::size(px(8.), px(8.)));
            window.paint_quad(fill(b, gpui_kit::white()));
            window.paint_quad(outline(b, self.accent, BorderStyle::Solid));
        }
    }
}
