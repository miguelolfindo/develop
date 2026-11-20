//! Per-photo settings, remembered between photos and between runs.
//!
//! Each edited or rated photo has a small sidecar file next to it (`DSC01234.ARW.dvlp`), so
//! its settings travel with it when the photo is moved or copied together with the sidecar.
//! A folder's sidecars are read when the folder is first shown. Only photos whose settings
//! differ from their defaults, or that have a rating, get a sidecar.
//!
//! `photo-settings.tsv` in the settings folder (see `settings_dir`) is the older
//! central store, keyed by path. It is still read; its entries are moved into sidecars as
//! their folders are opened, and it keeps only photos in folders that cannot be written.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Number of slider values stored per photo (the fields of `Adjust`, in order).
pub const VALUES: usize = 9;
/// How long after the last change the file is written.
const WRITE_DELAY: Duration = Duration::from_millis(1500);

#[derive(Clone, PartialEq, Debug)]
pub struct Saved {
    pub values: [f32; VALUES],
    pub quarter_turns: u8,
    pub profile: String,
    /// AI denoise switched on for this photo.
    pub denoise: bool,
    /// Crop rectangle (see crop.rs) on the picture as stored, and straighten angle in degrees.
    pub crop: [f32; 4],
    pub angle: f32,
    /// Sharpening: amount, radius, detail, masking.
    /// Sharpening (amount, radius, detail, masking), then noise reduction (luminance, color).
    pub sharpen: [f32; 10],
    /// Tint relative to the as-shot white balance: negative = greener, positive = more magenta.
    pub tint: f32,
    /// Texture, clarity, dehaze (-100..100).
    pub presence: [f32; 3],
    pub lens: crate::lens::LensSettings,
    /// Heal and clone spots (see spots.rs).
    pub spots: Vec<crate::spots::Spot>,
    /// Tone curve: the point curve (0..1) and the parametric regions (Highlights, Lights,
    /// Darks, Shadows, -100..100).
    pub curve: Vec<[f32; 2]>,
    pub regions: [f32; 4],
    /// Color Mixer: hue, saturation, luminance per colour (-100..100), see tone::COLOURS.
    pub hsl: [[f32; 8]; 3],
    /// Black & white, and how bright each colour becomes in grey (-100..100, see tone::COLOURS).
    pub mono: bool,
    pub mono_mix: [f32; 8],
    /// Masks: sliders that change only the parts brushed in (see masks.rs).
    pub masks: Vec<crate::masks::Mask>,
}

/// A saved version of a photo's edit: a name, when it was saved (seconds since 1970), and the
/// settings. Versions live in the photo's sidecar, so they travel with it.
#[derive(Clone, PartialEq, Debug)]
pub struct Version {
    pub name: String,
    pub time: u64,
    pub saved: Saved,
}

/// A name made safe for the tab-separated files (no tabs or line breaks), at most 60 characters.
pub fn clean_name(name: &str) -> String {
    name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim().chars().take(60).collect()
}

pub fn now_seconds() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Sharpening a photo starts with (Lightroom's defaults for raw files).
/// Sharpening (amount, radius, detail, masking), then noise reduction: luminance, color,
/// luminance detail, luminance contrast, color detail, color smoothness (as Lightroom's Manual
/// Noise Reduction).
pub const SHARPEN_DEFAULT: [f32; 10] = [40.0, 1.0, 25.0, 0.0, 0.0, 0.0, 50.0, 0.0, 50.0, 50.0];

pub struct Store {
    /// The central file. It only holds photos whose folder cannot take a sidecar, and
    /// photos saved by earlier versions that have not been moved to sidecars yet.
    file: Option<PathBuf>,
    photos: HashMap<PathBuf, Saved>,
    /// Star ratings, 1 to 5 (unrated photos are absent).
    ratings: HashMap<PathBuf, u8>,
    /// Saved versions of each photo's edit (photos without any are absent).
    versions: HashMap<PathBuf, Vec<Version>>,
    /// Keywords and people's names per photo (photos without any are absent).
    keywords: HashMap<PathBuf, Vec<String>>,
    people: HashMap<PathBuf, Vec<String>>,
    /// Photos whose data is in the central file rather than a sidecar.
    central: HashSet<PathBuf>,
    central_changed: bool,
    /// Folders whose sidecars have been read.
    folders: HashSet<PathBuf>,
    /// Photos with changes not yet written.
    dirty: HashSet<PathBuf>,
    changed: Option<Instant>,
    /// Each sidecar's text as Develop last wrote or read it ("" = no sidecar), so a change
    /// to the file can be told apart from Develop's own saves.
    known: HashMap<PathBuf, String>,
}

/// The app's settings folder: `%APPDATA%\Develop` on Windows,
/// `~/Library/Application Support/Develop` on macOS, `~/.config/develop` elsewhere.
pub fn settings_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Develop"))
    } else if cfg!(target_os = "macos") {
        Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Develop"))
    } else if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        Some(PathBuf::from(dir).join("develop"))
    } else {
        Some(PathBuf::from(std::env::var_os("HOME")?).join(".config/develop"))
    }
}

/// Where earlier versions kept their settings (`SonyRawProcessor`).
fn legacy_settings_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(dir)
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join(".config")
    };
    Some(base.join("SonyRawProcessor"))
}

