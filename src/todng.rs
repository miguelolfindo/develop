//! Convert to DNG: the original raw data in Adobe's open raw format.
//!
//! Develop builds the file itself on rawler's low-level DNG writer (which does the lossless
//! compression and the file structure), filling in what rawler's own converter leaves out:
//! - the camera and lens EXIF as Develop reads it (rawler's decoders miss some lens names),
//! - the baseline exposure Develop uses for the camera (Adobe's usual +0.35, per-model
//!   offsets, Fujifilm DR200 / DR400 compensation), so the DNG opens at the same brightness,
//! - the lens distortion and colour-fringe correction Lightroom applies to this camera by
//!   default (Nikon Z, Panasonic, Olympus / OM System, Fujifilm), as a WarpRectilinear opcode
//!   fitted to Develop's own lens correction, so every DNG reader corrects it the same way.
//!
//! Each photo's Develop edits (its `.dvlp` sidecar) are copied beside the DNG, so Develop opens
//! it with the same edits. Other applications open it unedited.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

use rawler::decoders::{RawDecodeParams, WellKnownIFD};
use rawler::dng::original::OriginalCompressed;
use rawler::dng::writer::DngWriter;
use rawler::dng::{CropMode, DngCompression, DngPhotometricConversion, DNG_VERSION_V1_4};
use rawler::formats::tiff::{Rational, SRational};
use rawler::tags::{DngTag, ExifTag, TiffCommonTag};

/// Sent from the conversion thread.
pub enum Event {
    /// About to convert photo `done + 1` of `total`, named `name`.
    Progress { done: usize, total: usize, name: String },
    /// All done: a summary for the completion window.
    Finished(Result<String, String>),
}

/// A name in `dir` that does not overwrite anything: `stem.dng`, `stem-2.dng`...
fn free_name(dir: &Path, stem: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.dng"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.dng"));
        n += 1;
    }
    path
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Convert one raw file into `dir`, under a name that overwrites nothing. Returns the DNG
/// written.
pub fn convert_one(photo: &Path, dir: &Path, embed_original: bool, artist: Option<String>) -> Result<PathBuf, String> {
    let stem = photo.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "photo".to_owned());
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    convert_to(photo, &free_name(dir, &stem), embed_original, artist)
}

/// What Develop adds to a DNG beyond the raw data: camera and lens, baseline exposure and
/// the lens correction.
pub struct Plan {
    pub exif: crate::lens::Exif,
    pub baseline: f32,
    /// WarpRectilinear coefficients for red, green, blue, and the worst fit error (fraction of
    /// the half diagonal).
    pub warp: Option<([[f64; 6]; 3], f64)>,
    /// The Lensfun profile the correction came from.
    pub profile: Option<String>,
}

fn lens_database() -> &'static crate::lens::Database {
    static DATABASE: OnceLock<crate::lens::Database> = OnceLock::new();
    DATABASE.get_or_init(crate::lens::Database::load)
}

