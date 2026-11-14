//! Export: resize the rendered image, encode it as JPEG / PNG / TIFF and write metadata.
//!
//! The pixels arrive already rendered by the GPU at full resolution (pipeline.rs), as
//! sRGB-encoded 16-bit RGB, so what the file contains is exactly what the viewport shows.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use image::codecs::jpeg::{JpegEncoder, PixelDensity};
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::{self, FilterType};
use image::{ExtendedColorType, ImageBuffer, ImageEncoder, Rgb};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Format {
    Jpeg,
    Png,
    Tiff,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Jpeg, Format::Png, Format::Tiff];

    pub fn label(self) -> &'static str {
        match self {
            Format::Jpeg => "JPEG",
            Format::Png => "PNG",
            Format::Tiff => "TIFF",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::Png => "png",
            Format::Tiff => "tif",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Format> {
        match ext.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Some(Format::Jpeg),
            "png" => Some(Format::Png),
            "tif" | "tiff" => Some(Format::Tiff),
            _ => None,
        }
    }

    /// JPEG is always 8-bit; PNG and TIFF can carry 16 bits per channel.
    pub fn supports_16_bit(self) -> bool {
        self != Format::Jpeg
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Resize {
    Original,
    LongEdge,
    ShortEdge,
    Width,
    Height,
    Megapixels,
    Percent,
}

impl Resize {
    pub const ALL: [Resize; 7] = [
        Resize::Original,
        Resize::LongEdge,
        Resize::ShortEdge,
        Resize::Width,
        Resize::Height,
        Resize::Megapixels,
        Resize::Percent,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Resize::Original => "Original size",
            Resize::LongEdge => "Long edge",
            Resize::ShortEdge => "Short edge",
            Resize::Width => "Width",
            Resize::Height => "Height",
            Resize::Megapixels => "Megapixels",
            Resize::Percent => "Percentage",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Resize::Original => "",
            Resize::Megapixels => " MP",
            Resize::Percent => " %",
            _ => " px",
        }
    }

    pub fn default_value(self) -> f32 {
        match self {
            Resize::Original => 0.0,
            Resize::LongEdge | Resize::Width => 2048.0,
            Resize::ShortEdge | Resize::Height => 1365.0,
            Resize::Megapixels => 12.0,
            Resize::Percent => 50.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Metadata {
    /// Camera, lens, exposure, capture date, artist and copyright.
    All,
    /// Artist and copyright only.
    CopyrightOnly,
    None,
}

impl Metadata {
    pub const ALL: [Metadata; 3] = [Metadata::All, Metadata::CopyrightOnly, Metadata::None];

    pub fn label(self) -> &'static str {
        match self {
            Metadata::All => "All",
            Metadata::CopyrightOnly => "Copyright only",
            Metadata::None => "None",
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub format: Format,
    /// JPEG quality, 1..=100.
    pub quality: u8,
    /// 16 bits per channel (PNG and TIFF only).
    pub sixteen_bit: bool,
    pub resize: Resize,
    /// Meaning depends on `resize`: pixels, megapixels or percent.
    pub resize_value: f32,
    /// Never make the picture larger than the original.
    pub no_enlarge: bool,
    /// Pixels per inch written to the file. Print size only; it does not change the pixels.
    pub ppi: u16,
    pub metadata: Metadata,
    /// Leave GPS position out even when metadata is "All".
    pub remove_location: bool,
    pub artist: String,
    pub copyright: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            format: Format::Jpeg,
            quality: 90,
            sixteen_bit: false,
            resize: Resize::Original,
            resize_value: 0.0,
            no_enlarge: true,
            ppi: 300,
            metadata: Metadata::All,
            remove_location: false,
            artist: String::new(),
            copyright: String::new(),
        }
    }
}

/// What the camera recorded about the shot, read once when the file is opened.
#[derive(Clone, Debug, Default)]
pub struct PhotoMeta {
    pub make: String,
    pub model: String,
    pub lens_model: Option<String>,
    pub exposure_time: Option<(u32, u32)>,
    pub fnumber: Option<(u32, u32)>,
    pub iso: Option<u32>,
    pub focal_length: Option<(u32, u32)>,
    pub exposure_bias: Option<(i32, i32)>,
    pub date_time_original: Option<String>,
    pub artist: Option<String>,
    pub copyright: Option<String>,
    pub gps: Option<Gps>,
}

#[derive(Clone, Debug, Default)]
pub struct Gps {
    /// "N" or "S", then degrees / minutes / seconds as rationals.
    pub latitude_ref: String,
    pub latitude: [(u32, u32); 3],
    /// "E" or "W".
    pub longitude_ref: String,
    pub longitude: [(u32, u32); 3],
    /// 0 = above sea level, 1 = below; metres.
    pub altitude: Option<(u8, (u32, u32))>,
}

/// Output size in pixels for a source of `width` x `height`.
pub fn target_size(settings: &Settings, width: u32, height: u32) -> (u32, u32) {
    let (w, h) = (width.max(1) as f64, height.max(1) as f64);
    let v = settings.resize_value.max(0.0) as f64;
    let mut scale = match settings.resize {
        Resize::Original => 1.0,
        Resize::LongEdge => v / w.max(h),
        Resize::ShortEdge => v / w.min(h),
        Resize::Width => v / w,
        Resize::Height => v / h,
        Resize::Megapixels => (v * 1.0e6 / (w * h)).sqrt(),
        Resize::Percent => v / 100.0,
    };
    if !scale.is_finite() || scale <= 0.0 {
        scale = 1.0;
    }
    if settings.no_enlarge {
        scale = scale.min(1.0);
    }
    // Encoders and viewers get unhappy far beyond this; it is also a sane memory bound.
    scale = scale.min(30000.0 / w.max(h));
    (((w * scale).round() as u32).max(1), ((h * scale).round() as u32).max(1))
}

pub struct Saved {
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

/// Resize, encode and write. `rgb` is sRGB-encoded 16-bit RGB, `width` x `height`, row-major.
pub fn save(path: &Path, rgb: Vec<u16>, width: u32, height: u32, settings: &Settings, meta: &PhotoMeta) -> Result<Saved, String> {
    let image: ImageBuffer<Rgb<u16>, Vec<u16>> =
        ImageBuffer::from_raw(width, height, rgb).ok_or("Rendered image has the wrong size.")?;
    let (tw, th) = target_size(settings, width, height);
    let image = if (tw, th) != (width, height) {
        // Lanczos keeps fine detail when shrinking without the ringing of sharper filters.
        imageops::resize(&image, tw, th, FilterType::Lanczos3)
    } else {
        image
    };

    let sixteen = settings.sixteen_bit && settings.format.supports_16_bit();
    let exif = build_exif(settings, meta, tw, th);
    let file = File::create(path).map_err(|e| format!("Could not create the file: {e}"))?;
    let writer = BufWriter::new(file);
    let failed = |e: image::ImageError| format!("Could not write the image: {e}");

    let samples: &[u16] = image.as_raw();
    let eight_bit = || -> Vec<u8> { samples.iter().map(|v| ((*v as u32 * 255 + 32767) / 65535) as u8).collect() };

    match settings.format {
        Format::Jpeg => {
            let mut encoder = JpegEncoder::new_with_quality(writer, settings.quality.clamp(1, 100));
            encoder.set_pixel_density(PixelDensity::dpi(settings.ppi.max(1)));
            if let Some(exif) = exif {
                let _ = encoder.set_exif_metadata(exif);
            }
            encoder.write_image(&eight_bit(), tw, th, ExtendedColorType::Rgb8).map_err(failed)?;
        }
        Format::Png => {
            // Encoded in memory first: the encoder cannot write the resolution chunk, so it
            // is spliced in afterwards.
            let mut encoded = Vec::new();
            let mut encoder = PngEncoder::new_with_quality(&mut encoded, CompressionType::Default, PngFilter::Adaptive);
            if let Some(exif) = exif {
                let _ = encoder.set_exif_metadata(exif);
            }
            if sixteen {
                encoder.write_image(bytemuck::cast_slice(samples), tw, th, ExtendedColorType::Rgb16).map_err(failed)?;
            } else {
                encoder.write_image(&eight_bit(), tw, th, ExtendedColorType::Rgb8).map_err(failed)?;
            }
            insert_png_resolution(&mut encoded, settings.ppi);
            let mut writer = writer;
            writer.write_all(&encoded).and_then(|_| writer.flush()).map_err(|e| format!("Could not write the file: {e}"))?;
        }
        Format::Tiff => {
            // Written with the TIFF library directly so resolution and the plain text tags
            // (camera, artist, copyright, date) can go in. TIFF gets no exposure details.
            let failed = |e: tiff::TiffError| format!("Could not write the image: {e}");
            let mut encoder = tiff::encoder::TiffEncoder::new(writer).map_err(failed)?;
            let ppi = tiff::encoder::Rational { n: settings.ppi.max(1) as u32, d: 1 };
            let all = settings.metadata == Metadata::All;
            let tags: Vec<(tiff::tags::Tag, String)> = [
                (tiff::tags::Tag::Artist, settings.metadata != Metadata::None, settings.artist.trim().to_owned()),
                (tiff::tags::Tag::Copyright, settings.metadata != Metadata::None, settings.copyright.trim().to_owned()),
                (tiff::tags::Tag::Make, all, meta.make.clone()),
                (tiff::tags::Tag::Model, all, meta.model.clone()),
                (tiff::tags::Tag::Software, all, "Develop".to_owned()),
                (tiff::tags::Tag::DateTime, all, meta.date_time_original.clone().unwrap_or_default()),
            ]
            .into_iter()
            .filter(|(_, wanted, text)| *wanted && !text.is_empty())
            .map(|(tag, _, text)| (tag, text))
            .collect();
            macro_rules! write_tiff {
                ($color:ty, $data:expr) => {{
                    let mut image = encoder.new_image::<$color>(tw, th).map_err(failed)?;
                    image.resolution(tiff::tags::ResolutionUnit::Inch, ppi);
                    for (tag, text) in &tags {
                        image.encoder().write_tag(*tag, text.as_str()).map_err(failed)?;
                    }
                    image.write_data($data).map_err(failed)?;
                }};
            }
            if sixteen {
                write_tiff!(tiff::encoder::colortype::RGB16, samples);
            } else {
                write_tiff!(tiff::encoder::colortype::RGB8, &eight_bit());
            }
        }
    }

    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(Saved { width: tw, height: th, bytes })
}

/// Add a pHYs chunk (pixels per metre) right after the PNG header chunk.
fn insert_png_resolution(png: &mut Vec<u8>, ppi: u16) {
    // 8-byte signature + IHDR chunk (4 length + 4 type + 13 data + 4 CRC).
    const AFTER_HEADER: usize = 8 + 25;
    if png.len() < AFTER_HEADER || &png[12..16] != b"IHDR" {
        return;
    }
    let per_metre = (ppi.max(1) as f64 / 0.0254).round() as u32;
    let mut body = Vec::with_capacity(13);
    body.extend_from_slice(b"pHYs");
    body.extend_from_slice(&per_metre.to_be_bytes());
    body.extend_from_slice(&per_metre.to_be_bytes());
    body.push(1); // unit: metre
    let mut chunk = Vec::with_capacity(21);
    chunk.extend_from_slice(&9u32.to_be_bytes());
    chunk.extend_from_slice(&body);
    chunk.extend_from_slice(&crc32fast::hash(&body).to_be_bytes());
    png.splice(AFTER_HEADER..AFTER_HEADER, chunk);
}

// ---------------------------------------------------------------------------------------
// EXIF: a small TIFF structure (little-endian) with the tags viewers and catalogues read.

struct Field {
    tag: u16,
    kind: u16,
    count: u32,
    data: Vec<u8>,
}

fn ascii(tag: u16, text: &str) -> Field {
    let mut data: Vec<u8> = text.bytes().filter(|b| *b != 0).collect();
    data.push(0);
    Field { tag, kind: 2, count: data.len() as u32, data }
}

fn short(tag: u16, value: u16) -> Field {
    Field { tag, kind: 3, count: 1, data: value.to_le_bytes().to_vec() }
}

fn long(tag: u16, value: u32) -> Field {
    Field { tag, kind: 4, count: 1, data: value.to_le_bytes().to_vec() }
}

fn rationals(tag: u16, values: &[(u32, u32)]) -> Field {
    let mut data = Vec::with_capacity(values.len() * 8);
    for (n, d) in values {
        data.extend_from_slice(&n.to_le_bytes());
        data.extend_from_slice(&(*d).max(1).to_le_bytes());
    }
    Field { tag, kind: 5, count: values.len() as u32, data }
}

fn srational(tag: u16, value: (i32, i32)) -> Field {
    let mut data = value.0.to_le_bytes().to_vec();
    data.extend_from_slice(&(if value.1 == 0 { 1 } else { value.1 }).to_le_bytes());
    Field { tag, kind: 10, count: 1, data }
}

fn bytes(tag: u16, kind: u16, data: &[u8]) -> Field {
    Field { tag, kind, count: data.len() as u32, data: data.to_vec() }
}

/// Size of one directory once written: entry table plus any values too big to sit inline.
fn ifd_size(fields: &[Field]) -> u32 {
    let overflow: usize = fields.iter().filter(|f| f.data.len() > 4).map(|f| f.data.len() + f.data.len() % 2).sum();
    (2 + fields.len() * 12 + 4 + overflow) as u32
}

fn write_ifd(out: &mut Vec<u8>, mut fields: Vec<Field>) {
    fields.sort_by_key(|f| f.tag);
    let start = out.len() as u32;
    let mut overflow_at = start + (2 + fields.len() * 12 + 4) as u32;
    let mut overflow = Vec::new();
    out.extend_from_slice(&(fields.len() as u16).to_le_bytes());
    for f in &fields {
        out.extend_from_slice(&f.tag.to_le_bytes());
        out.extend_from_slice(&f.kind.to_le_bytes());
        out.extend_from_slice(&f.count.to_le_bytes());
        if f.data.len() <= 4 {
            let mut inline = f.data.clone();
            inline.resize(4, 0);
            out.extend_from_slice(&inline);
        } else {
            out.extend_from_slice(&overflow_at.to_le_bytes());
            overflow.extend_from_slice(&f.data);
            if f.data.len() % 2 == 1 {
                overflow.push(0);
            }
            overflow_at = start + (2 + fields.len() * 12 + 4) as u32 + overflow.len() as u32;
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // no further directory
    out.extend_from_slice(&overflow);
}

fn build_exif(settings: &Settings, meta: &PhotoMeta, width: u32, height: u32) -> Option<Vec<u8>> {
    if settings.metadata == Metadata::None {
        return None;
    }
    let all = settings.metadata == Metadata::All;
    let text = |s: &str| !s.trim().is_empty();

    let mut main = Vec::new();
    if text(&settings.artist) {
        main.push(ascii(0x013B, settings.artist.trim()));
    }
    if text(&settings.copyright) {
        main.push(ascii(0x8298, settings.copyright.trim()));
    }

    let mut photo = Vec::new();
    let mut gps = Vec::new();
    if all {
        if text(&meta.make) {
            main.push(ascii(0x010F, &meta.make));
        }
        if text(&meta.model) {
            main.push(ascii(0x0110, &meta.model));
        }
        main.push(short(0x0112, 1)); // the pixels are already upright
        main.push(rationals(0x011A, &[(settings.ppi.max(1) as u32, 1)]));
        main.push(rationals(0x011B, &[(settings.ppi.max(1) as u32, 1)]));
        main.push(short(0x0128, 2)); // inches
        main.push(ascii(0x0131, "Develop"));
        if let Some(date) = &meta.date_time_original {
            main.push(ascii(0x0132, date));
            photo.push(ascii(0x9003, date));
            photo.push(ascii(0x9004, date));
        }

        if let Some(v) = meta.exposure_time {
            photo.push(rationals(0x829A, &[v]));
        }
        if let Some(v) = meta.fnumber {
            photo.push(rationals(0x829D, &[v]));
        }
        if let Some(iso) = meta.iso {
            photo.push(short(0x8827, iso.min(65535) as u16));
        }
        photo.push(bytes(0x9000, 7, b"0232"));
        if let Some(v) = meta.exposure_bias {
            photo.push(srational(0x9204, v));
        }
        if let Some(v) = meta.focal_length {
            photo.push(rationals(0x920A, &[v]));
        }
        photo.push(short(0xA001, 1)); // sRGB
        photo.push(long(0xA002, width));
        photo.push(long(0xA003, height));
        if let Some(lens) = meta.lens_model.as_deref().filter(|l| text(l)) {
            photo.push(ascii(0xA434, lens));
        }

        if let Some(g) = meta.gps.as_ref().filter(|_| !settings.remove_location) {
            gps.push(bytes(0x0000, 1, &[2, 3, 0, 0]));
            gps.push(ascii(0x0001, &g.latitude_ref));
            gps.push(rationals(0x0002, &g.latitude));
            gps.push(ascii(0x0003, &g.longitude_ref));
            gps.push(rationals(0x0004, &g.longitude));
            if let Some((reference, altitude)) = g.altitude {
                gps.push(bytes(0x0005, 1, &[reference]));
                gps.push(rationals(0x0006, &[altitude]));
            }
        }
    }
    if main.is_empty() && photo.is_empty() {
        return None;
    }

    // Layout: header, main directory, photo directory, GPS directory. The main directory
    // holds pointers to the other two, so their positions are worked out first.
    let pointers = (!photo.is_empty()) as usize + (!gps.is_empty()) as usize;
    let main_size = ifd_size(&main) + pointers as u32 * 12;
    let photo_at = 8 + main_size;
    let gps_at = photo_at + if photo.is_empty() { 0 } else { ifd_size(&photo) };
    if !photo.is_empty() {
        main.push(long(0x8769, photo_at));
    }
    if !gps.is_empty() {
        main.push(long(0x8825, gps_at));
    }

    let mut out = Vec::with_capacity(1024);
    out.extend_from_slice(b"II*\0");
    out.extend_from_slice(&8u32.to_le_bytes());
    write_ifd(&mut out, main);
    if !photo.is_empty() {
        write_ifd(&mut out, photo);
    }
    if !gps.is_empty() {
        write_ifd(&mut out, gps);
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------
// Remembered export settings and named presets
// ---------------------------------------------------------------------------------------

/// The last-used export settings, the last export folder and the user's named presets,
/// kept in `export-settings.tsv` next to the per-photo settings.
pub struct Presets {
    file: Option<std::path::PathBuf>,
    pub list: Vec<(String, Settings)>,
    pub last_dir: Option<std::path::PathBuf>,
    written: Option<(Settings, Option<std::path::PathBuf>)>,
}

fn settings_to_line(name: &str, s: &Settings) -> String {
    let clean = |text: &str| text.replace(['\t', '\n', '\r'], " ");
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        clean(name),
        s.format.extension(),
        s.quality,
        s.sixteen_bit as u8,
        s.resize.label(),
        s.resize_value,
        s.no_enlarge as u8,
        s.ppi,
        s.metadata.label(),
        s.remove_location as u8,
        clean(&s.artist),
        clean(&s.copyright)
    )
}

fn settings_from_line(line: &str) -> Option<(String, Settings)> {
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 12 {
        return None;
    }
    let settings = Settings {
        format: Format::from_extension(f[1])?,
        quality: f[2].parse::<u8>().ok()?.clamp(1, 100),
        sixteen_bit: f[3] == "1",
        resize: Resize::ALL.into_iter().find(|r| r.label() == f[4])?,
        resize_value: f[5].parse::<f32>().ok().filter(|v| v.is_finite())?,
        no_enlarge: f[6] == "1",
        ppi: f[7].parse::<u16>().ok()?.max(1),
        metadata: Metadata::ALL.into_iter().find(|m| m.label() == f[8])?,
        remove_location: f[9] == "1",
        artist: f[10].to_owned(),
        copyright: f[11].to_owned(),
    };
    Some((f[0].to_owned(), settings))
}

impl Presets {
    /// Nothing remembered and nothing written (command-line runs).
    pub fn disabled() -> Self {
        Self { file: None, list: Vec::new(), last_dir: None, written: None }
    }

    /// Read the file. Returns the presets and the settings last used, if any.
    pub fn load(file: Option<std::path::PathBuf>) -> (Self, Option<Settings>) {
        let text = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        let mut presets = Self { file, list: Vec::new(), last_dir: None, written: None };
        let mut last = None;
        for line in text.lines() {
            if let Some(dir) = line.strip_prefix("*dir\t") {
                presets.last_dir = Some(std::path::PathBuf::from(dir));
            } else if let Some((name, settings)) = settings_from_line(line) {
                if name == "*last" {
                    last = Some(settings);
                } else {
                    presets.list.push((name, settings));
                }
            }
        }
        presets.written = last.clone().map(|s| (s, presets.last_dir.clone()));
        (presets, last)
    }

    /// Add a preset, replacing one of the same name.
    pub fn add(&mut self, name: &str, settings: &Settings) {
        let name = name.trim();
        if name.is_empty() || name.starts_with('*') {
            return;
        }
        match self.list.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = settings.clone(),
            None => self.list.push((name.to_owned(), settings.clone())),
        }
        self.written = None;
    }

    pub fn remove(&mut self, name: &str) {
        self.list.retain(|(n, _)| n != name);
        self.written = None;
    }

    /// Write the file if the current settings, folder or presets differ from what is on disk.
    pub fn save_if_changed(&mut self, current: &Settings) {
        let Some(file) = &self.file else { return };
        if self.written.as_ref().is_some_and(|(s, d)| s == current && *d == self.last_dir) {
            return;
        }
        let mut text = settings_to_line("*last", current);
        if let Some(dir) = &self.last_dir {
            text.push_str(&format!("*dir\t{}\n", dir.display()));
        }
        for (name, settings) in &self.list {
            text.push_str(&settings_to_line(name, settings));
        }
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let temp = file.with_extension("tmp");
        if std::fs::write(&temp, text).is_ok() {
            let _ = std::fs::rename(&temp, file);
        }
        self.written = Some((current.clone(), self.last_dir.clone()));
    }
}