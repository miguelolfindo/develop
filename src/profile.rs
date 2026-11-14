//! Turns a profile choice (built-in or an Adobe .dcp) into what the shader needs: two
//! matrices, optional hue/saturation tables and a tone-curve lookup table.
//!
//! DCP rendering follows the DNG specification's order: white-balanced camera RGB ->
//! ForwardMatrix -> ProPhoto RGB -> HueSatMap -> exposure -> LookTable -> tone curve.

use crate::color::{self, CameraColor, Mat3};
use crate::dcp::{Dcp, HsvTable};

/// XYZ (D50) -> linear ProPhoto RGB.
const PROPHOTO_FROM_XYZ: Mat3 = [
    [1.345_943_3, -0.255_607_5, -0.051_111_8],
    [-0.544_598_9, 1.508_167_3, 0.020_535_1],
    [0.0, 0.0, 1.211_812_8],
];
/// Linear ProPhoto RGB -> XYZ (D50). Its middle row is ProPhoto's luminance weights.
const XYZ_FROM_PROPHOTO: Mat3 = [
    [0.797_674_9, 0.135_191_7, 0.031_353_4],
    [0.288_040_2, 0.711_874_1, 0.000_085_7],
    [0.0, 0.0, 0.825_210_0],
];
/// XYZ (D50) -> linear sRGB, Bradford-adapted to D65.
const SRGB_FROM_XYZ_D50: Mat3 = [
    [3.133_856_1, -1.616_866_7, -0.490_614_6],
    [-0.978_768_4, 1.916_141_5, 0.033_454_0],
    [0.071_945_3, -0.228_991_4, 1.405_242_7],
];
const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
const REC709_LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Adobe Camera Raw's default tone curve (linear in -> linear out), every 16th entry of the
/// 1025-entry table published in the DNG SDK. Used when a profile has no curve of its own,
/// which is the case for "Adobe Standard".
const ACR_DEFAULT_CURVE: [f32; 65] = [
    0.00000, 0.01485, 0.04002, 0.07057, 0.10433, 0.14061, 0.17901, 0.21929, 0.25961, 0.29833, 0.33531,
    0.37050, 0.40389, 0.43554, 0.46551, 0.49387, 0.52069, 0.54607, 0.57007, 0.59281, 0.61436, 0.63482,
    0.65426, 0.67275, 0.69035, 0.70711, 0.72309, 0.73834, 0.75289, 0.76679, 0.78006, 0.79274, 0.80486,
    0.81645, 0.82753, 0.83812, 0.84825, 0.85794, 0.86720, 0.87606, 0.88453, 0.89262, 0.90035, 0.90773,
    0.91478, 0.92150, 0.92792, 0.93403, 0.93986, 0.94540, 0.95067, 0.95567, 0.96042, 0.96493, 0.96919,
    0.97322, 0.97702, 0.98061, 0.98398, 0.98714, 0.99009, 0.99285, 0.99542, 0.99780, 1.00000,
];

/// Number of entries in the tone-curve texture. Entry i holds curve((i / (N-1))^2): the
/// square puts most of the resolution in the shadows, where the curve bends fastest.
pub const CURVE_LUT_SIZE: usize = 1024;

/// A hue/sat/value table laid out for a 3D texture: x = hue, y = saturation, z = value.
pub struct GpuTable {
    pub dims: [u32; 3],
    /// RGBA half-float bits: (hue shift in degrees, saturation scale, value scale, 1).
    pub rgba: Vec<u16>,
    pub srgb_value_axis: bool,
}