/// Work out the camera, lens, exposure and lens correction for `photo`.
pub fn plan(photo: &Path, raw: &rawler::RawImage, metadata: &rawler::decoders::RawMetadata) -> Plan {
    let info = crate::filmstrip::tiff_info(photo);
    let e = &metadata.exif;
    let ratio = |r: Option<&Rational>| r.filter(|r| r.d > 0).map(|r| r.n as f32 / r.d as f32);
    let text = |a: Option<String>, b: Option<String>| a.filter(|s| !s.trim().is_empty()).or(b).unwrap_or_default();
    let exif = crate::lens::Exif {
        make: raw.clean_make.clone(),
        model: raw.clean_model.clone(),
        lens_make: text(e.lens_make.clone(), info.as_ref().and_then(|i| i.lens_make.clone())),
        lens_model: text(e.lens_model.clone(), info.as_ref().and_then(|i| i.lens_model.clone())),
        focal: ratio(e.focal_length.as_ref()).or(info.as_ref().and_then(|i| i.focal)).unwrap_or(0.0),
        fnumber: ratio(e.fnumber.as_ref()).or(info.as_ref().and_then(|i| i.fnumber)).unwrap_or(0.0),
    };
    let baseline = crate::BASELINE_EXPOSURE + crate::maker_baseline(&exif.make, &exif.model) + crate::raw::exposure_compensation_of(info.as_ref());

    let db = lens_database();
    let mut warp = None;
    let mut profile = None;
    if let Some(i) = db.auto_match(&exif).filter(|&i| crate::lens::builtin_distortion(&exif.make, &db.lenses[i])) {
        let settings = crate::lens::LensSettings {
            enabled: true,
            source: crate::lens::Source::Lensfun,
            profile_distortion: 100.0,
            profile_vignetting: 0.0,
            ..Default::default()
        };
        let area = raw.crop_area.or(raw.active_area);
        let aspect = area.map_or(raw.width as f32 / raw.height.max(1) as f32, |a| a.d.w as f32 / a.d.h.max(1) as f32);
        let gpu = crate::lens::Gpu::new(&settings, &crate::lens::Active::Lensfun(i), db, &exif, None, aspect);
        warp = gpu.dng_warp(aspect);
        profile = Some(format!("{} {}", db.lenses[i].maker, db.lenses[i].model));
    }
    Plan { exif, baseline, warp, profile }
}

/// An OpcodeList holding one WarpRectilinear opcode (DNG 1.3, opcode 1): three planes of six
/// coefficients, then the optical centre (0.5, 0.5). All big-endian.
fn warp_opcode_list(planes: &[[f64; 6]; 3]) -> Vec<u8> {
    let mut params = Vec::new();
    params.extend(3u32.to_be_bytes());
    for plane in planes {
        for v in plane {
            params.extend(v.to_be_bytes());
        }
    }
    params.extend(0.5f64.to_be_bytes());
    params.extend(0.5f64.to_be_bytes());
    let mut list = Vec::new();
    list.extend(1u32.to_be_bytes()); // one opcode
    list.extend(1u32.to_be_bytes()); // WarpRectilinear
    list.extend([1u8, 3, 0, 0]); // needs DNG 1.3
    list.extend(1u32.to_be_bytes()); // optional: readers without it may skip it
    list.extend((params.len() as u32).to_be_bytes());
    list.extend(params);
    list
}

/// Convert one raw file to the DNG file `target` (replacing it). Returns `target`.
pub fn convert_to(photo: &Path, target: &Path, embed_original: bool, artist: Option<String>) -> Result<PathBuf, String> {
    let target = target.to_path_buf();
    if crate::filmstrip::extension_of(photo) == "dng" || !crate::filmstrip::is_raw(photo) {
        return Err("Only camera raw files can be converted to DNG".into());
    }
    if let Err(message) = write_dng(photo, &target, embed_original, artist) {
        let _ = std::fs::remove_file(&target);
        return Err(message);
    }
    // Carry the edits (and rating) along.
    let sidecar = crate::settings::sidecar_path(photo);
    if sidecar.is_file() {
        let _ = std::fs::copy(&sidecar, crate::settings::sidecar_path(&target));
    }
    Ok(target)
}

