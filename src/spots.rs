//! Spot removal: Heal and Clone, as circles or painted strokes.
//!
//! A spot is stored as settings, never as pixels: the target (a circle, or a stroke painted
//! with a round brush), where it copies from, its brush size, feather and opacity, and whether
//! it heals (blends the copied texture into the surroundings) or clones (copies it as it is).
//! Positions are on the stored picture in texture coordinates (0..1 across and down, before
//! crop, rotation and lens correction); the radius is a fraction of the picture's height. The
//! source is the whole target shifted by one offset. The shader applies the spots wherever it
//! reads the picture, so every later adjustment, every zoom and every export sees the healed
//! photo.
//!
//! Healing needs to know how the surroundings of the target differ from those of the source.
//! The shader measures that itself (`heal_gain` in shader.wgsl), at full resolution. Here, a
//! small copy of the photo is used to choose a source automatically when a spot is placed.

/// The most spots a photo can have, and the most stroke points all of them together can have
/// (the shader's uniform arrays hold this many).
pub const MAX_SPOTS: usize = 64;
pub const MAX_POINTS: usize = 1024;
/// The most points one stroke keeps (longer strokes are thinned out).
const STROKE_POINTS: usize = 192;

/// Directions around a spot in which surroundings are compared when choosing a source.
pub const RING: usize = 8;

#[derive(Clone, PartialEq, Debug)]
pub struct Spot {
    /// The target's first point and the matching point of the source, in texture coordinates.
    pub x: f32,
    pub y: f32,
    pub sx: f32,
    pub sy: f32,
    /// The rest of a painted stroke (empty for a circle).
    pub path: Vec<[f32; 2]>,
    /// Brush radius as a fraction of the picture's height.
    pub radius: f32,
    /// Soft edge, as a fraction of the radius (0 hard, 1 soft all the way in).
    pub feather: f32,
    /// 0..1.
    pub opacity: f32,
    /// Copy the source as it is instead of healing it into the surroundings.
    pub clone: bool,
}

impl Spot {
    /// The target's points: the first, then the rest of the stroke.
    pub fn points(&self) -> impl Iterator<Item = [f32; 2]> + '_ {
        std::iter::once([self.x, self.y]).chain(self.path.iter().copied())
    }

    /// Where the source is relative to the target.
    pub fn offset(&self) -> [f32; 2] {
        [self.sx - self.x, self.sy - self.y]
    }

    /// Move the target (the source stays where it is).
    pub fn move_target(&mut self, x: f32, y: f32) {
        let (dx, dy) = (x - self.x, y - self.y);
        self.x = x;
        self.y = y;
        for p in &mut self.path {
            p[0] += dx;
            p[1] += dy;
        }
    }

    /// Distance from `t` to the target's centre line, in picture heights (`aspect` is the
    /// picture's width / height).
    pub fn distance(&self, t: [f32; 2], aspect: f32) -> f32 {
        path_distance(&self.points().collect::<Vec<_>>(), t, aspect)
    }

    /// The same for the source.
    pub fn source_distance(&self, t: [f32; 2], aspect: f32) -> f32 {
        let [dx, dy] = self.offset();
        self.distance([t[0] - dx, t[1] - dy], aspect)
    }
}

/// Distance from `t` to the polyline through `points`, in picture heights.
pub fn path_distance(points: &[[f32; 2]], t: [f32; 2], aspect: f32) -> f32 {
    let scaled = |p: [f32; 2]| [(p[0] - t[0]) * aspect, p[1] - t[1]];
    let Some(&first) = points.first() else { return f32::MAX };
    let mut a = scaled(first);
    let mut best = a[0].hypot(a[1]);
    for &p in &points[1..] {
        let b = scaled(p);
        let ba = [b[0] - a[0], b[1] - a[1]];
        let h = (-(a[0] * ba[0] + a[1] * ba[1]) / (ba[0] * ba[0] + ba[1] * ba[1]).max(1e-12)).clamp(0.0, 1.0);
        best = best.min((a[0] + ba[0] * h).hypot(a[1] + ba[1] * h));
        a = b;
    }
    best
}

