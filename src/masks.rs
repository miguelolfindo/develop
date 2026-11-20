//! Masks: sliders that change only the parts of the photo brushed in, as Lightroom's brush
//! masks do.
//!
//! A mask is stored as settings, never as pixels: its slider values and the brush strokes
//! that make it (each a line through points with a round brush of some size, feather and
//! flow, painting in or erasing, following edges or not). Positions are on the stored picture
//! in texture coordinates (0..1 across and down, before crop, rotation and lens correction),
//! the brush radius a fraction of the picture's height, as for spots.
//!
//! For showing, each mask is drawn into a picture of how much it covers each point (0..1),
//! at the size of the small copy of the photo the spot tool uses, and handed to the shader.
//! The shader adds every mask's slider values, times its coverage, to the photo's own
//! sliders at that point, so a mask behaves as if the sliders were moved there.
//!
//! Auto Mask keeps a stroke to what looks like the place each dab of the brush starts on:
//! points whose brightness and colour differ much from the brush's centre are left out, so
//! the brush stops at edges.

use std::hash::{Hash, Hasher};

/// The most masks a photo can have (the shader's texture has this many layers).
pub const MAX_MASKS: usize = 8;

/// The sliders a mask has, in the order stored: name, range, Lightroom's step, unit.
pub const SLIDERS: [(&str, f32, f32, &str); 12] = [
    ("Temp", -100.0, 100.0, ""),
    ("Tint", -100.0, 100.0, ""),
    ("Exposure", -4.0, 4.0, " EV"),
    ("Contrast", -100.0, 100.0, ""),
    ("Highlights", -100.0, 100.0, ""),
    ("Shadows", -100.0, 100.0, ""),
    ("Whites", -100.0, 100.0, ""),
    ("Blacks", -100.0, 100.0, ""),
    ("Texture", -100.0, 100.0, ""),
    ("Clarity", -100.0, 100.0, ""),
    ("Dehaze", -100.0, 100.0, ""),
    ("Saturation", -100.0, 100.0, ""),
];
pub const LOCAL: usize = SLIDERS.len();
pub const TEMP: usize = 0;
pub const TINT: usize = 1;
pub const EXPOSURE: usize = 2;
pub const CONTRAST: usize = 3;

/// One brush stroke.
#[derive(Clone, PartialEq, Debug)]
pub struct Stroke {
    pub points: Vec<[f32; 2]>,
    /// Brush radius as a fraction of the picture's height.
    pub radius: f32,
    /// Soft edge, as a fraction of the radius (0 hard, 1 soft all the way in).
    pub feather: f32,
    /// How much one stroke paints in (or takes out), 0..1.
    pub flow: f32,
    pub erase: bool,
    /// Follow edges (Auto Mask).
    pub auto: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Mask {
    pub name: String,
    /// Shown and applied (the eye in the list).
    pub on: bool,
    pub values: [f32; LOCAL],
    pub strokes: Vec<Stroke>,
}

impl Mask {
    pub fn new(name: String) -> Self {
        Self { name, on: true, values: [0.0; LOCAL], strokes: Vec::new() }
    }

