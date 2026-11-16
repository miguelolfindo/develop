//! Stage 1 (runs once per file, off the UI thread): decode the file, normalise black/white
//! levels, demosaic, and build the RGBA16F mip chain that gets uploaded to the GPU.
//!
//! The result is *camera-space* linear RGB. White balance and the camera -> sRGB matrix are
//! applied later in the shader, which is what lets the Kelvin slider work without re-decoding.

use std::path::Path;

use rayon::prelude::*;

use crate::color::Mat3;

pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    /// RGBA, IEEE half-float bits, row-major.
    pub rgba: Vec<u16>,
}

pub struct DecodedImage {
    /// Level 0 first. If the sensor image is larger than the GPU's texture limit the chain
    /// simply starts at the first level that fits.
    pub mips: Vec<MipLevel>,
    /// Size of the demosaiced sensor image before any GPU-limit reduction.
    pub full_width: u32,
    pub full_height: u32,
    pub xyz_to_cam: Mat3,
    pub as_shot_wb: [f32; 3],
    /// Clockwise quarter turns needed to display the picture upright.
    pub quarter_turns: u8,
    pub camera: String,
    /// Shot details (exposure, lens, date...) to write into exported files.
    pub photo: crate::export::PhotoMeta,
    /// Whether AI denoise was applied, and a line about it (or about why it could not be).
    pub denoised: bool,
    pub denoise_note: Option<String>,
    /// What the texture holds: 0 = demosaiced Bayer raw, 1 = linear raw that was already RGB
    /// (linear DNG: iPhone ProRAW, Samsung Expert RAW...), 2 = a finished picture (JPEG, PNG,
    /// TIFF) turned back into linear light.
    pub input_mode: u8,
    /// DNG BaselineExposure in stops, when the file gives one.
    pub baseline_exposure: Option<f32>,
    /// Exposure (stops) to add for how the camera recorded this raw: Fujifilm underexposes
    /// DR200 / DR400 shots by a further stop or two, which Lightroom brings back.
    pub exposure_compensation: f32,
    /// Bits per sample in the file.
    pub bits: u8,
    /// Camera and lens, and the camera's own lens corrections (if the file has them), for lens correction.
    pub exif: crate::lens::Exif,
    pub embedded: Option<crate::lens::Embedded>,
}

/// What the decoder produced.
enum Pixels {
    Bayer(Bayer),
    /// Interleaved RGB of the visible area, black = 0, saturation = 1.
    Rgb { data: Vec<f32>, width: usize, height: usize },
}

/// Normalised Bayer mosaic: 0.0 = black level, 1.0 = sensor saturation.
struct Bayer {
    width: usize,
    height: usize,
    data: Vec<f32>,
    /// Colour (0 = R, 1 = G, 2 = B) at [row & 1][col & 1].
    cfa: [[u8; 2]; 2],
    /// Fujifilm X-Trans: colour at [row % 6][col % 6] instead of `cfa`.
    xtrans: Option<[[u8; 6]; 6]>,
    /// Visible area: x, y, width, height.
    crop: (usize, usize, usize, usize),
}

struct Meta {
    bits: u8,
    /// Camera and lens, for lens-profile matching.
    exif: crate::lens::Exif,
    xyz_to_cam: Mat3,
    as_shot_wb: [f32; 3],
    quarter_turns: u8,
    camera: String,
    photo: crate::export::PhotoMeta,
}

