//! Tone Curve and Color Mixer settings, and what the shader needs from them.
//!
//! - Tone curve: Lightroom's four parametric regions (Highlights, Lights, Darks, Shadows,
//!   -100..100) and a point curve through any number of points, both on 0..1 scales. They
//!   are combined into one lookup table of `LUT_SIZE` entries, applied per channel to the
//!   working-space colour on a gamma 2.2 scale (as Lightroom does, measured), after the
//!   profile's own tone curve.
//! - Color Mixer (HSL): hue, saturation and luminance (-100..100) for eight colours.

/// Entries in the tone curve lookup table (the shader interpolates between them).
pub const LUT_SIZE: usize = 256;

/// The Color Mixer's colours: name, hue centre in degrees, and a swatch for the panel.
pub const COLOURS: [(&str, f32, [u8; 3]); 8] = [
    ("Red", 0.0, [0xE0, 0x4A, 0x3C]),
    ("Orange", 30.0, [0xEB, 0x8A, 0x2E]),
    ("Yellow", 60.0, [0xE8, 0xCF, 0x3A]),
    ("Green", 120.0, [0x5C, 0xB8, 0x4A]),
    ("Aqua", 180.0, [0x3F, 0xC4, 0xC4]),
    ("Blue", 240.0, [0x3F, 0x72, 0xE0]),
    ("Purple", 280.0, [0x8E, 0x55, 0xD8]),
    ("Magenta", 320.0, [0xD6, 0x4F, 0xB3]),
];

/// The point curve with nothing changed.
pub fn identity_curve() -> Vec<[f32; 2]> {
    vec![[0.0, 0.0], [1.0, 1.0]]
}

/// True when the curve and regions leave every tone as it is.
pub fn curve_is_identity(points: &[[f32; 2]], parametric: &[f32; 4]) -> bool {
    parametric.iter().all(|v| v.abs() < 1e-3) && points.iter().all(|p| (p[0] - p[1]).abs() < 1e-4)
}

/// Adobe's tone-curve spline (the DNG SDK's dng_spline_solver): a cubic through the points
/// whose slopes are smoothed weighted averages of the neighbouring segments.
pub struct Spline {
    x: Vec<f64>,
    y: Vec<f64>,
    s: Vec<f64>,
}

impl Spline {
    pub fn new(points: &[[f32; 2]]) -> Self {
        let mut pts: Vec<[f64; 2]> = points.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
        pts.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
        pts.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-6);
        if pts.len() < 2 {
            pts = vec![[0.0, 0.0], [1.0, 1.0]];
        }
        let x: Vec<f64> = pts.iter().map(|p| p[0]).collect();
        let y: Vec<f64> = pts.iter().map(|p| p[1]).collect();
        let n = x.len();
        let mut s = vec![0.0; n];
        let mut a = x[1] - x[0];
        let mut b = (y[1] - y[0]) / a;
        s[0] = b;
        for j in 2..n {
            let c = x[j] - x[j - 1];
            let d = (y[j] - y[j - 1]) / c;
            s[j - 1] = (b * c + d * a) / (a + c);
            a = c;
            b = d;
        }
        s[n - 1] = 2.0 * b - s[n - 2];
        s[0] = 2.0 * s[0] - s[1];
        if n > 2 {
            let (mut e, mut f, mut g) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
            f[0] = 0.5;
            e[n - 1] = 0.5;
            g[0] = 0.75 * (s[0] + s[1]);
            g[n - 1] = 0.75 * (s[n - 2] + s[n - 1]);
            for j in 1..n - 1 {
                let a = (x[j + 1] - x[j - 1]) * 2.0;
                e[j] = (x[j + 1] - x[j]) / a;
                f[j] = (x[j] - x[j - 1]) / a;
                g[j] = 1.5 * s[j];
            }
            for j in 1..n {
                let a = 1.0 - f[j - 1] * e[j];
                if j != n - 1 {
                    f[j] /= a;
                }
                g[j] = (g[j] - g[j - 1] * e[j]) / a;
            }
            for j in (0..n - 1).rev() {
                g[j] -= f[j] * g[j + 1];
            }
            s = g;
        }
        Self { x, y, s }
    }

    pub fn eval(&self, v: f32) -> f32 {
        let v = v as f64;
        let n = self.x.len();
        if v <= self.x[0] {
            return self.y[0] as f32;
        }
        if v >= self.x[n - 1] {
            return self.y[n - 1] as f32;
        }
        let j = self.x.partition_point(|&x| x < v).max(1);
        let (x0, x1, y0, y1, s0, s1) = (self.x[j - 1], self.x[j], self.y[j - 1], self.y[j], self.s[j - 1], self.s[j]);
        let a = x1 - x0;
        let b = (v - x0) / a;
        let c = (x1 - v) / a;
        (((y0 * (2.0 - c + b) + s0 * a * b) * (c * c)) + ((y1 * (2.0 - b + c) - s1 * a * c) * (b * b))) as f32
    }
}