    /// True when it changes nothing (no strokes painted in, or every slider at 0).
    pub fn is_idle(&self) -> bool {
        !self.on || self.values.iter().all(|v| v.abs() < 1e-4) || !self.strokes.iter().any(|s| !s.erase)
    }
}

/// A name for a new mask: "Mask 1", "Mask 2"... the first not taken.
pub fn next_name(masks: &[Mask]) -> String {
    (1..).map(|n| format!("Mask {n}")).find(|n| !masks.iter().any(|m| &m.name == n)).unwrap_or_default()
}

/// Names may not hold the characters the stored form uses.
fn clean(name: &str) -> String {
    let s: String = name.chars().filter(|c| !matches!(c, ';' | '|' | '/' | ',' | '\t' | '\n' | '\r')).collect();
    let s = s.trim();
    if s.is_empty() { "Mask".to_owned() } else { s.chars().take(40).collect() }
}

/// The stored form, one field of a settings line: masks separated by `;`, each
/// `name|on|values|strokes`, values separated by `,`, strokes by `/`, each stroke
/// `radius,feather,flow,erase,auto,x,y,x,y...`.
pub fn to_field(masks: &[Mask]) -> String {
    masks
        .iter()
        .map(|m| {
            let values: Vec<String> = m.values.iter().map(|v| format!("{}", (v * 1000.0).round() / 1000.0)).collect();
            let strokes: Vec<String> = m
                .strokes
                .iter()
                .map(|s| {
                    let mut parts = vec![format!("{:.5}", s.radius), format!("{:.3}", s.feather), format!("{:.3}", s.flow), (s.erase as u8).to_string(), (s.auto as u8).to_string()];
                    for p in &s.points {
                        parts.push(format!("{:.5}", p[0]));
                        parts.push(format!("{:.5}", p[1]));
                    }
                    parts.join(",")
                })
                .collect();
            format!("{}|{}|{}|{}", clean(&m.name), m.on as u8, values.join(","), strokes.join("/"))
        })
        .collect::<Vec<_>>()
        .join(";")
}

pub fn from_field(text: &str) -> Vec<Mask> {
    let number = |s: &str| s.trim().parse::<f32>().ok().filter(|v| v.is_finite());
    text.split(';')
        .filter_map(|m| {
            let mut parts = m.split('|');
            let name = clean(parts.next()?);
            let on = parts.next()? != "0";
            let mut values = [0.0; LOCAL];
            for (i, v) in parts.next()?.split(',').enumerate().take(LOCAL) {
                values[i] = number(v).unwrap_or(0.0).clamp(SLIDERS[i].1, SLIDERS[i].2);
            }
            let strokes = parts
                .next()
                .unwrap_or("")
                .split('/')
                .filter_map(|s| {
                    let n: Vec<f32> = s.split(',').filter_map(number).collect();
                    if n.len() < 7 {
                        return None;
                    }
                    let points = n[5..].chunks_exact(2).map(|p| [p[0], p[1]]).collect();
                    Some(Stroke { radius: n[0].clamp(0.0005, 1.0), feather: n[1].clamp(0.0, 1.0), flow: n[2].clamp(0.0, 1.0), erase: n[3] > 0.5, auto: n[4] > 0.5, points })
                })
                .collect();
            Some(Mask { name, on, values, strokes })
        })
        .take(MAX_MASKS)
        .collect()
}

/// What the shader needs for `masks`: per mask, three vec4 of local changes (exposure in
/// stops, contrast as log2 of the power, highlights, shadows; whites, blacks, texture,
/// clarity; dehaze, saturation), and the white balance change as log2 gains on the
/// white-balanced camera colours. `gains` gives the camera white balance gains for a
/// temperature and tint; `kelvin` and `tint` are the photo's own.
pub fn fill_uniforms(masks: &[Mask], kelvin: f32, tint: f32, contrast_units: f32, gains: impl Fn(f32, f32) -> [f32; 3], u: &mut crate::pipeline::Uniforms) {
    let mut n = 0;
    let base = gains(kelvin, tint);
    for (i, m) in masks.iter().enumerate().take(MAX_MASKS) {
        let v = if m.on { m.values } else { [0.0; LOCAL] };
        u.mask_vals[i * 3] = [v[EXPOSURE], v[CONTRAST] / contrast_units, v[4] / 100.0, v[5] / 100.0];
        u.mask_vals[i * 3 + 1] = [v[6] / 100.0, v[7] / 100.0, v[8] / 100.0, v[9] / 100.0];
        u.mask_vals[i * 3 + 2] = [v[10] / 100.0, v[11] / 100.0, 0.0, 0.0];
        // Temp +100 is like moving the photo's own temperature 60 mired warmer; Tint +100 like
        // 60 steps of its tint towards magenta. Brightness is kept (green gain unchanged).
        let wb = if v[TEMP].abs() > 1e-3 || v[TINT].abs() > 1e-3 {
            let mired = (1.0e6 / kelvin.max(1000.0) - 0.6 * v[TEMP]).max(20.0);
            let shifted = gains(1.0e6 / mired, tint + 0.6 * v[TINT]);
            let ratio = [shifted[0] / base[0], shifted[1] / base[1], shifted[2] / base[2]];
            [(ratio[0] / ratio[1]).log2(), 0.0, (ratio[2] / ratio[1]).log2(), 0.0]
        } else {
            [0.0; 4]
        };
        u.mask_wb[i] = wb;
        // Masks after the last one that changes anything need not be read at all.
        if !m.is_idle() {
            n = i + 1;
        }
    }
    u.mask_info[0] = n as f32;
}

/// A picture of where a mask applies: `width` x `height` coverage values, 0..255.
#[derive(Clone, Default)]
pub struct Layer {
    /// Which strokes it was drawn from (see `strokes_key`); 0 = nothing drawn.
    pub key: u64,
    pub data: Vec<u8>,
}

/// What identifies a mask's strokes, for knowing when its picture must be drawn again.
pub fn strokes_key(strokes: &[Stroke], width: usize, height: usize) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (width, height, strokes.len()).hash(&mut h);
    for s in strokes {
        (s.radius.to_bits(), s.feather.to_bits(), s.flow.to_bits(), s.erase, s.auto, s.points.len()).hash(&mut h);
        for p in &s.points {
            (p[0].to_bits(), p[1].to_bits()).hash(&mut h);
        }
    }
    h.finish() | 1
}