/// Copy settings from the folder earlier versions used into the current one, once: only
/// files the new folder does not have yet. The old folder is left as it was.
pub fn migrate_legacy_settings() {
    let (Some(old), Some(new)) = (legacy_settings_dir(), settings_dir()) else { return };
    if !old.is_dir() || new.join("photo-settings.tsv").exists() {
        return;
    }
    fn copy_missing(from: &Path, to: &Path) {
        let _ = std::fs::create_dir_all(to);
        for entry in std::fs::read_dir(from).into_iter().flatten().filter_map(|e| e.ok()) {
            let (src, dst) = (entry.path(), to.join(entry.file_name()));
            if src.is_dir() {
                copy_missing(&src, &dst);
            } else if !dst.exists() {
                let _ = std::fs::copy(&src, &dst);
            }
        }
    }
    copy_missing(&old, &new);
}

fn store_file() -> Option<PathBuf> {
    Some(settings_dir()?.join("photo-settings.tsv"))
}

/// Where the export settings and presets are kept.
pub fn export_file() -> Option<PathBuf> {
    Some(store_file()?.parent()?.join("export-settings.tsv"))
}

/// Where the edited thumbnail of `photo` is kept (a small JPEG named after the path).
pub fn thumb_file(photo: &Path) -> Option<PathBuf> {
    // FNV-1a: stable between builds, unlike the standard library's hasher.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in photo.to_string_lossy().to_lowercase().bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(store_file()?.parent()?.join("thumbs").join(format!("{hash:016x}.jpg")))
}

fn parse_line(line: &str) -> Option<(PathBuf, Saved)> {
    let mut fields = line.split('\t');
    let path = fields.next().filter(|p| !p.is_empty())?;
    let mut values = [0.0f32; VALUES];
    for value in &mut values {
        *value = fields.next()?.parse().ok().filter(|v: &f32| v.is_finite())?;
    }
    let quarter_turns = fields.next()?.parse::<u8>().ok()?;
    let profile = fields.next().unwrap_or("").to_owned();
    let denoise = fields.next() == Some("1");
    let mut number = || fields.next().and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite());
    let crop = match (number(), number(), number(), number()) {
        (Some(x0), Some(y0), Some(x1), Some(y1)) if x1 > x0 && y1 > y0 => [x0, y0, x1, y1],
        _ => crate::crop::FULL,
    };
    let angle = number().unwrap_or(0.0).clamp(-45.0, 45.0);
    let sharpen = match (number(), number(), number(), number()) {
        (Some(a), Some(r), Some(d), Some(m)) => [a.clamp(0.0, 150.0), r.clamp(0.5, 3.0), d.clamp(0.0, 100.0), m.clamp(0.0, 100.0), 0.0, 0.0, 50.0, 0.0, 50.0, 50.0],
        _ => SHARPEN_DEFAULT,
    };
    let tint = number().unwrap_or(0.0).clamp(-150.0, 150.0);
    let presence = [0; 3].map(|_| number().unwrap_or(0.0).clamp(-100.0, 100.0));
    let lens = crate::lens::LensSettings::from_fields(&mut fields);
    let spots = fields.next().map(crate::spots::from_field).unwrap_or_default();
    let curve = fields.next().map(crate::tone::curve_from_field).unwrap_or_else(crate::tone::identity_curve);
    let regions = fields.next().map(|f| crate::tone::numbers_from_field::<4>(f, -100.0, 100.0)).unwrap_or_default();
    let hsl_flat = fields.next().map(|f| crate::tone::numbers_from_field::<24>(f, -100.0, 100.0)).unwrap_or([0.0; 24]);
    let hsl = std::array::from_fn(|k| std::array::from_fn(|i| hsl_flat[k * 8 + i]));
    let mono = fields.next() == Some("1");
    let mono_mix = fields.next().map(|f| crate::tone::numbers_from_field::<8>(f, -100.0, 100.0)).unwrap_or_default();
    // Noise reduction, at the end of the line (added later: older lines have none, and an
    // older Develop ignores it).
    let mut sharpen = sharpen;
    for (i, value) in sharpen.iter_mut().enumerate().skip(4) {
        *value = fields.next().and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite()).unwrap_or(SHARPEN_DEFAULT[i]).clamp(0.0, 100.0);
    }
    // Masks, after that (added in 0.6.0).
    let masks = fields.next().map(crate::masks::from_field).unwrap_or_default();
    Some((PathBuf::from(path), Saved { values, quarter_turns, profile, denoise, crop, angle, sharpen, tint, presence, lens, spots, curve, regions, hsl, mono, mono_mix, masks }))
}

/// What identifies a version's thumbnail: the photo, the version's name and when it was saved
/// (updating a version gives it a new time, so its thumbnail is drawn again).
pub fn version_key(photo: &Path, version: &Version) -> String {
    format!("{}|{}|{}", photo.display(), version.name, version.time)
}

/// Where a version's thumbnail is kept (beside the photos' own thumbnails).
pub fn version_thumb_file(key: &str) -> Option<PathBuf> {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.to_lowercase().bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(store_file()?.parent()?.join("thumbs").join(format!("v{hash:016x}.jpg")))
}

/// The settings saved in `photo`'s sidecar, read directly (command-line and MCP use).
pub fn read_sidecar(photo: &Path) -> Option<Saved> {
    std::fs::read_to_string(sidecar_path(photo)).ok().and_then(|text| parse_sidecar(&text).saved)
}

