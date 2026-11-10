//! A scrolling capture: frames of one region, grabbed while what is in it
//! scrolls down, joined into one tall picture.
//!
//! Each frame is matched against the last one kept, row by row. Only the
//! columns that changed between the two are compared, so what stays put
//! beside the part that scrolls (a sidebar, another window) doesn't stop
//! rows from matching. Rows that stay put at the top and bottom (a
//! toolbar, a status bar) are left out of the match, the rows between are
//! tried at every shift, and the rows that scrolled in are added above the
//! bottom ones.
//!
//! A row is compared as [`SEGS`] hashes, one per stretch of its compared
//! columns, and matches with one stretch off: the overlay scroller macOS
//! shows while scrolling, a blinking caret, a spinner.

use image::RgbaImage;

/// The tallest picture kept, in pixels (about 190 MB at 1500 px wide).
pub const MAX_HEIGHT: u32 = 32_000;
/// The most memory the picture may take, whatever its width: a wide
/// region stops short of [`MAX_HEIGHT`].
const MAX_BYTES: usize = 256 << 20;

/// Of the rows a shift is checked on, the share that must match.
const MATCH: f32 = 0.95;
/// Rows with something on them that a shift must be checked on.
const MIN_ROWS: usize = 6;
/// The stretches a row is hashed in.
const SEGS: usize = 8;

/// What became of a frame given to [`Stitcher::push`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Push {
    /// Nothing scrolled.
    Same,
    /// It scrolled by this many pixels, which were added.
    Added(u32),
    /// It doesn't follow on from the last frame kept (scrolled too far at
    /// once, or up); it is left out, and the next ones are matched against
    /// the same frame.
    Lost,
    /// The picture is as tall as it may get; nothing more is added.
    Full,
}

pub struct Stitcher {
    width: u32,
    height: u32,
    /// The picture so far, RGBA rows; it always ends with the bottom rows
    /// of the last frame kept.
    rows: Vec<u8>,
    last: RgbaImage,
}

impl Stitcher {
    /// Start with the first frame, whole.
    pub fn new(first: RgbaImage) -> Self {
        let mut rows = Vec::with_capacity(first.as_raw().len() * 4);
        rows.extend_from_slice(first.as_raw());
        Self { width: first.width(), height: first.height(), rows, last: first }
    }

    /// The tallest the picture may get, in pixels.
    fn max_height(&self) -> usize {
        (MAX_BYTES / self.row_bytes().max(1)).clamp(self.height as usize, MAX_HEIGHT as usize)
    }

    /// The picture's height so far, in pixels.
    pub fn height(&self) -> u32 {
        (self.rows.len() / self.row_bytes()) as u32
    }

    fn row_bytes(&self) -> usize {
        self.width as usize * 4
    }

    /// Add the part of `frame` that scrolled into view since the last
    /// frame kept. A frame of another size is [`Push::Lost`].
    pub fn push(&mut self, frame: RgbaImage) -> Push {
        if frame.dimensions() != (self.width, self.height) {
            return Push::Lost;
        }
        let cols = changed_columns(&self.last, &frame);
        if cols.is_empty() {
            return Push::Same;
        }
        let old = Rows::new(&self.last, &cols);
        let new = Rows::new(&frame, &cols);
        let Some(m) = find_shift(&old, &new) else { return Push::Lost };
        if m.dy == 0 {
            return Push::Same;
        }
        let max = self.max_height();
        if self.height() as usize >= max {
            return Push::Full;
        }
        // Drop the old bottom rows, then add what scrolled in and the new
        // bottom rows.
        let h = self.height as usize;
        let rb = self.row_bytes();
        let keep = self.rows.len() - m.bottom * rb;
        self.rows.truncate(keep);
        let from = h - m.bottom - m.dy;
        let add = &frame.as_raw()[from * rb..];
        // Grown by half again, not doubled: no more than a third left over.
        if self.rows.capacity() - self.rows.len() < add.len() {
            let want = (self.rows.len() + add.len()).max(self.rows.len() * 3 / 2).min(max * rb);
            self.rows.reserve_exact(want.saturating_sub(self.rows.len()));
        }
        self.rows.extend_from_slice(add);
        if self.rows.len() > max * rb {
            self.rows.truncate(max * rb);
        }
        self.last = frame;
        Push::Added(m.dy as u32)
    }

    /// The picture.
    pub fn finish(mut self) -> RgbaImage {
        let h = self.height();
        self.rows.shrink_to_fit();
        RgbaImage::from_raw(self.width, h, self.rows).expect("whole rows")
    }
}

/// How a frame lines up with the one before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shift {
    /// Rows that stayed put at the bottom.
    bottom: usize,
    /// How far the rows between scrolled up.
    dy: usize,
}