/// The only function that touches the decoder crate.
fn decode(path: &Path) -> Result<(Pixels, Meta), String> {
    use rawler::imgop::xyz::Illuminant;
    use rawler::{Orientation, RawImageData};

    let failed = |e: rawler::RawlerError| format!("Could not decode file: {e}");
    let source = rawler::rawsource::RawSource::new(path).map_err(|e| format!("Could not read file: {e}"))?;
    let decoder = rawler::get_decoder(&source).map_err(failed)?;
    let params = rawler::decoders::RawDecodeParams::default();
    let raw = decoder.raw_image(&source, &params, false).map_err(failed)?;
    // Shot details for export. Missing metadata is not a reason to refuse the picture.
    let photo = match decoder.raw_metadata(&source, &params) {
        Ok(md) => {
            let e = &md.exif;
            let r = |v: &rawler::formats::tiff::Rational| (v.n, v.d);
            crate::export::PhotoMeta {
                make: md.make.clone(),
                model: md.model.clone(),
                lens_model: e.lens_model.clone(),
                lens_make: e.lens_make.clone(),
                exposure_time: e.exposure_time.as_ref().map(r),
                fnumber: e.fnumber.as_ref().map(r),
                iso: e.iso_speed.or(e.recommended_exposure_index).or(e.iso_speed_ratings.map(u32::from)),
                focal_length: e.focal_length.as_ref().map(r),
                exposure_bias: e.exposure_bias.as_ref().map(|v| (v.n, v.d)),
                date_time_original: e.date_time_original.clone().or_else(|| e.create_date.clone()),
                artist: e.artist.clone(),
                copyright: e.copyright.clone(),
                gps: e.gps.as_ref().and_then(|g| {
                    let dms = |v: &[rawler::formats::tiff::Rational; 3]| [r(&v[0]), r(&v[1]), r(&v[2])];
                    Some(crate::export::Gps {
                        latitude_ref: g.gps_latitude_ref.clone()?,
                        latitude: dms(g.gps_latitude.as_ref()?),
                        longitude_ref: g.gps_longitude_ref.clone()?,
                        longitude: dms(g.gps_longitude.as_ref()?),
                        altitude: g.gps_altitude.as_ref().map(|a| (g.gps_altitude_ref.unwrap_or(0), r(a))),
                    })
                }),
            }
        }
        Err(_) => crate::export::PhotoMeta::default(),
    };
    // The decoder leaves some of these out for some cameras (Nikon NEF: the lens); the
    // file's own EXIF block has them.
    let mut photo = photo;
    if photo.lens_model.is_none() || photo.lens_make.is_none() {
        if let Some(info) = crate::filmstrip::tiff_info(path) {
            photo.lens_model = photo.lens_model.or(info.lens_model);
            photo.lens_make = photo.lens_make.or(info.lens_make);
        }
    }
    let (width, height) = (raw.width, raw.height);
    if width < 8 || height < 8 {
        return Err("Raw image is too small.".into());
    }
    let crop = match raw.crop_area.or(raw.active_area) {
        Some(r) if r.d.w >= 8 && r.d.h >= 8 && r.p.x + r.d.w <= width && r.p.y + r.d.h <= height => {
            (r.p.x, r.p.y, r.d.w, r.d.h)
        }
        _ => (0, 0, width, height),
    };
    let quarter_turns = match raw.orientation {
        Orientation::Rotate90 => 1,
        Orientation::Rotate180 => 2,
        Orientation::Rotate270 => 3,
        _ => 0,
    };
    let matrix = raw
        .color_matrix
        .get(&Illuminant::D65)
        .or_else(|| raw.color_matrix.values().next())
        .filter(|m| m.len() >= 9)
        .ok_or("The decoder has no colour matrix for this camera.")?;
    let ratio = |r: Option<(u32, u32)>| r.map_or(0.0, |(n, d)| if d > 0 { n as f32 / d as f32 } else { 0.0 });
    let exif = crate::lens::Exif {
        make: raw.clean_make.clone(),
        model: raw.clean_model.clone(),
        lens_make: photo.lens_make.clone().unwrap_or_default(),
        lens_model: photo.lens_model.clone().unwrap_or_default(),
        focal: ratio(photo.focal_length),
        fnumber: ratio(photo.fnumber),
    };
    let meta = Meta {
        exif,
        // The sensor's real depth, from its white level (the container is often 16-bit).
        bits: raw.whitelevel.as_vec().first().map_or(raw.bps as u8, |&w| if w > 1.5 { (w + 1.0).log2().ceil() as u8 } else { raw.bps as u8 }),
        xyz_to_cam: [[matrix[0], matrix[1], matrix[2]], [matrix[3], matrix[4], matrix[5]], [matrix[6], matrix[7], matrix[8]]],
        as_shot_wb: [raw.wb_coeffs[0], raw.wb_coeffs[1], raw.wb_coeffs[2]],
        quarter_turns,
        camera: format!("{} {}", raw.clean_make, raw.clean_model),
        photo,
    };

    // Linear raw (three colours per pixel already): no demosaic, just levels and crop.
    if raw.cpp == 3 {
        let (black, white) = (raw.blacklevel.as_vec(), raw.whitelevel.as_vec());
        let level = |v: &[f32], c: usize, fallback: f32| if v.is_empty() { fallback } else { v[c % v.len()] };
        let offset: [f32; 3] = std::array::from_fn(|c| level(&black, c, 0.0));
        let scale: [f32; 3] = std::array::from_fn(|c| 1.0 / (level(&white, c, 1.0) - offset[c]).max(1.0e-6));
        let (cx, cy, cw, ch) = crop;
        let mut data = vec![0f32; cw * ch * 3];
        let fill = |get: &(dyn Fn(usize) -> f32 + Sync), data: &mut [f32]| {
            data.par_chunks_mut(cw * 3).enumerate().for_each(|(y, row)| {
                let start = ((y + cy) * width + cx) * 3;
                for (i, v) in row.iter_mut().enumerate() {
                    *v = ((get(start + i) - offset[i % 3]) * scale[i % 3]).max(0.0);
                }
            });
        };
        // Lossy DNGs (Lightroom smart previews, some phone raws) store 8-bit values that go
        // through the file's LinearizationTable; the decoder leaves that to us.
        let table = crate::filmstrip::tiff_info(path)
            .and_then(|info| info.linearization)
            .filter(|table| level(&white, 0, 1.0) < table.len() as f32 && table.iter().any(|&v| v as usize >= table.len()));
        match (&raw.data, table) {
            (RawImageData::Integer(src), Some(table)) => {
                let top = table.iter().copied().max().unwrap_or(1).max(1) as f32;
                let last = table.len() - 1;
                data.par_chunks_mut(cw * 3).enumerate().for_each(|(y, row)| {
                    let start = ((y + cy) * width + cx) * 3;
                    for (i, v) in row.iter_mut().enumerate() {
                        *v = table[(src[start + i] as usize).min(last)] as f32 / top;
                    }
                });
            }
            (RawImageData::Integer(src), None) => fill(&|i| src[i] as f32, &mut data),
            (RawImageData::Float(src), _) => fill(&|i| src[i], &mut data),
        }
        return Ok((Pixels::Rgb { data, width: cw, height: ch }, meta));
    }
    if raw.cpp != 1 {
        return Err("This file is not a Bayer raw (unsupported sensor layout).".into());
    }

    let rawler::rawimage::RawPhotometricInterpretation::Cfa(cfa_config) = &raw.photometric else {
        return Err("This file is not a Bayer raw (unsupported sensor layout).".into());
    };
    let mut cfa = [[1u8; 2]; 2];
    for (r, row) in cfa.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = cfa_config.cfa.color_at(r, c) as u8;
        }
    }
    let xtrans = match (cfa_config.cfa.width, cfa_config.cfa.height) {
        (2, 2) => None,
        (6, 6) => {
            // rawler's DNG reader turns the DNG pattern (stored relative to the active area,
            // as the DNG specification says) the wrong way: it shifts it forward by the active
            // area's offset instead of back, which leaves a 6x6 X-Trans pattern off by twice
            // that offset (a 2x2 Bayer pattern is not affected). Undo that for DNG files.
            let (dy, dx) = match (crate::filmstrip::extension_of(path) == "dng", raw.active_area) {
                (true, Some(area)) => ((6 - (2 * area.p.y) % 6) % 6, (6 - (2 * area.p.x) % 6) % 6),
                _ => (0, 0),
            };
            let mut pattern = [[1u8; 6]; 6];
            for (r, row) in pattern.iter_mut().enumerate() {
                for (c, v) in row.iter_mut().enumerate() {
                    *v = cfa_config.cfa.color_at(r + dy, c + dx) as u8;
                }
            }
            Some(pattern)
        }
        _ => return Err("Unsupported colour filter array.".into()),
    };
    if cfa.iter().flatten().any(|c| *c > 2) || xtrans.is_some_and(|p| p.iter().flatten().any(|c| *c > 2)) {
        return Err("Unsupported colour filter array.".into());
    }

    let black = raw.blacklevel.as_bayer_array();
    let white = raw.whitelevel.as_bayer_array();
    let mut offset = [0f32; 4];
    let mut scale = [1f32; 4];
    for i in 0..4 {
        offset[i] = black[i];
        scale[i] = 1.0 / (white[i] - black[i]).max(1.0);
    }
    // X-Trans files give one black level per pattern site, all the same in practice: use the first.
    let uniform = xtrans.is_some();
    let norm = |y: usize, x: usize, v: f32| {
        let i = if uniform { 0 } else { (y & 1) * 2 + (x & 1) };
        ((v - offset[i]) * scale[i]).clamp(0.0, 1.0)
    };

    let mut data = vec![0f32; width * height];
    match &raw.data {
        RawImageData::Integer(src) => {
            data.par_chunks_mut(width).zip(src.par_chunks(width)).enumerate().for_each(|(y, (dst, src))| {
                for x in 0..width {
                    dst[x] = norm(y, x, src[x] as f32);
                }
            });
        }
        RawImageData::Float(src) => {
            data.par_chunks_mut(width).zip(src.par_chunks(width)).enumerate().for_each(|(y, (dst, src))| {
                for x in 0..width {
                    dst[x] = norm(y, x, src[x]);
                }
            });
        }
    }

    Ok((Pixels::Bayer(Bayer { width, height, data, cfa, xtrans, crop }), meta))
}

