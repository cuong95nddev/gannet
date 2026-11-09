//! What is drawn on a capture: shapes in image pixels, their style, the
//! undo history, and the geometry both the editor and the export use so
//! they draw the same thing.

#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct P {
    pub x: f32,
    pub y: f32,
}

pub const fn p(x: f32, y: f32) -> P {
    P { x, y }
}

impl std::ops::Sub for P {
    type Output = P;
    fn sub(self, o: P) -> P {
        p(self.x - o.x, self.y - o.y)
    }
}

impl std::ops::Add for P {
    type Output = P;
    fn add(self, o: P) -> P {
        p(self.x + o.x, self.y + o.y)
    }
}

impl P {
    pub fn scale(self, k: f32) -> P {
        p(self.x * k, self.y * k)
    }

    pub fn len(self) -> f32 {
        self.x.hypot(self.y)
    }
}

/// RGBA, not premultiplied.
pub type Rgba = [u8; 4];

/// The colour swatches, in toolbar order.
pub const COLORS: [Rgba; 8] = [
    [0xE5, 0x39, 0x35, 0xFF],
    [0x2F, 0x6B, 0xFF, 0xFF],
    [0x3C, 0xB8, 0x4A, 0xFF],
    [0xE0, 0xA4, 0x2B, 0xFF],
    [0xEC, 0x4C, 0x8B, 0xFF],
    [0x1A, 0x1A, 0x1A, 0xFF],
    [0x7A, 0x7F, 0x87, 0xFF],
    [0xFF, 0xFF, 0xFF, 0xFF],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Rect,
    Ellipse,
    Line,
    Arrow,
    Pen,
    Text,
    Marker,
    Mosaic,
}

impl Tool {
    pub const ALL: [Tool; 8] = [Tool::Rect, Tool::Ellipse, Tool::Line, Tool::Arrow, Tool::Pen, Tool::Text, Tool::Marker, Tool::Mosaic];
}

/// Colour and one of three sizes; the sizes are in points, times the
/// capture's backing scale in pixels.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Style {
    pub color: Rgba,
    /// 0 (thin), 1, 2 (thick).
    pub size: u8,
}

impl Default for Style {
    fn default() -> Self {
        Style { color: COLORS[0], size: 1 }
    }
}

impl Style {
    fn pick(self, sizes: [f32; 3]) -> f32 {
        sizes[usize::from(self.size.min(2))]
    }

    /// Line width in points.
    pub fn stroke(self) -> f32 {
        self.pick([2., 4., 6.])
    }

    /// Text size in points.
    pub fn font(self) -> f32 {
        self.pick([16., 22., 30.])
    }

    /// Numbered marker radius in points.
    pub fn marker_radius(self) -> f32 {
        self.pick([11., 14., 18.])
    }

    /// The colour text on a filled marker of this colour is drawn in.
    pub fn marker_text(self) -> Rgba {
        let [r, g, b, _] = self.color;
        let luma = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
        if luma > 170. { [0, 0, 0, 0xFF] } else { [0xFF; 4] }
    }
}

/// Mosaic block edge in points.
pub const MOSAIC_BLOCK: f32 = 8.;

/// Kept in a capture's edits file, so a change here must still read the
/// shapes written before it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    Rect {
        a: P,
        b: P,
        style: Style,
    },
    Ellipse {
        a: P,
        b: P,
        style: Style,
    },
    Line {
        a: P,
        b: P,
        style: Style,
    },
    Arrow {
        a: P,
        b: P,
        style: Style,
    },
    Pen {
        points: Vec<P>,
        style: Style,
    },
    /// `at` is the top left of the first line.
    Text {
        at: P,
        text: String,
        style: Style,
    },
    Marker {
        at: P,
        n: u32,
        style: Style,
    },
    Mosaic {
        a: P,
        b: P,
    },
    /// The rectangle `a`–`b` painted over with blank paper: what was
    /// there, erased.
    Erase {
        a: P,
        b: P,
    },
    /// A picture filling the rectangle from `a` (top left) to `b`: the
    /// capture as taken ([`ORIGINAL`]) or a pasted image, by number.
    Image {
        image: u32,
        a: P,
        b: P,
    },
}