/// Points across the tone range in `REGIONS` (0, 1/32 ... 1).
const REGION_POINTS: usize = 33;

/// How Lightroom's four regions move each tone, measured on its reference exports (photo
/// DSC05736, grey areas, on the tone curve's gamma 2.2 scale): for Highlights, Lights, Darks
/// and Shadows, the change at -100, -50, +50 and +100, at `REGION_POINTS` evenly spaced
/// tones. The regions are much wider than their names suggest (Darks reaches almost from
/// black to white) and not symmetric between + and -. Pulled down, Highlights and Lights
/// bring even pure white down a little, as Lightroom does.
#[rustfmt::skip]
const REGIONS: [[[f32; REGION_POINTS]; 4]; 4] = [
    [
        [0.0000, 0.0054, 0.0026, -0.0024, -0.0011, -0.0017, -0.0017, -0.0013, 0.0009, 0.0010, 0.0007, -0.0005, -0.0011, -0.0003, 0.0018, 0.0000, -0.0027, -0.0072, -0.0154, -0.0288, -0.0462, -0.0663, -0.0863, -0.1032, -0.1234, -0.1387, -0.1465, -0.1528, -0.1540, -0.1526, -0.1206, -0.0657, -0.0391],
        [0.0000, 0.0054, 0.0026, -0.0024, -0.0011, -0.0017, -0.0017, -0.0013, 0.0009, 0.0010, 0.0007, -0.0005, -0.0011, -0.0003, 0.0018, 0.0000, -0.0019, -0.0034, -0.0077, -0.0128, -0.0197, -0.0293, -0.0380, -0.0422, -0.0537, -0.0624, -0.0643, -0.0677, -0.0698, -0.0714, -0.0560, -0.0274, -0.0136],
        [0.0000, 0.0054, 0.0026, -0.0024, -0.0011, -0.0017, -0.0017, -0.0013, 0.0006, 0.0001, 0.0007, -0.0005, -0.0011, 0.0003, 0.0024, 0.0000, -0.0011, 0.0012, 0.0065, 0.0149, 0.0243, 0.0310, 0.0411, 0.0577, 0.0612, 0.0664, 0.0769, 0.0776, 0.0727, 0.0598, 0.0421, 0.0246, 0.0000],
        [0.0000, 0.0054, 0.0026, -0.0024, -0.0019, -0.0018, -0.0017, -0.0013, 0.0006, 0.0001, 0.0007, -0.0005, -0.0011, 0.0003, 0.0024, 0.0000, -0.0003, 0.0041, 0.0155, 0.0354, 0.0572, 0.0755, 0.0979, 0.1315, 0.1430, 0.1503, 0.1519, 0.1366, 0.1137, 0.0880, 0.0596, 0.0303, 0.0000],
    ],
    [
        [0.0000, 0.0044, 0.0018, -0.0051, -0.0090, -0.0151, -0.0229, -0.0324, -0.0438, -0.0583, -0.0738, -0.0915, -0.1101, -0.1286, -0.1472, -0.1659, -0.1822, -0.1955, -0.2071, -0.2148, -0.2209, -0.2269, -0.2294, -0.2242, -0.2274, -0.2247, -0.2106, -0.2008, -0.1893, -0.1760, -0.1339, -0.0700, -0.0391],
        [0.0000, 0.0054, 0.0026, -0.0031, -0.0048, -0.0074, -0.0100, -0.0140, -0.0182, -0.0234, -0.0301, -0.0388, -0.0468, -0.0548, -0.0621, -0.0710, -0.0801, -0.0874, -0.0933, -0.0979, -0.1021, -0.1063, -0.1088, -0.1042, -0.1081, -0.1088, -0.1000, -0.0961, -0.0914, -0.0869, -0.0660, -0.0316, -0.0149],
        [0.0000, 0.0054, 0.0031, -0.0007, 0.0021, 0.0038, 0.0077, 0.0124, 0.0200, 0.0264, 0.0337, 0.0399, 0.0482, 0.0592, 0.0723, 0.0808, 0.0901, 0.1018, 0.1139, 0.1246, 0.1346, 0.1400, 0.1460, 0.1560, 0.1519, 0.1468, 0.1394, 0.1234, 0.1032, 0.0794, 0.0537, 0.0284, 0.0000],
        [0.0000, 0.0054, 0.0037, 0.0004, 0.0049, 0.0105, 0.0196, 0.0299, 0.0447, 0.0600, 0.0789, 0.0980, 0.1207, 0.1484, 0.1797, 0.2047, 0.2296, 0.2552, 0.2776, 0.2909, 0.2993, 0.2979, 0.2858, 0.2661, 0.2406, 0.2133, 0.1861, 0.1562, 0.1250, 0.0938, 0.0625, 0.0312, 0.0000],
    ],
    [
        [0.0000, -0.0312, -0.0625, -0.0938, -0.1250, -0.1562, -0.1623, -0.1885, -0.2052, -0.2191, -0.2300, -0.2360, -0.2340, -0.2258, -0.2146, -0.2068, -0.1965, -0.1832, -0.1677, -0.1515, -0.1354, -0.1227, -0.1071, -0.0840, -0.0759, -0.0655, -0.0466, -0.0358, -0.0290, -0.0250, -0.0172, -0.0048, 0.0000],
        [0.0000, -0.0211, -0.0423, -0.0602, -0.0758, -0.0917, -0.1034, -0.1106, -0.1122, -0.1121, -0.1094, -0.1081, -0.1047, -0.0997, -0.0928, -0.0901, -0.0858, -0.0792, -0.0722, -0.0646, -0.0570, -0.0529, -0.0459, -0.0323, -0.0318, -0.0286, -0.0182, -0.0156, -0.0141, -0.0148, -0.0115, -0.0029, 0.0000],
        [0.0000, 0.0169, 0.0245, 0.0332, 0.0486, 0.0574, 0.0650, 0.0714, 0.0784, 0.0836, 0.0879, 0.0887, 0.0894, 0.0907, 0.0920, 0.0872, 0.0817, 0.0774, 0.0717, 0.0664, 0.0606, 0.0526, 0.0464, 0.0442, 0.0342, 0.0261, 0.0237, 0.0175, 0.0101, 0.0030, 0.0001, 0.0009, 0.0000],
        [0.0000, 0.0286, 0.0490, 0.0723, 0.0997, 0.1190, 0.1358, 0.1527, 0.1712, 0.1867, 0.2008, 0.2093, 0.2155, 0.2202, 0.2220, 0.2155, 0.2061, 0.1943, 0.1796, 0.1644, 0.1482, 0.1298, 0.1123, 0.0983, 0.0793, 0.0625, 0.0510, 0.0373, 0.0259, 0.0138, 0.0059, 0.0028, 0.0000],
    ],
    [
        [0.0000, -0.0312, -0.0625, -0.0847, -0.0973, -0.1106, -0.1142, -0.1112, -0.0979, -0.0836, -0.0669, -0.0526, -0.0374, -0.0219, -0.0076, -0.0028, -0.0024, -0.0018, -0.0008, 0.0006, 0.0012, 0.0004, 0.0010, 0.0060, 0.0011, -0.0013, 0.0027, 0.0014, -0.0013, -0.0056, -0.0057, -0.0010, 0.0000],
        [0.0000, -0.0146, -0.0327, -0.0460, -0.0528, -0.0583, -0.0561, -0.0511, -0.0421, -0.0352, -0.0276, -0.0225, -0.0168, -0.0096, -0.0023, -0.0010, -0.0019, -0.0018, -0.0008, 0.0006, 0.0012, -0.0005, 0.0006, 0.0060, 0.0011, -0.0013, 0.0027, 0.0014, -0.0013, -0.0056, -0.0057, -0.0010, 0.0000],
        [0.0000, 0.0169, 0.0245, 0.0312, 0.0424, 0.0460, 0.0469, 0.0447, 0.0410, 0.0359, 0.0292, 0.0211, 0.0140, 0.0096, 0.0066, 0.0011, -0.0019, -0.0018, -0.0008, 0.0006, 0.0012, -0.0005, -0.0003, 0.0056, 0.0011, -0.0013, 0.0027, 0.0014, -0.0013, -0.0056, -0.0057, -0.0010, 0.0000],
        [0.0000, 0.0293, 0.0496, 0.0709, 0.0946, 0.1064, 0.1110, 0.1077, 0.0997, 0.0865, 0.0699, 0.0511, 0.0348, 0.0220, 0.0118, 0.0021, -0.0019, -0.0018, -0.0008, 0.0006, 0.0012, -0.0005, -0.0003, 0.0056, 0.0011, -0.0013, 0.0027, 0.0014, -0.0013, -0.0056, -0.0057, -0.0010, 0.0000],
    ],
];

