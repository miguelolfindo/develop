//! Face fingerprints: which faces are the same person.
//!
//! Each face found by the face finder (skin.rs) is lined up from its five landmarks onto the
//! standard 112 x 112 pose and turned into 128 numbers by SFace (OpenCV's face recognition model,
//! `models/face_recognition_sface_2021dec.onnx`, Apache-2.0). Two faces of the same person point
//! the same way: their cosine similarity is high (OpenCV suggests 0.363 or more for "same"). The
//! fingerprints never leave this computer and are never written into the photos' files.

use std::sync::{Mutex, OnceLock};

use ort::session::Session;
use ort::value::Tensor;

const MODEL: &str = "face_recognition_sface_2021dec.onnx";
/// Faces at least this similar are grouped as the same person. OpenCV suggests 0.363, but two
/// different people at one table (perhaps related) measured 0.37-0.41, and the same person
/// across a burst 0.93-0.98, so grouping waits for clearly similar faces.
pub const SAME_PERSON_GROUP: f32 = 0.50;
/// Below the grouping line but this alike: shown as possible matches for the user to confirm.
/// (Measured: the same person 0.93-0.98; two different women 0.37-0.41.)
pub const MAYBE_SAME_PERSON: f32 = 0.35;
const SIDE: usize = 112;
/// Where the five landmarks sit in the 112 x 112 face SFace expects (the ArcFace template).
const TEMPLATE: [[f32; 2]; 5] = [[38.2946, 51.6963], [73.5318, 51.5014], [56.0252, 71.7366], [41.5493, 92.3655], [70.7299, 92.2041]];

static SESSION: OnceLock<Result<Mutex<Session>, String>> = OnceLock::new();

fn session() -> Result<&'static Mutex<Session>, String> {
    SESSION
        .get_or_init(|| {
            let path = crate::denoise::find_model(MODEL).ok_or_else(|| format!("The face recognition model ({MODEL}) was not found."))?;
            let environment = crate::denoise::environment()?;
            let mut builder = Session::builder(environment).map_err(|e| format!("ONNX Runtime: {e}"))?;
            builder.commit_from_file(&path).map(Mutex::new).map_err(|e| format!("ONNX Runtime: {e}"))
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// The similarity transform (scale, rotation, shift) taking `from` onto `to` in the least-squares
/// sense: returns (a, b, tx, ty) for x' = a x - b y + tx, y' = b x + a y + ty.
fn similarity(from: &[[f32; 2]; 5], to: &[[f32; 2]; 5]) -> (f32, f32, f32, f32) {
    let mean = |p: &[[f32; 2]; 5]| {
        let (sx, sy) = p.iter().fold((0.0, 0.0), |(x, y), q| (x + q[0], y + q[1]));
        (sx / 5.0, sy / 5.0)
    };
    let ((mx, my), (nx, ny)) = (mean(from), mean(to));
    let (mut num_a, mut num_b, mut den) = (0.0, 0.0, 0.0);
    for (f, t) in from.iter().zip(to) {
        let (xs, ys, xd, yd) = (f[0] - mx, f[1] - my, t[0] - nx, t[1] - ny);
        num_a += xs * xd + ys * yd;
        num_b += xs * yd - ys * xd;
        den += xs * xs + ys * ys;
    }
    let (a, b) = (num_a / den.max(1e-6), num_b / den.max(1e-6));
    (a, b, nx - a * mx + b * my, ny - b * mx - a * my)
}

/// The face, lined up onto the standard pose: 112 x 112 RGB, bilinear.
pub fn align(rgb: &[u8], width: usize, height: usize, landmarks: &[[f32; 2]; 5]) -> Vec<u8> {
    let (a, b, tx, ty) = similarity(landmarks, &TEMPLATE);
    let det = (a * a + b * b).max(1e-9);
    let mut out = vec![0u8; SIDE * SIDE * 3];
    for v in 0..SIDE {
        for u in 0..SIDE {
            // Back from the template to the picture.
            let (du, dv) = (u as f32 - tx, v as f32 - ty);
            let x = (a * du + b * dv) / det;
            let y = (-b * du + a * dv) / det;
            let (x0, y0) = (x.floor(), y.floor());
            let (fx, fy) = (x - x0, y - y0);
            for c in 0..3 {
                let px = |xx: f32, yy: f32| -> f32 {
                    let (xi, yi) = (xx.clamp(0.0, (width - 1) as f32) as usize, yy.clamp(0.0, (height - 1) as f32) as usize);
                    rgb[(yi * width + xi) * 3 + c] as f32
                };
                let value = px(x0, y0) * (1.0 - fx) * (1.0 - fy) + px(x0 + 1.0, y0) * fx * (1.0 - fy) + px(x0, y0 + 1.0) * (1.0 - fx) * fy + px(x0 + 1.0, y0 + 1.0) * fx * fy;
                out[(v * SIDE + u) * 3 + c] = value.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// The face's fingerprint: 128 numbers, of length 1.
pub fn fingerprint(rgb: &[u8], width: usize, height: usize, landmarks: &[[f32; 2]; 5]) -> Result<Vec<f32>, String> {
    let face = align(rgb, width, height, landmarks);
    // RGB planes, 0-255, as OpenCV's FaceRecognizerSF feeds it.
    let mut input = vec![0f32; 3 * SIDE * SIDE];
    for i in 0..SIDE * SIDE {
        for c in 0..3 {
            input[c * SIDE * SIDE + i] = face[i * 3 + c] as f32;
        }
    }
    let tensor = Tensor::from_array(([1usize, 3, SIDE, SIDE], input.into_boxed_slice())).map_err(|e| e.to_string())?;
    let mut session = session()?.lock().map_err(|_| "The face recognition model is unavailable.".to_owned())?;
    let outputs = session.run(ort::inputs![tensor]).map_err(|e| format!("Face recognition failed: {e}"))?;
    let (_, data) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
    let norm = data.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-9);
    Ok(data.iter().map(|v| v / norm).collect())
}

/// How alike two fingerprints are: 1 the same, around 0 unrelated.
pub fn similarity_of(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similarity_transform_recovers_scale_rotation_and_shift() {
        // The template scaled by 2, turned 30 degrees and moved.
        let (s, r) = (2.0f32, 30f32.to_radians());
        let mut moved = [[0.0f32; 2]; 5];
        for (m, t) in moved.iter_mut().zip(&TEMPLATE) {
            *m = [s * (r.cos() * t[0] - r.sin() * t[1]) + 100.0, s * (r.sin() * t[0] + r.cos() * t[1]) + 50.0];
        }
        let (a, b, tx, ty) = similarity(&moved, &TEMPLATE);
        for (m, t) in moved.iter().zip(&TEMPLATE) {
            let (x, y) = (a * m[0] - b * m[1] + tx, b * m[0] + a * m[1] + ty);
            assert!((x - t[0]).abs() < 0.01 && (y - t[1]).abs() < 0.01, "{x},{y} vs {t:?}");
        }
    }
}