/// Brightness and colour of every point of the small copy of the photo, as Auto Mask compares
/// them: log2 brightness, and two colour ratios.
pub struct Features {
    pub width: usize,
    pub height: usize,
    data: Vec<[f32; 3]>,
}

impl Features {
    pub fn new(preview: &crate::spots::Preview) -> Self {
        let data = preview
            .rgb
            .chunks_exact(3)
            .map(|p| {
                let sum = (p[0] + p[1] + p[2]).max(1e-5);
                let lum = 0.3 * p[0] + 0.55 * p[1] + 0.15 * p[2];
                [(lum + 1e-4).log2(), (p[0] - p[1]) / sum, (p[2] - p[1]) / sum]
            })
            .collect();
        Self { width: preview.width, height: preview.height, data }
    }

    fn at(&self, x: usize, y: usize) -> [f32; 3] {
        self.data[y * self.width + x]
    }

    /// The average over a 3 x 3 square: the brush centre's reference.
    fn reference(&self, x: usize, y: usize) -> [f32; 3] {
        let mut out = [0.0; 3];
        let mut n = 0.0f32;
        for yy in y.saturating_sub(1)..(y + 2).min(self.height) {
            for xx in x.saturating_sub(1)..(x + 2).min(self.width) {
                let f = self.at(xx, yy);
                for c in 0..3 {
                    out[c] += f[c];
                }
                n += 1.0;
            }
        }
        out.map(|v| v / n.max(1.0))
    }
}

/// How alike two points look to Auto Mask, 1 (the same) to 0 (across an edge).
fn alike(a: [f32; 3], b: [f32; 3]) -> f32 {
    // About a stop of brightness, or a clear change of colour, is an edge (a sky darkens
    // gradually across a brush, so brightness is allowed more than colour).
    let d = ((a[0] - b[0]) / 0.8).powi(2) + ((a[1] - b[1]) / 0.06).powi(2) + ((a[2] - b[2]) / 0.06).powi(2);
    let d = d.sqrt();
    1.0 - smoothstep(0.8, 2.0, d)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The brush's strength at `d` (distance from the dab's centre over its radius).
fn falloff(d: f32, feather: f32) -> f32 {
    let inner = 1.0 - feather;
    if d <= inner {
        1.0
    } else if d >= 1.0 {
        0.0
    } else {
        let t = 1.0 - (d - inner) / feather.max(1e-4);
        t * t * (3.0 - 2.0 * t)
    }
}

