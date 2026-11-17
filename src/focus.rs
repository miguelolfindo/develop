//! Focus: how sharp the sharpest part of a photo is, measured on the full-size rendered picture.
//!
//! The picture is lightly smoothed (to tame noise) and split into square tiles. In each tile,
//! edges are found where brightness rises clearly above the tile's own noise, and each edge's
//! width is measured as the distance over which it rises from 10% to 90% of its height (times
//! 1.25, the full width of an even ramp). A tile's value is the 10th percentile of its edge widths,
//! and it needs edges on at least 0.1% of its pixels, so a few specks or hot pixels do not count.
//! The best tile is the sharpest part of the photo: a sharp subject counts even on a soft
//! background, and working at full size catches slightly missed focus that a small preview hides.
//! About 2.7 px is the sharpest this can read (a perfect edge after the smoothing).

use serde_json::{json, Value};
use std::path::Path;

/// Edges must rise at least this much (0-255) to count, well above the noise.
const MIN_EDGE: f32 = 24.0;
/// How far either side of an edge its height is looked for, in pixels.
const REACH: usize = 16;
/// A tile needs this many edges in a direction to be judged in it.
const MIN_EDGES: usize = 30;
/// Side of the 100% crop of the sharpest tile.
const CROP: u32 = 512;

struct Tile {
    x: u32,
    y: u32,
    /// Edge width in pixels: across (from left-right steps) and down (from up-down steps).
    across: f32,
    down: f32,
}

impl Tile {
    fn width(&self) -> f32 {
        self.across.min(self.down)
    }
}

/// Measure `rgb` (sRGB-encoded 16-bit, `width` x `height`) and write the report as JSON to
/// `path`, and a 100% crop of the sharpest tile as a JPEG beside it (`path` + ".jpg").
pub fn write(path: &Path, rgb: &[u16], width: u32, height: u32) -> Result<(), String> {
    let (report, crop) = measure(rgb, width, height);
    std::fs::write(path, serde_json::to_string(&report).unwrap_or_default()).map_err(|e| e.to_string())?;
    let jpg = crop_path(path);
    let _ = std::fs::remove_file(&jpg);
    if let Some((x, y, w, h)) = crop {
        let mut pixels = Vec::with_capacity((w * h * 3) as usize);
        for row in y..y + h {
            let start = ((row * width + x) * 3) as usize;
            pixels.extend(rgb[start..start + (w * 3) as usize].iter().map(|v| ((*v as u32 * 255 + 32767) / 65535) as u8));
        }
        let file = std::fs::File::create(&jpg).map_err(|e| e.to_string())?;
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), 90);
        encoder.encode(&pixels, w, h, image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Where `write` puts the crop of the sharpest tile.
pub fn crop_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".jpg");
    name.into()
}