/// Malvar-He-Cutler high-quality linear demosaic (5x5), parallel over rows.
/// Returns interleaved RGB f32 for the crop area.
fn demosaic(b: &Bayer) -> Vec<f32> {
    let (w, h) = (b.width as isize, b.height as isize);
    let (cx, cy, cw, ch) = b.crop;
    let src = &b.data;
    // Mirror without repeating the edge sample, which keeps the CFA phase intact.
    let reflect = |i: isize, n: isize| -> usize {
        (if i < 0 {
            -i
        } else if i >= n {
            2 * (n - 1) - i
        } else {
            i
        }) as usize
    };

    let mut out = vec![0f32; cw * ch * 3];
    out.par_chunks_mut(cw * 3).enumerate().for_each(|(oy, row)| {
        let y = (oy + cy) as isize;
        let rows: [&[f32]; 5] = std::array::from_fn(|k| {
            let yy = reflect(y + k as isize - 2, h);
            &src[yy * b.width..(yy + 1) * b.width]
        });
        let cfa_row = b.cfa[(y & 1) as usize];

        for ox in 0..cw {
            let x = (ox + cx) as isize;
            let xs: [usize; 5] = std::array::from_fn(|k| reflect(x + k as isize - 2, w));
            let p = |dy: usize, dx: usize| rows[dy][xs[dx]];

            let c = p(2, 2);
            let (n, s, we, e) = (p(1, 2), p(3, 2), p(2, 1), p(2, 3));
            let (n2, s2, w2, e2) = (p(0, 2), p(4, 2), p(2, 0), p(2, 4));
            let diag = p(1, 1) + p(1, 3) + p(3, 1) + p(3, 3);

            let here = cfa_row[(x & 1) as usize];
            let (r, g, bl) = if here == 1 {
                // Green site: one chroma has neighbours left/right, the other above/below.
                let horiz = (5.0 * c + 4.0 * (we + e) - diag - (w2 + e2) + 0.5 * (n2 + s2)) * 0.125;
                let vert = (5.0 * c + 4.0 * (n + s) - diag - (n2 + s2) + 0.5 * (w2 + e2)) * 0.125;
                if cfa_row[((x & 1) ^ 1) as usize] == 0 {
                    (horiz, c, vert)
                } else {
                    (vert, c, horiz)
                }
            } else {
                let g = (4.0 * c + 2.0 * (n + s + we + e) - (n2 + s2 + w2 + e2)) * 0.125;
                let opposite = (6.0 * c + 2.0 * diag - 1.5 * (n2 + s2 + w2 + e2)) * 0.125;
                if here == 0 {
                    (c, g, opposite)
                } else {
                    (opposite, g, c)
                }
            };

            let o = ox * 3;
            row[o] = r.max(0.0);
            row[o + 1] = g.max(0.0);
            row[o + 2] = bl.max(0.0);
        }
    });
    out
}