/// The parametric regions applied to `x` (0..1). `parametric` is Highlights, Lights, Darks,
/// Shadows, -100..100; between the measured settings the change is interpolated.
pub fn parametric(x: f32, parametric: &[f32; 4]) -> f32 {
    let f = x.clamp(0.0, 1.0) * (REGION_POINTS - 1) as f32;
    let i = (f.floor() as usize).min(REGION_POINTS - 2);
    let t = f - i as f32;
    let mut y = x;
    for (region, &amount) in parametric.iter().enumerate() {
        if amount.abs() < 1e-3 {
            continue;
        }
        let at = |row: usize| REGIONS[region][row][i] * (1.0 - t) + REGIONS[region][row][i + 1] * t;
        // Settings measured: -100, -50, (0), +50, +100.
        let a = amount.clamp(-100.0, 100.0);
        y += match a {
            a if a <= -50.0 => at(1) + (at(0) - at(1)) * (-a - 50.0) / 50.0,
            a if a < 0.0 => at(1) * (-a / 50.0),
            a if a <= 50.0 => at(2) * (a / 50.0),
            a => at(2) + (at(3) - at(2)) * (a - 50.0) / 50.0,
        };
    }
    y.clamp(0.0, 1.0)
}

/// The combined lookup table: the parametric regions, then the point curve (as Lightroom
/// applies them).
pub fn lut(points: &[[f32; 2]], regions: &[f32; 4]) -> [f32; LUT_SIZE] {
    let spline = Spline::new(points);
    std::array::from_fn(|i| {
        let x = i as f32 / (LUT_SIZE - 1) as f32;
        spline.eval(parametric(x, regions)).clamp(0.0, 1.0)
    })
}

