//! Crop and straighten geometry.
//!
//! A crop is a rectangle `[x0, y0, x1, y1]` in the *straightened frame*: the picture turned
//! by the straighten angle about its centre, in units where the unrotated picture spans
//! 0..1 on each axis. With no angle that is simply a rectangle on the picture. The rectangle
//! must stay inside the turned picture, which is what `Geometry` checks and enforces.

pub type Rect = [f32; 4];
pub const FULL: Rect = [0.0, 0.0, 1.0, 1.0];
/// Smallest crop side, as a fraction of the picture.
pub const MIN_SIDE: f32 = 0.02;

/// A point of the picture as stored (camera orientation) to where it is displayed after
/// `turns` clockwise quarter turns, and back.
fn display_from_stored(turns: u8, (u, v): (f32, f32)) -> (f32, f32) {
    match turns % 4 {
        0 => (u, v),
        1 => (1.0 - v, u),
        2 => (1.0 - u, 1.0 - v),
        _ => (v, 1.0 - u),
    }
}

fn stored_from_display(turns: u8, (x, y): (f32, f32)) -> (f32, f32) {
    match turns % 4 {
        0 => (x, y),
        1 => (y, 1.0 - x),
        2 => (1.0 - x, 1.0 - y),
        _ => (1.0 - y, x),
    }
}

fn through(r: Rect, map: impl Fn((f32, f32)) -> (f32, f32)) -> Rect {
    let (a, b) = (map((r[0], r[1])), map((r[2], r[3])));
    [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)]
}

/// The stored crop as it lies on the displayed (quarter-turned) picture, and back.
pub fn to_display(turns: u8, stored: Rect) -> Rect {
    through(stored, |p| display_from_stored(turns, p))
}

pub fn to_stored(turns: u8, display: Rect) -> Rect {
    through(display, |p| stored_from_display(turns, p))
}

/// Which sides of the rectangle a drag moves; none = move the whole rectangle.
#[derive(Clone, Copy, PartialEq, Default)]
pub struct Handle {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Handle {
    pub fn is_move(self) -> bool {
        !(self.left || self.right || self.top || self.bottom)
    }
}

/// The rectangle a drag asks for, before it is kept inside the picture.
/// `ratio` locks width / height (in these normalised units).
pub fn dragged(start: Rect, handle: Handle, (dx, dy): (f32, f32), ratio: Option<f32>) -> Rect {
    let [mut x0, mut y0, mut x1, mut y1] = start;
    if handle.is_move() {
        return [x0 + dx, y0 + dy, x1 + dx, y1 + dy];
    }
    if handle.left {
        x0 = (x0 + dx).min(x1 - MIN_SIDE);
    }
    if handle.right {
        x1 = (x1 + dx).max(x0 + MIN_SIDE);
    }
    if handle.top {
        y0 = (y0 + dy).min(y1 - MIN_SIDE);
    }
    if handle.bottom {
        y1 = (y1 + dy).max(y0 + MIN_SIDE);
    }
    if let Some(ratio) = ratio {
        let (mut w, mut h) = (x1 - x0, y1 - y0);
        let (horizontal, vertical) = (handle.left || handle.right, handle.top || handle.bottom);
        if horizontal && vertical {
            // A corner: the opposite corner stays put.
            if w / h > ratio {
                w = h * ratio;
            } else {
                h = w / ratio;
            }
            if handle.left {
                x0 = x1 - w;
            } else {
                x1 = x0 + w;
            }
            if handle.top {
                y0 = y1 - h;
            } else {
                y1 = y0 + h;
            }
        } else if horizontal {
            h = w / ratio;
            let centre = (start[1] + start[3]) * 0.5;
            y0 = centre - h * 0.5;
            y1 = centre + h * 0.5;
        } else {
            w = h * ratio;
            let centre = (start[0] + start[2]) * 0.5;
            x0 = centre - w * 0.5;
            x1 = centre + w * 0.5;
        }
    }
    [x0, y0, x1, y1]
}

/// The largest rectangle of the given width / height that fits inside `r`, same centre.
pub fn with_ratio(r: Rect, ratio: f32) -> Rect {
    let (mut w, mut h) = (r[2] - r[0], r[3] - r[1]);
    if w / h > ratio {
        w = h * ratio;
    } else {
        h = w / ratio;
    }
    let (cx, cy) = ((r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5);
    [cx - w * 0.5, cy - h * 0.5, cx + w * 0.5, cy + h * 0.5]
}

fn lerp(a: Rect, b: Rect, t: f32) -> Rect {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// The displayed picture's shape and straighten angle.
#[derive(Clone, Copy)]
pub struct Geometry {
    /// Width / height of the displayed picture.
    pub aspect: f32,
    pub cos: f32,
    pub sin: f32,
}

impl Geometry {
    pub fn new(width: f32, height: f32, angle_degrees: f32) -> Self {
        let a = angle_degrees.to_radians();
        Self { aspect: width / height.max(1.0), cos: a.cos(), sin: a.sin() }
    }

    /// Where a point of the straightened frame falls on the picture (0..1 = inside).
    /// The shader does the same sum.
    pub fn source(&self, (x, y): (f32, f32)) -> (f32, f32) {
        let (qx, qy) = (x - 0.5, y - 0.5);
        (self.cos * qx - self.sin * qy / self.aspect + 0.5, self.sin * qx * self.aspect + self.cos * qy + 0.5)
    }

    /// True if the whole rectangle shows picture (no empty corners).
    pub fn inside(&self, r: Rect) -> bool {
        let e = 1.0e-4;
        [(r[0], r[1]), (r[2], r[1]), (r[0], r[3]), (r[2], r[3])].into_iter().all(|corner| {
            let (x, y) = self.source(corner);
            x >= -e && y >= -e && x <= 1.0 + e && y <= 1.0 + e
        })
    }

    /// Size of the turned picture's bounding box, in the same normalised units.
    pub fn bounds(&self) -> (f32, f32) {
        (self.cos.abs() + self.sin.abs() / self.aspect, self.sin.abs() * self.aspect + self.cos.abs())
    }

    /// As far from the valid rectangle `from` towards `wanted` as stays inside the picture.
    pub fn toward(&self, from: Rect, wanted: Rect) -> Rect {
        if self.inside(wanted) {
            return wanted;
        }
        if !self.inside(from) {
            return self.constrain(wanted);
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (lo + hi) * 0.5;
            if self.inside(lerp(from, wanted, mid)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lerp(from, wanted, lo)
    }

    /// `wanted` pulled towards the centre and shrunk (keeping its shape) until it fits.
    pub fn constrain(&self, wanted: Rect) -> Rect {
        if self.inside(wanted) {
            return wanted;
        }
        let (w, h) = (wanted[2] - wanted[0], wanted[3] - wanted[1]);
        let (mut cx, mut cy) = ((wanted[0] + wanted[2]) * 0.5, (wanted[1] + wanted[3]) * 0.5);
        let centre_ok = |cx: f32, cy: f32| {
            let (x, y) = self.source((cx, cy));
            (0.02..=0.98).contains(&x) && (0.02..=0.98).contains(&y)
        };
        for _ in 0..60 {
            if centre_ok(cx, cy) {
                break;
            }
            cx = 0.5 + (cx - 0.5) * 0.9;
            cy = 0.5 + (cy - 0.5) * 0.9;
        }
        let at = |s: f32| [cx - w * s * 0.5, cy - h * s * 0.5, cx + w * s * 0.5, cy + h * s * 0.5];
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (lo + hi) * 0.5;
            if self.inside(at(mid)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        at(lo.max(1.0e-3))
    }
}
