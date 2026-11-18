//! Skin tone: how far the skin in a photo sits from the skin line, for judging white balance
//! where a photo has nothing grey or white to go by (firelight, tungsten, sunsets).
//!
//! Faces are found with YuNet (OpenCV's face detector, `models/face_detection_yunet_2023mar.onnx`,
//! MIT licence), run on the processor. Inside each face, the skin-coloured pixels of the cheeks
//! and forehead are taken, and their hue is measured on the vectorscope: skin of every complexion
//! lies close to the line 123 degrees anticlockwise from +Cb (the amber line on Develop's
//! vectorscope). A hue above the line is more yellow or orange (too warm), below it more pink or
//! magenta. No face, no reading: colour alone cannot tell skin from wood or flame in warm light.

use std::sync::{Mutex, OnceLock};

use ort::session::Session;
use ort::value::Tensor;
use serde_json::{json, Value};

const MODEL: &str = "face_detection_yunet_2023mar.onnx";
/// The skin line, degrees anticlockwise from +Cb.
pub const SKIN_LINE: f32 = 123.0;
/// The picture is scaled to this long side for finding faces.
const DETECT_SIDE: usize = 640;
const MIN_SCORE: f32 = 0.6;
/// Natural skin saturation (chroma / luma) at FACE_TARGET, from a firelit photo the user judged.
pub const SKIN_SATURATION: f32 = 0.36;
/// Faces are judged at this brightness (linear, 20%): measure skin colour after setting exposure
/// so the face sits here, so colour readings are comparable between photos.
pub const FACE_TARGET: f32 = 0.20;

static SESSION: OnceLock<Result<Mutex<Session>, String>> = OnceLock::new();

