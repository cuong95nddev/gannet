//! The exported image: the selected part of a capture with its shapes
//! drawn in, at the capture's full resolution.

use std::sync::OnceLock;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use image::RgbaImage;
use tiny_skia::{FillRule, FilterQuality, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};

use crate::model::{self, Doc, MOSAIC_BLOCK, P, Rgba, Shape};

/// A region of the capture in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The system UI font (SF, else Helvetica); `None` leaves text out.
pub fn font() -> Option<&'static FontVec> {
    static FONT: OnceLock<Option<FontVec>> = OnceLock::new();
    FONT.get_or_init(|| {
        FONT_PATHS.iter().find_map(|path| {
            let bytes = std::fs::read(path).ok()?;
            FontVec::try_from_vec_and_index(bytes, 0).ok()
        })
    })
    .as_ref()
}

const FONT_PATHS: [&str; 2] = ["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc"];

/// `crop` of `base` with `doc`'s shapes drawn on it, oldest first; hidden
/// ones are left out.
pub fn render(base: &RgbaImage, crop: Crop, doc: &Doc, font: Option<&FontVec>) -> RgbaImage {
    let crop = clamp(crop, base.width(), base.height());
    let part = image::imageops::crop_imm(base, crop.x, crop.y, crop.w.max(1), crop.h.max(1)).to_image();
    let mut pm = to_pixmap(&part);
    let off = model::p(crop.x as f32, crop.y as f32);
    for shape in doc.visible() {
        draw(&mut pm, shape, off, doc, font);
    }
    from_pixmap(&pm)
}

/// The layers of `doc` drawn on a blank `w` × `h` canvas, the bottom one
/// first; `image` finds the pixels of an image layer by its number.
pub fn compose<'a>(w: u32, h: u32, doc: &Doc, image: impl Fn(u32) -> Option<&'a RgbaImage>, font: Option<&FontVec>) -> RgbaImage {
    let mut shapes = doc.visible().peekable();
    // Most often the bottom layer is the capture, covering the canvas
    // exactly and opaque: it is the start, rather than paper drawn over.
    let base = match shapes.peek() {
        Some(Shape::Image { image: n, a, b }) => image(*n).filter(|img| {
            *a == P::default()
                && *b == model::p(w as f32, h as f32)
                && img.dimensions() == (w, h)
                && img.as_raw().as_chunks::<4>().0.iter().all(|px| px[3] == 255)
        }),
        _ => None,
    };
    let mut pm = match base {
        Some(img) => {
            shapes.next();
            to_pixmap(img)
        },
        None => {
            let mut pm = Pixmap::new(w.max(1), h.max(1)).expect("non-empty pixmap");
            let [r, g, b, a] = PAPER;
            pm.fill(tiny_skia::Color::from_rgba8(r, g, b, a));
            pm
        },
    };
    for shape in shapes {
        match shape {
            Shape::Image { image: n, a, b } => {
                if let Some(img) = image(*n) {
                    draw_image(&mut pm, img, *a, *b);
                }
            },
            _ => draw(&mut pm, shape, P::default(), doc, font),
        }
    }
    from_pixmap(&pm)
}

/// `img` scaled into the rectangle `a`–`b`.
fn draw_image(pm: &mut Pixmap, img: &RgbaImage, a: P, b: P) {
    let (lo, hi) = model::norm(a, b);
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let (sx, sy) = ((hi.x - lo.x) / iw, (hi.y - lo.y) / ih);
    if sx <= 0. || sy <= 0. {
        return;
    }
    // At its own size on whole pixels, copied as it is.
    let exact = sx == 1. && sy == 1. && lo.x.fract() == 0. && lo.y.fract() == 0.;
    let paint = PixmapPaint { quality: if exact { FilterQuality::Nearest } else { FilterQuality::Bicubic }, ..PixmapPaint::default() };
    pm.draw_pixmap(0, 0, to_pixmap(img).as_ref(), &paint, Transform::from_row(sx, 0., 0., sy, lo.x, lo.y), None);
}