fn write_dng(photo: &Path, target: &Path, embed_original: bool, artist: Option<String>) -> Result<(), String> {
    let source = rawler::rawsource::RawSource::new(photo).map_err(|e| e.to_string())?;
    let decoder = rawler::get_decoder(&source).map_err(|e| e.to_string())?;
    let params = RawDecodeParams::default();
    let rawimage = decoder.raw_image(&source, &params, false).map_err(|e| e.to_string())?;
    let metadata = decoder.raw_metadata(&source, &params).map_err(|e| e.to_string())?;
    let plan = plan(photo, &rawimage, &metadata);
    let original = if embed_original { Some(OriginalCompressed::compress(&mut source.reader()).map_err(|e| e.to_string())?) } else { None };

    let file = std::fs::File::create(target).map_err(|e| e.to_string())?;
    let mut out = std::io::BufWriter::new(file);
    {
        let mut dng = DngWriter::new(&mut out, DNG_VERSION_V1_4).map_err(|e| e.to_string())?;

        // The raw image, with the camera's own DNG raw tags and Develop's lens correction.
        let mut raw = dng.subframe(0);
        raw.raw_image(&rawimage, CropMode::Best, DngCompression::Lossless, DngPhotometricConversion::Original, 1).map_err(|e| e.to_string())?;
        if let Some(ifd) = decoder.ifd(WellKnownIFD::VirtualDngRawTags).map_err(|e| e.to_string())? {
            raw.ifd_mut().copy(ifd.value_iter());
        }
        if let Some((planes, _)) = &plan.warp {
            raw.ifd_mut().add_tag_undefined(DngTag::OpcodeList3, warp_opcode_list(planes));
        }
        raw.finalize().map_err(|e| e.to_string())?;

        // Preview and thumbnail from the camera's own preview.
        let preview = decoder.preview_image(&source, &params).ok().flatten().or_else(|| decoder.full_image(&source, &params).ok().flatten());
        if let Some(image) = preview {
            let mut sub = dng.subframe(1);
            sub.preview(&image, 0.8).map_err(|e| e.to_string())?;
            sub.finalize().map_err(|e| e.to_string())?;
            dng.thumbnail(&image).map_err(|e| e.to_string())?;
        }

        // Metadata: every EXIF field of the original first, then rawler's, then what Develop
        // knows better (each later one replaces an earlier one for the same field).
        for (tag, value) in original_exif(photo) {
            dng.exif_ifd_mut().add_untyped_tag(tag, value);
        }
        dng.load_base_tags(&rawimage).map_err(|e| e.to_string())?;
        dng.load_metadata(&metadata).map_err(|e| e.to_string())?;
        if !dng.root_ifd().contains(ExifTag::Orientation) {
            dng.root_ifd_mut().add_tag(ExifTag::Orientation, rawimage.orientation.to_u16());
        }
        if let Some(ifd) = decoder.ifd(WellKnownIFD::VirtualDngRootTags).map_err(|e| e.to_string())? {
            dng.root_ifd_mut().copy(ifd.value_iter());
        }
        dng.root_ifd_mut().add_tag(TiffCommonTag::Software, format!("Develop {}", env!("CARGO_PKG_VERSION")));
        if let Some(artist) = artist.filter(|a| !a.is_empty()) {
            dng.root_ifd_mut().add_tag(TiffCommonTag::Artist, artist);
        }
        dng.root_ifd_mut().add_tag(DngTag::BaselineExposure, SRational::new((plan.baseline * 100.0).round() as i32, 100));
        let x = &plan.exif;
        if !x.lens_model.is_empty() {
            dng.exif_ifd_mut().add_tag(ExifTag::LensModel, x.lens_model.as_str());
        }
        if !x.lens_make.is_empty() {
            dng.exif_ifd_mut().add_tag(ExifTag::LensMake, x.lens_make.as_str());
        }
        if x.focal > 0.0 && !dng.exif_ifd().contains(ExifTag::FocalLength) {
            dng.exif_ifd_mut().add_tag(ExifTag::FocalLength, Rational::new((x.focal * 100.0).round() as u32, 100));
        }
        if x.fnumber > 0.0 && !dng.exif_ifd().contains(ExifTag::FNumber) {
            dng.exif_ifd_mut().add_tag(ExifTag::FNumber, Rational::new((x.fnumber * 100.0).round() as u32, 100));
        }
        if let Some(xpacket) = decoder.xpacket(&source, &params).map_err(|e| e.to_string())? {
            dng.xpacket(&xpacket).map_err(|e| e.to_string())?;
        }
        if let Some(original) = &original {
            let name = photo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            dng.original_file(original, name).map_err(|e| e.to_string())?;
        }
        dng.close().map_err(|e| e.to_string())?;
    }
    use std::io::Write;
    out.flush().map_err(|e| e.to_string())
}