/// The image number of the capture as taken.
pub const ORIGINAL: u32 = u32::MAX;

impl Shape {
    /// The tool it is drawn with; none for an image.
    pub fn tool(&self) -> Option<Tool> {
        Some(match self {
            Shape::Rect { .. } => Tool::Rect,
            Shape::Ellipse { .. } => Tool::Ellipse,
            Shape::Line { .. } => Tool::Line,
            Shape::Arrow { .. } => Tool::Arrow,
            Shape::Pen { .. } => Tool::Pen,
            Shape::Text { .. } => Tool::Text,
            Shape::Marker { .. } => Tool::Marker,
            Shape::Mosaic { .. } => Tool::Mosaic,
            Shape::Image { .. } | Shape::Erase { .. } => return None,
        })
    }

    /// An image's rectangle, (top left, bottom right).
    pub fn image_rect(&self) -> Option<(P, P)> {
        if let Shape::Image { a, b, .. } = self { Some(norm(*a, *b)) } else { None }
    }

    /// What it is called in a list of layers.
    pub fn name(&self) -> String {
        match self {
            Shape::Rect { .. } => "Rectangle".to_owned(),
            Shape::Ellipse { .. } => "Ellipse".to_owned(),
            Shape::Line { .. } => "Line".to_owned(),
            Shape::Arrow { .. } => "Arrow".to_owned(),
            Shape::Pen { .. } => "Pen".to_owned(),
            Shape::Text { text, .. } => text.lines().next().unwrap_or_default().to_owned(),
            Shape::Marker { n, .. } => format!("Marker {n}"),
            Shape::Mosaic { .. } => "Mosaic".to_owned(),
            Shape::Image { image: ORIGINAL, .. } => "Picture".to_owned(),
            Shape::Image { .. } => "Image".to_owned(),
            Shape::Erase { .. } => "Erased".to_owned(),
        }
    }

    /// Its colour and size; a mosaic has none.
    pub fn style(&self) -> Option<Style> {
        match self {
            Shape::Rect { style, .. }
            | Shape::Ellipse { style, .. }
            | Shape::Line { style, .. }
            | Shape::Arrow { style, .. }
            | Shape::Pen { style, .. }
            | Shape::Text { style, .. }
            | Shape::Marker { style, .. } => Some(*style),
            Shape::Mosaic { .. } | Shape::Image { .. } | Shape::Erase { .. } => None,
        }
    }

    /// It drawn in `style` instead; a mosaic stays as it is.
    pub fn with_style(&self, new: Style) -> Shape {
        let mut s = self.clone();
        match &mut s {
            Shape::Rect { style, .. }
            | Shape::Ellipse { style, .. }
            | Shape::Line { style, .. }
            | Shape::Arrow { style, .. }
            | Shape::Pen { style, .. }
            | Shape::Text { style, .. }
            | Shape::Marker { style, .. } => *style = new,
            Shape::Mosaic { .. } | Shape::Image { .. } | Shape::Erase { .. } => {},
        }
        s
    }

    /// It moved by `d`.
    pub fn moved(&self, d: P) -> Shape {
        let mut s = self.clone();
        match &mut s {
            Shape::Rect { a, b, .. }
            | Shape::Ellipse { a, b, .. }
            | Shape::Line { a, b, .. }
            | Shape::Arrow { a, b, .. }
            | Shape::Mosaic { a, b }
            | Shape::Erase { a, b }
            | Shape::Image { a, b, .. } => {
                *a = *a + d;
                *b = *b + d;
            },
            Shape::Pen { points, .. } => points.iter_mut().for_each(|q| *q = *q + d),
            Shape::Text { at, .. } | Shape::Marker { at, .. } => *at = *at + d,
        }
        s
    }
}

/// How far `q` is from the segment `a`–`b`.
pub fn to_segment(q: P, a: P, b: P) -> f32 {
    let d = b - a;
    let l2 = d.x * d.x + d.y * d.y;
    if l2 < 1e-6 {
        return (q - a).len();
    }
    let t = (((q.x - a.x) * d.x + (q.y - a.y) * d.y) / l2).clamp(0., 1.);
    (q - (a + d.scale(t))).len()
}