/// `img` as a pixmap, its colours premultiplied.
fn to_pixmap(img: &RgbaImage) -> Pixmap {
    let mut pm = Pixmap::new(img.width().max(1), img.height().max(1)).expect("non-empty pixmap");
    // A screen capture is opaque all over: copied as it is, which is what
    // premultiplying by 255 gives, without the sums.
    if img.as_raw().as_chunks::<4>().0.iter().all(|px| px[3] == 255) {
        pm.data_mut().copy_from_slice(img.as_raw());
        return pm;
    }
    for (dst, src) in pm.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(img.as_raw().as_chunks::<4>().0) {
        let a = u16::from(src[3]);
        if a == 255 {
            *dst = *src;
            continue;
        }
        for c in 0..3 {
            dst[c] = ((u16::from(src[c]) * a + 127) / 255) as u8;
        }
        dst[3] = src[3];
    }
    pm
}

fn from_pixmap(pm: &Pixmap) -> RgbaImage {
    // The pixmap's bytes are RGBA premultiplied: opaque pixels are already
    // what the image wants.
    let mut out = RgbaImage::from_raw(pm.width(), pm.height(), pm.data().to_vec()).expect("whole pixmap");
    for (dst, px) in out.as_chunks_mut::<4>().0.iter_mut().zip(pm.pixels()) {
        if dst[3] != 255 {
            let c = px.demultiply();
            *dst = [c.red(), c.green(), c.blue(), c.alpha()];
        }
    }
    out
}

/// `base` with `doc`'s shapes drawn on it, whole.
pub fn flatten(base: &RgbaImage, doc: &Doc, font: Option<&FontVec>) -> RgbaImage {
    render(base, Crop { x: 0, y: 0, w: base.width(), h: base.height() }, doc, font)
}

/// The colour new canvas is filled with.
pub const PAPER: Rgba = [0xFF; 4];

/// An image read from encoded bytes (a clipboard's).
pub fn decode(bytes: &[u8]) -> Option<RgbaImage> {
    crate::store::decode(bytes).map_err(|e| log::warn!("can't read the pasted image: {e}")).ok()
}

fn clamp(c: Crop, w: u32, h: u32) -> Crop {
    let x = c.x.min(w.saturating_sub(1));
    let y = c.y.min(h.saturating_sub(1));
    Crop { x, y, w: c.w.min(w - x).max(1), h: c.h.min(h - y).max(1) }
}

fn paint(color: Rgba) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
    paint.anti_alias = true;
    paint
}

fn stroke(width: f32) -> Stroke {
    Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() }
}

fn polyline(points: &[P], off: P, close: bool) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let (first, rest) = points.split_first()?;
    pb.move_to(first.x - off.x, first.y - off.y);
    for q in rest {
        pb.line_to(q.x - off.x, q.y - off.y);
    }
    if rest.is_empty() {
        // A single point: a dot.
        pb.line_to(first.x - off.x + 0.01, first.y - off.y);
    }
    if close {
        pb.close();
    }
    pb.finish()
}

fn draw(pm: &mut Pixmap, shape: &Shape, off: P, doc: &Doc, font: Option<&FontVec>) {
    let id = Transform::identity();
    match shape {
        Shape::Rect { a, b, style } => {
            let (lo, hi) = model::norm(*a, *b);
            let pts = [lo, model::p(hi.x, lo.y), hi, model::p(lo.x, hi.y)];
            if let Some(path) = polyline(&pts, off, true) {
                pm.stroke_path(&path, &paint(style.color), &stroke(doc.px(style.stroke())), id, None);
            }
        },
        Shape::Ellipse { a, b, style } => {
            if let Some(path) = polyline(&model::ellipse(*a, *b, 96), off, true) {
                pm.stroke_path(&path, &paint(style.color), &stroke(doc.px(style.stroke())), id, None);
            }
        },
        Shape::Line { a, b, style } => {
            if let Some(path) = polyline(&[*a, *b], off, false) {
                pm.stroke_path(&path, &paint(style.color), &stroke(doc.px(style.stroke())), id, None);
            }
        },
        Shape::Arrow { a, b, style } => {
            let w = doc.px(style.stroke());
            let (shaft, head) = model::arrow(*a, *b, w);
            if let Some(path) = polyline(&[*a, shaft], off, false) {
                pm.stroke_path(&path, &paint(style.color), &stroke(w), id, None);
            }
            if let Some(path) = polyline(&head, off, true) {
                pm.fill_path(&path, &paint(style.color), FillRule::Winding, id, None);
            }
        },
        Shape::Pen { points, style } => {
            if let Some(path) = polyline(points, off, false) {
                pm.stroke_path(&path, &paint(style.color), &stroke(doc.px(style.stroke())), id, None);
            }
        },
        Shape::Text { at, text, style } => {
            if let Some(font) = font {
                draw_text(pm, font, text, *at - off, doc.px(style.font()), style.color);
            }
        },
        Shape::Marker { at, n, style } => {
            let r = doc.px(style.marker_radius());
            let c = *at - off;
            if let Some(path) = PathBuilder::from_circle(c.x, c.y, r) {
                pm.fill_path(&path, &paint(style.color), FillRule::Winding, id, None);
            }
            if let Some(font) = font {
                let label = n.to_string();
                let size = r * 1.25;
                let (w, h) = text_size(font, &label, size);
                draw_text(pm, font, &label, model::p(c.x - w / 2., c.y - h / 2.), size, style.marker_text());
            }
        },
        Shape::Mosaic { a, b } => mosaic(pm, *a - off, *b - off, doc.px(MOSAIC_BLOCK)),
        Shape::Erase { a, b } => {
            let (lo, hi) = model::norm(*a - off, *b - off);
            if let Some(r) = tiny_skia::Rect::from_ltrb(lo.x, lo.y, hi.x, hi.y) {
                pm.fill_rect(r, &paint(PAPER), id, None);
            }
        },
        // Laid down by `compose`, which has the pixels.
        Shape::Image { .. } => {},
    }
}