/// The edge of everything within `radius` of the polyline through `points`, as line pieces
/// in texture coordinates: the outline of a painted stroke as one shape, however often it
/// crosses itself. Traced with marching squares on a grid a sixth of the radius apart.
pub fn outline_segments(points: &[[f32; 2]], radius: f32, aspect: f32) -> Vec<[[f32; 2]; 2]> {
    if points.is_empty() || radius <= 0.0 {
        return Vec::new();
    }
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in points {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    let margin = [1.3 * radius / aspect, 1.3 * radius];
    let (x0, y0) = (lo[0] - margin[0], lo[1] - margin[1]);
    let (w, h) = (hi[0] - lo[0] + 2.0 * margin[0], hi[1] - lo[1] + 2.0 * margin[1]);
    // Grid step in picture heights, coarser for very long strokes.
    let step = (radius / 6.0).max((w * aspect).max(h) / 320.0);
    let (sx, sy) = (step / aspect, step);
    let (nx, ny) = ((w / sx).ceil() as usize + 1, (h / sy).ceil() as usize + 1);
    let at = |i: usize, j: usize| [x0 + i as f32 * sx, y0 + j as f32 * sy];
    let field: Vec<f32> = (0..ny).flat_map(|j| (0..nx).map(move |i| (i, j))).map(|(i, j)| path_distance(points, at(i, j), aspect) - radius).collect();
    let value = |i: usize, j: usize| field[j * nx + i];
    let mut out = Vec::new();
    for j in 0..ny - 1 {
        for i in 0..nx - 1 {
            // Corners in order around the cell, and where the edge crosses each side.
            let corners = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
            let v = corners.map(|(a, b)| value(a, b));
            let mut cross = Vec::with_capacity(4);
            for k in 0..4 {
                let (a, b) = (v[k], v[(k + 1) % 4]);
                if (a < 0.0) != (b < 0.0) {
                    let t = a / (a - b);
                    let (pa, pb) = (at(corners[k].0, corners[k].1), at(corners[(k + 1) % 4].0, corners[(k + 1) % 4].1));
                    cross.push([pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t]);
                }
            }
            match cross.len() {
                2 => out.push([cross[0], cross[1]]),
                4 => {
                    // A saddle: pair the crossings by whether the centre is inside.
                    let centre_inside = v.iter().sum::<f32>() < 0.0;
                    if centre_inside == (v[0] < 0.0) {
                        out.push([cross[0], cross[3]]);
                        out.push([cross[1], cross[2]]);
                    } else {
                        out.push([cross[0], cross[1]]);
                        out.push([cross[2], cross[3]]);
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Thin a painted stroke: points closer than `step` (picture heights) to the last one kept
/// are dropped, then every other point until it fits `STROKE_POINTS`. The last point stays.
pub fn simplify(points: &[[f32; 2]], step: f32, aspect: f32) -> Vec<[f32; 2]> {
    let mut out: Vec<[f32; 2]> = Vec::with_capacity(points.len());
    for (i, &p) in points.iter().enumerate() {
        let far = out.last().map_or(true, |q| ((p[0] - q[0]) * aspect).hypot(p[1] - q[1]) >= step);
        if far || i + 1 == points.len() {
            out.push(p);
        }
    }
    while out.len() > STROKE_POINTS {
        let last = *out.last().unwrap_or(&[0.0, 0.0]);
        out = out.iter().step_by(2).copied().collect();
        if out.last() != Some(&last) {
            out.push(last);
        }
    }
    out
}

/// The spots as one sidecar field: `x,y,sx,sy,radius,feather,opacity,mode[,px,py...];...`,
/// where a stroke's other points follow as pairs (earlier versions read the first eight
/// numbers, so they see a stroke as a circle at its start).
pub fn to_field(spots: &[Spot]) -> String {
    spots
        .iter()
        .map(|s| {
            let mut item = format!("{:.6},{:.6},{:.6},{:.6},{:.6},{:.3},{:.3},{}", s.x, s.y, s.sx, s.sy, s.radius, s.feather, s.opacity, s.clone as u8);
            for p in &s.path {
                item.push_str(&format!(",{:.6},{:.6}", p[0], p[1]));
            }
            item
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Read what `to_field` wrote, skipping anything malformed.
pub fn from_field(text: &str) -> Vec<Spot> {
    text.split(';')
        .filter_map(|item| {
            let v: Vec<f32> = item.split(',').map(|n| n.trim().parse::<f32>().ok().filter(|v| v.is_finite())).collect::<Option<Vec<_>>>()?;
            if v.len() < 8 {
                return None;
            }
            let at = |v: f32| v.clamp(-0.5, 1.5);
            Some(Spot {
                x: at(v[0]),
                y: at(v[1]),
                sx: at(v[2]),
                sy: at(v[3]),
                path: v[8..].chunks_exact(2).map(|p| [at(p[0]), at(p[1])]).take(STROKE_POINTS).collect(),
                radius: v[4].clamp(0.0005, 0.5),
                feather: v[5].clamp(0.0, 1.0),
                opacity: v[6].clamp(0.0, 1.0),
                clone: v[7] > 0.5,
            })
        })
        .take(MAX_SPOTS)
        .collect()
}

/// A small linear copy of the photo (camera colour, as in the GPU texture), for measuring
/// surroundings and choosing sources.
#[derive(Clone, Default)]
pub struct Preview {
    pub width: usize,
    pub height: usize,
    /// Interleaved RGB.
    pub rgb: Vec<f32>,
}

impl Preview {
    /// Bilinear sample at texture coordinates (clamped to the picture).
    pub fn sample(&self, u: f32, v: f32) -> [f32; 3] {
        if self.width == 0 || self.height == 0 {
            return [0.0; 3];
        }
        let x = (u * self.width as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let y = (v * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |xx: usize, yy: usize| {
            let i = (yy * self.width + xx) * 3;
            [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
        };
        let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
        let mut out = [0.0; 3];
        for ch in 0..3 {
            out[ch] = (a[ch] * (1.0 - fx) + b[ch] * fx) * (1.0 - fy) + (c[ch] * (1.0 - fx) + d[ch] * fx) * fy;
        }
        out
    }

    fn aspect(&self) -> f32 {
        self.width as f32 / self.height.max(1) as f32
    }

    /// The average over a small square (`half` pixels each way) around texture point
    /// (`u`, `v`): what the white balance eyedropper reads.
    pub fn average(&self, u: f32, v: f32, half: isize) -> [f32; 3] {
        if self.width == 0 || self.height == 0 {
            return [0.0; 3];
        }
        let (cx, cy) = ((u * self.width as f32) as isize, (v * self.height as f32) as isize);
        let mut sum = [0.0f32; 3];
        let mut n: f32 = 0.0;
        for y in cy - half..=cy + half {
            for x in cx - half..=cx + half {
                if x < 0 || y < 0 || x >= self.width as isize || y >= self.height as isize {
                    continue;
                }
                let i = (y as usize * self.width + x as usize) * 3;
                for c in 0..3 {
                    sum[c] += self.rgb[i + c];
                }
                n += 1.0;
            }
        }
        sum.map(|s| s / n.max(1.0))
    }

    /// Average colour of the band just outside a circle, in each of the `RING` directions.
    fn ring(&self, cx: f32, cy: f32, radius: f32) -> [[f32; 3]; RING] {
        let aspect = self.aspect();
        let mut out = [[0.0f32; 3]; RING];
        for (k, slot) in out.iter_mut().enumerate() {
            let mut sum = [0.0f32; 3];
            let mut n = 0.0;
            // A small patch of the band: three distances out, three angles across.
            for dr in [1.1f32, 1.25, 1.4] {
                for da in [-0.2f32, 0.0, 0.2] {
                    let a = (k as f32 + da) * std::f32::consts::TAU / RING as f32;
                    let p = self.sample(cx + a.cos() * radius * dr / aspect, cy + a.sin() * radius * dr);
                    for ch in 0..3 {
                        sum[ch] += p[ch];
                    }
                    n += 1.0;
                }
            }
            *slot = sum.map(|v| v / n);
        }
        out
    }

    /// A good place to copy from for a spot whose target runs through `points` (one point for
    /// a circle), returned as the offset from target to source: nearby, inside the picture,
    /// clear of the target itself, with nothing inside it that its own surroundings don't
    /// explain (healing carries the surroundings' brightness across, but whatever else is
    /// inside the source, such as a cloud, an edge or a speck, comes along), and with
    /// surroundings like the target's. A stroke is judged at up to eight points along it.
    pub fn find_offset(&self, points: &[[f32; 2]], radius: f32) -> [f32; 2] {
        let aspect = self.aspect();
        let log = |v: f32| (v + 0.002).ln();
        let luma = |ring: &[[f32; 3]; RING]| ring.map(|c| log(c[0] + c[1] + c[2]));
        let picks: Vec<[f32; 2]> = if points.len() <= 8 {
            points.to_vec()
        } else {
            (0..8).map(|i| points[i * (points.len() - 1) / 7]).collect()
        };
        let targets: Vec<_> = picks.iter().map(|p| self.ring(p[0], p[1], radius)).collect();
        // Points inside a circle: centre, then rings at 0.3, 0.55 and 0.8 of the radius.
        let inside: Vec<(f32, f32)> = std::iter::once((0.0, 0.0))
            .chain([0.3f32, 0.55, 0.8].into_iter().flat_map(|rho| (0..12).map(move |k| (rho, k as f32 * std::f32::consts::TAU / 12.0))))
            .collect();
        // How far the inside of a circle strays from what its surroundings predict there
        // (blended as the shader blends healing gains), on a log scale.
        let stray = |cx: f32, cy: f32, ring: &[[f32; 3]; RING]| {
            let around = luma(ring);
            let mut sum = 0.0;
            for &(rho, theta) in &inside {
                let (mut predicted, mut weights) = (0.0, 0.0);
                for (k, l) in around.iter().enumerate() {
                    let w = (4.0 * rho * (theta - k as f32 * std::f32::consts::TAU / RING as f32).cos()).exp();
                    predicted += w * l;
                    weights += w;
                }
                let p = self.sample(cx + theta.cos() * rho * radius / aspect, cy + theta.sin() * rho * radius);
                let d = log(p[0] + p[1] + p[2]) - predicted / weights;
                sum += d * d;
            }
            sum / inside.len() as f32
        };
        let margin = |p: [f32; 2]| {
            let mx = 1.5 * radius / aspect;
            p[0] >= mx && p[1] >= 1.5 * radius && p[0] <= 1.0 - mx && p[1] <= 1.0 - 1.5 * radius
        };
        // Distances in brush radii: close by for circles and thin strokes, and beyond the
        // stroke's own size for wide ones (a scribbled patch).
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in points {
            for i in 0..2 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        let extent = ((hi[0] - lo[0]) * aspect).max(hi[1] - lo[1]) / radius;
        let mut distances = vec![2.1f32, 2.6, 3.2, 4.0, 5.0, 6.5];
        if extent > 1.0 {
            distances.extend([2.2f32, 3.0, 4.5].map(|d| extent + d));
        }
        // If nothing nearby is clear of the target, the farthest candidate tried.
        let far = distances.iter().copied().fold(0.0f32, f32::max);
        let mut best = (f32::MAX, [far * radius / aspect, 0.0]);
        for distance in distances {
            for step in 0..16 {
                let a = (step as f32 + 0.5 * (distance * 2.0).fract()) * std::f32::consts::TAU / 16.0;
                let offset = [a.cos() * radius * distance / aspect, a.sin() * radius * distance];
                let shifted = |p: [f32; 2]| [p[0] + offset[0], p[1] + offset[1]];
                // The whole source inside the picture and clear of the target.
                if !points.iter().all(|&p| margin(shifted(p))) || points.iter().any(|&p| path_distance(points, shifted(p), aspect) < 2.05 * radius) {
                    continue;
                }
                let mut score = 0.0005 * distance;
                for (p, target) in picks.iter().zip(&targets) {
                    let [cx, cy] = shifted(*p);
                    let ring = self.ring(cx, cy, radius);
                    let mut alike = 0.0;
                    for k in 0..RING {
                        for ch in 0..3 {
                            let d = log(ring[k][ch]) - log(target[k][ch]);
                            alike += d * d;
                        }
                    }
                    score += (8.0 * stray(cx, cy, &ring) + 0.4 * alike / (3 * RING) as f32) / picks.len() as f32;
                }
                if score < best.0 {
                    best = (score, offset);
                }
            }
        }
        best.1
    }
}

impl Preview {
    /// The first level of the picture's mip chain no larger than `limit` pixels on its long
    /// side (or the smallest there is), as linear RGB floats.
    pub fn from_mips(mips: &[crate::raw::MipLevel], limit: u32) -> Self {
        let Some(level) = mips.iter().find(|m| m.width.max(m.height) <= limit).or(mips.last()) else { return Self::default() };
        let rgb = level.rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).map(|bits| half::f16::from_bits(bits).to_f32().max(0.0)).collect();
        Self { width: level.width as usize, height: level.height as usize, rgb }
    }
}

/// The atlas the healing gains are uploaded to: square slots of `HEAL_CELL` texels, one per
/// spot, `HEAL_SLOTS_X` across.
pub const HEAL_CELL: usize = 128;
pub const HEAL_SLOTS_X: usize = 8;
pub const HEAL_ATLAS: usize = HEAL_CELL * HEAL_SLOTS_X;

/// A Heal spot's brightness correction, worked out across the whole healed shape: on a grid
/// over the spot, the log of (target / source) per colour, measured just outside the shape
/// and filled in smoothly inside it (a membrane: each point the average of its neighbours,
/// as in seamless cloning). The shader multiplies the copied source by exp() of it, so
/// the patch meets its surroundings at every point of its edge, however wide it is.
#[derive(Clone)]
pub struct HealGrid {
    /// What it was worked out for (the spot and every Clone spot before it).
    pub key: u64,
    pub nx: usize,
    pub ny: usize,
    /// Texture coordinates of the first node, and the node spacing.
    pub origin: [f32; 2],
    pub step: [f32; 2],
    /// Log gain per node, row by row.
    pub log: Vec<[f32; 3]>,
}

/// What a spot's healing depends on: its own shape, source and size, and the shape and source
/// of every Clone spot before it (which a later Heal sees).
pub fn heal_key(spots: &[Spot], i: usize, preview: &Preview) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (preview.width, preview.height, preview.rgb.len()).hash(&mut h);
    let mut add = |s: &Spot| {
        for p in s.points() {
            (p[0].to_bits(), p[1].to_bits()).hash(&mut h);
        }
        (s.sx.to_bits(), s.sy.to_bits(), s.radius.to_bits(), s.feather.to_bits(), s.opacity.to_bits(), s.clone).hash(&mut h);
    };
    for s in spots[..i].iter().filter(|s| s.clone) {
        add(s);
    }
    add(&spots[i]);
    h.finish()
}

impl Preview {
    /// The photo at `t` with the given Clone spots applied, as the shader's `cloned_at` does.
    fn layered(&self, t: [f32; 2], clones: &[(&Spot, Vec<[f32; 2]>, [f32; 4])]) -> [f32; 3] {
        let aspect = self.aspect();
        let mut c = self.sample(t[0], t[1]);
        for (s, points, b) in clones {
            if t[0] < b[0] || t[1] < b[1] || t[0] > b[2] || t[1] > b[3] {
                continue;
            }
            let dist = path_distance(points, t, aspect) / s.radius;
            if dist >= 1.0 {
                continue;
            }
            let soft = s.feather.max(0.02);
            let x = ((dist - (1.0 - soft)) / soft).clamp(0.0, 1.0);
            let mask = (1.0 - x * x * (3.0 - 2.0 * x)) * s.opacity;
            let [dx, dy] = s.offset();
            let src = self.sample(t[0] + dx, t[1] + dy);
            for ch in 0..3 {
                c[ch] += (src[ch] - c[ch]) * mask;
            }
        }
        c
    }

    /// Work out spot `i`'s healing gains (see `HealGrid`).
    pub fn heal_grid(&self, spots: &[Spot], i: usize) -> HealGrid {
        let aspect = self.aspect();
        let spot = &spots[i];
        let r = spot.radius;
        let points: Vec<[f32; 2]> = spot.points().collect();
        let clones: Vec<(&Spot, Vec<[f32; 2]>, [f32; 4])> = spots[..i]
            .iter()
            .filter(|s| s.clone)
            .map(|s| {
                let pts: Vec<[f32; 2]> = s.points().collect();
                let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
                for p in &pts {
                    for k in 0..2 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                    }
                }
                let reach = [s.radius / aspect, s.radius];
                (s, pts, [lo[0] - reach[0], lo[1] - reach[1], hi[0] + reach[0], hi[1] + reach[1]])
            })
            .collect();
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in &points {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        // Nodes a quarter of the radius apart (in picture heights), coarser if the shape is
        // too big for one atlas slot; two nodes of margin outside the shape.
        let extent = ((hi[0] - lo[0]) * aspect).max(hi[1] - lo[1]) + 2.0 * r;
        let step = (r / 4.0).max(extent / (HEAL_CELL - 5) as f32);
        let margin = r + 2.0 * step;
        let origin = [lo[0] - margin / aspect, lo[1] - margin];
        let nx = ((((hi[0] - lo[0]) * aspect + 2.0 * margin) / step).ceil() as usize + 1).min(HEAL_CELL);
        let ny = ((((hi[1] - lo[1]) + 2.0 * margin) / step).ceil() as usize + 1).min(HEAL_CELL);
        let at = |x: usize, y: usize| [origin[0] + x as f32 * step / aspect, origin[1] + y as f32 * step];
        let [dx, dy] = spot.offset();
        let log = |v: f32| (v.max(0.0) + 0.002).ln();
        // A small average around a point, so single pixels of noise don't set the edge.
        let near = |t: [f32; 2]| {
            let mut sum = [0.0f32; 3];
            let d = 0.5 * step;
            for (ox, oy) in [(0.0, 0.0), (d, 0.0), (-d, 0.0), (0.0, d), (0.0, -d)] {
                let p = self.layered([t[0] + ox / aspect, t[1] + oy], &clones);
                for ch in 0..3 {
                    sum[ch] += p[ch] / 5.0;
                }
            }
            sum
        };
        let n = nx * ny;
        let mut field = vec![[0.0f32; 3]; n];
        let mut inside = vec![false; n];
        let (mut edge_sum, mut edge_count) = ([0.0f32; 3], 0usize);
        for y in 0..ny {
            for x in 0..nx {
                let t = at(x, y);
                let k = y * nx + x;
                if path_distance(&points, t, aspect) < r {
                    inside[k] = true;
                    continue;
                }
                // Outside: the measured ratio, which fixes the solution at the edge.
                let (target, source) = (near(t), near([t[0] + dx, t[1] + dy]));
                for ch in 0..3 {
                    field[k][ch] = (log(target[ch]) - log(source[ch])).clamp(-1.4, 1.4);
                    edge_sum[ch] += field[k][ch];
                }
                edge_count += 1;
            }
        }
        // Start inside from the edge's average, then relax: each node becomes the average of
        // its four neighbours (over-relaxed to converge quickly).
        let start = edge_sum.map(|v| v / edge_count.max(1) as f32);
        for k in 0..n {
            if inside[k] {
                field[k] = start;
            }
        }
        let size = nx.max(ny) as f32;
        let omega = 2.0 / (1.0 + (std::f32::consts::PI / size).sin());
        let rounds = (3.0 * size) as usize + 20;
        for _ in 0..rounds {
            for y in 1..ny - 1 {
                for x in 1..nx - 1 {
                    let k = y * nx + x;
                    if !inside[k] {
                        continue;
                    }
                    for ch in 0..3 {
                        let mean = 0.25 * (field[k - 1][ch] + field[k + 1][ch] + field[k - nx][ch] + field[k + nx][ch]);
                        field[k][ch] += omega * (mean - field[k][ch]);
                    }
                }
            }
        }
        HealGrid { key: heal_key(spots, i, self), nx, ny, origin, step: [step / aspect, step], log: field }
    }
}

/// Put `spots` into the shader's settings (as many as fit, oldest first). `aspect` is the
/// picture's width / height; `grids` are the Heal spots' gains, in atlas slot `i` for spot
/// `i` (a Heal spot without one falls back to the shader's own estimate).
pub fn fill_uniforms(spots: &[Spot], grids: &[Option<&HealGrid>], aspect: f32, u: &mut crate::pipeline::Uniforms) {
    let mut n = 0;
    let mut used = 0;
    for s in spots.iter().take(MAX_SPOTS) {
        let count = 1 + s.path.len();
        if used + count > MAX_POINTS {
            break;
        }
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for (k, p) in s.points().enumerate() {
            let slot = &mut u.spot_pts[(used + k) / 2];
            let half = if (used + k) % 2 == 0 { 0 } else { 2 };
            slot[half] = p[0];
            slot[half + 1] = p[1];
            for i in 0..2 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        let reach = [s.radius / aspect, s.radius];
        u.spot_a[n] = [s.x, s.y, s.sx, s.sy];
        u.spot_b[n] = [s.radius, s.feather, s.opacity, s.clone as u8 as f32];
        let grid = grids.get(n).copied().flatten().filter(|_| !s.clone);
        u.spot_c[n] = [used as f32, count as f32, grid.is_some() as u8 as f32, 0.0];
        if let Some(g) = grid {
            let cell = [(n % HEAL_SLOTS_X) * HEAL_CELL, (n / HEAL_SLOTS_X) * HEAL_CELL];
            u.spot_g[n] = [g.origin[0], g.origin[1], 1.0 / g.step[0], 1.0 / g.step[1]];
            u.spot_t[n] = [(cell[0] as f32 + 0.5) / HEAL_ATLAS as f32, (cell[1] as f32 + 0.5) / HEAL_ATLAS as f32, (g.nx - 1) as f32, (g.ny - 1) as f32];
        }
        u.spot_box[n] = [lo[0] - reach[0], lo[1] - reach[1], hi[0] + reach[0], hi[1] + reach[1]];
        used += count;
        n += 1;
    }
    u.spot_info = [n as f32, 0.0, 0.0, 0.0];
}

/// Where a point on screen lies on the stored picture and back, for the picture as the shader
/// draws it into `rect` with `u`'s view, straighten, rotation and lens correction.
pub struct ScreenMap {
    rect: eframe::egui::Rect,
    view: [f32; 4],
    straighten: [f32; 4],
    /// Perspective correction: shown -> picture, and picture -> shown (None when off).
    persp: Option<(crate::transform::M3, crate::transform::M3)>,
    rot: [f32; 4],
    rot_off: [f32; 2],
    lens: crate::lens::Gpu,
    /// Stored picture width / height.
    aspect: f32,
}

impl ScreenMap {
    pub fn new(u: &crate::pipeline::Uniforms, rect: eframe::egui::Rect, aspect: f32) -> Self {
        let lens = crate::lens::Gpu {
            dist: u.lens_dist,
            geo: u.lens_geo,
            strength: u.lens_str,
            tca_r: u.tca_r,
            tca_b: u.tca_b,
            vig: u.vig,
            vig2: u.vig2,
            fringe: u.fringe,
            knots: u.knots,
        };
        let persp = (u.persp_info[0] > 0.5).then(|| {
            let back: crate::transform::M3 = std::array::from_fn(|i| [u.persp[i][0], u.persp[i][1], u.persp[i][2]]);
            (back, crate::transform::invert(&back).unwrap_or(back))
        });
        Self { rect, view: u.view, straighten: u.straighten, persp, rot: u.rot, rot_off: [u.rot_off[0], u.rot_off[1]], lens, aspect }
    }

    /// Where the lens put an output point (texture coordinates before lens correction).
    fn lens_source(&self, p: [f32; 2]) -> [f32; 2] {
        let g = self.lens.warp([p[0] - 0.5, p[1] - 0.5], self.aspect, self.lens.geo[3])[0];
        [g[0] + 0.5, g[1] + 0.5]
    }

    /// Screen point to stored-picture texture coordinates. Mirrors the start of `fs_main`.
    pub fn to_texture(&self, p: eframe::egui::Pos2) -> [f32; 2] {
        let d = self.to_picture(p);
        let r = self.rot;
        let out = [r[0] * d[0] + r[1] * d[1] + self.rot_off[0], r[2] * d[0] + r[3] * d[1] + self.rot_off[1]];
        self.lens_source(out)
    }

    /// Screen point to the displayed picture (0..1 across and down, before perspective
    /// correction, after lens correction).
    pub fn to_picture(&self, p: eframe::egui::Pos2) -> [f32; 2] {
        let v = [(p.x - self.rect.min.x) / self.rect.width(), (p.y - self.rect.min.y) / self.rect.height()];
        let frame = [(v[0] - 0.5) * self.view[0] + self.view[2], (v[1] - 0.5) * self.view[1] + self.view[3]];
        let q = [frame[0] - 0.5, frame[1] - 0.5];
        let [c, s, a, _] = self.straighten;
        let d = [c * q[0] - s * q[1] / a + 0.5, s * q[0] * a + c * q[1] + 0.5];
        match &self.persp {
            Some((back, _)) => match crate::transform::apply(back, [(d[0] - 0.5) * a, d[1] - 0.5]) {
                Some(p) => [p[0] / a + 0.5, p[1] + 0.5],
                None => [-9.0, -9.0],
            },
            None => d,
        }
    }

    /// Texture coordinates back to the screen (the lens part solved by iteration).
    pub fn to_screen(&self, t: [f32; 2]) -> eframe::egui::Pos2 {
        self.picture_to_screen(self.texture_to_picture(t))
    }

    /// Stored-picture texture coordinates to the displayed picture (as `to_picture`).
    pub fn texture_to_picture(&self, t: [f32; 2]) -> [f32; 2] {
        let mut out = t;
        for _ in 0..12 {
            let got = self.lens_source(out);
            out = [out[0] + (t[0] - got[0]), out[1] + (t[1] - got[1])];
        }
        let r = self.rot;
        let p = [out[0] - self.rot_off[0], out[1] - self.rot_off[1]];
        [r[0] * p[0] + r[2] * p[1], r[1] * p[0] + r[3] * p[1]]
    }

    /// A point of the displayed picture to the screen.
    pub fn picture_to_screen(&self, d: [f32; 2]) -> eframe::egui::Pos2 {
        let [c, s, a, _] = self.straighten;
        let d = match &self.persp {
            Some((_, fwd)) => match crate::transform::apply(fwd, [(d[0] - 0.5) * a, d[1] - 0.5]) {
                Some(p) => [p[0] / a + 0.5, p[1] + 0.5],
                None => [-9.0, -9.0],
            },
            None => d,
        };
        let d = [d[0] - 0.5, d[1] - 0.5];
        let q = [c * d[0] + s * d[1] / a, -s * a * d[0] + c * d[1]];
        let frame = [q[0] + 0.5, q[1] + 0.5];
        let v = [(frame[0] - self.view[2]) / self.view[0] + 0.5, (frame[1] - self.view[3]) / self.view[1] + 0.5];
        eframe::egui::pos2(self.rect.min.x + v[0] * self.rect.width(), self.rect.min.y + v[1] * self.rect.height())
    }

    /// The outline of a circle on the picture, as screen points.
    pub fn outline(&self, cx: f32, cy: f32, radius: f32) -> Vec<eframe::egui::Pos2> {
        (0..48)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / 48.0;
                self.to_screen([cx + a.cos() * radius / self.aspect, cy + a.sin() * radius])
            })
            .collect()
    }

    /// The picture's width / height.
    pub fn aspect(&self) -> f32 {
        self.aspect
    }

    /// Screen points per picture height, around texture point `t`.
    pub fn points_per_height(&self, t: [f32; 2]) -> f32 {
        let step = 0.01;
        (self.to_screen([t[0], t[1] + step]) - self.to_screen(t)).length() / step
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    #[test]
    fn field_round_trip() {
        let spots = vec![
            Spot { x: 0.25, y: 0.5, sx: 0.3, sy: 0.55, path: Vec::new(), radius: 0.012, feather: 0.5, opacity: 1.0, clone: false },
            Spot { x: 0.75, y: 0.1, sx: 0.7, sy: 0.12, path: vec![[0.8, 0.15], [0.85, 0.2]], radius: 0.05, feather: 0.0, opacity: 0.4, clone: true },
        ];
        let back = from_field(&to_field(&spots));
        assert_eq!(back.len(), 2);
        for (a, b) in spots.iter().zip(&back) {
            assert!((a.x - b.x).abs() < 1e-5 && (a.sy - b.sy).abs() < 1e-5 && (a.radius - b.radius).abs() < 1e-5);
            assert_eq!(a.clone, b.clone);
            assert_eq!(a.path.len(), b.path.len());
        }
        assert!(from_field("").is_empty());
        assert!(from_field("1,2,x").is_empty());
    }

    #[test]
    fn screen_map_round_trip() {
        let rect = eframe::egui::Rect::from_min_size(eframe::egui::pos2(20.0, 30.0), eframe::egui::vec2(800.0, 600.0));
        let rotations = [([1.0, 0.0, 0.0, 1.0], [0.0, 0.0]), ([0.0, 1.0, -1.0, 0.0], [0.0, 1.0]), ([-1.0, 0.0, 0.0, -1.0], [1.0, 1.0]), ([0.0, -1.0, 1.0, 0.0], [1.0, 0.0])];
        for (rot, off) in rotations {
            let mut u = crate::pipeline::Uniforms::zeroed();
            u.view = [0.6, 0.7, 0.45, 0.52];
            let angle = 0.2f32;
            u.straighten = [angle.cos(), angle.sin(), 1.4, 0.0];
            u.rot = rot;
            u.rot_off = [off[0], off[1], 0.0, 0.0];
            // Poly3 distortion, a little manual distortion and enlargement.
            u.lens_dist = [2.0, 0.05, 0.0, 0.0];
            u.lens_geo = [0.9, 1.0, 0.02, 1.05];
            u.lens_str = [1.0, 1.0, 0.0, 0.0];
            u.tca_r = [1.0, 0.0, 0.0, 0.0];
            u.tca_b = [1.0, 0.0, 0.0, 0.0];
            let map = ScreenMap::new(&u, rect, 1.5);
            for p in [eframe::egui::pos2(100.0, 100.0), eframe::egui::pos2(420.0, 330.0), eframe::egui::pos2(780.0, 600.0)] {
                let back = map.to_screen(map.to_texture(p));
                assert!((back - p).length() < 0.05, "{p:?} -> {back:?}");
            }
        }
    }
}