pub struct ProfileRender {
    /// White-balanced camera RGB -> working space.
    pub cam_to_work: Mat3,
    /// Working space -> linear sRGB.
    pub work_to_srgb: Mat3,
    /// Luminance weights of the working space.
    pub luma: [f32; 3],
    /// Stops added to the Exposure slider (the profile's baseline exposure).
    pub exposure_bias: f32,
    /// 0 = no curve, 1 = the built-in fitted curve, 2 = `curve` lookup table.
    pub curve_mode: f32,
    /// Saturation applied before the built-in fitted curve.
    pub pre_saturation: f32,
    /// Contrast applied after the tone curve (1 = none); see `ADOBE_COLOR_CONTRAST`.
    pub post_contrast: f32,
    pub hue_sat: Option<GpuTable>,
    pub look: Option<GpuTable>,
    /// RGBA half-float bits, CURVE_LUT_SIZE texels, curve value in the red channel.
    pub curve: Option<Vec<u16>>,
}

/// The two built-in renderings, which work directly in linear sRGB.
pub fn builtin(camera: &CameraColor, fitted_curve: bool) -> ProfileRender {
    ProfileRender {
        cam_to_work: camera.cam_to_srgb,
        work_to_srgb: IDENTITY,
        luma: REC709_LUMA,
        exposure_bias: 0.0,
        curve_mode: if fitted_curve { 1.0 } else { 0.0 },
        pre_saturation: 0.85,
        post_contrast: 1.0,
        hue_sat: None,
        look: None,
        curve: None,
    }
}

/// Weight of calibration 1 (the warmer illuminant) at `kelvin`, interpolating in 1/T as the
/// DNG specification prescribes.
fn warm_weight(dcp: &Dcp, kelvin: f32) -> f32 {
    let temp = |code: u16| -> f32 {
        match code {
            17 => 2856.0, // Standard light A
            18 => 4874.0, // B
            19 => 6774.0, // C
            20 => 5503.0, // D55
            21 => 6504.0, // D65
            22 => 7504.0, // D75
            23 => 5003.0, // D50
            3 => 2850.0,  // Tungsten
            _ => 5500.0,
        }
    };
    let (t1, t2) = (temp(dcp.illuminant[0]), temp(dcp.illuminant[1]));
    if (t1 - t2).abs() < 1.0 {
        return 0.5;
    }
    let (lo, hi, lo_is_first) = if t1 < t2 { (t1, t2, true) } else { (t2, t1, false) };
    let w_lo = ((1.0 / kelvin - 1.0 / hi) / (1.0 / lo - 1.0 / hi)).clamp(0.0, 1.0);
    if lo_is_first {
        w_lo
    } else {
        1.0 - w_lo
    }
}

fn lerp_mat(a: &Mat3, b: &Mat3, wa: f32) -> Mat3 {
    let mut m = [[0.0; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            m[r][c] = a[r][c] * wa + b[r][c] * (1.0 - wa);
        }
    }
    m
}

fn gpu_table(t: &HsvTable, other: Option<(&HsvTable, f32)>) -> GpuTable {
    let f16 = |v: f32| half::f16::from_f32(v).to_bits();
    let mut rgba = Vec::with_capacity(t.data.len() * 4);
    for v in 0..t.vals {
        for s in 0..t.sats {
            for h in 0..t.hues {
                let i = (h * t.sats + s) * t.vals + v;
                let mut e = t.data[i];
                if let Some((o, w)) = other {
                    let b = o.data[i];
                    e = [e[0] * w + b[0] * (1.0 - w), e[1] * w + b[1] * (1.0 - w), e[2] * w + b[2] * (1.0 - w)];
                }
                rgba.extend_from_slice(&[f16(e[0]), f16(e[1]), f16(e[2]), f16(1.0)]);
            }
        }
    }
    GpuTable { dims: [t.hues as u32, t.sats as u32, t.vals as u32], rgba, srgb_value_axis: t.srgb_value_axis }
}