/// A photo's rating, keywords and people, read directly from its sidecar (for the library
/// index). A photo without a sidecar has none.
pub fn read_tags(photo: &Path) -> (u8, Vec<String>, Vec<String>) {
    std::fs::read_to_string(sidecar_path(photo)).map(|text| {
        let s = parse_sidecar(&text);
        (s.rating, s.keywords, s.people)
    }).unwrap_or_default()
}

/// The sidecar that holds `photo`'s settings and rating: `DSC01234.ARW.dvlp` beside it.
pub fn sidecar_path(photo: &Path) -> PathBuf {
    sidecar_with(photo, SIDECAR_EXTENSION)
}

fn sidecar_with(photo: &Path, extension: &str) -> PathBuf {
    let mut name = photo.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".");
    name.push(extension);
    photo.with_file_name(name)
}

pub const SIDECAR_EXTENSION: &str = "dvlp";
/// Earlier versions' sidecar extension; such files are read and replaced by `.dvlp` ones.
const LEGACY_SIDECAR_EXTENSION: &str = "srp";
const SIDECAR_HEADER: &str = "Develop settings\t1";

/// The settings fields of a stored line (everything after the path).
fn saved_fields(s: &Saved) -> String {
    let values: Vec<String> = s.values.iter().map(|v| v.to_string()).collect();
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        values.join("\t"),
        s.quarter_turns,
        s.profile,
        s.denoise as u8,
        s.crop[0],
        s.crop[1],
        s.crop[2],
        s.crop[3],
        s.angle,
        s.sharpen[0],
        s.sharpen[1],
        s.sharpen[2],
        s.sharpen[3],
        s.tint,
        s.presence[0],
        s.presence[1],
        s.presence[2],
        s.lens.to_fields(),
        crate::spots::to_field(&s.spots),
        crate::tone::curve_to_field(&s.curve),
        crate::tone::numbers_to_field(&s.regions),
        crate::tone::numbers_to_field(&s.hsl.concat()),
        s.mono as u8,
        crate::tone::numbers_to_field(&s.mono_mix),
        s.sharpen[4],
        s.sharpen[5],
        s.sharpen[6],
        s.sharpen[7],
        s.sharpen[8],
        s.sharpen[9],
        crate::masks::to_field(&s.masks)
    )
}

/// What a sidecar holds.
#[derive(Default)]
pub struct Sidecar {
    pub saved: Option<Saved>,
    pub rating: u8,
    pub versions: Vec<Version>,
    /// Keywords, in the order added.
    pub keywords: Vec<String>,
    /// Names of the people in the photo (see people.rs); plain words, no face data.
    pub people: Vec<String>,
}

/// Words for the keywords and people lines: no tabs or line breaks, at most 60 characters, no
/// empty ones, and no repeats (ignoring case).
pub fn clean_words(words: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in words {
        let w = clean_name(&w);
        if !w.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(&w)) {
            out.push(w);
        }
    }
    out
}

fn parse_sidecar(text: &str) -> Sidecar {
    let mut out = Sidecar::default();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("settings\t") {
            out.saved = parse_line(&format!("photo\t{rest}")).map(|(_, s)| s);
        } else if let Some(stars) = line.strip_prefix("rating\t") {
            out.rating = stars.trim().parse::<u8>().unwrap_or(0).min(5);
        } else if let Some(rest) = line.strip_prefix("keywords\t") {
            out.keywords = clean_words(rest.split('\t').map(str::to_owned));
        } else if let Some(rest) = line.strip_prefix("people\t") {
            out.people = clean_words(rest.split('\t').map(str::to_owned));
        } else if let Some(rest) = line.strip_prefix("version\t") {
            // version <tab> name <tab> time <tab> settings fields
            let mut parts = rest.splitn(3, '\t');
            if let (Some(name), Some(time), Some(fields)) = (parts.next(), parts.next(), parts.next()) {
                if let Some((_, saved)) = parse_line(&format!("photo\t{fields}")) {
                    out.versions.push(Version { name: clean_name(name), time: time.trim().parse().unwrap_or(0), saved });
                }
            }
        }
    }
    out
}

/// Write a file by writing a temporary one and swapping it in, so a crash mid-write cannot
/// leave half a file.
fn write_whole(file: &Path, text: &str) -> bool {
    let mut temp_name = file.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    temp_name.push(".tmp");
    let temp = file.with_file_name(temp_name);
    if std::fs::write(&temp, text).is_err() {
        return false;
    }
    if std::fs::rename(&temp, file).is_err() {
        let _ = std::fs::remove_file(&temp);
        return false;
    }
    true
}