/// Demosaic for Fujifilm X-Trans (6x6 pattern). Green first, from the green sites around
/// each pixel (the 3x3 neighbourhood always has some, orthogonal ones weighted double); then
/// red and blue as colour differences against that green, averaged over the sites of that
/// colour in the 5x5 neighbourhood with inverse-distance weights. Returns interleaved RGB
/// for the crop area.
fn demosaic_xtrans(b: &Bayer) -> Vec<f32> {
    let pattern = b.xtrans.expect("X-Trans pattern");
    let (w, h) = (b.width, b.height);
    let (cx, cy, cw, ch) = b.crop;
    let colour = |y: usize, x: usize| pattern[y % 6][x % 6];
    let src = &b.data;

    // Pass 1: a full green plane.
    let mut green = vec![0f32; w * h];
    green.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            if colour(y, x) == 1 {
                row[x] = src[y * w + x];
                continue;
            }
            let (mut sum, mut weight) = (0.0f32, 0.0f32);
            for radius in 1..=2isize {
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        let (yy, xx) = (y as isize + dy, x as isize + dx);
                        if (dy == 0 && dx == 0) || yy < 0 || xx < 0 || yy >= h as isize || xx >= w as isize {
                            continue;
                        }
                        let (yy, xx) = (yy as usize, xx as usize);
                        if colour(yy, xx) == 1 {
                            let wgt = if dy == 0 || dx == 0 { 2.0 } else { 1.0 } / (dy * dy + dx * dx) as f32;
                            sum += wgt * src[yy * w + xx];
                            weight += wgt;
                        }
                    }
                }
                if weight > 0.0 {
                    break;
                }
            }
            row[x] = if weight > 0.0 { sum / weight } else { src[y * w + x] };
        }
    });

    // Pass 2: red and blue as differences from green.
    let mut out = vec![0f32; cw * ch * 3];
    out.par_chunks_mut(cw * 3).enumerate().for_each(|(oy, row)| {
        let y = oy + cy;
        for ox in 0..cw {
            let x = ox + cx;
            let g = green[y * w + x];
            let here = colour(y, x);
            let mut rgb = [0f32; 3];
            rgb[1] = g;
            for c in [0u8, 2u8] {
                if here == c {
                    rgb[c as usize] = src[y * w + x];
                    continue;
                }
                let (mut sum, mut weight) = (0.0f32, 0.0f32);
                for dy in -2isize..=2 {
                    for dx in -2isize..=2 {
                        let (yy, xx) = (y as isize + dy, x as isize + dx);
                        if yy < 0 || xx < 0 || yy >= h as isize || xx >= w as isize {
                            continue;
                        }
                        let (yy, xx) = (yy as usize, xx as usize);
                        if colour(yy, xx) == c {
                            let wgt = 1.0 / (1 + dy * dy + dx * dx) as f32;
                            sum += wgt * (src[yy * w + xx] - green[yy * w + xx]);
                            weight += wgt;
                        }
                    }
                }
                rgb[c as usize] = g + if weight > 0.0 { sum / weight } else { 0.0 };
            }
            let o = ox * 3;
            row[o] = rgb[0].max(0.0);
            row[o + 1] = rgb[1].max(0.0);
            row[o + 2] = rgb[2].max(0.0);
        }
    });
    out
}

