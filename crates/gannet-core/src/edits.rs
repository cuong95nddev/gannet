//! What was done to a capture, as a list of steps on its original, kept
//! beside it so every step can be undone and redone, even after the app
//! quits (as darktable keeps a history stack beside a photo). The capture
//! itself is never changed; what it looks like now is the steps played
//! over it.
//!
//! The capture is a stack of layers on a canvas: at the bottom the picture
//! as taken, then the shapes drawn and the images pasted, each a layer
//! counted from the bottom. A crop changes the canvas, never the layers on
//! it, so all of them can still be moved, resized, hidden or taken out.

use std::collections::HashMap;
use std::sync::Arc;

use ab_glyph::FontVec;
use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::model::{Doc, ORIGINAL, P, Shape, p};
use crate::raster;

/// The pasted images, by number.
pub type Images = HashMap<u32, Arc<RgbaImage>>;

/// The edits file's version; a file from a newer Gannet isn't read.
/// 3 made every step a layer on a canvas; 1's steps read the same.
pub const VERSION: u32 = 3;

/// One step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// A new layer on top.
    Shape {
        shape: Shape,
    },
    /// The canvas cut to the `w` × `h` area with its top left at
    /// (`x`, `y`); past its edges it adds blank room. The layers stay.
    Crop {
        x: i64,
        y: i64,
        w: u32,
        h: u32,
    },
    /// Image `image` (see [`Edits::images`]) laid on top, scaled to `w` ×
    /// `h` at (`x`, `y`); the canvas grows to hold it.
    Paste {
        image: u32,
        x: i64,
        y: i64,
        w: u32,
        h: u32,
    },
    /// Layer `layer` replaced by `shape`: moved, resized, or drawn in
    /// another style.
    Change {
        layer: usize,
        shape: Shape,
    },
    /// Layer `layer` hidden, or shown again.
    Hide {
        layer: usize,
        hidden: bool,
    },
    Remove {
        layer: usize,
    },
    /// Layer `layer` moved to place `to`.
    Reorder {
        layer: usize,
        to: usize,
    },
    /// Several steps done, and undone, as one.
    Steps {
        steps: Vec<Op>,
    },
}

/// The steps done, oldest first, and those undone, to redo (the last one
/// undone last).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Edits {
    pub done: Vec<Op>,
    pub undone: Vec<Op>,
}

#[derive(Serialize, Deserialize)]
struct File {
    v: u32,
    #[serde(flatten)]
    edits: Edits,
}

impl Edits {
    pub fn is_empty(&self) -> bool {
        self.done.is_empty() && self.undone.is_empty()
    }

    /// Do `op`; what was undone can't be redone after it.
    pub fn push(&mut self, op: Op) {
        self.done.push(op);
        self.undone.clear();
    }

    pub fn undo(&mut self) -> Option<&Op> {
        let op = self.done.pop()?;
        self.undone.push(op);
        self.undone.last()
    }

    pub fn redo(&mut self) -> Option<&Op> {
        let op = self.undone.pop()?;
        self.done.push(op);
        self.done.last()
    }

    /// The pasted images any step uses, done or undone.
    pub fn images(&self) -> impl Iterator<Item = u32> + '_ {
        fn uses(op: &Op, out: &mut Vec<u32>) {
            match op {
                Op::Paste { image, .. } => out.push(*image),
                Op::Shape { shape: Shape::Image { image, .. } } | Op::Change { shape: Shape::Image { image, .. }, .. } => out.push(*image),
                Op::Steps { steps } => steps.iter().for_each(|op| uses(op, out)),
                _ => {},
            }
        }
        let mut out = Vec::new();
        self.done.iter().chain(&self.undone).for_each(|op| uses(op, &mut out));
        out.into_iter().filter(|&n| n != ORIGINAL)
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(&File { v: VERSION, edits: self.clone() }).expect("edits serialize")
    }

    pub fn from_json(bytes: &[u8]) -> Result<Edits, String> {
        #[derive(Deserialize)]
        struct Head {
            v: u32,
        }
        let head: Head = serde_json::from_slice(bytes).map_err(|e| format!("can't read the edits: {e}"))?;
        if head.v > VERSION {
            return Err("the edits were made by a newer Gannet".to_owned());
        }
        if head.v == 2 {
            return Ok(from_v2(bytes));
        }
        let file: File = serde_json::from_slice(bytes).map_err(|e| format!("can't read the edits: {e}"))?;
        Ok(file.edits)
    }
}