/// The report, and the crop of the sharpest tile (x, y, width, height) if any tile had detail.
fn measure(rgb: &[u16], width: u32, height: u32) -> (Value, Option<(u32, u32, u32, u32)>) {
    let (w, h) = (width as usize, height as usize);
    let luma: Vec<f32> = rgb
        .chunks_exact(3)
        .map(|p| (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 257.0)
        .collect();
    // Steps are measured on a lightly smoothed copy (1-2-1 in both directions), which cuts
    // pixel-to-pixel noise to about a third while widening a perfect edge only to about 2.7 px.
    let smooth = smooth121(&luma, w, h);
    let side = (width.max(height) / 16).max(64);
    let (nx, ny) = (width / side, height / side);
    let origins: Vec<(u32, u32)> = (0..ny).flat_map(|ty| (0..nx).map(move |tx| (tx * side, ty * side))).collect();

    use rayon::prelude::*;
    let tiles: Vec<Tile> = origins
        .par_iter()
        .filter_map(|&(x0, y0)| {
            let (x0, y0, s) = (x0 as usize, y0 as usize, side as usize);
            let across = edge_widths(&luma, &smooth, w, h, x0, y0, s, 1);
            let down = edge_widths(&luma, &smooth, w, h, x0, y0, s, w);
            if across.is_none() && down.is_none() {
                return None;
            }
            Some(Tile { x: x0 as u32, y: y0 as u32, across: across.unwrap_or(f32::INFINITY), down: down.unwrap_or(f32::INFINITY) })
        })
        .collect();

    let how = "Edge width in pixels of the full-size photo: about 2.7 is the sharpest this measures, crisp detail \
usually reads 2.5-3.5, and the bigger, the softer. sharpest_* is the sharpest part of the picture (the in-focus \
subject, if anything is in focus); median_edge_px is typical of the whole picture (high with a shallow depth of \
field). Judge focus by comparing frames of the same scene (bursts, neighbours): a frame clearly wider than its \
neighbours missed focus or moved. If across and down differ a lot, the softer direction may have motion blur or \
camera shake (or that tile simply has few edges that way). Sharpening, texture and clarity make edges steeper. \
If you can see images, look at the 100% crop of the sharpest area to confirm.";
    if tiles.is_empty() {
        return (json!({"size": [width, height], "tiles_measured": 0, "verdict": "no detail to judge (flat or very dark picture)", "how_to_read": how}), None);
    }
    let best = tiles.iter().min_by(|a, b| a.width().total_cmp(&b.width())).unwrap();
    let mut widths: Vec<f32> = tiles.iter().map(Tile::width).collect();
    widths.sort_by(f32::total_cmp);
    let median = widths[widths.len() / 2];
    let sharpest = best.width();
    // Bands from real photos: crisp detail from good lenses reads about 2.5-3.5 px at full size.
    let verdict = if sharpest <= 3.5 {
        "sharp"
    } else if sharpest <= 5.5 {
        "slightly soft"
    } else {
        "soft (missed focus, motion blur, or nothing with fine detail in focus)"
    };
    let centre = [round3((best.x + side / 2) as f32 / width as f32), round3((best.y + side / 2) as f32 / height as f32)];
    let cw = CROP.min(width);
    let ch = CROP.min(height);
    let cx = (best.x + side / 2).saturating_sub(cw / 2).min(width - cw);
    let cy = (best.y + side / 2).saturating_sub(ch / 2).min(height - ch);
    (
        json!({
            "size": [width, height],
            "verdict": verdict,
            "sharpest_edge_px": round2(sharpest),
            "sharpest_edge_across_px": round2(best.across),
            "sharpest_edge_down_px": round2(best.down),
            "sharpest_at": centre,
            "sharpest_at_how_to_read": "Centre of the sharpest area as fractions of the picture's width and height (0,0 = top left).",
            "median_edge_px": round2(median),
            "tiles_measured": tiles.len(),
            "tiles_total": nx * ny,
            "how_to_read": how,
        }),
        Some((cx, cy, cw, ch)),
    )
}

/// 1-2-1 smoothing across and then down (edges of the picture copied).
fn smooth121(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    use rayon::prelude::*;
    let mut across = src.to_vec();
    across.par_chunks_mut(w).zip(src.par_chunks(w)).for_each(|(out, row)| {
        for x in 1..w.saturating_sub(1) {
            out[x] = (row[x - 1] + 2.0 * row[x] + row[x + 1]) * 0.25;
        }
    });
    let mut down = across.clone();
    down.par_chunks_mut(w).enumerate().for_each(|(y, out)| {
        if y == 0 || y + 1 >= h {
            return;
        }
        for x in 0..w {
            out[x] = (across[(y - 1) * w + x] + 2.0 * across[y * w + x] + across[(y + 1) * w + x]) * 0.25;
        }
    });
    down
}

/// The width of the sharpest edges in a tile, in one direction (`step` 1 = across, `w` = down).
/// Edges are found where the brightness step is largest along that direction and clearly above
/// the tile's noise, and rise by at least `MIN_EDGE` (and well above the noise) within `REACH`
/// pixels either side. Each edge's width is the distance over which it rises from 10% to 90% of
/// its height, times 1.25 (the full width of an even ramp); an edge still rising at `REACH`
/// counts as very wide. The tile's value is the 10th percentile of its edge widths: its sharpest
/// edges, but not a single lucky pixel.
fn edge_widths(raw: &[f32], img: &[f32], w: usize, h: usize, x0: usize, y0: usize, s: usize, step: usize) -> Option<f32> {
    let (x1, y1) = ((x0 + s).min(w), (y0 + s).min(h));
    // Skip the outer 1% of the picture: its border can hold artefacts (the sensor's edge,
    // lens-correction fill) that look like perfectly sharp edges.
    let (mx, my) = ((w / 100).max(REACH + 2), (h / 100).max(REACH + 2));
    let (xa, xb, ya, yb) = (x0.max(mx), x1.min(w.saturating_sub(mx)), y0.max(my), y1.min(h.saturating_sub(my)));
    if xa >= xb || ya >= yb {
        return None;
    }
    // The tile's noise, from the unsmoothed picture: the median of |a - 2b + c| over neighbouring
    // pixels is about 1.65 times the pixel noise (in a busy, textured tile it is higher, which
    // only makes the tests stricter there). Smoothing leaves about 0.375 of it.
    let mut curv = Vec::with_capacity((xb - xa) * (yb - ya));
    for y in ya..yb {
        for x in xa..xb {
            let i = y * w + x;
            curv.push((raw[i + step] - 2.0 * raw[i] + raw[i - step]).abs());
        }
    }
    let mid = curv.len() / 2;
    curv.select_nth_unstable_by(mid, f32::total_cmp);
    let noise = curv[mid] / 1.65 * 0.375;
    let (min_step, min_height) = ((2.0 * noise).max(0.5), (15.0 * noise).max(MIN_EDGE));
    let too_wide = 2.0 * REACH as f32 * 1.25;
    let mut widths = Vec::new();
    for y in ya..yb {
        for x in xa..xb {
            let i = y * w + x;
            let g = (img[i + step] - img[i - step]) * 0.5;
            if g.abs() < min_step {
                continue;
            }
            // Only the steepest point of each edge.
            let before = (img[i] - img[i - 2 * step]).abs() * 0.5;
            let after = (img[i + 2 * step] - img[i]).abs() * 0.5;
            if g.abs() < before || g.abs() < after {
                continue;
            }
            // The profile through the edge, from its dark side to its light side.
            let at = |k: isize| -> f32 {
                let k = if g > 0.0 { k } else { -k };
                img[(i as isize + k * step as isize) as usize]
            };
            let r = REACH as isize;
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for k in -r..=r {
                lo = lo.min(at(k));
                hi = hi.max(at(k));
            }
            let height = hi - lo;
            if height < min_height {
                continue;
            }
            let (low, high) = (lo + 0.1 * height, hi - 0.1 * height);
            // The steepest point must be inside the rise, not at its foot or top (as at the edge of
            // a speck or a thin line).
            if at(0) <= low || at(0) >= high {
                continue;
            }
            // Where the profile last rises through 10% before the edge and first reaches 90%
            // after it, with linear interpolation between pixels.
            let mut start = None;
            for k in (-r..0).rev() {
                if at(k) <= low {
                    let (a, b) = (at(k), at(k + 1));
                    start = Some(k as f32 + if b > a { (low - a) / (b - a) } else { 0.0 });
                    break;
                }
            }
            let mut end = None;
            for k in 1..=r {
                if at(k) >= high {
                    let (a, b) = (at(k - 1), at(k));
                    end = Some((k - 1) as f32 + if b > a { (high - a) / (b - a) } else { 1.0 });
                    break;
                }
            }
            widths.push(match (start, end) {
                (Some(a), Some(b)) => ((b - a) * 1.25).min(too_wide),
                _ => too_wide,
            });
        }
    }
    // Enough edges to be real detail: a few isolated specks (hot pixels, noise peaks) also look
    // perfectly sharp, so a tile needs edges on at least 0.1% of its pixels.
    if widths.len() < MIN_EDGES.max(s * s / 1000) {
        return None;
    }
    let k = widths.len() / 10;
    widths.select_nth_unstable_by(k, f32::total_cmp);
    Some(widths[k])
}

fn round2(v: f32) -> f64 {
    (v as f64 * 100.0).round() / 100.0
}

fn round3(v: f32) -> f64 {
    (v as f64 * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grey picture with a vertical dark-to-light edge ramped over `ramp` pixels.
    fn edge(ramp: usize) -> (Vec<u16>, u32, u32) {
        let (w, h) = (1024usize, 1024usize);
        let mut rgb = Vec::with_capacity(w * h * 3);
        for _y in 0..h {
            for x in 0..w {
                let t = ((x as f32 - 520.0) / ramp as f32 + 0.5).clamp(0.0, 1.0);
                let v = (8000.0 + t * 50000.0) as u16;
                rgb.extend([v, v, v]);
            }
        }
        (rgb, w as u32, h as u32)
    }

    #[test]
    fn edge_width_follows_the_blur() {
        for ramp in [1usize, 4, 8] {
            let (rgb, w, h) = edge(ramp);
            let (report, crop) = measure(&rgb, w, h);
            let px = report["sharpest_edge_px"].as_f64().unwrap();
            // Smoothing widens a perfect edge to about 2.7 px and longer ramps a little.
            let expected = [(1, 2.75), (4, 4.3), (8, 8.1)].iter().find(|e| e.0 == ramp).unwrap().1;
            assert!((px - expected).abs() < 0.5, "ramp {ramp}: measured {px}");
            assert!(crop.is_some());
        }
    }

    #[test]
    fn flat_picture_has_nothing_to_judge() {
        let rgb = vec![30000u16; 512 * 512 * 3];
        let (report, crop) = measure(&rgb, 512, 512);
        assert_eq!(report["tiles_measured"], 0);
        assert!(crop.is_none());
    }
}
