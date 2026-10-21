//! Platform-independent geometry: which monitor edges touch, and how a point
//! that leaves one monitor maps onto its neighbour.
//!
//! Everything expensive happens once in [`Layout::build`]. The per-event path
//! ([`Layout::on_exit`]) is a handful of comparisons and one multiply-add, with
//! no allocation, so it is safe to call from a low-level mouse hook.

use crate::config::{Align, Mode};

/// Pixel rectangle in virtual-desktop coordinates; `right`/`bottom` are exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self { left, top, right, bottom }
    }

    #[inline(always)]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    fn clamp(&self, x: i32, y: i32) -> (i32, i32) {
        (x.clamp(self.left, self.right - 1), y.clamp(self.top, self.bottom - 1))
    }
}

/// A monitor as the layout needs it: where it is in pixels and how big it is
/// physically.
#[derive(Clone, Debug)]
pub struct Screen {
    pub rect: Rect,
    /// Physical size of the visible area in millimetres (width, height).
    pub size_mm: (f64, f64),
}

impl Screen {
    fn mm_per_px_x(&self) -> f64 {
        self.size_mm.0 / self.rect.width() as f64
    }

    fn mm_per_px_y(&self) -> f64 {
        self.size_mm.1 / self.rect.height() as f64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    const ALL: [Side; 4] = [Side::Left, Side::Right, Side::Top, Side::Bottom];

    fn index(self) -> usize {
        self as usize
    }

    fn is_vertical_edge(self) -> bool {
        matches!(self, Side::Left | Side::Right)
    }
}

/// One stretch of a source edge that leads to `target`.
///
/// Coordinates along the edge are continuous (pixel `p` covers `[p, p+1)`), so
/// `target_coord = s * scale + offset` works for both directions of travel.
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub target: usize,
    /// Part of the source edge that leads to `target`, `[s_min, s_max)`.
    pub s_min: f64,
    pub s_max: f64,
    pub scale: f64,
    pub offset: f64,
    /// Ratio of physical pixel pitch across the edge, used to carry overshoot.
    pub perp_scale: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Leave the event alone.
    Pass,
    /// Swallow the event and put the cursor here instead.
    Move(i32, i32),
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub screens: Vec<Screen>,
    /// `links[screen][side]`.
    pub links: Vec<[Vec<Link>; 4]>,
}

impl Layout {
    pub fn build(screens: Vec<Screen>, mode: Mode, align: Align) -> Self {
        let mut links: Vec<[Vec<Link>; 4]> = (0..screens.len()).map(|_| Default::default()).collect();
        for (si, s) in screens.iter().enumerate() {
            for (ti, t) in screens.iter().enumerate() {
                if si == ti {
                    continue;
                }
                for side in Side::ALL {
                    if let Some(link) = make_link(s, t, ti, side, mode, align) {
                        links[si][side.index()].push(link);
                    }
                }
            }
        }
        Self { screens, links }
    }

    #[inline]
    pub fn screen_at(&self, x: i32, y: i32) -> Option<usize> {
        self.screens.iter().position(|s| s.rect.contains(x, y))
    }

    /// Decide what to do when the cursor, currently on screen `src`, is about
    /// to move to `(x, y)`.
    pub fn on_exit(&self, src: usize, x: i32, y: i32) -> Decision {
        let r = self.screens[src].rect;
        if r.contains(x, y) {
            return Decision::Pass;
        }

        // How far past each edge the point is; the larger overshoot wins at corners.
        let ox = if x < r.left {
            r.left - x
        } else if x >= r.right {
            x - r.right + 1
        } else {
            0
        };
        let oy = if y < r.top {
            r.top - y
        } else if y >= r.bottom {
            y - r.bottom + 1
        } else {
            0
        };
        let hside = if x < r.left { Side::Left } else { Side::Right };
        let vside = if y < r.top { Side::Top } else { Side::Bottom };
        let order: &[Side] = match (ox > 0, oy > 0) {
            (true, false) => &[hside][..],
            (false, true) => &[vside][..],
            _ if ox >= oy => &[hside, vside][..],
            _ => &[vside, hside][..],
        };

        for &side in order {
            let links = &self.links[src][side.index()];
            if links.is_empty() {
                continue;
            }
            let (along, over) = if side.is_vertical_edge() {
                (y.clamp(r.top, r.bottom - 1), ox)
            } else {
                (x.clamp(r.left, r.right - 1), oy)
            };
            return self.cross(side, links, along, over);
        }

        // Nothing is linked here. A diagonal-only neighbour is still reachable
        // the native way; otherwise hold the cursor on its current screen so
        // Windows does not snap it somewhere unexpected.
        if self.screen_at(x, y).is_some() {
            return Decision::Pass;
        }
        let (cx, cy) = r.clamp(x, y);
        Decision::Move(cx, cy)
    }

