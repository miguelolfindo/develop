//! AI raw denoise: the RawNIND UtNet2 Bayer model (darktable-ai's ONNX export, GPL-3.0),
//! run through ONNX Runtime. It works on the sensor data before demosaicing and returns a
//! clean, already-demosaiced picture in the camera's own colour, so it takes the place of
//! the demosaic step in stage 1.
//!
//! Model contract: input `[1, 4, 512, 512]` = R, G1, G2, B planes of an RGGB mosaic at half
//! resolution, black = 0 and sensor saturation = 1, no white balance. Output
//! `[1, 3, 1024, 1024]` = camera RGB at full resolution with an arbitrary overall gain,
//! which is matched back to the input's brightness here.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use ort::session::Session;
use ort::value::Tensor;

const TILE: usize = 512;
/// Tiles overlap by this many (half-resolution) pixels and are cross-faded across it.
const OVERLAP: usize = 48;
const MODEL_FILE: &str = "rawnind_bayer.onnx";

/// Progress for the UI: tiles done in the high 16 bits, total tiles in the low 16.
pub static PROGRESS: AtomicU32 = AtomicU32::new(0);

pub fn progress() -> Option<(u32, u32)> {
    let p = PROGRESS.load(Ordering::Relaxed);
    (p & 0xffff != 0).then_some((p >> 16, p & 0xffff))
}

struct Engine {
    session: Mutex<Session>,
    device: &'static str,
    _environment: ort::environment::Environment,
}

static ENGINE: OnceLock<Result<Engine, String>> = OnceLock::new();

/// The model file: `models/rawnind_bayer.onnx` beside the program or in a folder above it.
pub fn model_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().skip(1).take(4).flat_map(|dir| [dir.join("models").join(MODEL_FILE), dir.join(MODEL_FILE), dir.join("Resources").join("models").join(MODEL_FILE)]).find(|p| p.is_file())
}