/// Paints a stroke dab by dab into a coverage buffer (the strongest dab wins at each point),
/// so a stroke can be drawn a piece at a time while it is being painted and come out the
/// same as drawn whole.
pub struct Stamper {
    /// Dabs placed this far apart along the stroke, as a fraction of the radius.
    spacing: f32,
    /// Distance travelled since the last dab, in picture heights.
    carried: f32,
    /// Points already painted through.
    pub done: usize,
    pub coverage: Vec<f32>,
    /// The part changed since `take_dirty` (x0, y0, x1, y1 in pixels), if any.
    dirty: Option<[usize; 4]>,
}

impl Stamper {
    pub fn new(width: usize, height: usize) -> Self {
        Self { spacing: 0.15, carried: 0.0, done: 0, coverage: vec![0.0; width * height], dirty: None }
    }

    /// Paint the points of `stroke` not painted yet.
    pub fn advance(&mut self, stroke: &Stroke, features: Option<&Features>, width: usize, height: usize) {
        let aspect = width as f32 / height.max(1) as f32;
        let step = (self.spacing * stroke.radius).max(0.5 / height as f32);
        while self.done < stroke.points.len() {
            let p = stroke.points[self.done];
            if self.done == 0 {
                self.dab(p, stroke, features, width, height);
            } else {
                let a = stroke.points[self.done - 1];
                let length = ((p[0] - a[0]) * aspect).hypot(p[1] - a[1]);
                let mut at = step - self.carried;
                while at <= length {
                    let t = at / length.max(1e-9);
                    self.dab([a[0] + (p[0] - a[0]) * t, a[1] + (p[1] - a[1]) * t], stroke, features, width, height);
                    at += step;
                }
                self.carried = length - (at - step);
            }
            self.done += 1;
        }
    }

    fn dab(&mut self, c: [f32; 2], stroke: &Stroke, features: Option<&Features>, width: usize, height: usize) {
        let r = stroke.radius * height as f32;
        let (cx, cy) = (c[0] * width as f32, c[1] * height as f32);
        let x0 = (cx - r).floor().max(0.0) as usize;
        let y0 = (cy - r).floor().max(0.0) as usize;
        let x1 = ((cx + r).ceil() as isize).clamp(0, width as isize) as usize;
        let y1 = ((cy + r).ceil() as isize).clamp(0, height as isize) as usize;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let features = features.filter(|f| stroke.auto && f.width == width && f.height == height);
        let reference = features.map(|f| f.reference((cx as usize).min(width - 1), (cy as usize).min(height - 1)));
        for y in y0..y1 {
            let dy = y as f32 + 0.5 - cy;
            for x in x0..x1 {
                let dx = x as f32 + 0.5 - cx;
                let d = (dx * dx + dy * dy).sqrt() / r.max(0.5);
                let mut v = falloff(d, stroke.feather);
                if v <= 0.0 {
                    continue;
                }
                if let (Some(f), Some(rf)) = (features, reference) {
                    v *= alike(f.at(x, y), rf);
                }
                let slot = &mut self.coverage[y * width + x];
                if v > *slot {
                    *slot = v;
                }
            }
        }
        self.dirty = Some(match self.dirty {
            Some(d) => [d[0].min(x0), d[1].min(y0), d[2].max(x1), d[3].max(y1)],
            None => [x0, y0, x1, y1],
        });
    }

    pub fn take_dirty(&mut self) -> Option<[usize; 4]> {
        self.dirty.take()
    }
}

/// Lay a stroke's coverage onto a mask's picture `base` over `area`, writing into `out`:
/// painting in adds (repeated strokes build up), erasing takes away.
pub fn composite(base: &[u8], coverage: &[f32], stroke: &Stroke, width: usize, area: [usize; 4], out: &mut [u8]) {
    for y in area[1]..area[3] {
        for x in area[0]..area[2] {
            let i = y * width + x;
            out[i] = blend(base[i], coverage[i], stroke);
        }
    }
}

