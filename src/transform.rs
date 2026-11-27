//! Perspective correction (Transform), as in Lightroom: Vertical and Horizontal tilt the
//! picture as if the camera had been pointed differently, Aspect stretches it, Scale and the
//! offsets place it; Upright finds the photo's straight lines and sets Vertical, Horizontal and
//! the angle so they stand straight; Guided does the same for lines the user draws.
//!
//! The correction is a homography on the displayed picture (after quarter turns, before the
//! straighten angle), in centred coordinates measured in picture heights (x times the picture's
//! width / height). Vertical and Horizontal are turns of a virtual camera about its horizontal
//! and vertical axes, through the lens's focal length, so lines converge and straighten as they
//! would had the camera been level.

/// Vertical, Horizontal, Aspect, Scale (%), X Offset, Y Offset.
pub type Settings = [f32; 6];
pub const DEFAULT: Settings = [0.0, 0.0, 0.0, 100.0, 0.0, 0.0];

/// Name, range, default, unit, what it does.
pub const SLIDERS: [(&str, f32, f32, f32, &str, &str); 6] = [
    ("Vertical", -100.0, 100.0, 0.0, "", "Tilts the photo forward or back: move left when buildings lean in at the top"),
    ("Horizontal", -100.0, 100.0, 0.0, "", "Turns the photo left or right, for a wall seen at an angle"),
    ("Aspect", -100.0, 100.0, 0.0, "", "Stretches the photo wider (right) or taller (left), after a strong correction"),
    ("Scale", 50.0, 150.0, 100.0, "%", "Makes the corrected photo bigger or smaller in the frame"),
    ("X Offset", -100.0, 100.0, 0.0, "", "Moves the photo left or right"),
    ("Y Offset", -100.0, 100.0, 0.0, "", "Moves the photo up or down"),
];

/// How far Vertical and Horizontal turn the virtual camera at ±100, in degrees.
const MAX_TILT: f32 = 40.0;

pub fn is_identity(s: &Settings) -> bool {
    s.iter().zip(DEFAULT).all(|(a, b)| (a - b).abs() < 1e-4)
}

pub fn clamp(s: Settings) -> Settings {
    std::array::from_fn(|i| if s[i].is_finite() { s[i].clamp(SLIDERS[i].1, SLIDERS[i].2) } else { DEFAULT[i] })
}

pub type M3 = [[f32; 3]; 3];
#[cfg(test)]
const IDENTITY: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn mul(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

pub fn invert(m: &M3) -> Option<M3> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let c = |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let adj = [
        [c(1, 2, 1, 2), -c(0, 2, 1, 2), c(0, 1, 1, 2)],
        [-c(1, 2, 0, 2), c(0, 2, 0, 2), -c(0, 1, 0, 2)],
        [c(1, 2, 0, 1), -c(0, 2, 0, 1), c(0, 1, 0, 1)],
    ];
    Some(adj.map(|row| row.map(|v| v / det)))
}

/// Where `m` takes point `p` (None behind the camera).
pub fn apply(m: &M3, p: [f32; 2]) -> Option<[f32; 2]> {
    let z = m[2][0] * p[0] + m[2][1] * p[1] + m[2][2];
    if z <= 1e-6 {
        return None;
    }
    Some([(m[0][0] * p[0] + m[0][1] * p[1] + m[0][2]) / z, (m[1][0] * p[0] + m[1][1] * p[1] + m[1][2]) / z])
}

/// The correction: from a point of the picture to where it is shown (centred, in picture
/// heights). `focal` is the lens's focal length in picture heights.
pub fn forward(s: &Settings, focal: f32) -> M3 {
    let tilt = tilt(s[0], s[1], focal);
    // The middle of the photo stays in the middle.
    let mid = apply(&tilt, [0.0, 0.0]).unwrap_or([0.0, 0.0]);
    let centre: M3 = [[1.0, 0.0, -mid[0]], [0.0, 1.0, -mid[1]], [0.0, 0.0, 1.0]];
    let stretch = 2f32.powf(0.5 * s[2] / 100.0);
    let scale = s[3] / 100.0;
    let place: M3 = [[stretch * scale, 0.0, s[4] / 100.0 * 0.5], [0.0, scale / stretch, -s[5] / 100.0 * 0.5], [0.0, 0.0, 1.0]];
    mul(&place, &mul(&centre, &tilt))
}