fn session() -> Result<&'static Mutex<Session>, String> {
    SESSION
        .get_or_init(|| {
            let path = crate::denoise::find_model(MODEL).ok_or_else(|| format!("The face model ({MODEL}) was not found."))?;
            let environment = crate::denoise::environment()?;
            let mut builder = Session::builder(environment).map_err(|e| format!("ONNX Runtime: {e}"))?;
            builder.commit_from_file(&path).map(Mutex::new).map_err(|e| format!("ONNX Runtime: {e}"))
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// A face: box in picture pixels, and the detector's confidence.
#[derive(Clone, Copy, Debug)]
pub struct Face {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
    /// Right eye, left eye, nose tip, right and left mouth corners (as seen in the picture),
    /// in picture pixels: for lining the face up (see faces.rs).
    pub landmarks: [[f32; 2]; 5],
}

fn iou(a: &Face, b: &Face) -> f32 {
    let (x1, y1) = (a.x.max(b.x), a.y.max(b.y));
    let (x2, y2) = ((a.x + a.w).min(b.x + b.w), (a.y + a.h).min(b.y + b.h));
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    inter / (a.w * a.h + b.w * b.h - inter).max(1e-6)
}

/// Find faces in an 8-bit RGB picture.
pub fn faces(rgb: &[u8], width: usize, height: usize) -> Result<Vec<Face>, String> {
    let scale = DETECT_SIDE as f32 / width.max(height) as f32;
    let (sw, sh) = (((width as f32 * scale).round() as usize).max(1), ((height as f32 * scale).round() as usize).max(1));
    // The network takes a fixed 640 x 640 picture; the rest is padded with black.
    let (pw, ph) = (DETECT_SIDE, DETECT_SIDE);
    let mut input = vec![0f32; 3 * pw * ph];
    for y in 0..sh {
        let sy = ((y as f32 + 0.5) / scale) as usize;
        for x in 0..sw {
            let sx = ((x as f32 + 0.5) / scale) as usize;
            let i = (sy.min(height - 1) * width + sx.min(width - 1)) * 3;
            // Blue, green, red planes, 0-255, as OpenCV feeds it.
            input[y * pw + x] = rgb[i + 2] as f32;
            input[pw * ph + y * pw + x] = rgb[i + 1] as f32;
            input[2 * pw * ph + y * pw + x] = rgb[i] as f32;
        }
    }
    let mut session = session()?.lock().map_err(|_| "The face finder is unavailable.".to_owned())?;
    let tensor = Tensor::from_array(([1usize, 3, ph, pw], input.into_boxed_slice())).map_err(|e| e.to_string())?;
    let outputs = session.run(ort::inputs![tensor]).map_err(|e| format!("Finding faces failed: {e}"))?;
    let mut found = Vec::new();
    for stride in [8usize, 16, 32] {
        let get = |name: String| -> Result<Vec<f32>, String> {
            let value = outputs.get(&name).ok_or_else(|| format!("The face model has no output {name}"))?;
            Ok(value.try_extract_tensor::<f32>().map_err(|e| e.to_string())?.1.to_vec())
        };
        let (cls, obj, bbox) = (get(format!("cls_{stride}"))?, get(format!("obj_{stride}"))?, get(format!("bbox_{stride}"))?);
        let kps = get(format!("kps_{stride}")).unwrap_or_default();
        let cols = pw / stride;
        for (idx, (&c, &o)) in cls.iter().zip(&obj).enumerate() {
            let score = (c.clamp(0.0, 1.0) * o.clamp(0.0, 1.0)).sqrt();
            if score < MIN_SCORE || idx * 4 + 3 >= bbox.len() {
                continue;
            }
            let (row, col) = ((idx / cols) as f32, (idx % cols) as f32);
            let s = stride as f32;
            let cx = (col + bbox[idx * 4]) * s;
            let cy = (row + bbox[idx * 4 + 1]) * s;
            let w = bbox[idx * 4 + 2].exp() * s;
            let h = bbox[idx * 4 + 3].exp() * s;
            let mut landmarks = [[0.0f32; 2]; 5];
            if kps.len() >= (idx + 1) * 10 {
                for (k, l) in landmarks.iter_mut().enumerate() {
                    *l = [(col + kps[idx * 10 + 2 * k]) * s / scale, (row + kps[idx * 10 + 2 * k + 1]) * s / scale];
                }
            }
            found.push(Face { x: (cx - w / 2.0) / scale, y: (cy - h / 2.0) / scale, w: w / scale, h: h / scale, score, landmarks });
        }
    }
    // Keep the best of overlapping boxes.
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Face> = Vec::new();
    for f in found {
        if kept.iter().all(|k| iou(k, &f) < 0.3) {
            kept.push(f);
        }
    }
    Ok(kept)
}

/// Hue (degrees anticlockwise from +Cb), chroma and luma of an sRGB pixel (0-255), in the
/// Rec.709 colour difference that Develop's vectorscope plots, so the numbers match its skin line.
fn ycc(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let cb = (b - y) / 1.8556;
    let cr = (r - y) / 1.5748;
    (cr.atan2(cb).to_degrees().rem_euclid(360.0), cb.hypot(cr), y)
}

fn median(v: &mut [f32]) -> f32 {
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// Skin of the faces in a picture, as numbers.
#[derive(Clone, Copy, Debug)]
pub struct Reading {
    /// Faces with enough skin to measure.
    pub faces: usize,
    /// Median hue, degrees anticlockwise from +Cb, and its distance from the skin line.
    pub hue: f32,
    pub off: f32,
    /// Median chroma / luma of the skin.
    pub saturation: f32,
    /// Median face brightness as linear reflectance (0-1).
    pub brightness: f32,
}

/// Why there is no reading.
pub enum NoReading {
    NoFace,
    TooSmall(usize),
    /// Faces found, but too dark to measure: how many, and their brightness (linear, 0-1), so
    /// the picture can be brightened for measuring.
    TooDark(usize, f32),
    Failed(String),
}

fn linear(encoded_0_255: f32) -> f32 {
    let e = encoded_0_255 / 255.0;
    if e <= 0.04045 { e / 12.92 } else { ((e + 0.055) / 1.055).powf(2.4) }
}

/// Find the faces in an 8-bit RGB picture and measure their skin.
pub fn read(rgb: &[u8], width: usize, height: usize) -> Result<Reading, NoReading> {
    let found = faces(rgb, width, height).map_err(NoReading::Failed)?;
    let (mut hues, mut chromas, mut lumas, mut used) = (Vec::new(), Vec::new(), Vec::new(), 0usize);
    // Every pixel of the faces' middles, to tell dark faces from small ones.
    let mut all_lumas = Vec::new();
    for f in found.iter().filter(|f| f.w >= 16.0 && f.h >= 16.0) {
        // Cheeks and forehead: the middle of the box, away from hair, background and the edges.
        let (x0, x1) = ((f.x + 0.2 * f.w).max(0.0) as usize, ((f.x + 0.8 * f.w) as usize).min(width));
        let (y0, y1) = ((f.y + 0.2 * f.h).max(0.0) as usize, ((f.y + 0.85 * f.h) as usize).min(height));
        let before = hues.len();
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * width + x) * 3;
                let (h, c, l) = ycc(rgb[i] as f32, rgb[i + 1] as f32, rgb[i + 2] as f32);
                all_lumas.push(l);
                // Skin-coloured in any light (pink to orange-yellow), not grey, not blown or black.
                if (75.0..=185.0).contains(&h) && c >= 4.0 && (35.0..=240.0).contains(&l) {
                    hues.push(h);
                    chromas.push(c / l.max(1.0));
                    lumas.push(l);
                }
            }
        }
        if hues.len() > before + 50 {
            used += 1;
        }
    }
    if found.is_empty() {
        return Err(NoReading::NoFace);
    }
    if hues.len() < 200 {
        if all_lumas.len() >= 200 {
            let brightness = linear(median(&mut all_lumas));
            if brightness < 0.08 {
                return Err(NoReading::TooDark(found.len(), brightness));
            }
        }
        return Err(NoReading::TooSmall(found.len()));
    }
    let hue = median(&mut hues);
    let saturation = median(&mut chromas);
    // Face brightness as linear reflectance: the encoded luma through the sRGB curve.
    let brightness = linear(median(&mut lumas));
    Ok(Reading { faces: used, hue, off: hue - SKIN_LINE, saturation, brightness })
}