/// How far `q` is from the polyline through `points` (closed with `closed`).
pub fn to_polyline(q: P, points: &[P], closed: bool) -> f32 {
    let open = points.windows(2).map(|w| to_segment(q, w[0], w[1]));
    let close = closed.then(|| points.first().zip(points.last()).map(|(a, b)| to_segment(q, *b, *a))).flatten();
    open.chain(close).fold(points.first().map_or(f32::INFINITY, |a| (q - *a).len()), f32::min)
}

/// The shapes drawn on one capture, oldest first (the bottom layer first),
/// with undo/redo. Points are image pixels; `scale` is pixels per point.
#[derive(Debug, Clone, Default)]
pub struct Doc {
    pub shapes: Vec<Shape>,
    /// Which shapes are hidden, by index; past its end, none.
    hidden: Vec<bool>,
    redo: Vec<Shape>,
    pub scale: f32,
}

impl Doc {
    pub fn new(scale: f32) -> Self {
        Doc { shapes: Vec::new(), hidden: Vec::new(), redo: Vec::new(), scale }
    }

    pub fn push(&mut self, shape: Shape) {
        // A shape popped by undo may have been hidden; this one isn't.
        self.hidden.truncate(self.shapes.len());
        self.shapes.push(shape);
        self.redo.clear();
    }

    pub fn is_hidden(&self, i: usize) -> bool {
        i < self.shapes.len() && self.hidden.get(i).copied().unwrap_or(false)
    }

    /// The shapes shown, bottom first.
    pub fn visible(&self) -> impl Iterator<Item = &Shape> {
        self.shapes.iter().enumerate().filter(|(i, _)| !self.is_hidden(*i)).map(|(_, s)| s)
    }

    /// Put `shape` in place of layer `i`. False if there is no such layer.
    pub fn set(&mut self, i: usize, shape: Shape) -> bool {
        let Some(s) = self.shapes.get_mut(i) else { return false };
        *s = shape;
        true
    }

    /// Hide layer `i`, or show it again.
    pub fn hide(&mut self, i: usize, hidden: bool) -> bool {
        if i >= self.shapes.len() {
            return false;
        }
        self.hidden.resize(self.shapes.len(), false);
        self.hidden[i] = hidden;
        true
    }

    /// Take layer `i` out.
    pub fn remove(&mut self, i: usize) -> bool {
        if i >= self.shapes.len() {
            return false;
        }
        self.shapes.remove(i);
        if i < self.hidden.len() {
            self.hidden.remove(i);
        }
        true
    }

    /// Move every layer by `d`.
    pub fn shift(&mut self, d: P) {
        for s in &mut self.shapes {
            *s = s.moved(d);
        }
    }

    /// Move layer `i` to place `to`, the others closing up behind it.
    pub fn reorder(&mut self, i: usize, to: usize) -> bool {
        let n = self.shapes.len();
        if i >= n || to >= n {
            return false;
        }
        self.hidden.resize(n, false);
        let s = self.shapes.remove(i);
        self.shapes.insert(to, s);
        let h = self.hidden.remove(i);
        self.hidden.insert(to, h);
        true
    }

    pub fn undo(&mut self) -> bool {
        self.hidden.truncate(self.shapes.len());
        self.shapes.pop().map(|s| self.redo.push(s)).is_some()
    }

    pub fn redo(&mut self) -> bool {
        self.redo.pop().map(|s| self.shapes.push(s)).is_some()
    }

