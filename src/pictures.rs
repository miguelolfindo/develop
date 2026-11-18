//! Pictures for the People view: photo thumbnails and face close-ups, read from the photos'
//! previews on a few background threads, only when they come into view.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui;

/// What to make from a photo: its thumbnail, or a close-up of the face in this box (fractions
/// of the picture, x, y, w, h).
#[derive(Clone, PartialEq, Eq, Hash)]
struct Want {
    path: PathBuf,
    /// The face box in thousandths, so it can be a key.
    face: Option<[u16; 4]>,
}

enum State {
    Pending,
    Ready(egui::TextureHandle),
    Failed,
}

pub struct Pictures {
    map: HashMap<Want, State>,
    jobs: Sender<Want>,
    done: Receiver<(Want, Option<egui::ColorImage>)>,
}

/// Longest side of a thumbnail, and of a face close-up, in pixels.
const THUMB: u32 = 480;
const CLOSE_UP: u32 = 640;

impl Pictures {
    pub fn new(ctx: &egui::Context) -> Self {
        let (jobs, job_rx) = mpsc::channel::<Want>();
        let (done_tx, done) = mpsc::channel();
        let job_rx = Arc::new(Mutex::new(job_rx));
        for _ in 0..3 {
            let (job_rx, done_tx, ctx) = (job_rx.clone(), done_tx.clone(), ctx.clone());
            std::thread::spawn(move || loop {
                let job = match job_rx.lock() {
                    Ok(rx) => rx.recv(),
                    Err(_) => return,
                };
                let Ok(want) = job else { return };
                let image = make(&want);
                if done_tx.send((want, image)).is_err() {
                    return;
                }
                ctx.request_repaint();
            });
        }
        Self { map: HashMap::new(), jobs, done }
    }

    /// Take in pictures finished since the last frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((want, image)) = self.done.try_recv() {
            let state = match image {
                Some(image) => State::Ready(ctx.load_texture(format!("people-{}-{:?}", want.path.display(), want.face), image, egui::TextureOptions::LINEAR)),
                None => State::Failed,
            };
            self.map.insert(want, state);
        }
    }

    fn get(&mut self, want: Want) -> Option<egui::TextureHandle> {
        match self.map.get(&want) {
            Some(State::Ready(t)) => Some(t.clone()),
            Some(_) => None,
            None => {
                let state = if self.jobs.send(want.clone()).is_ok() { State::Pending } else { State::Failed };
                self.map.insert(want, state);
                None
            }
        }
    }

    /// A photo's thumbnail (asked for if not made yet).
    pub fn thumb(&mut self, path: &Path) -> Option<egui::TextureHandle> {
        self.get(Want { path: path.to_path_buf(), face: None })
    }

    /// A close-up of the face in `area` (asked for if not made yet).
    pub fn face(&mut self, path: &Path, area: [f32; 4]) -> Option<egui::TextureHandle> {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 1000.0).round() as u16;
        self.get(Want { path: path.to_path_buf(), face: Some([q(area[0]), q(area[1]), q(area[2]), q(area[3])]) })
    }
}

fn make(want: &Want) -> Option<egui::ColorImage> {
    let picture = match want.face {
        None => crate::filmstrip::preview(&want.path, THUMB)?,
        Some(face) => {
            let pic = crate::filmstrip::preview(&want.path, 4000)?;
            let (w, h) = (pic.width() as f32, pic.height() as f32);
            let [x, y, fw, fh] = face.map(|v| v as f32 / 1000.0);
            // A square around the face with room for hair and chin.
            let side = (fw * w).max(fh * h) * 2.0;
            let (cx, cy) = ((x + fw / 2.0) * w, (y + fh / 2.0) * h);
            let x0 = (cx - side / 2.0).clamp(0.0, (w - 1.0).max(0.0));
            let y0 = (cy - side / 2.0).clamp(0.0, (h - 1.0).max(0.0));
            let s = side.min(w - x0).min(h - y0).max(1.0);
            let crop = image::imageops::crop_imm(&pic, x0 as u32, y0 as u32, s as u32, s as u32).to_image();
            let out = (s as u32).min(CLOSE_UP).max(1);
            image::imageops::resize(&crop, out, out, image::imageops::FilterType::CatmullRom)
        }
    };
    Some(egui::ColorImage::from_rgb([picture.width() as usize, picture.height() as usize], picture.as_raw()))
}