/// 2x2 box downsample of interleaved RGB.
fn downsample(src: &[f32], w: usize, h: usize) -> (Vec<f32>, usize, usize) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0f32; nw * nh * 3];
    out.par_chunks_mut(nw * 3).enumerate().for_each(|(y, row)| {
        let y0 = (y * 2).min(h - 1);
        let y1 = (y * 2 + 1).min(h - 1);
        for x in 0..nw {
            let x0 = (x * 2).min(w - 1);
            let x1 = (x * 2 + 1).min(w - 1);
            for ch in 0..3 {
                let s = src[(y0 * w + x0) * 3 + ch]
                    + src[(y0 * w + x1) * 3 + ch]
                    + src[(y1 * w + x0) * 3 + ch]
                    + src[(y1 * w + x1) * 3 + ch];
                row[x * 3 + ch] = s * 0.25;
            }
        }
    });
    (out, nw, nh)
}

fn to_rgba16f(src: &[f32], w: usize, h: usize) -> MipLevel {
    let one = half::f16::from_f32(1.0).to_bits();
    let mut rgba = vec![0u16; w * h * 4];
    rgba.par_chunks_mut(w * 4).zip(src.par_chunks(w * 3)).for_each(|(dst, src)| {
        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
            d[0] = half::f16::from_f32(s[0]).to_bits();
            d[1] = half::f16::from_f32(s[1]).to_bits();
            d[2] = half::f16::from_f32(s[2]).to_bits();
            d[3] = one;
        }
    });
    MipLevel { width: w as u32, height: h as u32, rgba }
}

/// AI denoise in place of the plain demosaic: run the model on the mosaic, then keep the
/// plain demosaic where the sensor is close to saturation (no noise there, and the
/// highlight recovery in the shader relies on the exact clipped values).
fn denoised_rgb(b: &Bayer, plain: &[f32]) -> Result<(Vec<f32>, String), String> {
    // The model wants RGGB: start from the first red site.
    let (oy, ox) = (0..2)
        .flat_map(|y| (0..2).map(move |x| (y, x)))
        .find(|&(y, x)| b.cfa[y][x] == 0 && b.cfa[y][x ^ 1] == 1 && b.cfa[y ^ 1][x] == 1 && b.cfa[y ^ 1][x ^ 1] == 2)
        .ok_or("AI denoise needs a standard Bayer sensor.")?;
    let (pw, ph) = ((b.width - ox) / 2, (b.height - oy) / 2);
    let plane = |dy: usize, dx: usize| -> Vec<f32> {
        let mut out = vec![0f32; pw * ph];
        out.par_chunks_mut(pw).enumerate().for_each(|(y, row)| {
            let src = &b.data[(oy + 2 * y + dy) * b.width..];
            for (x, v) in row.iter_mut().enumerate() {
                *v = src[ox + 2 * x + dx];
            }
        });
        out
    };
    let planes = [plane(0, 0), plane(0, 1), plane(1, 0), plane(1, 1)];
    let result = crate::denoise::run(&planes, pw, ph)?;

    let (cx, cy, cw, _) = b.crop;
    let (mw, mh) = (pw * 2, ph * 2);
    let mut out = plain.to_vec();
    out.par_chunks_mut(cw * 3).enumerate().for_each(|(row, line)| {
        let y = row + cy;
        if y < oy || y - oy >= mh {
            return;
        }
        let my = y - oy;
        for (col, px) in line.chunks_exact_mut(3).enumerate() {
            let x = col + cx;
            if x < ox || x - ox >= mw {
                continue;
            }
            let mx = x - ox;
            let p = (my / 2) * pw + mx / 2;
            let brightest = planes[0][p].max(planes[1][p]).max(planes[2][p]).max(planes[3][p]);
            let t = ((brightest - 0.80) / 0.15).clamp(0.0, 1.0);
            let keep_plain = t * t * (3.0 - 2.0 * t);
            let m = &result.rgb[(my * mw + mx) * 3..][..3];
            for c in 0..3 {
                px[c] = m[c] + (px[c] - m[c]) * keep_plain;
            }
        }
    });
    Ok((out, result.note))
}