/// The cells of a mosaic over `a`..`b` on an image `w` × `h` big, each
/// `[x0, y0, x1, y1]`: `block` pixels square, counted from the mosaic's
/// corner (so cutting the image elsewhere doesn't move them), and cut to
/// the image.
pub fn mosaic_cells(a: P, b: P, (w, h): (i64, i64), block: f32) -> Vec<[i64; 4]> {
    let (lo, hi) = model::norm(a, b);
    let block = block.round().max(2.) as i64;
    let (ox, oy) = (lo.x.floor() as i64, lo.y.floor() as i64);
    let x0 = ox.clamp(0, w);
    let y0 = oy.clamp(0, h);
    let x1 = (hi.x.ceil() as i64).clamp(0, w);
    let y1 = (hi.y.ceil() as i64).clamp(0, h);
    // The first cell's start at or before the image's edge.
    let first = |o: i64, start: i64| o + (start - o) / block * block;
    let mut out = Vec::new();
    let mut by = first(oy, y0);
    while by < y1 {
        let (sy, ey) = (by.max(y0), (by + block).min(y1));
        let mut bx = first(ox, x0);
        while bx < x1 {
            out.push([bx.max(x0), sy, (bx + block).min(x1), ey]);
            bx += block;
        }
        by += block;
    }
    out
}

/// Fill each `block`-sized cell of the rectangle with its average colour.
fn mosaic(pm: &mut Pixmap, a: P, b: P, block: f32) {
    let w = pm.width() as i64;
    let cells = mosaic_cells(a, b, (w, pm.height() as i64), block);
    let data = pm.data_mut();
    for [bx, by, ex, ey] in cells {
        let mut sum = [0u64; 4];
        for y in by..ey {
            for x in bx..ex {
                let i = ((y * w + x) * 4) as usize;
                for c in 0..4 {
                    sum[c] += u64::from(data[i + c]);
                }
            }
        }
        let n = ((ey - by) * (ex - bx)) as u64;
        let avg = sum.map(|s| (s / n) as u8);
        for y in by..ey {
            for x in bx..ex {
                let i = ((y * w + x) * 4) as usize;
                data[i..i + 4].copy_from_slice(&avg);
            }
        }
    }
}