/// The exposure change, in stops, that would put the faces at FACE_TARGET.
pub fn stops_to_target(r: &Reading) -> f32 {
    (FACE_TARGET / r.brightness.max(1e-4)).log2()
}

/// The skin reading for an 8-bit RGB picture, for check_photo.
pub fn measure(rgb: &[u8], width: usize, height: usize) -> Value {
    let r = match read(rgb, width, height) {
        Ok(r) => r,
        Err(NoReading::Failed(e)) => return json!({"faces": 0, "note": e}),
        Err(NoReading::NoFace) => return json!({"faces": 0, "note": "No face found, so there is no skin to judge white balance by."}),
        Err(NoReading::TooSmall(n)) => return json!({"faces": n, "note": "Faces are too small to judge skin tone."}),
        Err(NoReading::TooDark(n, b)) => {
            // check_photo brightens the picture for measuring and asks again (see mcp.rs).
            return json!({
                "faces": n,
                "note": "Faces are too dark to judge skin tone at this brightness.",
                "face_brightness_percent": (b * 1000.0).round() / 10.0,
                "exposure_to_target_stops": ((FACE_TARGET / b.max(1e-4)).log2() * 100.0).round() / 100.0,
            });
        }
    };
    let (hue, saturation, linear, used) = (r.hue, r.saturation, r.brightness, r.faces);
    let to_target = stops_to_target(&r);
    let off = hue - SKIN_LINE;
    // Skin colour is only comparable once the face is at the target brightness: first exposure,
    // then white balance. Saturation is the main signal (orange light pushes skin along the skin
    // line, making it stronger rather than a different hue); hue guards against going pink.
    let reading = if to_target.abs() > 0.3 {
        "the faces could not be measured at 20% brightness, so this reading is less reliable".to_owned()
    } else if off < -6.0 {
        "skin is pinker or cooler than natural: raise temperature (or lower tint if it looks magenta)".to_owned()
    } else if saturation > SKIN_SATURATION + 0.05 {
        "skin is too strongly coloured: the picture is too warm (lower temperature) or too saturated".to_owned()
    } else if saturation < SKIN_SATURATION - 0.08 && off < 0.0 {
        "skin is pale and slightly cool: raise temperature a little".to_owned()
    } else {
        "skin looks natural".to_owned()
    };
    json!({
        "faces": used,
        "hue_deg": (hue * 10.0).round() / 10.0,
        "off_skin_line_deg": (off * 10.0).round() / 10.0,
        "saturation": (saturation * 100.0).round() / 100.0,
        "face_brightness_percent": (linear * 1000.0).round() / 10.0,
        "exposure_to_target_stops": (to_target * 100.0).round() / 100.0,
        "reading": reading,
        "how_to_read": "Skin is measured as if the faces were at 20% brightness (Develop shifts exposure for the measurement only; do not change exposure for this, and leave the photo's exposure as it is). saturation is skin colour strength at that brightness: about 0.36 looks natural; much higher means the picture is too warm (orange light makes skin stronger rather than a different hue), so lower temperature, measuring again, until it is near 0.36. off_skin_line_deg is the hue against the skin line (123 degrees on Develop's vectorscope): below about -6 the skin is turning pink or cool, so stop or go back up. Change only temperature and tint for this. Use it when neutral_areas has little to measure (warm light, fire, tungsten); when both measure something and disagree, the light is mixed: favour skin for photos of people",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_hues_fall_on_the_expected_side_of_the_line() {
        // A typical skin tone sits near the line; an orange-lit one well above it; a pink one below.
        let (natural, _, _) = ycc(224.0, 172.0, 145.0);
        let (orange, _, _) = ycc(240.0, 150.0, 70.0);
        let (pink, _, _) = ycc(230.0, 160.0, 170.0);
        assert!((natural - SKIN_LINE).abs() < 8.0, "natural {natural}");
        assert!(orange - SKIN_LINE > 10.0, "orange {orange}");
        assert!(pink - SKIN_LINE < -8.0, "pink {pink}");
    }
}