/// The virtual camera's turn alone (Vertical and Horizontal).
fn tilt(vertical: f32, horizontal: f32, focal: f32) -> M3 {
    let (s0, s1) = (vertical, horizontal);
    let (tv, th) = ((-s0 / 100.0 * MAX_TILT).to_radians(), (s1 / 100.0 * MAX_TILT).to_radians());
    let rx: M3 = [[1.0, 0.0, 0.0], [0.0, tv.cos(), -tv.sin()], [0.0, tv.sin(), tv.cos()]];
    let ry: M3 = [[th.cos(), 0.0, th.sin()], [0.0, 1.0, 0.0], [-th.sin(), 0.0, th.cos()]];
    let k: M3 = [[focal, 0.0, 0.0], [0.0, focal, 0.0], [0.0, 0.0, 1.0]];
    let k_inv: M3 = [[1.0 / focal, 0.0, 0.0], [0.0, 1.0 / focal, 0.0], [0.0, 0.0, 1.0]];
    mul(&k, &mul(&mul(&rx, &ry), &k_inv))
}

/// The other way: from a point shown to the point of the picture it comes from.
#[cfg(test)]
pub fn backward(s: &Settings, focal: f32) -> M3 {
    invert(&forward(s, focal)).unwrap_or(IDENTITY)
}

/// The lens's focal length in heights of the displayed picture: from the EXIF focal length
/// and the sensor size (a full-frame sensor's 24 x 36 mm divided by the camera's crop factor);
/// a normal lens when the photo does not say.
pub fn focal_heights(focal_mm: f32, crop_factor: f32, portrait: bool) -> f32 {
    if focal_mm <= 0.0 {
        return 1.4;
    }
    let crop = if crop_factor > 0.2 { crop_factor } else { 1.0 };
    let height_mm = if portrait { 36.0 } else { 24.0 } / crop;
    (focal_mm / height_mm).clamp(0.3, 30.0)
}