/// Convert `photos` into `dir` one after another, reporting progress. Files that are not
/// raw (or are DNG already) are skipped. `cancel` stops after the current photo.
pub fn convert_all(photos: Vec<PathBuf>, dir: PathBuf, embed_original: bool, artist: Option<String>, events: Sender<Event>, cancel: Arc<AtomicBool>) {
    let total = photos.len();
    let (mut done, mut skipped, mut failed) = (0usize, Vec::new(), Vec::new());
    for (i, photo) in photos.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let _ = events.send(Event::Progress { done: i, total, name: name_of(photo) });
        let is_dng = crate::filmstrip::extension_of(photo) == "dng";
        if !crate::filmstrip::is_raw(photo) || is_dng {
            skipped.push(name_of(photo));
            continue;
        }
        match convert_one(photo, &dir, embed_original, artist.clone()) {
            Ok(_) => done += 1,
            Err(message) => failed.push(format!("{} ({message})", name_of(photo))),
        }
    }
    let mut message = format!("Converted {done} of {total} photos to DNG in\n{}", dir.display());
    if cancel.load(Ordering::Relaxed) {
        message.push_str("\nStopped before the end.");
    }
    if !skipped.is_empty() {
        message.push_str(&format!("\nSkipped (not raw, or DNG already): {}", skipped.join(", ")));
    }
    if !failed.is_empty() {
        message.push_str(&format!("\nFailed: {}", failed.join(", ")));
    }
    let _ = events.send(Event::Finished(if failed.is_empty() { Ok(message) } else { Err(message) }));
}

/// A one-line summary of what Develop adds to `photo`'s DNG (for `--to-dng`).
pub fn describe(photo: &Path) -> String {
    let run = || -> Result<String, String> {
        let source = rawler::rawsource::RawSource::new(photo).map_err(|e| e.to_string())?;
        let decoder = rawler::get_decoder(&source).map_err(|e| e.to_string())?;
        let params = RawDecodeParams::default();
        let rawimage = decoder.raw_image(&source, &params, false).map_err(|e| e.to_string())?;
        let metadata = decoder.raw_metadata(&source, &params).map_err(|e| e.to_string())?;
        let p = plan(photo, &rawimage, &metadata);
        let warp = match &p.warp {
            Some((planes, worst)) => format!(
                "warp from {} (green kr {:.5} {:.5} {:.5} {:.5}, fit within {:.4}% of the half diagonal)",
                p.profile.clone().unwrap_or_default(),
                planes[1][0],
                planes[1][1],
                planes[1][2],
                planes[1][3],
                worst * 100.0
            ),
            None => "no lens correction".to_owned(),
        };
        Ok(format!("lens {:?}, {} mm f/{}, baseline exposure {:+.2}, {warp}", p.exif.lens_model, p.exif.focal, p.exif.fnumber, p.baseline))
    };
    run().unwrap_or_else(|e| format!("could not read: {e}"))
}

/// EXIF fields of the original file that describe its own layout rather than the photo:
/// maker notes (kept or dropped by rawler according to the DNG maker-note safety rule),
/// pointers to other directories, and the embedded JPEG's pixel size and compression.
const SKIP_EXIF: [u16; 8] = [0x927C, 0xA005, 0x8825, 0x8769, 0xA002, 0xA003, 0x9101, 0x9102];