    pub fn can_undo(&self) -> bool {
        !self.shapes.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The number the next marker gets.
    pub fn next_marker(&self) -> u32 {
        self.shapes.iter().filter_map(|s| if let Shape::Marker { n, .. } = s { Some(*n) } else { None }).max().unwrap_or(0) + 1
    }

    /// Points to pixels.
    pub fn px(&self, pt: f32) -> f32 {
        pt * self.scale
    }
}

/// The corners of the rectangle spanned by `a` and `b`: (top left, bottom right).
pub fn norm(a: P, b: P) -> (P, P) {
    (p(a.x.min(b.x), a.y.min(b.y)), p(a.x.max(b.x), a.y.max(b.y)))
}

/// An arrow from `a` to `b` drawn `width` wide: where its shaft stops, and
/// its head (tip, left, right). Short arrows get a smaller head.
pub fn arrow(a: P, b: P, width: f32) -> (P, [P; 3]) {
    let d = b - a;
    let len = d.len();
    if len < 0.01 {
        return (a, [b, b, b]);
    }
    let u = d.scale(1. / len);
    let n = p(-u.y, u.x);
    let head = (width * 3. + 8.).min(len * 0.6);
    let half = head * 0.55;
    let base = b - u.scale(head);
    // The shaft stops inside the head, so its round cap doesn't poke out.
    let shaft = b - u.scale(head * 0.6);
    (shaft, [b, base + n.scale(half), base - n.scale(half)])
}

/// The ellipse inscribed in the rectangle spanned by `a` and `b`, as a
/// closed polyline of `n` points.
pub fn ellipse(a: P, b: P, n: usize) -> Vec<P> {
    let (lo, hi) = norm(a, b);
    let c = (lo + hi).scale(0.5);
    let (rx, ry) = ((hi.x - lo.x) / 2., (hi.y - lo.y) / 2.);
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            p(c.x + rx * t.cos(), c.y + ry * t.sin())
        })
        .collect()
}

/// Whether a drag from `a` to `b` is too small to be a shape.
pub fn too_small(a: P, b: P, scale: f32) -> bool {
    (b - a).len() < 3. * scale
}

/// A part of a rectangle a drag can take hold of: one of its eight
/// handles, or its inside to move it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grip {
    Move,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Grip {
    /// Which edges it moves: (left, top, right, bottom).
    fn edges(self) -> (bool, bool, bool, bool) {
        match self {
            Grip::Move => (true, true, true, true),
            Grip::N => (false, true, false, false),
            Grip::S => (false, false, false, true),
            Grip::E => (false, false, true, false),
            Grip::W => (true, false, false, false),
            Grip::NE => (false, true, true, false),
            Grip::NW => (true, true, false, false),
            Grip::SE => (false, false, true, true),
            Grip::SW => (true, false, false, true),
        }
    }
}

/// The handles of the rectangle `lo`–`hi`, where they are drawn.
pub fn handles(lo: P, hi: P) -> [(Grip, P); 8] {
    let c = (lo + hi).scale(0.5);
    [
        (Grip::NW, lo),
        (Grip::N, p(c.x, lo.y)),
        (Grip::NE, p(hi.x, lo.y)),
        (Grip::E, p(hi.x, c.y)),
        (Grip::SE, hi),
        (Grip::S, p(c.x, hi.y)),
        (Grip::SW, p(lo.x, hi.y)),
        (Grip::W, p(lo.x, c.y)),
    ]
}

/// What a press at `q` takes hold of on the rectangle `lo`–`hi`: an edge
/// or corner within `reach` of it, else its inside.
pub fn grip_at(lo: P, hi: P, q: P, reach: f32) -> Option<Grip> {
    edge_grip(lo, hi, q, reach, reach).or_else(|| (q.x > lo.x && q.x < hi.x && q.y > lo.y && q.y < hi.y).then_some(Grip::Move))
}

/// The edge or corner of the rectangle `lo`–`hi` that `q` is on, counting
/// `inside` in from each edge and `outside` out from it.
pub fn edge_grip(lo: P, hi: P, q: P, inside: f32, outside: f32) -> Option<Grip> {
    if q.x < lo.x - outside || q.x > hi.x + outside || q.y < lo.y - outside || q.y > hi.y + outside {
        return None;
    }
    let near = |at: f32, edge: f32, out_is_less: bool| {
        let d = at - edge;
        if out_is_less { d >= -outside && d <= inside } else { d <= outside && d >= -inside }
    };
    // Of two opposite edges both in reach (a thin rectangle), the nearer.
    let (l, r) = (near(q.x, lo.x, true), near(q.x, hi.x, false));
    let (t, b) = (near(q.y, lo.y, true), near(q.y, hi.y, false));
    let (l, r) = if l && r { ((q.x - lo.x).abs() <= (q.x - hi.x).abs(), (q.x - lo.x).abs() > (q.x - hi.x).abs()) } else { (l, r) };
    let (t, b) = if t && b { ((q.y - lo.y).abs() <= (q.y - hi.y).abs(), (q.y - lo.y).abs() > (q.y - hi.y).abs()) } else { (t, b) };
    match (l, t, r, b) {
        (true, true, _, _) => Some(Grip::NW),
        (_, true, true, _) => Some(Grip::NE),
        (true, _, _, true) => Some(Grip::SW),
        (_, _, true, true) => Some(Grip::SE),
        (true, ..) => Some(Grip::W),
        (_, true, ..) => Some(Grip::N),
        (_, _, true, _) => Some(Grip::E),
        (.., true) => Some(Grip::S),
        _ => None,
    }
}