/// Version 2 only lived in test builds, and counted its layers from the
/// last crop: its shapes, crops and pastes are kept, its other steps not.
fn from_v2(bytes: &[u8]) -> Edits {
    #[derive(Deserialize)]
    struct Loose {
        #[serde(default)]
        done: Vec<serde_json::Value>,
        #[serde(default)]
        undone: Vec<serde_json::Value>,
    }
    let keep = |ops: Vec<serde_json::Value>| -> Vec<Op> {
        ops.into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .filter(|op| matches!(op, Op::Shape { .. } | Op::Crop { .. } | Op::Paste { .. }))
            .collect()
    };
    let loose: Loose = serde_json::from_slice(bytes).unwrap_or(Loose { done: Vec::new(), undone: Vec::new() });
    Edits { done: keep(loose.done), undone: keep(loose.undone) }
}

/// The canvas's size, in pixels.
pub type Size = (u32, u32);

/// The layers on a capture `size` big as taken, before any step.
pub fn start(size: Size, scale: f32) -> Doc {
    let mut doc = Doc::new(scale);
    doc.push(Shape::Image { image: ORIGINAL, a: P::default(), b: p(size.0 as f32, size.1 as f32) });
    doc
}

/// Do `op` to the layers of `doc`, on a canvas `size` big.
pub fn step(doc: &mut Doc, size: &mut Size, op: &Op) {
    match op {
        Op::Shape { shape } => doc.push(shape.clone()),
        Op::Change { layer, shape } => {
            doc.set(*layer, shape.clone());
        },
        Op::Hide { layer, hidden } => {
            doc.hide(*layer, *hidden);
        },
        Op::Remove { layer } => {
            doc.remove(*layer);
        },
        Op::Reorder { layer, to } => {
            doc.reorder(*layer, *to);
        },
        Op::Steps { steps } => steps.iter().for_each(|op| step(doc, size, op)),
        &Op::Crop { x, y, w, h } => {
            doc.shift(p(-x as f32, -y as f32));
            *size = (w.max(1), h.max(1));
        },
        &Op::Paste { image, x, y, w, h } => {
            let (w, h) = (i64::from(w.max(1)), i64::from(h.max(1)));
            let (cw, ch) = (i64::from(size.0), i64::from(size.1));
            let (left, top) = (x.min(0), y.min(0));
            let (right, bottom) = ((x + w).max(cw), (y + h).max(ch));
            doc.shift(p(-left as f32, -top as f32));
            *size = ((right - left) as u32, (bottom - top) as u32);
            let a = p((x - left) as f32, (y - top) as f32);
            doc.push(Shape::Image { image, a, b: a + p(w as f32, h as f32) });
        },
    }
}

/// `ops` played on a capture `size` big as taken: its layers, and the
/// canvas's size.
pub fn play(size: Size, ops: &[Op], scale: f32) -> (Doc, Size) {
    let mut doc = start(size, scale);
    let mut size = size;
    for op in ops {
        step(&mut doc, &mut size, op);
    }
    (doc, size)
}