    fn cross(&self, side: Side, links: &[Link], along: i32, over: i32) -> Decision {
        let s = along as f64 + 0.5;
        let mut best = &links[0];
        let mut best_dist = f64::INFINITY;
        for l in links {
            let d = if s < l.s_min {
                l.s_min - s
            } else if s >= l.s_max {
                s - l.s_max
            } else {
                0.0
            };
            if d < best_dist {
                best_dist = d;
                best = l;
            }
        }
        let s = s.clamp(best.s_min, best.s_max - 1e-6);
        let t = (s * best.scale + best.offset).floor() as i32;
        let depth = ((over as f64 * best.perp_scale).round() as i32).max(1);

        let tr = self.screens[best.target].rect;
        let (x, y) = match side {
            Side::Left => (tr.right - depth, t),
            Side::Right => (tr.left + depth - 1, t),
            Side::Top => (t, tr.bottom - depth),
            Side::Bottom => (t, tr.top + depth - 1),
        };
        let (x, y) = tr.clamp(x, y);
        Decision::Move(x, y)
    }
}

/// Build the link from `s`'s `side` edge to `t`, if the two touch there.
fn make_link(s: &Screen, t: &Screen, ti: usize, side: Side, mode: Mode, align: Align) -> Option<Link> {
    let (sr, tr) = (s.rect, t.rect);
    let touches = match side {
        Side::Left => tr.right == sr.left,
        Side::Right => tr.left == sr.right,
        Side::Top => tr.bottom == sr.top,
        Side::Bottom => tr.top == sr.bottom,
    };
    if !touches {
        return None;
    }

    // Edge intervals along the shared axis, and pixel pitch along / across it.
    let (s0, s1, t0, t1, ms, mt, perp_scale) = if side.is_vertical_edge() {
        (sr.top, sr.bottom, tr.top, tr.bottom, s.mm_per_px_y(), t.mm_per_px_y(), s.mm_per_px_x() / t.mm_per_px_x())
    } else {
        (sr.left, sr.right, tr.left, tr.right, s.mm_per_px_x(), t.mm_per_px_x(), s.mm_per_px_y() / t.mm_per_px_y())
    };
    // Windows only lets the cursor cross where the edges overlap in pixels.
    if t0 >= s1 || s0 >= t1 {
        return None;
    }
    let (s0f, s1f, t0f, t1f) = (s0 as f64, s1 as f64, t0 as f64, t1 as f64);

    let proportional = || {
        let scale = (t1f - t0f) / (s1f - s0f);
        Link { target: ti, s_min: s0f, s_max: s1f, scale, offset: t0f - s0f * scale, perp_scale }
    };

    let link = match mode {
        Mode::Proportional => proportional(),
        Mode::Physical => {
            let ls = (s1f - s0f) * ms;
            let lt = (t1f - t0f) * mt;
            // `d`: where the target edge starts, in mm from the source edge start.
            let d = match resolve_align(align, s0, s1, t0, t1) {
                Align::Start => 0.0,
                Align::End => ls - lt,
                _ => (ls - lt) / 2.0,
            };
            let scale = ms / mt;
            let offset = t0f - s0f * scale - d / mt;
            let s_min = (s0f + d / ms).max(s0f);
            let s_max = (s0f + (d + lt) / ms).min(s1f);
            if s_max - s_min < 1.0 {
                proportional()
            } else {
                Link { target: ti, s_min, s_max, scale, offset, perp_scale }
            }
        }
    };
    Some(link)
}