/// Every EXIF field of the original raw file, as rawler values: from the embedded JPEG for
/// Fujifilm RAF, the CMT2 box for Canon CR3, the TIFF structure for the others.
fn original_exif(photo: &Path) -> Vec<(u16, rawler::formats::tiff::Value)> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(photo) else { return Vec::new() };
    let mut read = |offset: u64, len: usize| -> Option<Vec<u8>> {
        let mut bytes = vec![0u8; len];
        file.seek(SeekFrom::Start(offset)).ok()?;
        let n = file.read(&mut bytes).ok()?;
        bytes.truncate(n);
        Some(bytes)
    };
    let ext = crate::filmstrip::extension_of(photo);
    // (bytes, where the TIFF header starts, whether its first directory is the EXIF one)
    let found = if ext == "raf" {
        read(0, 100).and_then(|h| {
            let be = |i: usize| u32::from_be_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]) as u64;
            let jpeg = read(be(84), be(88).min(64 << 20) as usize)?;
            let at = jpeg.windows(6).take(256 * 1024).position(|w| w == b"Exif\0\0")? + 6;
            Some((jpeg, at, false))
        })
    } else if ext == "cr3" {
        read(0, 4 << 20).and_then(|head| {
            let at = head.windows(4).position(|w| w == b"CMT2")? + 4;
            Some((head, at, true))
        })
    } else {
        read(0, 8 << 20).map(|head| (head, 0, false))
    };
    let Some((bytes, base, exif_first)) = found else { return Vec::new() };
    parse_exif(&bytes, base, exif_first)
}

fn parse_exif(b: &[u8], base: usize, exif_first: bool) -> Vec<(u16, rawler::formats::tiff::Value)> {
    use rawler::formats::tiff::Value;
    let get = |o: usize, n: usize| b.get(base + o..base + o + n);
    let Some(head) = get(0, 8) else { return Vec::new() };
    let little = match &head[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Vec::new(),
    };
    let u16_at = |x: &[u8]| if little { u16::from_le_bytes([x[0], x[1]]) } else { u16::from_be_bytes([x[0], x[1]]) };
    let u32_at = |x: &[u8]| if little { u32::from_le_bytes([x[0], x[1], x[2], x[3]]) } else { u32::from_be_bytes([x[0], x[1], x[2], x[3]]) };
    let entries = |offset: usize| -> Vec<(u16, u16, usize, [u8; 4])> {
        let Some(count) = get(offset, 2).map(u16_at) else { return Vec::new() };
        (0..count as usize)
            .filter_map(|i| {
                let e = get(offset + 2 + 12 * i, 12)?;
                Some((u16_at(e), u16_at(&e[2..]), u32_at(&e[4..]) as usize, [e[8], e[9], e[10], e[11]]))
            })
            .collect()
    };
    let first = u32_at(&head[4..]) as usize;
    let exif_offset = if exif_first { Some(first) } else { entries(first).iter().find(|e| e.0 == 0x8769).map(|e| u32_at(&e.3) as usize) };
    let Some(exif_offset) = exif_offset else { return Vec::new() };
    let mut out = Vec::new();
    for (tag, kind, count, inline) in entries(exif_offset) {
        if SKIP_EXIF.contains(&tag) || count == 0 || count > 1 << 16 {
            continue;
        }
        let unit = match kind {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 => 4,
            5 | 10 => 8,
            _ => continue,
        };
        let size = unit * count;
        let data: Vec<u8> = if size <= 4 {
            inline[..size].to_vec()
        } else {
            match get(u32_at(&inline) as usize, size) {
                Some(d) => d.to_vec(),
                None => continue,
            }
        };
        let words = |n: usize| data.chunks_exact(n);
        let value = match kind {
            1 => Value::Byte(data),
            2 => {
                let text = String::from_utf8_lossy(&data).trim_end_matches('\0').to_string();
                if text.trim().is_empty() {
                    continue;
                }
                Value::from(text)
            }
            3 => Value::Short(words(2).map(u16_at).collect()),
            4 => Value::Long(words(4).map(u32_at).collect()),
            5 => Value::Rational(words(8).map(|w| rawler::formats::tiff::Rational::new(u32_at(w), u32_at(&w[4..]))).collect()),
            6 => Value::SByte(data.iter().map(|&v| v as i8).collect()),
            7 => Value::Undefined(data),
            8 => Value::SShort(words(2).map(|w| u16_at(w) as i16).collect()),
            9 => Value::SLong(words(4).map(|w| u32_at(w) as i32).collect()),
            10 => Value::SRational(words(8).map(|w| rawler::formats::tiff::SRational::new(u32_at(w) as i32, u32_at(&w[4..]) as i32)).collect()),
            _ => continue,
        };
        out.push((tag, value));
    }
    out
}