fn blend(base: u8, coverage: f32, stroke: &Stroke) -> u8 {
    let m = base as f32 / 255.0;
    let c = coverage * stroke.flow;
    let v = if stroke.erase { m * (1.0 - c) } else { m + c * (1.0 - m) };
    (v * 255.0 + 0.5) as u8
}

/// Draw a whole mask from its strokes.
pub fn draw(strokes: &[Stroke], features: Option<&Features>, width: usize, height: usize) -> Vec<u8> {
    let mut data = vec![0u8; width * height];
    for s in strokes {
        let mut stamper = Stamper::new(width, height);
        stamper.advance(s, features, width, height);
        if let Some(area) = stamper.take_dirty() {
            for y in area[1]..area[3] {
                for x in area[0]..area[2] {
                    let i = y * width + x;
                    data[i] = blend(data[i], stamper.coverage[i], s);
                }
            }
        }
    }
    data
}

/// The masks' pictures as drawn for the open photo, and what still has to go to the GPU.
#[derive(Default)]
pub struct Drawn {
    /// Size of the pictures (that of the small copy of the photo).
    pub size: (usize, usize),
    features: Option<Features>,
    pub layers: Vec<Layer>,
    painting: Option<Painting>,
    /// Rows to upload: layer, first row, end row.
    pub pending: Vec<(usize, usize, usize)>,
    /// Size of the GPU texture as last made ((0, 0) = make it again, empty).
    pub gpu_size: (usize, usize),
}

/// A stroke being painted: on which mask, the mask's picture before it, and the stroke so far.
struct Painting {
    mask: usize,
    base_key: u64,
    base: Vec<u8>,
    stamper: Stamper,
}

impl Drawn {
    /// Bring every layer up to date with `masks`, and paint the newest part of the stroke
    /// being painted (`painting`: on which mask, and the stroke).
    pub fn sync(&mut self, masks: &[Mask], painting: Option<(usize, &Stroke)>, preview: &crate::spots::Preview) {
        let (w, h) = (preview.width, preview.height);
        if self.size != (w, h) {
            *self = Drawn { size: (w, h), gpu_size: self.gpu_size, ..Default::default() };
        }
        self.layers.resize(MAX_MASKS, Layer::default());
        let auto = masks.iter().flat_map(|m| &m.strokes).any(|s| s.auto) || painting.is_some_and(|(_, s)| s.auto);
        if auto && self.features.is_none() {
            self.features = Some(Features::new(preview));
        }
        for i in 0..MAX_MASKS {
            let want = masks.get(i).map_or(0, |m| strokes_key(&m.strokes, w, h));
            match painting {
                Some((pi, stroke)) if pi == i => {
                    let fresh = !matches!(&self.painting, Some(p) if p.mask == i && p.base_key == want);
                    if fresh {
                        self.bring(i, want, masks);
                        let base = if self.layers[i].data.is_empty() { vec![0; w * h] } else { self.layers[i].data.clone() };
                        self.painting = Some(Painting { mask: i, base_key: want, base, stamper: Stamper::new(w, h) });
                    }
                    self.paint(i, stroke);
                }
                _ => self.bring(i, want, masks),
            }
        }
        if painting.is_none() {
            self.painting = None;
        }
    }

    /// Make layer `i` show `masks[i]` as stored (`want` = its key).
    fn bring(&mut self, i: usize, want: u64, masks: &[Mask]) {
        let (w, h) = self.size;
        if self.layers[i].key == want {
            return;
        }
        if want == 0 {
            self.layers[i] = Layer::default();
        } else if let Some(j) = (0..self.layers.len()).find(|&j| j != i && self.layers[j].key == want) {
            // The same mask, moved in the list.
            self.layers[i] = self.layers[j].clone();
        } else {
            let data = draw(&masks[i].strokes, self.features.as_ref(), w, h);
            self.layers[i] = Layer { key: want, data };
        }
        self.pending.push((i, 0, h));
    }

