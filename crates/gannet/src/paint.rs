//! Painting a capture's shapes on screen, shared by the overlay and the
//! editor; `raster` draws the same shapes into the exported image.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Weak};

use gpui_kit::*;
use image::RgbaImage;
use lyon_tessellation::{LineCap, LineJoin, StrokeOptions};

use gannet_core::model::{self, MOSAIC_BLOCK, P, Rgba, Shape, p};
use gannet_core::raster;

pub fn hsla(c: Rgba) -> Hsla {
    gpui_kit::Rgba { r: f32::from(c[0]) / 255., g: f32::from(c[1]) / 255., b: f32::from(c[2]) / 255., a: f32::from(c[3]) / 255. }.into()
}

pub fn to_point(q: P) -> Point<Pixels> {
    point(px(q.x), px(q.y))
}

pub fn from_point(q: Point<Pixels>) -> P {
    p(q.x.as_f32(), q.y.as_f32())
}

/// The longest side of a texture the GPU takes (Metal's limit on every
/// Mac); a picture past it isn't drawn at all.
const MAX_TEXTURE: u32 = 16_384;

/// An image as GPUI wants it (BGRA). One too big for the GPU (a tall
/// scrolling capture) is shown scaled down to fit; what is exported is
/// still made from the full image.
pub fn picture(img: &RgbaImage) -> Arc<RenderImage> {
    match fit_texture(img) {
        Some(small) => picture_owned(small),
        None => picture_owned(img.clone()),
    }
}

/// Like [`picture`], turning `img` itself rather than a copy of it.
pub fn picture_owned(img: RgbaImage) -> Arc<RenderImage> {
    let mut bgra = fit_texture(&img).unwrap_or(img);
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    Arc::new(RenderImage::new(vec![image::Frame::new(bgra)]))
}

/// `img` scaled down to the largest texture the GPU takes, if it is
/// bigger.
fn fit_texture(img: &RgbaImage) -> Option<RgbaImage> {
    let (w, h) = img.dimensions();
    if w.max(h) <= MAX_TEXTURE {
        return None;
    }
    let k = MAX_TEXTURE as f32 / w.max(h) as f32;
    let (tw, th) = (((w as f32 * k) as u32).max(1), ((h as f32 * k) as u32).max(1));
    Some(image::imageops::resize(img, tw, th, image::imageops::FilterType::Triangle))
}

/// Where image pixels land in the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// The image's top left, in window points.
    pub origin: P,
    /// Window points per image pixel.
    pub k: f32,
    /// The capture's pixels per point, which shape sizes are given in.
    pub scale: f32,
}

impl View {
    /// Image pixels to window points.
    pub fn pt(&self, q: P) -> Point<Pixels> {
        to_point(self.origin + q.scale(self.k))
    }

    /// Window points per point of a shape's style.
    pub fn zoom(&self) -> f32 {
        self.scale * self.k
    }
}

fn stroke(points: &[Point<Pixels>], closed: bool, width: f32, color: Hsla, window: &mut Window) {
    if points.len() < 2 {
        return;
    }
    let opts = StrokeOptions::default().with_line_width(width).with_line_cap(LineCap::Round).with_line_join(LineJoin::Round);
    let mut pb = PathBuilder::stroke(px(width)).with_style(PathStyle::Stroke(opts));
    pb.add_polygon(points, closed);
    if let Ok(path) = pb.build() {
        window.paint_path(path, color);
    }
}

fn fill_poly(points: &[Point<Pixels>], color: Hsla, window: &mut Window) {
    if points.len() < 3 {
        return;
    }
    let mut pb = PathBuilder::fill();
    pb.add_polygon(points, true);
    if let Ok(path) = pb.build() {
        window.paint_path(path, color);
    }
}

fn text_line(text: &str, size: f32, color: Hsla, window: &Window) -> ShapedLine {
    let run = TextRun { len: text.len(), font: font(".SystemUIFont"), color, background_color: None, underline: None, strikethrough: None };
    window.text_system().shape_line(text.to_owned().into(), px(size), &[run], None)
}