/// A frame's rows, on the columns compared: each row's stretch hashes,
/// and whether it is all one colour (which can't tell shifts apart).
struct Rows {
    hash: Vec<[u64; SEGS]>,
    blank: Vec<bool>,
    /// The stretches in use (fewer than [`SEGS`] when few columns are).
    segs: usize,
}

impl Rows {
    fn new(img: &RgbaImage, cols: &[usize]) -> Self {
        let segs = cols.len().clamp(1, SEGS);
        let (pixels, _) = img.as_raw().as_chunks::<4>();
        let w = img.width() as usize;
        let mut hash = Vec::with_capacity(img.height() as usize);
        let mut blank = Vec::with_capacity(img.height() as usize);
        for row in pixels.chunks_exact(w) {
            let mut h = [0u64; SEGS];
            for (k, h) in h.iter_mut().enumerate().take(segs) {
                let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
                for &x in &cols[k * cols.len() / segs..(k + 1) * cols.len() / segs] {
                    acc = (acc ^ u64::from(u32::from_le_bytes(row[x]))).wrapping_mul(0x0100_0000_01b3).rotate_left(23);
                }
                *h = acc;
            }
            hash.push(h);
            let first = row[cols[0]];
            blank.push(cols.iter().all(|&x| row[x] == first));
        }
        Self { hash, blank, segs }
    }

    /// Whether row `a` of these and row `b` of `other` match: all their
    /// stretches but one, when there are a few.
    fn matches(&self, a: usize, other: &Rows, b: usize) -> bool {
        let same = self.hash[a].iter().zip(&other.hash[b]).take(self.segs).filter(|(x, y)| x == y).count();
        let slack = usize::from(self.segs >= 4);
        same + slack >= self.segs
    }
}

/// The columns where `a` and `b` differ in some row.
fn changed_columns(a: &RgbaImage, b: &RgbaImage) -> Vec<usize> {
    let w = a.width() as usize;
    let (pa, _) = a.as_raw().as_chunks::<4>();
    let (pb, _) = b.as_raw().as_chunks::<4>();
    let mut changed = vec![false; w];
    for (ra, rb) in pa.chunks_exact(w).zip(pb.chunks_exact(w)) {
        if ra == rb {
            continue;
        }
        for (x, c) in changed.iter_mut().enumerate() {
            *c |= ra[x] != rb[x];
        }
    }
    changed.iter().enumerate().filter(|(_, c)| **c).map(|(x, _)| x).collect()
}