/// The rectangle `lo`–`hi` after `grip` is dragged by `d`. An edge stops
/// `min` short of the one across from it instead of passing it.
pub fn drag((lo, hi): (P, P), grip: Grip, d: P, min: f32) -> (P, P) {
    if grip == Grip::Move {
        return (lo + d, hi + d);
    }
    let (l, t, r, b) = grip.edges();
    let mut lo2 = lo;
    let mut hi2 = hi;
    if l {
        lo2.x = (lo.x + d.x).min(hi.x - min);
    }
    if t {
        lo2.y = (lo.y + d.y).min(hi.y - min);
    }
    if r {
        hi2.x = (hi.x + d.x).max(lo.x + min);
    }
    if b {
        hi2.y = (hi.y + d.y).max(lo.y + min);
    }
    (lo2, hi2)
}

/// `r`, from dragging a corner of `was`, given `was`'s shape again; the
/// corner across from the grip stays put. Edges and moves are left as is.
pub fn keep_aspect(was: (P, P), grip: Grip, r: (P, P)) -> (P, P) {
    if !matches!(grip, Grip::NE | Grip::NW | Grip::SE | Grip::SW) {
        return r;
    }
    let (w0, h0) = (was.1.x - was.0.x, was.1.y - was.0.y);
    if w0 <= 0. || h0 <= 0. {
        return r;
    }
    // The drag projected on the diagonal, so either way of moving counts.
    let k = ((r.1.x - r.0.x) * w0 + (r.1.y - r.0.y) * h0) / (w0 * w0 + h0 * h0);
    let (w, h) = (w0 * k, h0 * k);
    let (l, t, _, _) = grip.edges();
    let (mut lo, mut hi) = r;
    if l {
        lo.x = hi.x - w;
    } else {
        hi.x = lo.x + w;
    }
    if t {
        lo.y = hi.y - h;
    } else {
        hi.y = lo.y + h;
    }
    (lo, hi)
}