/// The point curve as a sidecar field: `x,y;x,y;...`.
pub fn curve_to_field(points: &[[f32; 2]]) -> String {
    points.iter().map(|p| format!("{:.4},{:.4}", p[0], p[1])).collect::<Vec<_>>().join(";")
}

pub fn curve_from_field(text: &str) -> Vec<[f32; 2]> {
    let mut points: Vec<[f32; 2]> = text
        .split(';')
        .filter_map(|item| {
            let (x, y) = item.split_once(',')?;
            let (x, y) = (x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?);
            (x.is_finite() && y.is_finite()).then(|| [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)])
        })
        .take(32)
        .collect();
    points.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
    if points.len() < 2 {
        identity_curve()
    } else {
        points
    }
}

/// Numbers as one comma-separated field, and back (missing ones become 0).
pub fn numbers_to_field(values: &[f32]) -> String {
    values.iter().map(|v| format!("{v}")).collect::<Vec<_>>().join(",")
}

pub fn numbers_from_field<const N: usize>(text: &str, lo: f32, hi: f32) -> [f32; N] {
    let mut out = [0.0; N];
    for (slot, item) in out.iter_mut().zip(text.split(',')) {
        *slot = item.trim().parse::<f32>().ok().filter(|v| v.is_finite()).unwrap_or(0.0).clamp(lo, hi);
    }
    out
}

/// The Color Mixer for the shader: row 0..1 hue, 2..3 saturation, 4..5 luminance (eight
/// values in two vec4 each, -1..1).
pub fn hsl_uniform(hsl: &[[f32; 8]; 3]) -> [[f32; 4]; 6] {
    let mut out = [[0.0; 4]; 6];
    for (kind, row) in hsl.iter().enumerate() {
        for (i, v) in row.iter().enumerate() {
            out[kind * 2 + i / 4][i % 4] = v / 100.0;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spline_through_points() {
        let s = Spline::new(&[[0.0, 0.0], [0.5, 0.627], [1.0, 1.0]]);
        assert!((s.eval(0.5) - 0.627).abs() < 1e-4);
        assert!(s.eval(0.25) > 0.25 && s.eval(0.75) > 0.75);
        let line = Spline::new(&identity_curve());
        assert!((line.eval(0.3) - 0.3).abs() < 1e-5);
    }

    #[test]
    fn identity_lut() {
        let l = lut(&identity_curve(), &[0.0; 4]);
        assert!((l[128] - 128.0 / 255.0).abs() < 1e-4);
    }
}