/// AI denoise for a raw file that is already RGB (linear DNG): camera colour is turned into
/// white-balanced linear Rec.2020, which the RawNIND linear model expects, and back again.
fn denoised_linear(data: &[f32], width: usize, height: usize, meta: &Meta) -> Result<(Vec<f32>, String), String> {
    use crate::color::{invert, mul, Mat3};
    const REC2020_FROM_XYZ: Mat3 = [[1.716_651, -0.355_671, -0.253_366], [-0.666_684, 1.616_481, 0.015_769], [0.017_640, -0.042_771, 0.942_103]];
    let cam_from_xyz = meta.xyz_to_cam;
    let xyz_from_cam = invert(&cam_from_xyz).ok_or("The camera colour matrix cannot be inverted.")?;
    // Camera -> Rec.2020, scaled so that the camera's white (1 / white-balance gains) comes
    // out neutral: each row then sums to one once the gains are applied.
    let mut m = mul(&REC2020_FROM_XYZ, &xyz_from_cam);
    let gains = meta.as_shot_wb;
    let g = if gains[1] > 0.0 { gains[1] } else { 1.0 };
    for row in m.iter_mut() {
        for (c, v) in row.iter_mut().enumerate() {
            *v *= if gains[c] > 0.0 { gains[c] / g } else { 1.0 };
        }
        let sum: f32 = row.iter().sum();
        if sum.abs() > 1.0e-6 {
            for v in row.iter_mut() {
                *v /= sum;
            }
        }
    }
    let back = invert(&m).ok_or("The colour conversion cannot be inverted.")?;
    let apply = |m: &Mat3, src: &[f32], dst: &mut [f32]| {
        dst.par_chunks_mut(3).zip(src.par_chunks(3)).for_each(|(d, s)| {
            for r in 0..3 {
                d[r] = m[r][0] * s[0] + m[r][1] * s[1] + m[r][2] * s[2];
            }
        });
    };
    let mut rec2020 = vec![0f32; data.len()];
    apply(&m, data, &mut rec2020);
    // Phones store very dark data and brighten it with a large baseline exposure (+3 EV on
    // the Galaxy S24 Ultra). The model, trained on normally exposed pictures, casts the
    // shadows of data that dark red, so it is brightened first (to an average of 0.25, at
    // most 32x) and darkened again afterwards.
    let mean = rec2020.par_iter().map(|&v| v.max(0.0) as f64).sum::<f64>() / rec2020.len().max(1) as f64;
    let k = if mean > 1e-6 { (0.25 / mean as f32).clamp(1.0, 32.0) } else { 1.0 };
    let lifted: Vec<f32> = rec2020.par_iter().map(|v| v * k).collect();
    let mut result = crate::denoise::run_linear(&lifted, width, height)?;
    drop(lifted);
    result.rgb.par_iter_mut().for_each(|v| *v /= k);
    // The linear model also draws coloured (mostly red) fringes around bright edges. Its
    // brightness detail is clean, so that is kept, and the colour comes from the original,
    // smoothed to take out colour noise, as colour noise reduction does.
    // Colour is carried as each channel's share of the brightness, averaged over a few
    // pixels weighted by brightness, so it never goes negative next to bright detail.
    let luma = |p: &[f32]| 0.2627 * p[0] + 0.6780 * p[1] + 0.0593 * p[2];
    let mut colour = rec2020;
    box_blur_rgb(&mut colour, width, height, 2);
    box_blur_rgb(&mut colour, width, height, 2);
    result.rgb.par_chunks_mut(3).zip(colour.par_chunks(3)).for_each(|(p, c)| {
        let y = luma(p).max(0.0);
        let around = luma(c);
        if around > 1e-9 {
            for i in 0..3 {
                p[i] = y * c[i] / around;
            }
        } else {
            p.fill(y);
        }
    });
    drop(colour);
    let mut out = vec![0f32; data.len()];
    apply(&back, &result.rgb, &mut out);
    out.par_iter_mut().for_each(|v| *v = v.max(0.0));
    Ok((out, result.note))
}

/// Box blur of interleaved RGB, `radius` pixels each way, horizontally then vertically.
fn box_blur_rgb(data: &mut [f32], width: usize, height: usize, radius: usize) {
    if width == 0 || height == 0 {
        return;
    }
    let pass = |src: &[f32], dst: &mut [f32], len: usize, lines: usize, at: &(dyn Fn(usize, usize) -> usize + Sync)| {
        let dst_ptr = dst.as_mut_ptr() as usize;
        (0..lines).into_par_iter().for_each(|line| {
            let mut acc = [0f32; 3];
            let r = radius as isize;
            let clampi = |i: isize| i.clamp(0, len as isize - 1) as usize;
            for i in -r..=r {
                let o = at(line, clampi(i)) * 3;
                for c in 0..3 {
                    acc[c] += src[o + c];
                }
            }
            let n = (2 * radius + 1) as f32;
            for i in 0..len as isize {
                let o = at(line, i as usize) * 3;
                // SAFETY: each line writes only its own pixels.
                unsafe {
                    let d = (dst_ptr as *mut f32).add(o);
                    for c in 0..3 {
                        *d.add(c) = acc[c] / n;
                    }
                }
                let (add, sub) = (at(line, clampi(i + r + 1)) * 3, at(line, clampi(i - r)) * 3);
                for c in 0..3 {
                    acc[c] += src[add + c] - src[sub + c];
                }
            }
        });
    };
    let mut temp = vec![0f32; data.len()];
    pass(data, &mut temp, width, height, &|line, i| line * width + i);
    pass(&temp, data, height, width, &|line, i| i * width + line);
}

