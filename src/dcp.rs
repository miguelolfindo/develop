//! Reader for Adobe DNG Camera Profiles (.dcp): a TIFF-like container holding the camera's
//! colour matrices, hue/saturation tables and tone curve. Only the tags the renderer uses
//! are decoded.

use std::path::Path;

use crate::color::Mat3;

/// A 3D table indexed [hue][saturation][value]; each entry is
/// (hue shift in degrees, saturation scale, value scale).
#[derive(Clone)]
pub struct HsvTable {
    pub hues: usize,
    pub sats: usize,
    pub vals: usize,
    pub data: Vec<[f32; 3]>,
    /// True when the value axis is sRGB-encoded rather than linear.
    pub srgb_value_axis: bool,
}

#[derive(Clone, Default)]
pub struct Dcp {
    pub name: String,
    /// CIE illuminant codes (EXIF LightSource) for the two calibrations: 17 = A, 21 = D65...
    pub illuminant: [u16; 2],
    /// XYZ -> camera.
    pub color_matrix: [Option<Mat3>; 2],
    /// White-balanced camera -> XYZ (D50).
    pub forward_matrix: [Option<Mat3>; 2],
    pub hue_sat: [Option<HsvTable>; 2],
    pub look: Option<HsvTable>,
    /// Points (input, output), both linear-light 0..1 on a gamma-encoded axis per the DNG spec.
    pub tone_curve: Option<Vec<[f32; 2]>>,
    pub baseline_exposure_offset: f32,
}

struct Reader<'a> {
    bytes: &'a [u8],
    big_endian: bool,
}

impl Reader<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b: [u8; 2] = self.bytes.get(at..at + 2)?.try_into().ok()?;
        Some(if self.big_endian { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) })
    }
    fn u32(&self, at: usize) -> Option<u32> {
        let b: [u8; 4] = self.bytes.get(at..at + 4)?.try_into().ok()?;
        Some(if self.big_endian { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    }
    fn f64(&self, at: usize) -> Option<f64> {
        let b: [u8; 8] = self.bytes.get(at..at + 8)?.try_into().ok()?;
        Some(if self.big_endian { f64::from_be_bytes(b) } else { f64::from_le_bytes(b) })
    }

    /// Decode one IFD entry's values as f64, whatever its TIFF type.
    fn values(&self, entry: usize) -> Option<Vec<f64>> {
        let kind = self.u16(entry + 2)?;
        let count = self.u32(entry + 4)? as usize;
        let size: usize = match kind {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            _ => return None,
        };
        let total = size.checked_mul(count)?;
        let start = if total <= 4 { entry + 8 } else { self.u32(entry + 8)? as usize };
        self.bytes.get(start..start.checked_add(total)?)?;
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let at = start + i * size;
            out.push(match kind {
                1 | 2 | 7 => self.bytes[at] as f64,
                6 => self.bytes[at] as i8 as f64,
                3 => self.u16(at)? as f64,
                8 => self.u16(at)? as i16 as f64,
                4 => self.u32(at)? as f64,
                9 => self.u32(at)? as i32 as f64,
                5 => self.u32(at)? as f64 / (self.u32(at + 4)? as f64).max(1.0),
                10 => {
                    let d = self.u32(at + 4)? as i32 as f64;
                    self.u32(at)? as i32 as f64 / if d == 0.0 { 1.0 } else { d }
                }
                11 => f32::from_bits(self.u32(at)?) as f64,
                _ => self.f64(at)?,
            });
        }
        Some(out)
    }
}

fn mat3(v: &[f64]) -> Option<Mat3> {
    if v.len() < 9 {
        return None;
    }
    let f = |i: usize| v[i] as f32;
    Some([[f(0), f(1), f(2)], [f(3), f(4), f(5)], [f(6), f(7), f(8)]])
}

fn table(dims: &[f64], data: &[f64], srgb_value_axis: bool) -> Option<HsvTable> {
    if dims.len() < 3 {
        return None;
    }
    let (hues, sats, vals) = (dims[0] as usize, dims[1] as usize, dims[2] as usize);
    let n = hues.checked_mul(sats)?.checked_mul(vals)?;
    if n == 0 || data.len() < n * 3 {
        return None;
    }
    // File order: value outermost, then hue, then saturation.
    let mut out = vec![[0f32; 3]; n];
    for v in 0..vals {
        for h in 0..hues {
            for s in 0..sats {
                let src = ((v * hues + h) * sats + s) * 3;
                out[(h * sats + s) * vals + v] = [data[src] as f32, data[src + 1] as f32, data[src + 2] as f32];
            }
        }
    }
    Some(HsvTable { hues, sats, vals, data: out, srgb_value_axis })
}