/// The box `shape` covers, in image pixels, its stroke included; `scale`
/// is pixels per point. Without a font, text is guessed at.
pub fn bounds(shape: &Shape, scale: f32, font: Option<&FontVec>) -> (P, P) {
    let pad = |(lo, hi): (P, P), by: f32| (lo - model::p(by, by), hi + model::p(by, by));
    match shape {
        Shape::Rect { a, b, style } | Shape::Ellipse { a, b, style } | Shape::Line { a, b, style } => {
            pad(model::norm(*a, *b), style.stroke() * scale / 2.)
        },
        Shape::Arrow { a, b, style } => {
            let (_, [tip, l, r]) = model::arrow(*a, *b, style.stroke() * scale);
            let pts = [*a, tip, l, r];
            pad(span(&pts), style.stroke() * scale / 2.)
        },
        Shape::Pen { points, style } => pad(span(points), style.stroke() * scale / 2.),
        Shape::Text { at, text, style } => {
            let size = style.font() * scale;
            let lines = text.split('\n').count().max(1) as f32;
            let (w, h) = match font {
                Some(font) => {
                    let sf = font.as_scaled(em(font, size));
                    let w = text.split('\n').map(|l| text_size(font, l, size).0).fold(0., f32::max);
                    (w, (lines - 1.) * (sf.height() + sf.line_gap()) + sf.ascent() - sf.descent())
                },
                None => (text.split('\n').map(|l| l.chars().count()).max().unwrap_or(0) as f32 * size * 0.55, lines * size * 1.2),
            };
            (*at, *at + model::p(w.max(size * 0.3), h))
        },
        Shape::Marker { at, style, .. } => {
            let r = style.marker_radius() * scale;
            (*at - model::p(r, r), *at + model::p(r, r))
        },
        Shape::Mosaic { a, b } | Shape::Image { a, b, .. } | Shape::Erase { a, b } => model::norm(*a, *b),
    }
}

/// The box around `points`.
fn span(points: &[P]) -> (P, P) {
    let first = points.first().copied().unwrap_or_default();
    points.iter().fold((first, first), |(lo, hi), q| (model::p(lo.x.min(q.x), lo.y.min(q.y)), model::p(hi.x.max(q.x), hi.y.max(q.y))))
}

/// Whether `q` is on `shape`, or within `reach` pixels of it. A rectangle
/// or an ellipse is held by its outline, so what is under it can be too.
pub fn hits(shape: &Shape, q: P, reach: f32, scale: f32, font: Option<&FontVec>) -> bool {
    let inside = |(lo, hi): (P, P)| q.x >= lo.x - reach && q.x <= hi.x + reach && q.y >= lo.y - reach && q.y <= hi.y + reach;
    let near = |d: f32, style: &model::Style| d <= reach + style.stroke() * scale / 2.;
    match shape {
        Shape::Rect { a, b, style } => {
            let (lo, hi) = model::norm(*a, *b);
            near(model::to_polyline(q, &[lo, model::p(hi.x, lo.y), hi, model::p(lo.x, hi.y)], true), style)
        },
        Shape::Ellipse { a, b, style } => near(model::to_polyline(q, &model::ellipse(*a, *b, 64), true), style),
        Shape::Line { a, b, style } => near(model::to_segment(q, *a, *b), style),
        // The head is wider than the line.
        Shape::Arrow { a, b, style } => model::to_segment(q, *a, *b) <= reach + style.stroke() * scale * 2.,
        Shape::Pen { points, style } => near(model::to_polyline(q, points, false), style),
        Shape::Text { .. } | Shape::Marker { .. } | Shape::Mosaic { .. } | Shape::Image { .. } | Shape::Erase { .. } => {
            inside(bounds(shape, scale, font))
        },
    }
}

/// The topmost shown layer of `doc` at `q`, within `reach` pixels.
pub fn layer_at(doc: &Doc, q: P, reach: f32, font: Option<&FontVec>) -> Option<usize> {
    (0..doc.shapes.len()).rev().find(|&i| !doc.is_hidden(i) && hits(&doc.shapes[i], q, reach, doc.scale, font))
}

/// The scale that draws `font` `size` pixels to the em, as the screen
/// does; ab_glyph's own scale is the height from descent to ascent.
fn em(font: &FontVec, size: f32) -> PxScale {
    match font.units_per_em() {
        Some(upem) if upem > 0. => PxScale::from(size * font.height_unscaled() / upem),
        _ => PxScale::from(size),
    }
}

/// The size of `text` (one line) at `size` pixels: (advance, ascent - descent).
pub fn text_size(font: &FontVec, text: &str, size: f32) -> (f32, f32) {
    let sf = font.as_scaled(em(font, size));
    let w = text.chars().map(|c| sf.h_advance(font.glyph_id(c))).sum();
    (w, sf.ascent() - sf.descent())
}

