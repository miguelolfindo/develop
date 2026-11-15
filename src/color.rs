//! Colour science that runs on the CPU: Kelvin -> camera white-balance gains and the
//! camera -> linear sRGB matrix. Both are tiny and are re-uploaded as uniforms when needed.

pub type Mat3 = [[f32; 3]; 3];

/// Linear sRGB (D65) -> CIE XYZ.
const XYZ_FROM_SRGB: Mat3 = [
    [0.412_456_4, 0.357_576_1, 0.180_437_5],
    [0.212_672_9, 0.715_152_2, 0.072_175_0],
    [0.019_333_9, 0.119_192_0, 0.950_304_1],
];

pub const KELVIN_MIN: f32 = 2000.0;
pub const KELVIN_MAX: f32 = 12000.0;
pub const KELVIN_DEFAULT: f32 = 5500.0;

pub fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut m = [[0.0; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            m[r][c] = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    m
}

fn mul_vec(m: &Mat3, v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

pub fn invert(m: &Mat3) -> Option<Mat3> {
    let c = |r1: usize, c1: usize, r2: usize, c2: usize| m[r1][c1] * m[r2][c2] - m[r1][c2] * m[r2][c1];
    let det = m[0][0] * c(1, 1, 2, 2) - m[0][1] * c(1, 0, 2, 2) + m[0][2] * c(1, 0, 2, 1);
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    Some([
        [c(1, 1, 2, 2) * d, -c(0, 1, 2, 2) * d, c(0, 1, 1, 2) * d],
        [-c(1, 0, 2, 2) * d, c(0, 0, 2, 2) * d, -c(0, 0, 1, 2) * d],
        [c(1, 0, 2, 1) * d, -c(0, 0, 2, 1) * d, c(0, 0, 1, 1) * d],
    ])
}

/// Tint slider units per unit of uv offset from the Planckian locus (the DNG SDK uses 3000;
/// fitted to Lightroom exports).
pub const TINT_UV_SCALE: f32 = 3000.0;

/// CIE xy chromaticity of a black body at `kelvin` (Kim et al. cubic-spline approximation
/// of the Planckian locus, valid 1667 K .. 25000 K).
fn planckian_xy(kelvin: f32) -> (f32, f32) {
    let t = kelvin.clamp(1667.0, 25000.0) as f64;
    let x = if t <= 4000.0 {
        -0.266_123_9e9 / (t * t * t) - 0.234_358_9e6 / (t * t) + 0.877_695_6e3 / t + 0.179_910
    } else {
        -3.025_846_9e9 / (t * t * t) + 2.107_037_9e6 / (t * t) + 0.222_634_7e3 / t + 0.240_390
    };
    let y = if t <= 2222.0 {
        -1.106_381_4 * x * x * x - 1.348_110_20 * x * x + 2.185_558_32 * x - 0.202_196_83
    } else if t <= 4000.0 {
        -0.954_947_6 * x * x * x - 1.374_185_93 * x * x + 2.091_370_15 * x - 0.167_488_67
    } else {
        3.081_758_0 * x * x * x - 5.873_386_70 * x * x + 3.751_129_97 * x - 0.370_014_83
    };
    (x as f32, y as f32)
}

/// Everything needed to turn a Kelvin slider value into shader uniforms for one camera/file.
#[derive(Clone, Debug)]
pub struct CameraColor {
    xyz_to_cam: Mat3,
    /// White-balanced camera RGB -> linear sRGB.
    pub cam_to_srgb: Mat3,
    /// Green/magenta offset of the as-shot white balance from the Planckian locus. It is kept
    /// constant while the temperature slider moves, so a shot opens exactly as the camera saw it.
    tint: f32,
    /// Colour temperature the camera recorded, snapped into the slider range.
    pub as_shot_kelvin: f32,
}

impl CameraColor {
    /// `xyz_to_cam` is the camera's colour matrix (XYZ -> camera RGB); `as_shot_wb` are the
    /// camera's R, G, B white-balance multipliers.
    pub fn new(xyz_to_cam: Mat3, as_shot_wb: [f32; 3]) -> Self {
        // dcraw-style: normalise rows so that white-balanced camera white maps to sRGB white.
        let mut cam_from_srgb = mul(&xyz_to_cam, &XYZ_FROM_SRGB);
        for row in &mut cam_from_srgb {
            let sum: f32 = row.iter().sum();
            if sum.abs() > 1e-6 {
                row.iter_mut().for_each(|v| *v /= sum);
            }
        }
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let cam_to_srgb = invert(&cam_from_srgb).unwrap_or(identity);

        let mut this = Self { xyz_to_cam, cam_to_srgb, tint: 1.0, as_shot_kelvin: KELVIN_DEFAULT };

        let valid = as_shot_wb.iter().all(|v| v.is_finite() && *v > 0.0);
        if valid {
            let target_r = as_shot_wb[0] / as_shot_wb[1];
            let target_b = as_shot_wb[2] / as_shot_wb[1];
            // The R/B gain ratio rises monotonically with temperature; bisect for the match.
            let (mut lo, mut hi) = (1667.0_f32, 25000.0_f32);
            for _ in 0..48 {
                let mid = 0.5 * (lo + hi);
                let g = this.locus_gains(mid);
                if g[0] / g[2] > target_r / target_b {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            let kelvin = (0.5 * (lo + hi)).clamp(KELVIN_MIN, KELVIN_MAX);
            let kelvin = (kelvin / 10.0).round() * 10.0;
            let g = this.locus_gains(kelvin);
            // Geometric mean of the two residuals: exact when the temperature was not clamped.
            let tint = ((target_r / g[0]) * (target_b / g[2])).sqrt();
            if tint.is_finite() && tint > 0.0 {
                this.tint = tint;
                this.as_shot_kelvin = kelvin;
            }
        }
        this
    }

    /// Gains that make a black body at `kelvin` neutral in camera space (green = 1).
    fn locus_gains(&self, kelvin: f32) -> [f32; 3] {
        let (x, y) = planckian_xy(kelvin);
        let xyz = [x / y, 1.0, (1.0 - x - y) / y];
        let cam = mul_vec(&self.xyz_to_cam, xyz);
        let safe = |v: f32| if v.is_finite() && v > 1e-4 { v } else { 1e-4 };
        [safe(cam[1]) / safe(cam[0]), 1.0, safe(cam[1]) / safe(cam[2])]
    }

    /// Camera-space R, G, B multipliers for a white-balance temperature. A higher temperature
    /// means "the light was bluer", so the picture is rendered warmer (as in any raw converter).
    pub fn wb_gains(&self, kelvin: f32) -> [f32; 3] {
        let g = self.locus_gains(kelvin);
        [g[0] * self.tint, 1.0, g[2] * self.tint]
    }

    /// `wb_gains` with the Tint slider applied as Adobe defines it (DNG specification): the
    /// white point moves off the Planckian locus along its normal in CIE 1960 uv, by
    /// `tint / TINT_UV_SCALE`. Positive tint = magenta. How the change splits between red and
    /// blue therefore depends on the temperature, as in Lightroom.
    pub fn wb_gains_tinted(&self, kelvin: f32, tint: f32) -> [f32; 3] {
        let base = self.wb_gains(kelvin);
        if tint == 0.0 {
            return base;
        }
        let shifted = self.off_locus_gains(kelvin, tint / TINT_UV_SCALE);
        let plain = self.off_locus_gains(kelvin, 0.0);
        [base[0] * shifted[0] / plain[0], 1.0, base[2] * shifted[2] / plain[2]]
    }

    /// Gains that make neutral the colour `offset` (uv units) off the locus at `kelvin`.
    fn off_locus_gains(&self, kelvin: f32, offset: f32) -> [f32; 3] {
        let to_uv = |(x, y): (f32, f32)| {
            let d = -2.0 * x + 12.0 * y + 3.0;
            (4.0 * x / d, 6.0 * y / d)
        };
        let k = kelvin.clamp(1700.0, 24000.0);
        let (u, v) = to_uv(planckian_xy(k));
        let (u2, v2) = to_uv(planckian_xy(k + 50.0));
        let len = ((u2 - u).powi(2) + (v2 - v).powi(2)).sqrt().max(1e-9);
        // Normal to the locus; with temperature rising along it, (+du, -dv)... rotated is the
        // green side, so subtracting moves towards magenta.
        let (nu, nv) = (-(v2 - v) / len, (u2 - u) / len);
        let (u, v) = (u - nu * offset, v - nv * offset);
        let d = 2.0 * u - 8.0 * v + 4.0;
        let (x, y) = (3.0 * u / d, 2.0 * v / d);
        let xyz = [x / y, 1.0, (1.0 - x - y) / y];
        let cam = mul_vec(&self.xyz_to_cam, xyz);
        let safe = |v: f32| if v.is_finite() && v > 1e-4 { v } else { 1e-4 };
        [safe(cam[1]) / safe(cam[0]), 1.0, safe(cam[1]) / safe(cam[2])]
    }
}