/// Put these names in the photo's `.dvlp`, changing only its `people` line (for photos whose
/// folder isn't open: no need to read the whole folder). An old-style sidecar is carried over.
/// Returns whether the file now says so.
pub fn write_people_line(photo: &Path, names: &[String]) -> bool {
    let sidecar = sidecar_path(photo);
    let legacy = sidecar_with(photo, LEGACY_SIDECAR_EXTENSION);
    let old = std::fs::read_to_string(&sidecar).or_else(|_| std::fs::read_to_string(&legacy)).unwrap_or_default();
    let names = clean_words(names.iter().cloned());
    let mut lines: Vec<String> = old.lines().filter(|l| !l.trim().is_empty() && !l.starts_with("people	")).map(str::to_owned).collect();
    if lines.first().map(String::as_str) != Some(SIDECAR_HEADER) {
        lines.retain(|l| l != SIDECAR_HEADER);
        lines.insert(0, SIDECAR_HEADER.to_owned());
    }
    if !names.is_empty() {
        lines.push(format!("people	{}", names.join("	")));
    }
    if lines.len() == 1 {
        // Nothing left to remember.
        let _ = std::fs::remove_file(&sidecar);
        return true;
    }
    let text = lines.join("
") + "
";
    if text == old {
        return true;
    }
    let ok = write_whole(&sidecar, &text);
    if ok {
        let _ = std::fs::remove_file(&legacy);
    }
    ok
}

impl Store {
    /// Whether this folder's sidecars have been read (its photos' settings are held here).
    pub fn has_folder(&self, dir: &Path) -> bool {
        self.folders.contains(dir)
    }

    pub fn load() -> Self {
        let file = store_file();
        let text = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        let mut photos = HashMap::new();
        let mut ratings = HashMap::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("*rating\t") {
                if let Some((path, stars)) = rest.rsplit_once('\t') {
                    if let Ok(stars @ 1..=5) = stars.parse::<u8>() {
                        ratings.insert(PathBuf::from(path), stars);
                    }
                }
            } else if let Some((path, saved)) = parse_line(line) {
                photos.insert(path, saved);
            }
        }
        let central = photos.keys().chain(ratings.keys()).cloned().collect();
        Self { file, photos, ratings, versions: HashMap::new(), keywords: HashMap::new(), people: HashMap::new(), central, central_changed: false, folders: HashSet::new(), dirty: HashSet::new(), changed: None, known: HashMap::new() }
    }

    /// Read the sidecars of a folder (once), and move any settings the central file still
    /// holds for its photos into sidecars.
    pub fn load_folder(&mut self, dir: &Path) {
        if self.folders.contains(dir) {
            return;
        }
        let t = std::time::Instant::now();
        self.load_folder_inner(dir);
        crate::timing("edits (.dvlp) read", t);
    }

    fn load_folder_inner(&mut self, dir: &Path) {
        self.folders.insert(dir.to_path_buf());
        for entry in std::fs::read_dir(dir).into_iter().flatten().filter_map(|e| e.ok()) {
            let path = entry.path();
            let extension = path.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            let legacy = extension == LEGACY_SIDECAR_EXTENSION;
            if extension != SIDECAR_EXTENSION && !legacy {
                continue;
            }
            let photo = path.with_extension(""); // DSC01234.ARW.dvlp -> DSC01234.ARW
            // An old-style sidecar is only used when there is no current one; it is then
            // written out as `.dvlp` (and removed) on the next save.
            if legacy {
                if sidecar_path(&photo).exists() {
                    continue;
                }
                self.touch(&photo);
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            if !legacy {
                self.known.insert(photo.clone(), text.clone());
            }
            self.take_sidecar(&photo, parse_sidecar(&text));
            // The sidecar is the truth; a central entry for the same photo is dropped.
            if self.central.remove(&photo) {
                self.central_changed = true;
                self.changed = Some(Instant::now());
            }
        }
        let to_move: Vec<PathBuf> = self.central.iter().filter(|p| p.parent() == Some(dir) && p.is_file()).cloned().collect();
        for photo in to_move {
            self.touch(&photo);
        }
    }

    /// Read `photo`'s sidecar again after another program changed it (an AI assistant through
    /// the MCP server, another copy of Develop). Its settings, rating and versions replace what
    /// is remembered, even changes not yet written: the newer change wins. Returns true when
    /// anything changed; Develop's own saves and unchanged files return false.
    pub fn reload_sidecar(&mut self, photo: &Path) -> bool {
        let text = std::fs::read_to_string(sidecar_path(photo)).unwrap_or_default();
        // No sidecar and none known: nothing was removed (a photo kept in the central file
        // because its folder cannot be written has none).
        if self.known.get(photo) == Some(&text) || (text.is_empty() && !self.known.contains_key(photo)) {
            return false;
        }
        self.known.insert(photo.to_path_buf(), text.clone());
        let side = parse_sidecar(&text);
        let changed = self.photos.get(photo) != side.saved.as_ref()
            || self.rating(photo) != side.rating
            || self.versions(photo) != side.versions.as_slice()
            || self.keywords(photo) != side.keywords.as_slice()
            || self.people(photo) != side.people.as_slice();
        self.take_sidecar(photo, side);
        self.dirty.remove(photo);
        if !text.is_empty() && self.central.remove(photo) {
            self.central_changed = true;
            self.changed = Some(Instant::now());
        }
        changed
    }

    /// Read a folder's sidecars, and read again any that changed since (for the MCP server,
    /// which runs beside the app: edits made in the app must not be written over).
    pub fn load_folder_fresh(&mut self, dir: &Path) {
        if self.folders.contains(dir) {
            for photo in self.photos_in(dir) {
                self.reload_sidecar(&photo);
            }
        } else {
            self.load_folder(dir);
        }
    }

    /// Every photo of `dir` that has a sidecar or that Develop remembers something for: the
    /// ones to read again when too much changed in the folder to be told which.
    pub fn photos_in(&self, dir: &Path) -> Vec<PathBuf> {
        let mut photos: HashSet<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(SIDECAR_EXTENSION)))
            .map(|p| p.with_extension(""))
            .collect();
        let remembered = self.photos.keys().chain(self.ratings.keys()).chain(self.versions.keys()).chain(self.keywords.keys()).chain(self.people.keys()).chain(self.known.keys());
        photos.extend(remembered.filter(|p| p.parent() == Some(dir)).cloned());
        photos.into_iter().collect()
    }

    /// Remember what a sidecar holds for `photo`.
    fn take_sidecar(&mut self, photo: &Path, side: Sidecar) {
        let photo = photo.to_path_buf();
        match side.versions.is_empty() {
            true => self.versions.remove(&photo),
            false => self.versions.insert(photo.clone(), side.versions),
        };
        match side.saved {
            Some(saved) => self.photos.insert(photo.clone(), saved),
            None => self.photos.remove(&photo),
        };
        match side.rating {
            0 => self.ratings.remove(&photo),
            stars => self.ratings.insert(photo.clone(), stars),
        };
        match side.keywords.is_empty() {
            true => self.keywords.remove(&photo),
            false => self.keywords.insert(photo.clone(), side.keywords),
        };
        match side.people.is_empty() {
            true => self.people.remove(&photo),
            false => self.people.insert(photo, side.people),
        };
    }

    /// The photo's keywords, in the order added.
    pub fn keywords(&self, photo: &Path) -> &[String] {
        self.keywords.get(photo).map_or(&[], |v| v.as_slice())
    }

    /// Replace the photo's keywords.
    pub fn set_keywords(&mut self, photo: &Path, words: Vec<String>) {
        let words = clean_words(words);
        if words.as_slice() == self.keywords(photo) {
            return;
        }
        match words.is_empty() {
            true => self.keywords.remove(photo),
            false => self.keywords.insert(photo.to_path_buf(), words),
        };
        self.touch(photo);
    }

    /// Names of the people in the photo.
    pub fn people(&self, photo: &Path) -> &[String] {
        self.people.get(photo).map_or(&[], |v| v.as_slice())
    }

    /// Replace the names of the people in the photo.
    pub fn set_people(&mut self, photo: &Path, names: Vec<String>) {
        let names = clean_words(names);
        if names.as_slice() == self.people(photo) {
            return;
        }
        match names.is_empty() {
            true => self.people.remove(photo),
            false => self.people.insert(photo.to_path_buf(), names),
        };
        self.touch(photo);
    }

    fn touch(&mut self, photo: &Path) {
        self.dirty.insert(photo.to_path_buf());
        self.changed = Some(Instant::now());
    }

    pub fn get(&self, photo: &Path) -> Option<&Saved> {
        self.photos.get(photo)
    }

    /// Store settings for a photo that is not open (pasted settings).
    pub fn put(&mut self, photo: &Path, saved: Saved) {
        self.photos.insert(photo.to_path_buf(), saved);
        self.touch(photo);
    }

    /// Star rating, 0 = unrated.
    pub fn rating(&self, photo: &Path) -> u8 {
        self.ratings.get(photo).copied().unwrap_or(0)
    }

    pub fn set_rating(&mut self, photo: &Path, stars: u8) {
        let changed = if stars == 0 {
            self.ratings.remove(photo).is_some()
        } else {
            self.ratings.insert(photo.to_path_buf(), stars.min(5)) != Some(stars.min(5))
        };
        if changed {
            self.touch(photo);
        }
    }

    /// The photo's saved versions, oldest first.
    pub fn versions(&self, photo: &Path) -> &[Version] {
        self.versions.get(photo).map_or(&[], |v| v.as_slice())
    }

    /// Replace the photo's saved versions.
    pub fn set_versions(&mut self, photo: &Path, versions: Vec<Version>) {
        if versions.as_slice() == self.versions(photo) {
            return;
        }
        match versions.is_empty() {
            true => self.versions.remove(photo),
            false => self.versions.insert(photo.to_path_buf(), versions),
        };
        self.touch(photo);
    }

    pub fn contains(&self, photo: &Path) -> bool {
        self.photos.contains_key(photo)
    }

    /// Record `photo`'s current settings; a photo back at its defaults is forgotten.
    pub fn update(&mut self, photo: &Path, current: &Saved, defaults: &Saved) {
        if current == defaults {
            if self.photos.remove(photo).is_some() {
                self.touch(photo);
            }
        } else if self.photos.get(photo) != Some(current) {
            self.photos.insert(photo.to_path_buf(), current.clone());
            self.touch(photo);
        }
    }

    /// True while there are changes not yet written.
    pub fn pending(&self) -> bool {
        self.changed.is_some()
    }

    /// Write changed photos' sidecars once changes have settled, or at once with `now`.
    pub fn flush(&mut self, now: bool) {
        let Some(changed) = self.changed else { return };
        if !now && changed.elapsed() < WRITE_DELAY {
            return;
        }
        self.changed = None;
        for photo in std::mem::take(&mut self.dirty) {
            let sidecar = sidecar_path(&photo);
            let (saved, rating) = (self.photos.get(&photo), self.rating(&photo));
            let legacy = sidecar_with(&photo, LEGACY_SIDECAR_EXTENSION);
            let versions = self.versions(&photo);
            let (keywords, people) = (self.keywords(&photo), self.people(&photo));
            if saved.is_none() && rating == 0 && versions.is_empty() && keywords.is_empty() && people.is_empty() {
                // Nothing left to remember: no sidecar, no central entry.
                let _ = std::fs::remove_file(&sidecar);
                let _ = std::fs::remove_file(&legacy);
                self.central_changed |= self.central.remove(&photo);
                self.known.insert(photo, String::new());
                continue;
            }
            let mut text = format!("{SIDECAR_HEADER}\n");
            if let Some(saved) = saved {
                text.push_str(&format!("settings\t{}\n", saved_fields(saved)));
            }
            if rating > 0 {
                text.push_str(&format!("rating\t{rating}\n"));
            }
            if !keywords.is_empty() {
                text.push_str(&format!("keywords\t{}\n", keywords.join("\t")));
            }
            if !people.is_empty() {
                text.push_str(&format!("people\t{}\n", people.join("\t")));
            }
            for v in versions {
                text.push_str(&format!("version\t{}\t{}\t{}\n", clean_name(&v.name), v.time, saved_fields(&v.saved)));
            }
            if write_whole(&sidecar, &text) {
                self.central_changed |= self.central.remove(&photo);
                let _ = std::fs::remove_file(&legacy);
                self.known.insert(photo, text);
            } else {
                // The folder cannot be written (a card, a read-only disk): keep it centrally.
                self.central.insert(photo);
                self.central_changed = true;
            }
        }
        if !self.central_changed {
            return;
        }
        self.central_changed = false;
        let Some(file) = &self.file else { return };
        let mut lines: Vec<String> = Vec::new();
        for photo in &self.central {
            if let Some(saved) = self.photos.get(photo) {
                lines.push(format!("{}\t{}\n", photo.display(), saved_fields(saved)));
            }
            if let Some(stars) = self.ratings.get(photo) {
                lines.push(format!("*rating\t{}\t{stars}\n", photo.display()));
            }
        }
        lines.sort();
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        write_whole(file, &lines.concat());
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.flush(true);
    }
}