/// The capture as it looks after `ops`, flat.
pub fn render(original: &RgbaImage, ops: &[Op], scale: f32, images: &Images, font: Option<&FontVec>) -> RgbaImage {
    let (doc, (w, h)) = play(original.dimensions(), ops, scale);
    let image = |n: u32| if n == ORIGINAL { Some(original) } else { images.get(&n).map(|i| &**i) };
    raster::compose(w, h, &doc, image, font)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{COLORS, Style};

    fn gray(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba([100, 100, 100, 255]))
    }

    fn red_line(y: f32) -> Shape {
        Shape::Line { a: p(0., y), b: p(200., y), style: Style { color: COLORS[0], size: 2 } }
    }

    fn line(y: f32) -> Op {
        Op::Shape { shape: red_line(y) }
    }

    fn none() -> Images {
        Images::new()
    }

    const GRAY: [u8; 4] = [100, 100, 100, 255];

    #[test]
    fn undo_and_redo_move_steps_between_the_lists() {
        let mut e = Edits::default();
        e.push(line(1.));
        e.push(Op::Crop { x: 0, y: 0, w: 5, h: 5 });
        assert_eq!(e.undo(), Some(&Op::Crop { x: 0, y: 0, w: 5, h: 5 }));
        assert_eq!(e.done.len(), 1);
        assert!(e.redo().is_some());
        e.undo();
        e.push(line(2.));
        assert!(e.undone.is_empty(), "a new step drops what was undone");
        e.undo();
        e.undo();
        assert!(e.undo().is_none());
        assert_eq!(e.undone.len(), 2);
    }

    #[test]
    fn the_picture_is_the_bottom_layer() {
        let (doc, size) = play((40, 30), &[line(5.)], 1.);
        assert_eq!(size, (40, 30));
        assert_eq!(doc.shapes[0], Shape::Image { image: ORIGINAL, a: p(0., 0.), b: p(40., 30.) });
        assert_eq!(doc.shapes.len(), 2);
    }

    #[test]
    fn a_crop_moves_the_canvas_not_the_layers() {
        let original = gray(100, 100);
        let ops = vec![line(10.), Op::Crop { x: 0, y: 5, w: 100, h: 50 }, line(30.)];
        let (doc, size) = play((100, 100), &ops, 1.);
        assert_eq!(size, (100, 50));
        assert_eq!(doc.shapes.len(), 3, "the picture and both lines are still layers");
        assert_eq!(doc.shapes[0], Shape::Image { image: ORIGINAL, a: p(0., -5.), b: p(100., 95.) });
        let flat = render(&original, &ops, 1., &none(), None);
        assert_eq!(flat.dimensions(), (100, 50));
        assert_eq!(flat.get_pixel(50, 5).0, COLORS[0], "the first line, moved up by the crop");
        assert_eq!(flat.get_pixel(50, 30).0, COLORS[0]);
    }

    #[test]
    fn a_crop_past_the_edges_adds_room() {
        let flat = render(&gray(10, 10), &[Op::Crop { x: -5, y: 0, w: 20, h: 10 }], 1., &none(), None);
        assert_eq!(flat.dimensions(), (20, 10));
        assert_eq!(flat.get_pixel(0, 0).0, raster::PAPER);
        assert_eq!(flat.get_pixel(5, 0).0, GRAY);
    }

    #[test]
    fn a_paste_is_a_layer_and_grows_the_canvas() {
        let mut images = Images::new();
        images.insert(3, Arc::new(RgbaImage::from_pixel(4, 4, image::Rgba(COLORS[1]))));
        let ops = vec![Op::Paste { image: 3, x: 8, y: 8, w: 4, h: 4 }];
        let flat = render(&gray(10, 10), &ops, 1., &images, None);
        assert_eq!(flat.dimensions(), (12, 12), "grown to hold it");
        assert_eq!(flat.get_pixel(9, 9).0, COLORS[1]);
        assert_eq!(flat.get_pixel(11, 0).0, raster::PAPER);
        let (doc, _) = play((10, 10), &ops, 1.);
        assert_eq!(doc.shapes[1], Shape::Image { image: 3, a: p(8., 8.), b: p(12., 12.) });
        // An image that's gone leaves its place empty.
        assert_eq!(render(&gray(10, 10), &ops, 1., &none(), None).get_pixel(9, 9).0, GRAY);
        // Pasted up and left of the canvas: everything moves over.
        let (doc, size) = play((10, 10), &[Op::Paste { image: 3, x: -2, y: -1, w: 4, h: 4 }], 1.);
        assert_eq!(size, (12, 11));
        assert_eq!(doc.shapes[0], Shape::Image { image: ORIGINAL, a: p(2., 1.), b: p(12., 11.) });
    }

    #[test]
    fn the_picture_moves_and_resizes_inside_the_canvas() {
        let ops = vec![Op::Change { layer: 0, shape: Shape::Image { image: ORIGINAL, a: p(5., 5.), b: p(10., 10.) } }];
        let flat = render(&gray(20, 20), &ops, 1., &none(), None);
        assert_eq!(flat.dimensions(), (20, 20), "the canvas keeps its size");
        assert_eq!(flat.get_pixel(2, 2).0, raster::PAPER);
        assert_eq!(flat.get_pixel(7, 7).0, GRAY);
        assert_eq!(flat.get_pixel(15, 15).0, raster::PAPER);
    }

    #[test]
    fn layer_steps_change_what_is_drawn() {
        let original = gray(100, 100);
        let ops = vec![
            line(10.),
            line(20.),
            Op::Hide { layer: 1, hidden: true },
            Op::Change { layer: 2, shape: Shape::Line { a: p(0., 40.), b: p(200., 40.), style: Style { color: COLORS[1], size: 2 } } },
        ];
        let flat = render(&original, &ops, 1., &none(), None);
        assert_eq!(flat.get_pixel(50, 10).0, GRAY, "hidden");
        assert_eq!(flat.get_pixel(50, 20).0, GRAY, "moved away");
        assert_eq!(flat.get_pixel(50, 40).0, COLORS[1]);
        let (doc, _) = play((100, 100), &ops, 1.);
        assert!(doc.is_hidden(1));
        // The picture hidden: blank paper.
        let flat = render(&original, &[Op::Hide { layer: 0, hidden: true }], 1., &none(), None);
        assert_eq!(flat.get_pixel(5, 5).0, raster::PAPER);
    }

    #[test]
    fn erasing_paints_paper_and_steps_go_as_one() {
        let ops =
            vec![Op::Steps { steps: vec![Op::Remove { layer: 0 }, Op::Shape { shape: Shape::Erase { a: p(0., 0.), b: p(5., 5.) } }] }];
        let (doc, _) = play((10, 10), &ops, 1.);
        assert_eq!(doc.shapes, vec![Shape::Erase { a: p(0., 0.), b: p(5., 5.) }]);
        let flat = render(&gray(10, 10), &[Op::Shape { shape: Shape::Erase { a: p(0., 0.), b: p(5., 5.) } }], 1., &none(), None);
        assert_eq!(flat.get_pixel(2, 2).0, raster::PAPER);
        assert_eq!(flat.get_pixel(7, 7).0, GRAY);
        let mut e = Edits::default();
        e.push(Op::Steps { steps: vec![Op::Paste { image: 2, x: 0, y: 0, w: 1, h: 1 }] });
        assert_eq!(e.images().collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn layers_reorder_and_go() {
        let ops = vec![line(1.), line(2.), line(3.), Op::Reorder { layer: 3, to: 1 }, Op::Remove { layer: 2 }];
        assert_eq!(play((10, 10), &ops, 1.).0.shapes[1..], [red_line(3.), red_line(2.)]);
    }

    #[test]
    fn the_file_reads_back_and_refuses_newer_versions() {
        let mut e = Edits::default();
        e.push(line(1.));
        e.push(Op::Shape { shape: Shape::Text { at: p(1., 2.), text: "Việt".into(), style: Style::default() } });
        e.push(Op::Change { layer: 1, shape: Shape::Image { image: 4, a: p(3., 4.), b: p(5., 6.) } });
        e.push(Op::Hide { layer: 0, hidden: true });
        e.push(Op::Reorder { layer: 0, to: 1 });
        e.push(Op::Remove { layer: 1 });
        e.push(Op::Paste { image: 0, x: -1, y: 2, w: 3, h: 4 });
        e.undo();
        let json = e.to_json();
        assert_eq!(Edits::from_json(&json).unwrap(), e);
        assert!(String::from_utf8_lossy(&json).contains(r#""op":"shape""#));
        assert_eq!(e.images().collect::<Vec<_>>(), vec![4, 0]);
        assert!(Edits::from_json(br#"{"v":99,"done":[],"undone":[]}"#).is_err());
        assert!(Edits::from_json(b"nope").is_err());
        // What Gannet wrote before.
        assert_eq!(Edits::from_json(br#"{"v":1,"done":[{"op":"crop","x":0,"y":0,"w":2,"h":2}],"undone":[]}"#).unwrap().done.len(), 1);
        let v2 = br#"{"v":2,"done":[{"op":"crop","x":0,"y":0,"w":2,"h":2},{"op":"resize","w":1,"h":1,"was_w":2,"was_h":2},{"op":"hide","layer":0,"hidden":true}],"undone":[]}"#;
        assert_eq!(Edits::from_json(v2).unwrap().done, vec![Op::Crop { x: 0, y: 0, w: 2, h: 2 }]);
    }
}