/// A finished picture (JPEG, PNG, TIFF, HEIC): decode it and turn its sRGB values back into
/// linear light, so the same sliders work on it. It has no camera matrix or raw white
/// balance; it is described as a "camera" whose colour is already sRGB.
fn load_picture(path: &Path) -> Result<(Vec<f32>, usize, usize, Meta), String> {
    let decoded = crate::heic::open_picture(path)?;
    let bits = (decoded.color().bits_per_pixel() / decoded.color().channel_count().max(1) as u16) as u8;
    let rgb = decoded.into_rgb32f();
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    if w < 8 || h < 8 {
        return Err("The picture is too small.".into());
    }
    let mut data = rgb.into_raw();
    data.par_iter_mut().for_each(|v| {
        let s = v.clamp(0.0, 1.0);
        *v = if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) };
    });
    let quarter_turns = match crate::filmstrip::exif_orientation(path) {
        Some(6) => 1,
        Some(3) => 2,
        Some(8) => 3,
        _ => 0,
    };
    let info = crate::filmstrip::tiff_info(path);
    let exif = match &info {
        Some(i) => crate::lens::Exif {
            make: i.make.clone().unwrap_or_default(),
            model: i.model.clone().unwrap_or_default(),
            lens_make: i.lens_make.clone().unwrap_or_default(),
            lens_model: i.lens_model.clone().unwrap_or_default(),
            focal: i.focal.unwrap_or(0.0),
            fnumber: i.fnumber.unwrap_or(0.0),
        },
        None => crate::lens::Exif::default(),
    };
    let meta = Meta {
        exif,
        bits,        // XYZ (D65) to linear sRGB.
        xyz_to_cam: [[3.2406, -1.5372, -0.4986], [-0.9689, 1.8758, 0.0415], [0.0557, -0.2040, 1.0570]],
        as_shot_wb: [1.0, 1.0, 1.0],
        quarter_turns,
        camera: format!("{} picture", crate::filmstrip::extension_of(path).to_uppercase()),
        photo: crate::export::PhotoMeta::default(),
    };
    Ok((data, w, h, meta))
}

/// Stage 1 for any supported file: decode (and demosaic or denoise a Bayer raw), then build
/// the mip chain. `max_dim` is the GPU's maximum 2D texture size. With `denoise`, the AI
/// denoiser runs on a Bayer raw's sensor data first.
pub fn load(path: &Path, max_dim: u32, denoise: bool) -> Result<DecodedImage, String> {
    let (mut denoised, mut denoise_note) = (false, None);
    let (mut rgb, full_w, full_h, meta, input_mode) = if crate::filmstrip::is_raw(path) {
        match decode(path)? {
            (Pixels::Bayer(bayer), meta) if bayer.xtrans.is_some() => {
                // X-Trans: demosaic first; the result is camera RGB like a linear DNG, so the
                // linear denoise model applies (input mode 1).
                let (_, _, w, h) = bayer.crop;
                let mut rgb = demosaic_xtrans(&bayer);
                if denoise {
                    match denoised_linear(&rgb, w, h, &meta) {
                        Ok((clean, note)) => {
                            rgb = clean;
                            denoised = true;
                            denoise_note = Some(note);
                        }
                        Err(message) => denoise_note = Some(format!("AI denoise did not run: {message}")),
                    }
                }
                (rgb, w, h, meta, 1u8)
            }
            (Pixels::Bayer(bayer), meta) => {
                let (_, _, w, h) = bayer.crop;
                let mut rgb = demosaic(&bayer);
                if denoise {
                    match denoised_rgb(&bayer, &rgb) {
                        Ok((clean, note)) => {
                            rgb = clean;
                            denoised = true;
                            denoise_note = Some(note);
                        }
                        Err(message) => denoise_note = Some(format!("AI denoise did not run: {message}")),
                    }
                }
                (rgb, w, h, meta, 0u8)
            }
            (Pixels::Rgb { mut data, width, height }, meta) => {
                if denoise {
                    match denoised_linear(&data, width, height, &meta) {
                        Ok((clean, note)) => {
                            data = clean;
                            denoised = true;
                            denoise_note = Some(note);
                        }
                        Err(message) => denoise_note = Some(format!("AI denoise did not run: {message}")),
                    }
                }
                (data, width, height, meta, 1u8)
            }
        }
    } else {
        let (data, w, h, meta) = load_picture(path)?;
        (data, w, h, meta, 2u8)
    };

    let (mut w, mut h) = (full_w, full_h);
    let mut mips = Vec::new();
    loop {
        if w.max(h) <= max_dim as usize {
            mips.push(to_rgba16f(&rgb, w, h));
        }
        if w.max(h) <= 64 {
            break;
        }
        let (next, nw, nh) = downsample(&rgb, w, h);
        rgb = next;
        w = nw;
        h = nh;
    }
    if mips.is_empty() {
        return Err("Image does not fit in a GPU texture.".into());
    }

    let info = crate::filmstrip::tiff_info(path);
    Ok(DecodedImage {
        mips,
        full_width: full_w as u32,
        full_height: full_h as u32,
        xyz_to_cam: meta.xyz_to_cam,
        as_shot_wb: meta.as_shot_wb,
        // The decoder does not report how the camera was held for ARW files, so read the
        // file's own orientation tag (6 = turn right, 3 = upside down, 8 = turn left).
        quarter_turns: match info.as_ref().map(|i| i.orientation) {
            Some(6) => 1,
            Some(3) => 2,
            Some(8) => 3,
            Some(_) => 0,
            None => meta.quarter_turns,
        },
        camera: meta.camera,
        photo: meta.photo,
        denoised,
        denoise_note,
        input_mode,
        bits: meta.bits,
        embedded: info.as_ref().and_then(|i| {
            let tail = |v: &Vec<i32>| v.get(1..).map(|s| s.to_vec()).unwrap_or_default();
            crate::lens::Embedded::from_tags(&tail(&i.embedded_distortion), &tail(&i.embedded_ca), &tail(&i.embedded_vignetting))
        }),
        exposure_compensation: exposure_compensation_of(info.as_ref()),
        exif: meta.exif,
        baseline_exposure: if crate::filmstrip::extension_of(path) == "dng" { info.and_then(|i| i.baseline_exposure) } else { None },
    })
}
/// Fujifilm's RawExposureBias at DR100 (stops); anything below this is DR200 and up.
const FUJI_NORMAL_BIAS: f32 = -0.72;