/// What was showing when the app was last closed: the folder in the filmstrip and the open
/// photo. Kept in `session.tsv`.
pub struct Session {
    file: Option<PathBuf>,
    pub folder: Option<PathBuf>,
    pub photo: Option<PathBuf>,
}

impl Session {
    pub fn load() -> Self {
        let file = store_file().and_then(|f| Some(f.parent()?.join("session.tsv")));
        let text = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        let value = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)).filter(|v| !v.is_empty()).map(PathBuf::from);
        Self { folder: value("folder\t"), photo: value("photo\t"), file }
    }

    /// Record the current folder and photo, writing the file when either changed.
    pub fn remember(&mut self, folder: Option<&Path>, photo: Option<&Path>) {
        if self.folder.as_deref() == folder && self.photo.as_deref() == photo {
            return;
        }
        self.folder = folder.map(Path::to_path_buf);
        self.photo = photo.map(Path::to_path_buf);
        let Some(file) = &self.file else { return };
        let show = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, format!("folder\t{}\nphoto\t{}\n", show(&self.folder), show(&self.photo)));
    }
}
/// How the window was laid out: which left panel, the panels' widths, the filmstrip's height,
/// the grid view and its thumbnail size. Kept in `layout.tsv`.
pub struct Layout {
    file: Option<PathBuf>,
    /// 0: Folders, 1: Faces & Metadata.
    pub left_tab: u8,
    pub left_width: f32,
    pub right_width: f32,
    pub strip_height: f32,
    pub grid: bool,
    pub grid_card: f32,
    written: String,
}