/// Draw `text` with its first line's top left at `at`.
fn draw_text(pm: &mut Pixmap, font: &FontVec, text: &str, at: P, size: f32, color: Rgba) {
    let scale = em(font, size);
    let sf = font.as_scaled(scale);
    let line_h = sf.height() + sf.line_gap();
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    for (row, line) in text.split('\n').enumerate() {
        let baseline = at.y + sf.ascent() + row as f32 * line_h;
        let mut x = at.x;
        let mut prev = None;
        for c in line.chars() {
            let id = font.glyph_id(c);
            if let Some(prev) = prev {
                x += sf.kern(prev, id);
            }
            prev = Some(id);
            let glyph = id.with_scale_and_position(scale, point(x, baseline));
            x += sf.h_advance(id);
            let Some(outline) = font.outline_glyph(glyph) else { continue };
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, cov| {
                let px = bounds.min.x as i32 + gx as i32;
                let py = bounds.min.y as i32 + gy as i32;
                if px < 0 || py < 0 || px >= w || py >= h {
                    return;
                }
                let i = ((py * w + px) * 4) as usize;
                let a = cov.clamp(0., 1.) * f32::from(color[3]) / 255.;
                for ch in 0..3 {
                    let src = f32::from(color[ch]) * a;
                    data[i + ch] = (src + f32::from(data[i + ch]) * (1. - a)).round() as u8;
                }
                data[i + 3] = (a * 255. + f32::from(data[i + 3]) * (1. - a)).round() as u8;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{COLORS, Style, p};

    #[test]
    fn mosaic_cells_stay_put_when_cut() {
        // Whole: cells from the mosaic's corner at 5.
        let whole = mosaic_cells(p(5., 5.), p(25., 15.), (100, 100), 8.);
        assert_eq!(whole[0], [5, 5, 13, 13]);
        assert_eq!(whole[2], [21, 5, 25, 13]);
        // Cut 10 px in from the left: the same cells, moved with the image.
        let cut = mosaic_cells(p(-5., 5.), p(15., 15.), (100, 100), 8.);
        assert_eq!(cut[0], [0, 5, 3, 13]);
        assert_eq!(cut[1], [3, 5, 11, 13]);
        assert!(mosaic_cells(p(200., 0.), p(300., 10.), (100, 100), 8.).is_empty());
    }

    fn gray(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba([100, 100, 100, 255]))
    }

    #[test]
    fn crops_to_the_region() {
        let mut base = gray(50, 40);
        base.put_pixel(10, 5, image::Rgba([1, 2, 3, 255]));
        let out = render(&base, Crop { x: 10, y: 5, w: 20, h: 10 }, &Doc::new(1.), None);
        assert_eq!(out.dimensions(), (20, 10));
        assert_eq!(out.get_pixel(0, 0).0, [1, 2, 3, 255]);
        // A region running past the edge is cut to the image.
        let out = render(&base, Crop { x: 40, y: 30, w: 100, h: 100 }, &Doc::new(1.), None);
        assert_eq!(out.dimensions(), (10, 10));
    }

    #[test]
    fn rect_draws_its_border_only() {
        let base = gray(100, 100);
        let mut doc = Doc::new(1.);
        doc.push(Shape::Rect { a: p(20., 20.), b: p(80., 80.), style: Style { color: COLORS[0], size: 1 } });
        let out = render(&base, Crop { x: 0, y: 0, w: 100, h: 100 }, &doc, None);
        assert_eq!(out.get_pixel(20, 50).0, COLORS[0]);
        assert_eq!(out.get_pixel(50, 50).0, [100, 100, 100, 255]);
        assert_eq!(out.get_pixel(5, 5).0, [100, 100, 100, 255]);
    }

    #[test]
    fn shapes_follow_the_crop() {
        let base = gray(100, 100);
        let mut doc = Doc::new(1.);
        doc.push(Shape::Line { a: p(30., 50.), b: p(70., 50.), style: Style { color: COLORS[1], size: 2 } });
        let out = render(&base, Crop { x: 20, y: 40, w: 60, h: 20 }, &doc, None);
        assert_eq!(out.get_pixel(30, 10).0, COLORS[1]);
    }

    #[test]
    fn mosaic_makes_uniform_blocks() {
        let mut base = gray(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                base.put_pixel(x, y, image::Rgba([(x * 8) as u8, (y * 8) as u8, 0, 255]));
            }
        }
        let mut doc = Doc::new(1.);
        doc.push(Shape::Mosaic { a: p(0., 0.), b: p(16., 16.) });
        let out = render(&base, Crop { x: 0, y: 0, w: 32, h: 32 }, &doc, None);
        let block = out.get_pixel(0, 0).0;
        assert_eq!(out.get_pixel(7, 7).0, block);
        assert_ne!(out.get_pixel(8, 0).0, block);
        // Outside the mosaic nothing changes.
        assert_eq!(out.get_pixel(20, 20).0, base.get_pixel(20, 20).0);
    }

    #[test]
    fn hidden_layers_are_not_drawn() {
        let mut doc = Doc::new(1.);
        doc.push(Shape::Rect { a: p(20., 20.), b: p(80., 80.), style: Style { color: COLORS[0], size: 1 } });
        doc.hide(0, true);
        let out = flatten(&gray(100, 100), &doc, None);
        assert_eq!(out.get_pixel(20, 50).0, [100, 100, 100, 255]);
    }

    #[test]
    fn layers_are_found_by_their_outline_or_box() {
        let style = Style { color: COLORS[0], size: 0 };
        let mut doc = Doc::new(1.);
        doc.push(Shape::Rect { a: p(10., 10.), b: p(90., 90.), style });
        doc.push(Shape::Marker { at: p(50., 50.), n: 1, style });
        doc.push(Shape::Line { a: p(0., 95.), b: p(100., 95.), style });
        assert_eq!(layer_at(&doc, p(50., 50.), 2., None), Some(1), "the marker, in the rectangle");
        assert_eq!(layer_at(&doc, p(30., 30.), 2., None), None, "inside the rectangle is free");
        assert_eq!(layer_at(&doc, p(11., 40.), 2., None), Some(0));
        assert_eq!(layer_at(&doc, p(40., 93.), 2., None), Some(2));
        doc.hide(1, true);
        assert_eq!(layer_at(&doc, p(50., 50.), 2., None), None, "hidden layers aren't found");
        let (lo, hi) = bounds(&doc.shapes[0], 1., None);
        assert_eq!((lo, hi), (p(9., 10. - 1.), p(91., 91.)));
    }

    #[test]
    fn compose_on_an_opaque_capture_matches_drawing_it() {
        let base = RgbaImage::from_fn(40, 30, |x, y| image::Rgba([(x * 6) as u8, (y * 8) as u8, 90, 255]));
        let rect = Shape::Rect { a: model::p(5., 5.), b: model::p(30., 20.), style: model::Style::default() };
        let mut doc = Doc::new(1.);
        doc.push(Shape::Image { image: 0, a: P::default(), b: model::p(40., 30.) });
        doc.push(rect.clone());
        let fast = compose(40, 30, &doc, |_| Some(&base), None);
        let mut marks = Doc::new(1.);
        marks.push(rect);
        assert_eq!(fast, flatten(&base, &marks, None));
        // Not opaque: drawn over paper, as before.
        let mut clear = base.clone();
        clear.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        assert_eq!(compose(40, 30, &doc, |_| Some(&clear), None).get_pixel(0, 0).0, PAPER);
    }

    #[test]
    fn decode_reads_a_png() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        gray(3, 2).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        assert_eq!(decode(bytes.get_ref()).map(|i| i.dimensions()), Some((3, 2)));
        assert!(decode(b"nope").is_none());
    }

    #[test]
    fn text_and_markers_draw_when_a_font_is_there() {
        let font = font().expect("system font");
        let base = gray(200, 100);
        let mut doc = Doc::new(1.);
        doc.push(Shape::Text { at: p(10., 10.), text: "Hi Việt".into(), style: Style { color: COLORS[0], size: 2 } });
        doc.push(Shape::Marker { at: p(160., 50.), n: 3, style: Style { color: COLORS[1], size: 2 } });
        let out = render(&base, Crop { x: 0, y: 0, w: 200, h: 100 }, &doc, Some(font));
        let red = out.pixels().filter(|px| px.0 == COLORS[0]).count();
        assert!(red > 20, "text pixels: {red}");
        assert_eq!(out.get_pixel(160 + 15, 50).0, COLORS[1], "marker disc");
        assert!(out.pixels().any(|px| px.0 == [255, 255, 255, 255]), "marker number");
    }
}