/// `Auto` picks whichever of start / center / end the Windows arrangement is
/// closest to, so the user expresses alignment in Display Settings.
fn resolve_align(align: Align, s0: i32, s1: i32, t0: i32, t1: i32) -> Align {
    if align != Align::Auto {
        return align;
    }
    let start = (t0 - s0).abs();
    let end = (t1 - s1).abs();
    let center = ((t0 + t1) - (s0 + s1)).abs() / 2;
    if start <= center && start <= end {
        Align::Start
    } else if end <= center {
        Align::End
    } else {
        Align::Center
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Eugene's setup: 24" 1920x1080 on the left, 27" 2560x1440 primary.
    fn fhd24(left: i32, top: i32) -> Screen {
        Screen { rect: Rect::new(left, top, left + 1920, top + 1080), size_mm: (531.4, 298.9) }
    }
    fn qhd27() -> Screen {
        Screen { rect: Rect::new(0, 0, 2560, 1440), size_mm: (597.7, 336.2) }
    }

    fn mv(d: Decision) -> (i32, i32) {
        match d {
            Decision::Move(x, y) => (x, y),
            Decision::Pass => panic!("expected Move, got Pass"),
        }
    }

    #[test]
    fn inside_passes() {
        let l = Layout::build(vec![fhd24(-1920, 0), qhd27()], Mode::Physical, Align::Auto);
        assert_eq!(l.on_exit(1, 100, 100), Decision::Pass);
    }

    #[test]
    fn bottom_aligned_physical() {
        // Bottoms aligned in Windows: 24" spans y 360..1440.
        let l = Layout::build(vec![fhd24(-1920, 360), qhd27()], Mode::Physical, Align::Auto);
        // Bottom row crosses to bottom row.
        assert_eq!(mv(l.on_exit(1, -1, 1439)), (-1, 1439));
        // Middle of the 24" (physically 149.5 mm above the bottom) is reached
        // from 149.5 mm above the 27" bottom, i.e. 1440 - 149.5/0.2335 = ~800.
        let (_, y) = mv(l.on_exit(1, -1, 800));
        assert!((y - (360 + 540)).abs() <= 2, "y = {y}");
        // Top strip of the 27" with nothing physically beside it clamps to the top row.
        assert_eq!(mv(l.on_exit(1, -1, 10)), (-1, 360));
    }

    #[test]
    fn round_trip_is_stable() {
        let l = Layout::build(vec![fhd24(-1920, 360), qhd27()], Mode::Physical, Align::Auto);
        for y in (400..1440).step_by(37) {
            let (x1, y1) = mv(l.on_exit(1, -1, y));
            assert_eq!(x1, -1);
            let (x2, y2) = mv(l.on_exit(0, 0, y1));
            assert_eq!(x2, 0);
            assert!((y2 - y).abs() <= 1, "{y} -> {y1} -> {y2}");
        }
    }

    #[test]
    fn top_aligned_windows_layout_is_not_sticky() {
        // Default Windows placement: tops aligned, so y > 1080 on the 27"
        // normally sticks at the left edge.
        let l = Layout::build(vec![fhd24(-1920, 0), qhd27()], Mode::Physical, Align::Auto);
        let (x, y) = mv(l.on_exit(1, -1, 1400));
        assert_eq!((x, y), (-1, 1079));
        // 24" top row maps to 27" top row.
        assert_eq!(mv(l.on_exit(0, 0, 0)), (0, 0));
    }

    #[test]
    fn proportional_maps_whole_edges() {
        let l = Layout::build(vec![fhd24(-1920, 0), qhd27()], Mode::Proportional, Align::Auto);
        assert_eq!(mv(l.on_exit(1, -1, 0)), (-1, 0));
        assert_eq!(mv(l.on_exit(1, -1, 1439)), (-1, 1079));
        assert_eq!(mv(l.on_exit(1, -1, 720)), (-1, 540));
        assert_eq!(mv(l.on_exit(0, 0, 540)), (0, 720));
    }

    #[test]
    fn overshoot_is_carried() {
        let l = Layout::build(vec![fhd24(-1920, 0), qhd27()], Mode::Proportional, Align::Auto);
        let (x, _) = mv(l.on_exit(1, -10, 500));
        assert!((-10..=-7).contains(&x), "x = {x}");
    }

    #[test]
    fn outer_edge_holds_cursor() {
        let l = Layout::build(vec![fhd24(-1920, 0), qhd27()], Mode::Physical, Align::Auto);
        // Above the 27" there is only the bounding-box gap: stay put.
        assert_eq!(mv(l.on_exit(1, 50, -3)), (50, 0));
        // Left of the 24" is the desktop edge.
        assert_eq!(mv(l.on_exit(0, -1921, 50)), (-1920, 50));
    }

    #[test]
    fn corner_prefers_linked_side() {
        let l = Layout::build(vec![fhd24(-1920, 0), qhd27()], Mode::Physical, Align::Auto);
        // Leaving the 27" top-left corner diagonally, more upward than leftward:
        // top has no neighbour, so it falls back to the left link.
        let (x, y) = mv(l.on_exit(1, -1, -5));
        assert_eq!(x, -1);
        assert_eq!(y, 0);
    }

    #[test]
    fn vertical_stack() {
        let top = Screen { rect: Rect::new(0, -1080, 1920, 0), size_mm: (531.4, 298.9) };
        let l = Layout::build(vec![top, qhd27()], Mode::Physical, Align::Center);
        let (x, y) = mv(l.on_exit(1, 1280, -1));
        assert_eq!(y, -1);
        assert!((x - 960).abs() <= 1, "x = {x}");
    }
}