impl Layout {
    pub fn load() -> Self {
        let file = store_file().and_then(|f| Some(f.parent()?.join("layout.tsv")));
        let text = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        let value = |key: &str, default: f32| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('\t')?.trim().parse::<f32>().ok()).unwrap_or(default);
        let mut l = Self {
            left_tab: value("left_tab", 0.0) as u8,
            left_width: value("left_width", 240.0),
            right_width: value("right_width", 300.0),
            strip_height: value("strip_height", 0.0),
            grid: value("grid", 0.0) > 0.5,
            grid_card: value("grid_card", 160.0),
            written: String::new(),
            file,
        };
        l.written = l.text();
        l
    }

    fn text(&self) -> String {
        format!(
            "left_tab\t{}\nleft_width\t{:.0}\nright_width\t{:.0}\nstrip_height\t{:.0}\ngrid\t{}\ngrid_card\t{:.0}\n",
            self.left_tab, self.left_width, self.right_width, self.strip_height, self.grid as u8, self.grid_card
        )
    }

    /// Write the file if anything changed since it was last written.
    pub fn save_if_changed(&mut self) {
        let text = self.text();
        if text == self.written {
            return;
        }
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(file, &text).is_ok() {
            self.written = text;
        }
    }
}

/// The release the user chose to skip in the update dialog, kept in `update-skip.txt`.
pub fn skipped_update() -> Option<String> {
    let file = store_file()?.parent()?.join("update-skip.txt");
    std::fs::read_to_string(file).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

pub fn skip_update(version: &str) {
    if let Some(file) = store_file().and_then(|f| Some(f.parent()?.join("update-skip.txt"))) {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, version);
    }
}

/// Network computers and shares the user added or opened photos from, kept in
/// `network.tsv` (one address per line, such as `\\server` or `\\server\photos`).
pub struct NetworkPlaces {
    file: Option<PathBuf>,
    pub list: Vec<String>,
}