/// How `new` follows on from `old`.
fn find_shift(old: &Rows, new: &Rows) -> Option<Shift> {
    let h = old.hash.len();
    // Rows that stayed put, at most a half each, and some left to match.
    let cap = h / 2;
    let top = old.hash.iter().zip(&new.hash).take_while(|(a, b)| a == b).count().min(cap);
    let bottom = old.hash.iter().rev().zip(new.hash.iter().rev()).take_while(|(a, b)| a == b).count().min(cap);
    let end = h.saturating_sub(bottom);
    if end <= top + MIN_ROWS {
        return None;
    }
    // How many rows that aren't blank `new` has before each row, so how
    // many a shift is checked on is known before checking it.
    let mut lit = vec![0usize; h + 1];
    for y in 0..h {
        lit[y + 1] = lit[y] + usize::from(!new.blank[y]);
    }
    // The share of `new`'s rows, from `top` to `end - dy`, that match
    // `old`'s `dy` rows further down. Given up as soon as too many miss
    // for it to pass, so most shifts cost a few rows, not all of them.
    let score = |dy: usize| -> Option<usize> {
        let seen = lit[end - dy] - lit[top];
        if seen < MIN_ROWS {
            return None;
        }
        let need = seen as f32 * MATCH;
        let (mut miss, mut hit) = (0, 0);
        for y in top..end - dy {
            if new.blank[y] {
                continue;
            }
            if new.matches(y, old, y + dy) {
                hit += 1;
            } else {
                miss += 1;
                if ((seen - miss) as f32) < need {
                    return None;
                }
            }
        }
        (hit as f32 >= need).then_some(hit)
    };
    if score(0).is_some() {
        return Some(Shift { bottom, dy: 0 });
    }
    // The shift with the most rows to back it: the smallest, of those a
    // repeating pattern would allow.
    let mut best: Option<(usize, usize)> = None;
    for dy in 1..end - top {
        if let Some(hit) = score(dy)
            && best.is_none_or(|(_, b)| hit > b)
        {
            best = Some((dy, hit));
        }
    }
    best.map(|(dy, _)| Shift { bottom, dy })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// A long page: each row a different pattern (with some blank rows).
    fn page(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            if y % 7 == 3 {
                Rgba([255, 255, 255, 255])
            } else {
                let v = (x.wrapping_mul(31) ^ y.wrapping_mul(2_654_435_761)).wrapping_mul(97);
                Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
            }
        })
    }

    /// What a `h` rows high view shows of `page` scrolled to `at`, with a
    /// `bar` rows high bar fixed at its top and at its bottom.
    fn view(page: &RgbaImage, at: u32, h: u32, bar: u32) -> RgbaImage {
        RgbaImage::from_fn(page.width(), h, |x, y| {
            if y < bar {
                Rgba([10, 20, 30, 255])
            } else if y >= h - bar {
                Rgba([200, (x % 256) as u8, 40, 255])
            } else {
                *page.get_pixel(x, at + y)
            }
        })
    }

    #[test]
    fn joins_a_page_scrolled_down() {
        let page = page(64, 400);
        let mut s = Stitcher::new(view(&page, 0, 100, 0));
        assert_eq!(s.push(view(&page, 30, 100, 0)), Push::Added(30));
        assert_eq!(s.push(view(&page, 30, 100, 0)), Push::Same);
        assert_eq!(s.push(view(&page, 95, 100, 0)), Push::Added(65));
        let out = s.finish();
        assert_eq!(out.height(), 195);
        assert_eq!(out, image::imageops::crop_imm(&page, 0, 0, 64, 195).to_image());
    }

    #[test]
    fn keeps_fixed_bars_once() {
        let page = page(64, 400);
        let bar = 12;
        let mut s = Stitcher::new(view(&page, 0, 100, bar));
        assert_eq!(s.push(view(&page, 20, 100, bar)), Push::Added(20));
        assert_eq!(s.push(view(&page, 50, 100, bar)), Push::Added(30));
        let out = s.finish();
        // The top bar, the page from `bar` to the last view's bottom, the
        // bottom bar.
        assert_eq!(out.height(), 150);
        let last = view(&page, 50, 100, bar);
        assert_eq!(out.get_pixel(5, 0), last.get_pixel(5, 0));
        assert_eq!(out.get_pixel(5, 149), last.get_pixel(5, 99));
        for y in bar..150 - bar {
            assert_eq!(out.get_pixel(9, y), page.get_pixel(9, y), "row {y}");
        }
    }

    #[test]
    fn a_jump_too_far_is_lost_and_matched_again_later() {
        let page = page(64, 600);
        let mut s = Stitcher::new(view(&page, 0, 100, 0));
        assert_eq!(s.push(view(&page, 300, 100, 0)), Push::Lost);
        assert_eq!(s.push(view(&page, 40, 100, 0)), Push::Added(40));
        assert_eq!(s.height(), 140);
    }

    #[test]
    fn scrolling_up_is_lost() {
        let page = page(64, 600);
        let mut s = Stitcher::new(view(&page, 100, 100, 0));
        assert_eq!(s.push(view(&page, 80, 100, 0)), Push::Lost);
        assert_eq!(s.height(), 100);
    }

    #[test]
    fn the_right_edge_is_not_compared() {
        let page = page(200, 400);
        let mut s = Stitcher::new(view(&page, 0, 100, 0));
        // A scroller shows up on the right as it scrolls.
        let mut next = view(&page, 25, 100, 0);
        for y in 10..60 {
            for x in 192..200 {
                next.put_pixel(x, y, Rgba([128, 128, 128, 255]));
            }
        }
        assert_eq!(s.push(next), Push::Added(25));
    }

    #[test]
    fn a_sidebar_that_stays_put_is_not_compared() {
        let side = page(60, 100);
        let page = page(200, 400);
        // A 60 px sidebar on the left, the page scrolling beside it.
        let with_side = |at: u32| {
            let mut v = view(&page, at, 100, 0);
            image::imageops::replace(&mut v, &side, 0, 0);
            v
        };
        let mut s = Stitcher::new(with_side(0));
        assert_eq!(s.push(with_side(30)), Push::Added(30));
        assert_eq!(s.push(with_side(70)), Push::Added(40));
        assert_eq!(s.height(), 170);
    }

    #[test]
    fn a_blinking_caret_does_not_stop_a_match() {
        let page = page(200, 400);
        let mut s = Stitcher::new(view(&page, 0, 100, 0));
        let mut next = view(&page, 25, 100, 0);
        for y in 0..100 {
            next.put_pixel(100, y, Rgba([0, 0, 0, 255]));
        }
        assert_eq!(s.push(next), Push::Added(25));
    }

    #[test]
    fn a_blank_region_never_matches() {
        let white = RgbaImage::from_pixel(50, 80, Rgba([255; 4]));
        let mut s = Stitcher::new(white.clone());
        assert_eq!(s.push(white), Push::Same);
        let mut other = RgbaImage::from_pixel(50, 80, Rgba([255; 4]));
        other.put_pixel(3, 40, Rgba([0, 0, 0, 255]));
        assert_eq!(s.push(other), Push::Lost);
    }

    #[test]
    fn another_size_is_lost() {
        let page = page(64, 400);
        let mut s = Stitcher::new(view(&page, 0, 100, 0));
        assert_eq!(s.push(view(&page, 10, 90, 0)), Push::Lost);
    }
}