/// The rectangle `lo`–`hi` kept within `w` × `h`: moved back in when it was
/// being moved, else cut to it.
pub fn keep_inside((lo, hi): (P, P), w: f32, h: f32, moved: bool) -> (P, P) {
    if moved {
        let dx = (-lo.x).max(0.) + (w - hi.x).min(0.);
        let dy = (-lo.y).max(0.) + (h - hi.y).min(0.);
        let d = p(dx, dy);
        let (lo, hi) = (lo + d, hi + d);
        return (p(lo.x.max(0.), lo.y.max(0.)), p(hi.x.min(w), hi.y.min(h)));
    }
    (p(lo.x.clamp(0., w), lo.y.clamp(0., h)), p(hi.x.clamp(0., w), hi.y.clamp(0., h)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Shape {
        Shape::Rect { a: p(0., 0.), b: p(10., 10.), style: Style::default() }
    }

    #[test]
    fn undo_redo() {
        let mut d = Doc::new(2.);
        d.push(rect());
        d.push(rect());
        assert!(d.undo());
        assert_eq!(d.shapes.len(), 1);
        assert!(d.redo());
        assert_eq!(d.shapes.len(), 2);
        assert!(!d.redo());
        d.undo();
        d.push(rect());
        assert!(!d.can_redo(), "a new shape drops the redo history");
        d.undo();
        d.undo();
        assert!(!d.undo());
    }

    #[test]
    fn markers_count_up_past_the_largest() {
        let mut d = Doc::new(1.);
        assert_eq!(d.next_marker(), 1);
        d.push(Shape::Marker { at: P::default(), n: 1, style: Style::default() });
        d.push(Shape::Marker { at: P::default(), n: 2, style: Style::default() });
        assert_eq!(d.next_marker(), 3);
        d.undo();
        assert_eq!(d.next_marker(), 2);
    }

    fn line(x: f32) -> Shape {
        Shape::Line { a: p(x, 0.), b: p(x, 10.), style: Style::default() }
    }

    #[test]
    fn layers_hide_move_and_go() {
        let mut d = Doc::new(1.);
        for x in [0., 1., 2.] {
            d.push(line(x));
        }
        assert!(d.hide(1, true));
        assert_eq!(d.visible().count(), 2);
        assert!(d.reorder(1, 2), "to the top");
        assert!(d.is_hidden(2) && !d.is_hidden(1), "hidden goes with it");
        assert_eq!(d.shapes[2], line(1.));
        assert!(d.remove(2));
        assert_eq!(d.shapes, vec![line(0.), line(2.)]);
        assert!(!d.is_hidden(0) && !d.is_hidden(1));
        assert!(d.set(0, line(5.)));
        assert_eq!(d.shapes[0], line(5.));
        assert!(!d.set(7, line(5.)) && !d.hide(7, true) && !d.remove(7) && !d.reorder(0, 7));
    }

    #[test]
    fn a_new_shape_after_undo_is_not_hidden() {
        let mut d = Doc::new(1.);
        d.push(line(0.));
        d.hide(0, true);
        d.undo();
        d.push(line(1.));
        assert!(!d.is_hidden(0));
    }

    #[test]
    fn shapes_move_and_restyle() {
        let pen = Shape::Pen { points: vec![p(1., 1.), p(2., 3.)], style: Style::default() };
        assert_eq!(pen.moved(p(1., -1.)), Shape::Pen { points: vec![p(2., 0.), p(3., 2.)], style: Style::default() });
        let blue = Style { color: COLORS[1], size: 2 };
        assert_eq!(pen.with_style(blue).style(), Some(blue));
        let m = Shape::Mosaic { a: p(0., 0.), b: p(4., 4.) };
        assert_eq!(m.with_style(blue), m);
        assert_eq!(m.style(), None);
        assert_eq!(Shape::Text { at: p(0., 0.), text: "Hi\nthere".into(), style: blue }.name(), "Hi");
    }

    #[test]
    fn images_move_and_have_no_style() {
        let img = Shape::Image { image: ORIGINAL, a: p(0., 0.), b: p(4., 3.) };
        assert_eq!(img.moved(p(1., 2.)).image_rect(), Some((p(1., 2.), p(5., 5.))));
        assert_eq!((img.style(), img.tool(), img.name()), (None, None, "Picture".to_owned()));
    }

    #[test]
    fn distance_to_lines() {
        assert_eq!(to_segment(p(5., 3.), p(0., 0.), p(10., 0.)), 3.);
        assert_eq!(to_segment(p(-4., 3.), p(0., 0.), p(10., 0.)), 5.);
        let square = [p(0., 0.), p(10., 0.), p(10., 10.), p(0., 10.)];
        assert_eq!(to_polyline(p(-2., 5.), &square, true), 2.);
        assert_eq!(to_polyline(p(-2., 5.), &square, false), 5f32.hypot(2.), "not closed: the left side is open");
    }

    #[test]
    fn norm_orders_corners() {
        assert_eq!(norm(p(5., 1.), p(2., 8.)), (p(2., 1.), p(5., 8.)));
    }

    #[test]
    fn arrow_head_points_at_the_end() {
        let (shaft, [tip, l, r]) = arrow(p(0., 0.), p(100., 0.), 4.);
        assert_eq!(tip, p(100., 0.));
        assert!(shaft.x < 100. && shaft.x > 80.);
        assert!((l.x - r.x).abs() < 1e-4 && l.x < 100.);
        assert!((l.y + r.y).abs() < 1e-4, "head is symmetric");
        // A tiny arrow keeps its head within its length.
        let (_, [_, l, _]) = arrow(p(0., 0.), p(10., 0.), 6.);
        assert!(l.x >= 3.9);
    }

    #[test]
    fn ellipse_fits_its_box() {
        let pts = ellipse(p(10., 20.), p(30., 60.), 64);
        assert_eq!(pts.len(), 64);
        for q in pts {
            assert!(q.x >= 9.99 && q.x <= 30.01 && q.y >= 19.99 && q.y <= 60.01);
        }
    }

    #[test]
    fn marker_text_contrasts() {
        assert_eq!(Style { color: COLORS[7], size: 0 }.marker_text(), [0, 0, 0, 0xFF]);
        assert_eq!(Style { color: COLORS[0], size: 0 }.marker_text(), [0xFF; 4]);
    }

    #[test]
    fn grips_find_handles_then_the_inside() {
        let (lo, hi) = (p(10., 10.), p(110., 60.));
        assert_eq!(grip_at(lo, hi, p(12., 9.), 5.), Some(Grip::NW));
        assert_eq!(grip_at(lo, hi, p(60., 61.), 5.), Some(Grip::S));
        assert_eq!(grip_at(lo, hi, p(111., 35.), 5.), Some(Grip::E));
        assert_eq!(grip_at(lo, hi, p(50., 30.), 5.), Some(Grip::Move));
        assert_eq!(grip_at(lo, hi, p(200., 30.), 5.), None);
    }

    #[test]
    fn dragging_a_handle_moves_its_edges_only() {
        let r = (p(10., 10.), p(110., 60.));
        assert_eq!(drag(r, Grip::SE, p(5., 7.), 1.), (p(10., 10.), p(115., 67.)));
        assert_eq!(drag(r, Grip::N, p(5., -7.), 1.), (p(10., 3.), p(110., 60.)));
        assert_eq!(drag(r, Grip::Move, p(-20., 5.), 1.), (p(-10., 15.), p(90., 65.)));
        // An edge doesn't cross the one across from it.
        assert_eq!(drag(r, Grip::W, p(500., 0.), 2.), (p(108., 10.), p(110., 60.)));
    }

    #[test]
    fn keep_inside_moves_back_or_cuts() {
        let r = (p(-10., 5.), p(40., 30.));
        assert_eq!(keep_inside(r, 100., 100., true), (p(0., 5.), p(50., 30.)));
        assert_eq!(keep_inside(r, 100., 100., false), (p(0., 5.), p(40., 30.)));
        // Too big to fit: cut to the image.
        assert_eq!(keep_inside((p(-10., -10.), p(200., 50.)), 100., 100., true), (p(0., 0.), p(100., 60.)));
    }

    #[test]
    fn keep_aspect_scales_from_the_far_corner() {
        let was = (p(0., 0.), p(100., 50.));
        let r = drag(was, Grip::SE, p(100., 50.), 1.);
        assert_eq!(keep_aspect(was, Grip::SE, r), (p(0., 0.), p(200., 100.)));
        let r = drag(was, Grip::NW, p(50., 0.), 1.);
        let (lo, hi) = keep_aspect(was, Grip::NW, r);
        assert!((lo - p(40., 20.)).len() < 1e-3 && hi == p(100., 50.), "{lo:?}");
        assert_eq!(keep_aspect(was, Grip::E, drag(was, Grip::E, p(10., 0.), 1.)), (p(0., 0.), p(110., 50.)));
    }

    #[test]
    fn edges_are_held_anywhere_along_them() {
        let (lo, hi) = (p(0., 0.), p(100., 50.));
        assert_eq!(edge_grip(lo, hi, p(105., 20.), 2., 8.), Some(Grip::E));
        assert_eq!(edge_grip(lo, hi, p(99., 20.), 2., 8.), Some(Grip::E));
        assert_eq!(edge_grip(lo, hi, p(96., 20.), 2., 8.), None, "too far inside");
        assert_eq!(edge_grip(lo, hi, p(109., 20.), 2., 8.), None, "too far out");
        assert_eq!(edge_grip(lo, hi, p(-3., 55.), 2., 8.), Some(Grip::SW));
        assert_eq!(edge_grip(lo, hi, p(30., -4.), 2., 8.), Some(Grip::N));
        assert_eq!(edge_grip(lo, hi, p(50., 25.), 2., 8.), None);
        // A thin rectangle: the nearer edge.
        assert_eq!(edge_grip(p(0., 0.), p(4., 50.), p(5., 20.), 8., 8.), Some(Grip::E));
    }
}