impl NetworkPlaces {
    pub fn load() -> Self {
        let file = store_file().and_then(|f| Some(f.parent()?.join("network.tsv")));
        let text = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        let list = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
        Self { file, list }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, self.list.iter().map(|l| format!("{l}\n")).collect::<String>());
    }

    /// Add an address (ignoring case for duplicates). Returns true when it was new.
    pub fn add(&mut self, address: &str) -> bool {
        let address = address.trim();
        if address.is_empty() || self.list.iter().any(|l| l.eq_ignore_ascii_case(address)) {
            return false;
        }
        self.list.push(address.to_owned());
        self.save();
        true
    }

    /// Forget every address for `host`.
    pub fn remove_host(&mut self, host: &str) {
        self.list.retain(|l| crate::network::parse_address(l).map_or(true, |(h, _)| !h.eq_ignore_ascii_case(host)));
        self.save();
    }
}

/// Crop ratios the user saved, kept in `crop-presets.tsv` (name, width, height per line).
pub struct CropPresets {
    file: Option<PathBuf>,
    pub list: Vec<(String, f32, f32)>,
}

impl CropPresets {
    pub fn load() -> Self {
        let file = store_file().and_then(|f| Some(f.parent()?.join("crop-presets.tsv")));
        let text = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        let list = text
            .lines()
            .filter_map(|line| {
                let mut f = line.split('\t');
                let name = f.next().filter(|n| !n.is_empty())?.to_owned();
                let w = f.next()?.parse::<f32>().ok().filter(|v| *v > 0.0)?;
                let h = f.next()?.parse::<f32>().ok().filter(|v| *v > 0.0)?;
                Some((name, w, h))
            })
            .collect();
        Self { file, list }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text: String = self.list.iter().map(|(n, w, h)| format!("{}\t{w}\t{h}\n", n.replace(['\t', '\n'], " "))).collect();
        let _ = std::fs::write(file, text);
    }

    /// Add a ratio, replacing one of the same name.
    pub fn add(&mut self, name: &str, w: f32, h: f32) {
        let name = name.trim();
        if name.is_empty() || w <= 0.0 || h <= 0.0 {
            return;
        }
        match self.list.iter_mut().find(|(n, _, _)| n == name) {
            Some(entry) => *entry = (name.to_owned(), w, h),
            None => self.list.push((name.to_owned(), w, h)),
        }
        self.save();
    }

    pub fn remove(&mut self, name: &str) {
        self.list.retain(|(n, _, _)| n != name);
        self.save();
    }
}

/// A develop preset: a name, whether it carries white balance, and the settings it applies.
#[derive(Clone, PartialEq, Debug)]
pub struct Preset {
    pub name: String,
    /// Apply the preset's white balance (temperature and tint) too.
    pub white_balance: bool,
    pub saved: Saved,
}

/// The user's develop presets, kept in `develop-presets.tsv` in the settings folder.
pub struct Presets {
    file: Option<PathBuf>,
    pub list: Vec<Preset>,
}

impl Presets {
    pub fn load() -> Self {
        let file = settings_dir().map(|d| d.join("develop-presets.tsv"));
        let text = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        let list = text
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\t');
                let (name, wb, fields) = (parts.next()?, parts.next()?, parts.next()?);
                let (_, saved) = parse_line(&format!("photo\t{fields}"))?;
                Some(Preset { name: clean_name(name), white_balance: wb == "1", saved })
            })
            .collect();
        Self { file, list }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text: String = self.list.iter().map(|p| format!("{}\t{}\t{}\n", clean_name(&p.name), p.white_balance as u8, saved_fields(&p.saved))).collect();
        write_whole(file, &text);
    }

    /// Add a preset, replacing one with the same name.
    pub fn add(&mut self, preset: Preset) {
        self.list.retain(|p| !p.name.eq_ignore_ascii_case(&preset.name));
        self.list.push(preset);
        self.list.sort_by_key(|p| p.name.to_lowercase());
        self.save();
    }

    pub fn remove(&mut self, name: &str) {
        self.list.retain(|p| p.name != name);
        self.save();
    }

    pub fn get(&self, name: &str) -> Option<&Preset> {
        self.list.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }
}