    fn paint(&mut self, i: usize, stroke: &Stroke) {
        let (w, h) = self.size;
        let Some(p) = self.painting.as_mut() else { return };
        p.stamper.advance(stroke, self.features.as_ref(), w, h);
        if let Some(area) = p.stamper.take_dirty() {
            if self.layers[i].data.len() != w * h {
                self.layers[i].data = vec![0; w * h];
            }
            composite(&p.base, &p.stamper.coverage, stroke, w, area, &mut self.layers[i].data);
            self.pending.push((i, area[1], area[3]));
        }
    }

    /// `stroke` is finished and goes onto mask `i` after its strokes `before`: paint what is
    /// left of it, and take the layer as drawn from all of them (so it is not drawn again).
    pub fn finish(&mut self, i: usize, before: &[Stroke], stroke: &Stroke, preview: &crate::spots::Preview) {
        let (w, h) = self.size;
        if (w, h) != (preview.width, preview.height) || i >= self.layers.len() {
            return;
        }
        let base_key = strokes_key(before, w, h);
        if !matches!(&self.painting, Some(p) if p.mask == i && p.base_key == base_key) {
            if self.layers[i].key != base_key {
                // Not drawn as it was: it is drawn whole on the next sync.
                return;
            }
            let base = if self.layers[i].data.is_empty() { vec![0; w * h] } else { self.layers[i].data.clone() };
            self.painting = Some(Painting { mask: i, base_key, base, stamper: Stamper::new(w, h) });
        }
        if stroke.auto && self.features.is_none() {
            self.features = Some(Features::new(preview));
        }
        self.paint(i, stroke);
        let mut all = before.to_vec();
        all.push(stroke.clone());
        self.layers[i].key = strokes_key(&all, w, h);
        self.painting = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_round_trip() {
        let mut m = Mask::new("Sky".into());
        m.values[EXPOSURE] = -0.75;
        m.values[11] = 30.0;
        m.strokes.push(Stroke { points: vec![[0.1, 0.2], [0.3, 0.25]], radius: 0.05, feather: 0.5, flow: 0.8, erase: false, auto: true });
        m.strokes.push(Stroke { points: vec![[0.2, 0.2]], radius: 0.02, feather: 1.0, flow: 1.0, erase: true, auto: false });
        let mut off = Mask::new("Face".into());
        off.on = false;
        let masks = vec![m, off];
        let back = from_field(&to_field(&masks));
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].name, "Sky");
        assert_eq!(back[0].values, masks[0].values);
        assert_eq!(back[0].strokes, masks[0].strokes);
        assert!(!back[1].on);
        assert!(from_field("").is_empty());
    }

    #[test]
    fn painting_in_pieces_matches_painting_whole() {
        let stroke = Stroke { points: vec![[0.2, 0.3], [0.25, 0.32], [0.4, 0.5], [0.6, 0.5]], radius: 0.04, feather: 0.6, flow: 1.0, erase: false, auto: false };
        let (w, h) = (300, 200);
        let mut whole = Stamper::new(w, h);
        whole.advance(&stroke, None, w, h);
        let mut pieces = Stamper::new(w, h);
        for n in 1..=stroke.points.len() {
            let part = Stroke { points: stroke.points[..n].to_vec(), ..stroke.clone() };
            pieces.advance(&part, None, w, h);
        }
        assert_eq!(whole.coverage, pieces.coverage);
    }

    #[test]
    fn erase_takes_away() {
        let (w, h) = (100, 100);
        let paint = Stroke { points: vec![[0.5, 0.5]], radius: 0.2, feather: 0.0, flow: 1.0, erase: false, auto: false };
        let erase = Stroke { erase: true, radius: 0.1, ..paint.clone() };
        let data = draw(&[paint.clone()], None, w, h);
        assert_eq!(data[50 * w + 50], 255);
        let data = draw(&[paint, erase], None, w, h);
        assert_eq!(data[50 * w + 50], 0);
        assert_eq!(data[50 * w + 35], 255);
    }
}