fn engine() -> Result<&'static Engine, String> {
    ENGINE
        .get_or_init(|| {
            let model = model_path().ok_or_else(|| format!("The denoise model ({MODEL_FILE}) was not found."))?;
            let text = |e: ort::Error| format!("ONNX Runtime: {e}");
            let environment = ort::init().with_name("sony_raw_processor").build().map_err(text)?;
            // GPU first (DirectML works on any DirectX 12 card); the processor if that fails.
            let gpu = || -> Result<Session, String> {
                let mut builder = Session::builder(&environment)
                    .map_err(text)?
                    .with_parallel_execution(false)
                    .map_err(|e| e.to_string())?
                    .with_memory_pattern(false)
                    .map_err(|e| e.to_string())?
                    .with_execution_providers([gpu_provider()])
                    .map_err(|e| e.to_string())?;
                builder.commit_from_file(&model).map_err(text)
            };
            let (session, device) = match gpu() {
                Ok(session) => (session, "GPU"),
                Err(_) => {
                    let mut builder = Session::builder(&environment).map_err(text)?;
                    (builder.commit_from_file(&model).map_err(text)?, "CPU")
                }
            };
            Ok(Engine { session: Mutex::new(session), device, _environment: environment })
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// The GPU backend for this platform: DirectML on Windows (any DirectX 12 card), CoreML on
/// macOS. Elsewhere there is none and the model runs on the processor.
#[cfg(windows)]
fn gpu_provider() -> ort::ep::ExecutionProviderDispatch {
    ort::ep::DirectML::default().build().error_on_failure()
}

#[cfg(target_os = "macos")]
fn gpu_provider() -> ort::ep::ExecutionProviderDispatch {
    ort::ep::CoreML::default().build().error_on_failure()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn gpu_provider() -> ort::ep::ExecutionProviderDispatch {
    ort::ep::CPU::default().build()
}

pub struct Denoised {
    /// Interleaved RGB, `2 * width` by `2 * height` of the planes given.
    pub rgb: Vec<f32>,
    /// A line for the info panel: device, time, tiles.
    pub note: String,
}

/// Tile start positions covering `n` with tiles of `TILE`, overlapping by at least `OVERLAP`.
fn starts(n: usize) -> Vec<usize> {
    if n <= TILE {
        return vec![0];
    }
    let stride = TILE - OVERLAP;
    let count = (n - TILE).div_ceil(stride) + 1;
    (0..count).map(|i| (i * stride).min(n - TILE)).collect()
}

fn mirror(i: usize, n: usize) -> usize {
    if i < n {
        i
    } else {
        (2 * n).saturating_sub(i + 2).min(n - 1)
    }
}

/// Denoise four half-resolution Bayer planes (R, G1, G2, B), each `width * height`.
pub fn run(planes: &[Vec<f32>; 4], width: usize, height: usize) -> Result<Denoised, String> {
    let started = std::time::Instant::now();
    let engine = engine()?;
    let mut session = engine.session.lock().map_err(|_| "The denoiser is unavailable.".to_owned())?;

    let (out_w, out_h) = (width * 2, height * 2);
    let mut sum = vec![0f32; out_w * out_h * 3];
    let mut weight = vec![0f32; out_w * out_h];
    let (xs, ys) = (starts(width), starts(height));
    let total = (xs.len() * ys.len()) as u32;
    let mut done = 0u32;
    PROGRESS.store(total.min(0xffff), Ordering::Relaxed);

    // Cross-fade weight along one axis of a tile, in output pixels.
    let ramp: Vec<f32> = (0..TILE * 2).map(|i| ((i.min(TILE * 2 - 1 - i) + 1) as f32 / (OVERLAP * 2) as f32).min(1.0)).collect();
    // For matching the model's overall gain to the input.
    let (mut input_total, mut output_total) = (0f64, 0f64);

    let result = (|| -> Result<(), String> {
        for &ty in &ys {
            for &tx in &xs {
                let mut input = vec![0f32; 4 * TILE * TILE];
                for (c, plane) in planes.iter().enumerate() {
                    let dst = &mut input[c * TILE * TILE..(c + 1) * TILE * TILE];
                    for y in 0..TILE {
                        let row = mirror(ty + y, height) * width;
                        for x in 0..TILE {
                            dst[y * TILE + x] = plane[row + mirror(tx + x, width)];
                        }
                    }
                }
                // Mean of the mosaic as RGB: R + (G1 + G2) / 2 + B.
                let plane_sum = |c: usize| input[c * TILE * TILE..(c + 1) * TILE * TILE].iter().map(|&v| v as f64).sum::<f64>();
                input_total += plane_sum(0) + 0.5 * (plane_sum(1) + plane_sum(2)) + plane_sum(3);

                let tensor = Tensor::from_array(([1usize, 4, TILE, TILE], input.into_boxed_slice())).map_err(|e| e.to_string())?;
                let outputs = session.run(ort::inputs![tensor]).map_err(|e| format!("Denoise failed: {e}"))?;
                let (_, data) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
                let side = TILE * 2;
                if data.len() != 3 * side * side {
                    return Err("The denoise model returned an unexpected size.".into());
                }
                // Each output pixel stands for a quarter of an input sample's area.
                output_total += data.iter().map(|&v| v as f64).sum::<f64>() * 0.25;

                for y in 0..side.min(out_h - ty * 2) {
                    let oy = ty * 2 + y;
                    for x in 0..side.min(out_w - tx * 2) {
                        let ox = tx * 2 + x;
                        let w = ramp[y] * ramp[x];
                        let o = oy * out_w + ox;
                        weight[o] += w;
                        for c in 0..3 {
                            sum[o * 3 + c] += w * data[c * side * side + y * side + x];
                        }
                    }
                }
                done += 1;
                PROGRESS.store((done.min(0xffff) << 16) | total.min(0xffff), Ordering::Relaxed);
            }
        }
        Ok(())
    })();
    PROGRESS.store(0, Ordering::Relaxed);
    result?;

    let gain = if output_total > 1e-9 { (input_total / output_total) as f32 } else { 1.0 };
    use rayon::prelude::*;
    sum.par_chunks_mut(3).zip(weight.par_iter()).for_each(|(rgb, &w)| {
        let k = gain / w.max(1e-6);
        for v in rgb {
            *v = (*v * k).max(0.0);
        }
    });
    let note = format!("AI denoise: {:.1} s on the {} ({total} tiles)", started.elapsed().as_secs_f32(), engine.device);
    Ok(Denoised { rgb: sum, note })
}