pub fn load(path: &Path) -> Result<Dcp, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Could not read profile: {e}"))?;
    let big_endian = match bytes.get(0..4) {
        Some([b'I', b'I', b'R', b'C']) => false,
        Some([b'M', b'M', b'C', b'R']) => true,
        _ => return Err("Not a DCP camera profile.".into()),
    };
    let r = Reader { bytes: &bytes, big_endian };
    let bad = || "Camera profile is damaged.".to_owned();
    let ifd = r.u32(4).ok_or_else(bad)? as usize;
    let count = r.u16(ifd).ok_or_else(bad)? as usize;

    let mut dcp = Dcp::default();
    let (mut hs_dims, mut hs1, mut hs2, mut look_dims, mut look) = (None, None, None, None, None);
    let (mut hs_encoding, mut look_encoding) = (0.0, 0.0);
    for i in 0..count {
        let entry = ifd + 2 + i * 12;
        let tag = r.u16(entry).ok_or_else(bad)?;
        let Some(v) = r.values(entry) else { continue };
        match tag {
            50936 => dcp.name = v.iter().map(|c| *c as u8 as char).filter(|c| *c != '\0').collect(),
            50778 => dcp.illuminant[0] = v.first().copied().unwrap_or(0.0) as u16,
            50779 => dcp.illuminant[1] = v.first().copied().unwrap_or(0.0) as u16,
            50721 => dcp.color_matrix[0] = mat3(&v),
            50722 => dcp.color_matrix[1] = mat3(&v),
            50964 => dcp.forward_matrix[0] = mat3(&v),
            50965 => dcp.forward_matrix[1] = mat3(&v),
            50937 => hs_dims = Some(v),
            50938 => hs1 = Some(v),
            50939 => hs2 = Some(v),
            50981 => look_dims = Some(v),
            50982 => look = Some(v),
            51107 => hs_encoding = v.first().copied().unwrap_or(0.0),
            51108 => look_encoding = v.first().copied().unwrap_or(0.0),
            51109 => dcp.baseline_exposure_offset = v.first().copied().unwrap_or(0.0) as f32,
            50940 => {
                dcp.tone_curve = Some(v.chunks_exact(2).map(|p| [p[0] as f32, p[1] as f32]).collect());
            }
            _ => {}
        }
    }
    if let Some(dims) = &hs_dims {
        dcp.hue_sat[0] = hs1.and_then(|d| table(dims, &d, hs_encoding == 1.0));
        dcp.hue_sat[1] = hs2.and_then(|d| table(dims, &d, hs_encoding == 1.0));
    }
    if let (Some(dims), Some(data)) = (&look_dims, look) {
        dcp.look = table(dims, &data, look_encoding == 1.0);
    }
    Ok(dcp)
}

/// Human-readable summary, used by `--dump-dcp`.
pub fn describe(dcp: &Dcp) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(s, "name: {}", dcp.name);
    let _ = writeln!(s, "illuminants: {:?}", dcp.illuminant);
    let _ = writeln!(s, "baseline exposure offset: {}", dcp.baseline_exposure_offset);
    for i in 0..2 {
        let _ = writeln!(s, "color matrix {}: {:?}", i + 1, dcp.color_matrix[i]);
        let _ = writeln!(s, "forward matrix {}: {:?}", i + 1, dcp.forward_matrix[i]);
        match &dcp.hue_sat[i] {
            Some(t) => {
                let _ = writeln!(s, "hue/sat map {}: {}x{}x{} srgb-axis={}", i + 1, t.hues, t.sats, t.vals, t.srgb_value_axis);
            }
            None => {
                let _ = writeln!(s, "hue/sat map {}: none", i + 1);
            }
        }
    }
    match &dcp.look {
        Some(t) => {
            let _ = writeln!(s, "look table: {}x{}x{} srgb-axis={}", t.hues, t.sats, t.vals, t.srgb_value_axis);
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for e in &t.data {
                for k in 0..3 {
                    lo[k] = lo[k].min(e[k]);
                    hi[k] = hi[k].max(e[k]);
                }
            }
            let _ = writeln!(s, "  hue shift {:.1}..{:.1} deg, sat x{:.2}..{:.2}, value x{:.2}..{:.2}", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]);
        }
        None => {
            let _ = writeln!(s, "look table: none");
        }
    }
    match &dcp.tone_curve {
        Some(c) => {
            let _ = writeln!(s, "tone curve: {} points", c.len());
            for p in c.iter().step_by((c.len() / 24).max(1)) {
                let _ = writeln!(s, "  {:.5} -> {:.5}", p[0], p[1]);
            }
        }
        None => {
            let _ = writeln!(s, "tone curve: none");
        }
    }
    s
}