impl Preset {
    /// `target` with this preset applied: the tone, presence, colour, curve, mixer, detail and
    /// profile settings (and white balance if the preset carries it). Crop, rotation, lens
    /// correction, retouching and AI denoise stay as they are.
    pub fn apply_to(&self, target: &Saved) -> Saved {
        let p = &self.saved;
        let mut out = target.clone();
        for i in 1..VALUES {
            out.values[i] = p.values[i];
        }
        if self.white_balance {
            out.values[0] = p.values[0];
            out.tint = p.tint;
        }
        if !p.profile.is_empty() {
            out.profile = p.profile.clone();
        }
        out.sharpen = p.sharpen;
        out.presence = p.presence;
        out.curve = p.curve.clone();
        out.regions = p.regions;
        out.hsl = p.hsl;
        out.mono = p.mono;
        out.mono_mix = p.mono_mix;
        out
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_people_line_changes_alone() {
        let dir = std::env::temp_dir().join(format!("develop-people-line-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let photo = dir.join("DSC00001.ARW");
        // A sidecar with a rating and keywords, and someone already named.
        std::fs::write(sidecar_path(&photo), "Develop settings	1
rating	4
keywords	beach
people	Kaye
").unwrap();
        assert!(write_people_line(&photo, &["Talia".to_owned(), "Kaye".to_owned()]));
        let (rating, keywords, people) = read_tags(&photo);
        assert_eq!((rating, keywords), (4, vec!["beach".to_owned()]));
        let mut people = people;
        people.sort();
        assert_eq!(people, vec!["Kaye".to_owned(), "Talia".to_owned()]);
        // No sidecar yet: one is made with just the names.
        let other = dir.join("DSC00002.ARW");
        assert!(write_people_line(&other, &["Talia".to_owned()]));
        assert_eq!(read_tags(&other).2, vec!["Talia".to_owned()]);
        // No names and nothing else: no sidecar.
        assert!(write_people_line(&other, &[]));
        assert!(!sidecar_path(&other).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;

    fn sample() -> Saved {
        let line = format!("photo\t{}", "5500\t0.5\t1\t0\t0\t0\t0\t0\t0\t0\tStandard");
        parse_line(&line).map(|(_, s)| s).expect("sample settings")
    }

    #[test]
    fn keywords_and_people_round_trip() {
        let text = format!("{SIDECAR_HEADER}
rating	3
keywords	wedding	Wedding	car	 
people	Talia	Miguel
");
        let side = parse_sidecar(&text);
        assert_eq!(side.rating, 3);
        assert_eq!(side.keywords, vec!["wedding".to_owned(), "car".to_owned()], "repeats and blanks dropped");
        assert_eq!(side.people, vec!["Talia".to_owned(), "Miguel".to_owned()]);
        assert!(side.saved.is_none());
    }

    #[test]
    fn masks_round_trip() {
        let mut a = sample();
        let mut m = crate::masks::Mask::new("Sky".into());
        m.values[crate::masks::EXPOSURE] = -1.0;
        m.strokes.push(crate::masks::Stroke { points: vec![[0.2, 0.1], [0.6, 0.12]], radius: 0.1, feather: 0.5, flow: 1.0, erase: false, auto: true });
        a.masks.push(m);
        let (_, back) = parse_line(&format!("photo	{}", saved_fields(&a))).expect("parses");
        assert_eq!(back.masks, a.masks);
        assert_eq!(back.sharpen, a.sharpen, "fields before the masks still read");
        // A line from before masks has none.
        assert!(sample().masks.is_empty());
    }

    #[test]
    fn versions_round_trip() {
        let mut a = sample();
        a.values[1] = 1.25;
        a.mono = true;
        a.mono_mix[2] = 40.0;
        let text = format!("{SIDECAR_HEADER}\nsettings\t{}\nversion\tWarm look\t1700000000\t{}\n", saved_fields(&a), saved_fields(&a));
        let side = parse_sidecar(&text);
        let (saved, versions) = (side.saved, side.versions);
        assert_eq!(saved.as_ref(), Some(&a));
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].name, "Warm look");
        assert_eq!(versions[0].time, 1_700_000_000);
        assert_eq!(versions[0].saved, a);
    }

    #[test]
    fn preset_keeps_crop_and_spots() {
        let mut look = sample();
        look.values[1] = 2.0;
        look.crop = [0.1, 0.1, 0.9, 0.9];
        let preset = Preset { name: "Look".into(), white_balance: false, saved: look };
        let mut target = sample();
        target.crop = [0.0, 0.2, 1.0, 0.8];
        target.values[0] = 3200.0;
        let out = preset.apply_to(&target);
        assert_eq!(out.values[1], 2.0);
        assert_eq!(out.crop, [0.0, 0.2, 1.0, 0.8]);
        assert_eq!(out.values[0], 3200.0, "white balance only when the preset carries it");
    }

    /// A sidecar changed by another program is taken in, Develop's own saves are not, and a
    /// photo kept only in the central file is not wiped for having no sidecar.
    #[test]
    fn outside_changes_are_taken_in() {
        let dir = std::env::temp_dir().join(format!("develop-test-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let photo = dir.join("DSC00001.ARW");
        let central = dir.join("DSC00002.ARW");
        let mut store = Store {
            file: None,
            photos: HashMap::from([(central.clone(), sample())]),
            ratings: HashMap::new(),
            versions: HashMap::new(),
            keywords: HashMap::new(),
            people: HashMap::new(),
            central: HashSet::from([central.clone()]),
            central_changed: false,
            folders: HashSet::new(),
            dirty: HashSet::new(),
            changed: None,
            known: HashMap::new(),
        };
        // Develop's own save is not a change.
        let mut mine = sample();
        mine.values[1] = 0.5;
        store.put(&photo, mine.clone());
        store.set_rating(&photo, 2);
        store.flush(true);
        assert!(!store.reload_sidecar(&photo));
        // Another program's save wins, even over a change not yet written.
        let mut theirs = sample();
        theirs.values[1] = -0.3;
        store.put(&photo, mine.clone());
        let text = format!("{SIDECAR_HEADER}
settings	{}
rating	4
", saved_fields(&theirs));
        std::fs::write(sidecar_path(&photo), text).unwrap();
        assert!(store.reload_sidecar(&photo));
        assert_eq!(store.get(&photo), Some(&theirs));
        assert_eq!(store.rating(&photo), 4);
        assert!(!store.dirty.contains(&photo));
        assert!(!store.reload_sidecar(&photo), "read once");
        // Removed by the other program: back to nothing.
        std::fs::remove_file(sidecar_path(&photo)).unwrap();
        assert!(store.reload_sidecar(&photo));
        assert_eq!(store.get(&photo), None);
        assert_eq!(store.rating(&photo), 0);
        // The central-only photo keeps its settings.
        assert!(store.photos_in(&dir).contains(&central));
        assert!(!store.reload_sidecar(&central));
        assert!(store.contains(&central));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