/// Paint `shape`; a mosaic averages `base`, the image it is drawn on.
/// Images are left to the caller.
pub fn shape(view: View, shape: &Shape, base: &Arc<RgbaImage>, window: &mut Window, cx: &mut App) {
    let k = view.scale;
    let z = view.zoom();
    match shape {
        Shape::Rect { a, b, style } => {
            let (lo, hi) = model::norm(*a, *b);
            let pts = [lo, p(hi.x, lo.y), hi, p(lo.x, hi.y)].map(|q| view.pt(q));
            stroke(&pts, true, style.stroke() * z, hsla(style.color), window);
        },
        Shape::Ellipse { a, b, style } => {
            let pts: Vec<_> = model::ellipse(*a, *b, 96).into_iter().map(|q| view.pt(q)).collect();
            stroke(&pts, true, style.stroke() * z, hsla(style.color), window);
        },
        Shape::Line { a, b, style } => stroke(&[view.pt(*a), view.pt(*b)], false, style.stroke() * z, hsla(style.color), window),
        Shape::Arrow { a, b, style } => {
            let (shaft, head) = model::arrow(*a, *b, style.stroke() * k);
            stroke(&[view.pt(*a), view.pt(shaft)], false, style.stroke() * z, hsla(style.color), window);
            fill_poly(&head.map(|q| view.pt(q)), hsla(style.color), window);
        },
        Shape::Pen { points, style } => {
            let pts: Vec<_> = points.iter().map(|q| view.pt(*q)).collect();
            if pts.len() == 1 {
                let r = style.stroke() / 2. * k;
                let dot: Vec<_> = model::ellipse(points[0] - p(r, r), points[0] + p(r, r), 16).into_iter().map(|q| view.pt(q)).collect();
                fill_poly(&dot, hsla(style.color), window);
            } else {
                stroke(&pts, false, style.stroke() * z, hsla(style.color), window);
            }
        },
        Shape::Text { at, text, style } => {
            let size = style.font() * z;
            let line = text_line(text, size, hsla(style.color), window);
            let _ = line.paint(view.pt(*at), px(size * 1.2), TextAlign::Left, None, window, cx);
        },
        Shape::Marker { at, n, style } => {
            let r = style.marker_radius() * k;
            let disc: Vec<_> = model::ellipse(*at - p(r, r), *at + p(r, r), 48).into_iter().map(|q| view.pt(q)).collect();
            fill_poly(&disc, hsla(style.color), window);
            let size = style.marker_radius() * 1.25 * z;
            let line = text_line(&n.to_string(), size, hsla(style.marker_text()), window);
            let c = view.pt(*at);
            let lh = px(size * 1.2);
            let _ = line.paint(point(c.x - line.width / 2., c.y - lh / 2.), lh, TextAlign::Left, None, window, cx);
        },
        Shape::Mosaic { a, b } => mosaic(view, *a, *b, base, window),
        Shape::Erase { a, b } => {
            let (lo, hi) = model::norm(*a, *b);
            window.paint_quad(fill(Bounds::from_corners(view.pt(lo), view.pt(hi)), hsla(gannet_core::raster::PAPER)));
        },
        // Painted by the editor, which has its pixels.
        Shape::Image { .. } => {},
    }
}

/// A mosaic's blocks, worked out once: each block's corners in image
/// pixels and its colour.
type Blocks = Rc<Vec<([i64; 4], Rgba)>>;

/// Mosaics worked out lately, by the image they average (held weakly: a
/// live `Weak` keeps the address from being reused by another image), the
/// block size and the pixels covered.
struct MosaicCache(Vec<(Weak<RgbaImage>, [i64; 5], Blocks)>);

/// How many mosaics are kept worked out.
const MOSAICS_KEPT: usize = 64;

thread_local! {
    static MOSAICS: RefCell<MosaicCache> = const { RefCell::new(MosaicCache(Vec::new())) };
}

/// The blocks of the mosaic over `a`..`b` of `base`.
fn blocks(base: &Arc<RgbaImage>, block: f32, a: P, b: P) -> Blocks {
    let (lo, hi) = model::norm(a, b);
    let key = [block.to_bits() as i64, lo.x.to_bits() as i64, lo.y.to_bits() as i64, hi.x.to_bits() as i64, hi.y.to_bits() as i64];
    let cached = MOSAICS.with_borrow(|c| {
        c.0.iter().find(|(w, k, _)| *k == key && w.upgrade().is_some_and(|b| Arc::ptr_eq(&b, base))).map(|(_, _, b)| b.clone())
    });
    if let Some(b) = cached {
        return b;
    }
    let (w, raw) = (base.width() as usize, base.as_raw());
    let mut out = Vec::new();
    for [bx, by, ex, ey] in raster::mosaic_cells(lo, hi, (base.width() as i64, base.height() as i64), block) {
        let mut sum = [0u64; 4];
        for y in by..ey {
            let row = &raw[(y as usize * w + bx as usize) * 4..(y as usize * w + ex as usize) * 4];
            for px in row.as_chunks::<4>().0 {
                for c in 0..4 {
                    sum[c] += u64::from(px[c]);
                }
            }
        }
        let n = ((ey - by) * (ex - bx)) as u64;
        out.push(([bx, by, ex, ey], sum.map(|s| (s / n) as u8)));
    }
    let out = Rc::new(out);
    MOSAICS.with_borrow_mut(|c| {
        c.0.retain(|(w, _, _)| w.strong_count() > 0);
        if c.0.len() >= MOSAICS_KEPT {
            c.0.remove(0);
        }
        c.0.push((Arc::downgrade(base), key, out.clone()));
    });
    out
}

/// The mosaic as it will be exported, from `base`.
fn mosaic(view: View, a: P, b: P, base: &Arc<RgbaImage>, window: &mut Window) {
    let block = MOSAIC_BLOCK * view.scale;
    for ([bx, by, ex, ey], color) in blocks(base, block, a, b).iter() {
        let cell = Bounds::from_corners(view.pt(p(*bx as f32, *by as f32)), view.pt(p(*ex as f32, *ey as f32)));
        window.paint_quad(fill(cell, hsla(*color)));
    }
}