/// Exposure (stops) to add for how the camera recorded the raw: Fujifilm DR200 / DR400 shots
/// are underexposed by a further stop or two, which Lightroom brings back.
pub fn exposure_compensation_of(info: Option<&crate::filmstrip::TiffInfo>) -> f32 {
    info.and_then(|i| i.raw_exposure_bias).map_or(0.0, |bias| -(bias - FUJI_NORMAL_BIAS)).clamp(0.0, 3.0)
}

/// Diagnostic (`app --dump-raw <file> <out.txt>`): what the decoder reports for a raw file.
pub fn describe(path: &Path) -> String {
    use rawler::RawImageData;
    let run = || -> Result<String, String> {
        let source = rawler::rawsource::RawSource::new(path).map_err(|e| e.to_string())?;
        let decoder = rawler::get_decoder(&source).map_err(|e| e.to_string())?;
        let raw = decoder.raw_image(&source, &rawler::decoders::RawDecodeParams::default(), false).map_err(|e| e.to_string())?;
        let (kind, min, max, mean) = match &raw.data {
            RawImageData::Integer(d) => {
                let (lo, hi) = d.iter().fold((u16::MAX, 0u16), |(lo, hi), &v| (lo.min(v), hi.max(v)));
                ("integer", lo as f64, hi as f64, d.iter().map(|&v| v as f64).sum::<f64>() / d.len().max(1) as f64)
            }
            RawImageData::Float(d) => {
                let (lo, hi) = d.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
                ("float", lo as f64, hi as f64, d.iter().map(|&v| v as f64).sum::<f64>() / d.len().max(1) as f64)
            }
        };
        Ok(format!(
            "camera: {} {}\nsize: {} x {}, cpp {}, bps {}\nphotometric: {:?}\nblack: {:?}\nwhite: {:?}\nwb: {:?}\ndata: {kind}, min {min}, max {max}, mean {mean:.2}\ncrop: {:?}\nactive: {:?}\nmatrices: {:?}\ntiff info: baseline exposure {:?}\nlinearization table: {:?}\n",
            raw.clean_make,
            raw.clean_model,
            raw.width,
            raw.height,
            raw.cpp,
            raw.bps,
            raw.photometric,
            raw.blacklevel.as_vec(),
            raw.whitelevel.as_vec(),
            raw.wb_coeffs,
            raw.crop_area,
            raw.active_area,
            raw.color_matrix,
            crate::filmstrip::tiff_info(path).and_then(|i| i.baseline_exposure),
            crate::filmstrip::tiff_info(path).and_then(|i| i.linearization).map(|t| (t.len(), t.iter().step_by((t.len() / 8).max(1)).copied().collect::<Vec<u16>>(), t.last().copied())),
        ))
    };
    let mut text = run().unwrap_or_else(|e| format!("could not decode: {e}"));
    // What lens matching sees.
    if let Ok((_, meta)) = decode(path) {
        let e = &meta.exif;
        let db = crate::lens::Database::load();
        let camera = db.find_camera(&e.make, &e.model).map(|c| format!("{} {} ({}, crop {})", c.maker, c.model, c.mount, c.crop));
        let lens = db.auto_match(e).map(|i| format!("{} {}", db.lenses[i].maker, db.lenses[i].model));
        text += &format!(
            "exif: make {:?}, model {:?}, lens make {:?}, lens model {:?}, focal {}, f/{}\nlensfun: {} files, camera {:?}, lens {:?}\n",
            e.make, e.model, e.lens_make, e.lens_model, e.focal, e.fnumber, db.files, camera, lens
        );
    }
    text
}