/// Smooth interpolation through curve points (cubic Hermite, finite-difference tangents).
fn sample_curve(points: &[[f32; 2]], x: f32) -> f32 {
    let n = points.len();
    if n == 0 {
        return x;
    }
    if x <= points[0][0] {
        return points[0][1];
    }
    if x >= points[n - 1][0] {
        return points[n - 1][1];
    }
    let i = points.partition_point(|p| p[0] <= x).clamp(1, n - 1) - 1;
    let (p0, p1) = (points[i], points[i + 1]);
    let h = (p1[0] - p0[0]).max(1e-9);
    let slope = |a: [f32; 2], b: [f32; 2]| (b[1] - a[1]) / (b[0] - a[0]).max(1e-9);
    let m0 = if i == 0 { slope(p0, p1) } else { slope(points[i - 1], p1) };
    let m1 = if i + 2 >= n { slope(p0, p1) } else { slope(p0, points[i + 2]) };
    let t = (x - p0[0]) / h;
    let (t2, t3) = (t * t, t * t * t);
    let y = (2.0 * t3 - 3.0 * t2 + 1.0) * p0[1]
        + (t3 - 2.0 * t2 + t) * h * m0
        + (-2.0 * t3 + 3.0 * t2) * p1[1]
        + (t3 - t2) * h * m1;
    y.clamp(p0[1].min(p1[1]), p0[1].max(p1[1]))
}

fn curve_lut(points: &[[f32; 2]]) -> Vec<u16> {
    let one = half::f16::from_f32(1.0).to_bits();
    let mut out = Vec::with_capacity(CURVE_LUT_SIZE * 4);
    for i in 0..CURVE_LUT_SIZE {
        let u = i as f32 / (CURVE_LUT_SIZE - 1) as f32;
        let y = half::f16::from_f32(sample_curve(points, u * u).clamp(0.0, 1.0)).to_bits();
        out.extend_from_slice(&[y, y, y, one]);
    }
    out
}

/// Rendering data for an Adobe camera profile at the given white-balance temperature.
/// `baseline_exposure` is the camera's own baseline (stops); the profile's offset is added.
pub fn from_dcp(
    dcp: &Dcp,
    camera: &CameraColor,
    kelvin: f32,
    baseline_exposure: f32,
    post_contrast: f32,
) -> ProfileRender {
    let w1 = warm_weight(dcp, kelvin);
    let work_to_srgb = color::mul(&SRGB_FROM_XYZ_D50, &XYZ_FROM_PROPHOTO);

    let forward = match (&dcp.forward_matrix[0], &dcp.forward_matrix[1]) {
        (Some(a), Some(b)) => Some(lerp_mat(a, b, w1)),
        (Some(a), None) | (None, Some(a)) => Some(*a),
        (None, None) => None,
    };
    let cam_to_work = match forward {
        Some(fm) => color::mul(&PROPHOTO_FROM_XYZ, &fm),
        // No forward matrix: go through the generic camera -> sRGB matrix instead.
        None => match color::invert(&work_to_srgb) {
            Some(srgb_to_work) => color::mul(&srgb_to_work, &camera.cam_to_srgb),
            None => camera.cam_to_srgb,
        },
    };

    let hue_sat = match (&dcp.hue_sat[0], &dcp.hue_sat[1]) {
        (Some(a), Some(b)) if a.data.len() == b.data.len() => Some(gpu_table(a, Some((b, w1)))),
        (Some(a), _) | (None, Some(a)) => Some(gpu_table(a, None)),
        (None, None) => None,
    };

    let acr: Vec<[f32; 2]> = ACR_DEFAULT_CURVE.iter().enumerate().map(|(i, y)| [i as f32 / 64.0, *y]).collect();
    let points = dcp.tone_curve.as_deref().filter(|c| c.len() >= 2).unwrap_or(&acr);

    ProfileRender {
        cam_to_work,
        work_to_srgb,
        luma: XYZ_FROM_PROPHOTO[1],
        exposure_bias: baseline_exposure + dcp.baseline_exposure_offset,
        curve_mode: 2.0,
        pre_saturation: 1.0,
        post_contrast,
        hue_sat,
        look: dcp.look.as_ref().map(|t| gpu_table(t, None)),
        curve: Some(curve_lut(points)),
    }
}