/// A straight line in the photo that should stand vertical or lie level, on the displayed
/// picture (0..1 across and down, before the correction), with how much it counts.
#[derive(Clone, Copy, Debug)]
pub struct Line {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub weight: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Upright {
    /// Only the angle, from the photo's level and upright lines.
    Level,
    /// The angle and Vertical.
    Vertical,
    /// The angle, Vertical and Horizontal.
    Full,
    /// As Full, but holding back where the correction would be strong.
    Auto,
    /// Lines the user drew: as Full, every line counted fully.
    Guided,
}

/// Vertical, Horizontal and the angle (degrees) that make `lines` stand straight. `aspect` is
/// the displayed picture's width / height.
pub fn solve(lines: &[Line], aspect: f32, focal: f32, mode: Upright) -> (f32, f32, f32) {
    if lines.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let centred = |p: [f32; 2]| [(p[0] - 0.5) * aspect, p[1] - 0.5];
    // Each line, and whether it should stand (else lie), from how it runs now.
    let lines: Vec<([f32; 2], [f32; 2], f32, bool)> = lines
        .iter()
        .map(|l| {
            let (a, b) = (centred(l.a), centred(l.b));
            (a, b, l.weight, (b[1] - a[1]).abs() > (b[0] - a[0]).abs())
        })
        .collect();
    // Lines off by more than this after the correction are taken to be meant otherwise (a
    // receding wall, a slope) and count no more.
    let cap: f32 = if mode == Upright::Guided { 45.0 } else { 6.0 };
    let shrink = match mode {
        Upright::Auto => 0.004,
        Upright::Guided => 0.0002,
        _ => 0.0005,
    };
    // What may change: the angle always; Vertical and Horizontal by the mode, or for guides,
    // only as far as the guides pin them down (two upright guides set Vertical, two level ones
    // Horizontal), as in Lightroom.
    let upright_lines = lines.iter().filter(|l| l.3).count();
    let level_lines = lines.len() - upright_lines;
    let (use_v, use_h) = match mode {
        Upright::Level => (false, false),
        Upright::Vertical => (true, false),
        Upright::Guided => (upright_lines >= 2, level_lines >= 2),
        _ => (true, true),
    };
    let unpack = |p: &[f32]| -> (f32, f32, f32) {
        let mut k = 1;
        let mut next = |on: bool| {
            if on {
                k += 1;
                p[k - 1]
            } else {
                0.0
            }
        };
        let v = next(use_v);
        let h = next(use_h);
        (v, h, p[0])
    };
    let cost = |p: &[f32]| -> f32 {
        let (v, h, angle) = unpack(p);
        let m = forward(&[v, h, 0.0, 100.0, 0.0, 0.0], focal);
        let (c, s) = ((-angle).to_radians().cos(), (-angle).to_radians().sin());
        let mut total = 0.0;
        for (a, b, w, stand) in &lines {
            let (Some(a), Some(b)) = (apply(&m, *a), apply(&m, *b)) else {
                total += w * cap * cap;
                continue;
            };
            let d = [b[0] - a[0], b[1] - a[1]];
            let d = [c * d[0] - s * d[1], s * d[0] + c * d[1]];
            let off = if *stand { d[0].atan2(d[1]) } else { d[1].atan2(d[0]) }.to_degrees();
            // Folded to -90..90 (a line has no direction).
            let off = (off + 90.0).rem_euclid(180.0) - 90.0;
            total += w * off.powi(2).min(cap * cap);
        }
        let weights: f32 = lines.iter().map(|l| l.2).sum::<f32>().max(1e-6);
        total / weights + shrink * (v * v + h * h)
    };
    let dims = 1 + use_v as usize + use_h as usize;
    // Several starts, as a strong tilt can hide behind a local best.
    let mut best = (f32::MAX, vec![0.0f32; dims]);
    for start in [0.0f32, -40.0, 40.0] {
        let mut x0 = vec![0.0f32; dims];
        if dims > 1 {
            x0[1] = start;
        }
        let x = nelder_mead(&cost, x0, 8.0, 300);
        let c = cost(&x);
        if c < best.0 {
            best = (c, x);
        }
    }
    let (v, h, angle) = unpack(&best.1);
    (v.clamp(-100.0, 100.0), h.clamp(-100.0, 100.0), angle.clamp(-45.0, 45.0))
}

/// Downhill simplex search for the lowest `f`.
fn nelder_mead(f: &dyn Fn(&[f32]) -> f32, start: Vec<f32>, step: f32, rounds: usize) -> Vec<f32> {
    let n = start.len();
    let mut points: Vec<Vec<f32>> = vec![start.clone()];
    for i in 0..n {
        let mut p = start.clone();
        p[i] += step;
        points.push(p);
    }
    let mut values: Vec<f32> = points.iter().map(|p| f(p)).collect();
    for _ in 0..rounds {
        let mut order: Vec<usize> = (0..=n).collect();
        order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
        points = order.iter().map(|&i| points[i].clone()).collect();
        values = order.iter().map(|&i| values[i]).collect();
        if (values[n] - values[0]).abs() < 1e-7 {
            break;
        }
        let centroid: Vec<f32> = (0..n).map(|d| points[..n].iter().map(|p| p[d]).sum::<f32>() / n as f32).collect();
        let along = |t: f32| -> Vec<f32> { (0..n).map(|d| centroid[d] + t * (points[n][d] - centroid[d])).collect() };
        let reflected = along(-1.0);
        let fr = f(&reflected);
        if fr < values[0] {
            let expanded = along(-2.0);
            let fe = f(&expanded);
            if fe < fr {
                points[n] = expanded;
                values[n] = fe;
            } else {
                points[n] = reflected;
                values[n] = fr;
            }
        } else if fr < values[n - 1] {
            points[n] = reflected;
            values[n] = fr;
        } else {
            let contracted = along(0.5);
            let fc = f(&contracted);
            if fc < values[n] {
                points[n] = contracted;
                values[n] = fc;
            } else {
                for i in 1..=n {
                    points[i] = (0..n).map(|d| points[0][d] + 0.5 * (points[i][d] - points[0][d])).collect();
                    values[i] = f(&points[i]);
                }
            }
        }
    }
    let i = (0..=n).min_by(|&a, &b| values[a].total_cmp(&values[b])).unwrap_or(0);
    points[i].clone()
}

/// The long straight edges in a small copy of the photo (`rgb`, `width` x `height`, row-major):
/// a Hough transform over strong edges, each edge point voting only for lines along its own
/// edge direction. Lines are returned as end points in pixels with their length as weight,
/// nearly upright and nearly level ones only.
pub fn find_lines(rgb: &[f32], width: usize, height: usize) -> Vec<([f32; 2], [f32; 2], f32)> {
    if width < 16 || height < 16 || rgb.len() < width * height * 3 {
        return Vec::new();
    }
    // Brightness on a perceptual scale.
    let raw: Vec<f32> = rgb.chunks_exact(3).map(|p| (0.3 * p[0] + 0.55 * p[1] + 0.15 * p[2]).max(0.0).sqrt()).collect();
    // Smoothed a little, so noise does not scatter the edge directions.
    let mut lum = raw.clone();
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let mut s = 0.0;
            for (dy, dx, w) in [(0, 0, 4.0), (0, 1, 2.0), (0, -1, 2.0), (1, 0, 2.0), (-1, 0, 2.0), (1, 1, 1.0), (1, -1, 1.0), (-1, 1, 1.0), (-1, -1, 1.0)] {
                s += w * raw[(y as isize + dy) as usize * width + (x as isize + dx) as usize];
            }
            lum[y * width + x] = s / 16.0;
        }
    }
    let at = |x: usize, y: usize| lum[y * width + x];
    let mut edges: Vec<(f32, f32, f32, f32)> = Vec::new(); // x, y, normal angle, strength
    let mut strengths = Vec::new();
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let gx = at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x - 1, y) - at(x - 1, y + 1);
            let gy = at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x, y - 1) - at(x + 1, y - 1);
            let g = (gx * gx + gy * gy).sqrt();
            strengths.push(g);
            edges.push((x as f32, y as f32, gy.atan2(gx), g));
        }
    }
    let mut sorted = strengths.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let threshold = sorted[sorted.len() * 90 / 100].max(1e-4);
    edges.retain(|e| e.3 >= threshold);
    // Accumulator: normal angle in quarter degrees over 0..180, distance in pixels.
    const STEPS: usize = 720;
    let diag = ((width * width + height * height) as f32).sqrt();
    let rhos = (2.0 * diag) as usize + 2;
    let mut votes = vec![0u32; STEPS * rhos];
    let tables: Vec<(f32, f32)> = (0..STEPS).map(|i| ((i as f32 * 0.25).to_radians().cos(), (i as f32 * 0.25).to_radians().sin())).collect();
    for &(x, y, normal, _) in &edges {
        let centre = (normal.to_degrees().rem_euclid(180.0) * 4.0) as i32;
        for k in -12..=12 {
            let i = (centre + k).rem_euclid(STEPS as i32) as usize;
            // Only near-upright or near-level lines: normals near 0 / 180 or near 90 degrees.
            let deg = i as f32 * 0.25;
            if !(deg < 35.0 || deg > 145.0 || (deg - 90.0).abs() < 35.0) {
                continue;
            }
            let (c, s) = tables[i];
            let rho = (x * c + y * s + diag) as usize;
            votes[i * rhos + rho] += 1;
        }
    }
    // Peaks, strongest first, keeping apart from those taken.
    let min_votes = (width.min(height) as f32 * 0.07) as u32;
    let mut cells: Vec<(u32, usize)> = votes.iter().enumerate().filter(|(_, &v)| v >= min_votes).map(|(i, &v)| (v, i)).collect();
    cells.sort_by(|a, b| b.0.cmp(&a.0));
    let mut taken: Vec<(usize, usize)> = Vec::new();
    let mut out = Vec::new();
    for (_, cell) in cells {
        let (i, r) = (cell / rhos, cell % rhos);
        if taken.iter().any(|&(ti, tr)| {
            let di = (ti as i32 - i as i32).rem_euclid(STEPS as i32).min((i as i32 - ti as i32).rem_euclid(STEPS as i32));
            di <= 8 && (tr as i32 - r as i32).abs() <= 8
        }) {
            continue;
        }
        taken.push((i, r));
        // The line's extent: its supporting edge points, from the 5th to the 95th percentile.
        let (c, s) = tables[i];
        let rho = r as f32 - diag;
        let normal_deg = i as f32 * 0.25;
        let mut along: Vec<f32> = edges
            .iter()
            .filter(|e| (e.0 * c + e.1 * s - rho).abs() < 2.0)
            .filter(|e| {
                let d = (e.2.to_degrees().rem_euclid(180.0) - normal_deg).abs();
                d.min(180.0 - d) < 6.0
            })
            .map(|e| -e.0 * s + e.1 * c)
            .collect();
        if along.len() < min_votes as usize {
            continue;
        }
        along.sort_by(|a, b| a.total_cmp(b));
        // The longest unbroken run along the line (gaps under 6 pixels), not points scattered
        // far along it.
        let (mut best, mut run_start, mut count, mut best_count) = ((0usize, 0usize), 0usize, 1usize, 0usize);
        for k in 1..=along.len() {
            if k == along.len() || along[k] - along[k - 1] > 6.0 {
                if count > best_count {
                    best = (run_start, k - 1);
                    best_count = count;
                }
                run_start = k;
                count = 1;
            } else {
                count += 1;
            }
        }
        if best_count < min_votes as usize {
            continue;
        }
        let (t0, t1) = (along[best.0], along[best.1]);
        let point = |t: f32| [rho * c - t * s, rho * s + t * c];
        out.push((point(t0), point(t1), best_count as f32));
        if out.len() >= 40 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_vertical_widens_the_top() {
        let m = forward(&[-50.0, 0.0, 0.0, 100.0, 0.0, 0.0], 1.4);
        let w = |y: f32| apply(&m, [0.75, y]).unwrap()[0] - apply(&m, [-0.75, y]).unwrap()[0];
        assert!(w(-0.5) > w(0.5), "top {} bottom {}", w(-0.5), w(0.5));
        // And back again.
        let back = backward(&[-50.0, 0.0, 0.0, 100.0, 0.0, 0.0], 1.4);
        let p = apply(&back, apply(&m, [0.3, -0.2]).unwrap()).unwrap();
        assert!((p[0] - 0.3).abs() < 1e-4 && (p[1] + 0.2).abs() < 1e-4);
    }

    #[test]
    fn upright_undoes_a_tilt() {
        // Upright lines photographed with the camera tilted: Vertical +30 makes them converge;
        // solving must bring Vertical back to about -30 from the converged lines.
        // What a camera tilted that way records: the turn alone, no re-centring.
        let tilt = tilt(30.0, 0.0, 1.4);
        let aspect = 1.5;
        let lines: Vec<Line> = [-0.6f32, -0.3, 0.3, 0.6]
            .iter()
            .map(|&x| {
                let a = apply(&tilt, [x, -0.4]).unwrap();
                let b = apply(&tilt, [x, 0.4]).unwrap();
                let un = |p: [f32; 2]| [p[0] / aspect + 0.5, p[1] + 0.5];
                Line { a: un(a), b: un(b), weight: 1.0 }
            })
            .collect();
        let (v, h, angle) = solve(&lines, aspect, 1.4, Upright::Vertical);
        assert!((v + 30.0).abs() < 3.0, "vertical {v}");
        assert!(h.abs() < 1e-3 && angle.abs() < 1.0, "h {h} angle {angle}");
        // Two of them as guides give the same, and leave Horizontal alone.
        let (v, h, angle) = solve(&lines[1..3], aspect, 1.4, Upright::Guided);
        assert!((v + 30.0).abs() < 3.0 && h == 0.0 && angle.abs() < 1.0, "guided: {v} {h} {angle}");
    }

    #[test]
    fn finds_a_drawn_line() {
        let (w, h) = (200, 120);
        let mut rgb = vec![0.05f32; w * h * 3];
        for y in 10..110 {
            for x in 100..w {
                for c in 0..3 {
                    rgb[(y * w + x) * 3 + c] = 0.8;
                }
            }
        }
        let lines = find_lines(&rgb, w, h);
        assert!(lines.iter().any(|(a, b, _)| (a[0] - 99.5).abs() < 2.0 && (b[0] - 99.5).abs() < 2.0 && (a[1] - b[1]).abs() > 60.0), "{lines:?}");
    }
}
