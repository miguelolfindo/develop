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
    pub sharpen: [f32; 4],
    /// Tint relative to the as-shot white balance: negative = greener, positive = more magenta.
    pub tint: f32,
    /// Texture, clarity, dehaze (-100..100).
    pub presence: [f32; 3],
    pub lens: crate::lens::LensSettings,
}

/// Sharpening a photo starts with (Lightroom's defaults for raw files).
pub const SHARPEN_DEFAULT: [f32; 4] = [40.0, 1.0, 25.0, 0.0];

pub struct Store {
    /// The central file. It only holds photos whose folder cannot take a sidecar, and
    /// photos saved by earlier versions that have not been moved to sidecars yet.
    file: Option<PathBuf>,
    photos: HashMap<PathBuf, Saved>,
    /// Star ratings, 1 to 5 (unrated photos are absent).
    ratings: HashMap<PathBuf, u8>,
    /// Photos whose data is in the central file rather than a sidecar.
    central: HashSet<PathBuf>,
    central_changed: bool,
    /// Folders whose sidecars have been read.
    folders: HashSet<PathBuf>,
    /// Photos with changes not yet written.
    dirty: HashSet<PathBuf>,
    changed: Option<Instant>,
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
        (Some(a), Some(r), Some(d), Some(m)) => [a.clamp(0.0, 150.0), r.clamp(0.5, 3.0), d.clamp(0.0, 100.0), m.clamp(0.0, 100.0)],
        _ => SHARPEN_DEFAULT,
    };
    let tint = number().unwrap_or(0.0).clamp(-150.0, 150.0);
    let presence = [0; 3].map(|_| number().unwrap_or(0.0).clamp(-100.0, 100.0));
    let lens = crate::lens::LensSettings::from_fields(&mut fields);
    Some((PathBuf::from(path), Saved { values, quarter_turns, profile, denoise, crop, angle, sharpen, tint, presence, lens }))
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

const SIDECAR_EXTENSION: &str = "dvlp";
/// Earlier versions' sidecar extension; such files are read and replaced by `.dvlp` ones.
const LEGACY_SIDECAR_EXTENSION: &str = "srp";
const SIDECAR_HEADER: &str = "Develop settings\t1";

/// The settings fields of a stored line (everything after the path).
fn saved_fields(s: &Saved) -> String {
    let values: Vec<String> = s.values.iter().map(|v| v.to_string()).collect();
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
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
        s.lens.to_fields()
    )
}

fn parse_sidecar(text: &str) -> (Option<Saved>, u8) {
    let mut saved = None;
    let mut rating = 0;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("settings\t") {
            saved = parse_line(&format!("photo\t{rest}")).map(|(_, s)| s);
        } else if let Some(stars) = line.strip_prefix("rating\t") {
            rating = stars.trim().parse::<u8>().unwrap_or(0).min(5);
        }
    }
    (saved, rating)
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

impl Store {
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
        Self { file, photos, ratings, central, central_changed: false, folders: HashSet::new(), dirty: HashSet::new(), changed: None }
    }

    /// Read the sidecars of a folder (once), and move any settings the central file still
    /// holds for its photos into sidecars.
    pub fn load_folder(&mut self, dir: &Path) {
        if self.folders.contains(dir) {
            return;
        }
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
            let (saved, rating) = parse_sidecar(&text);
            match saved {
                Some(saved) => self.photos.insert(photo.clone(), saved),
                None => self.photos.remove(&photo),
            };
            match rating {
                0 => self.ratings.remove(&photo),
                stars => self.ratings.insert(photo.clone(), stars),
            };
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
            if saved.is_none() && rating == 0 {
                // Nothing left to remember: no sidecar, no central entry.
                let _ = std::fs::remove_file(&sidecar);
                let _ = std::fs::remove_file(&legacy);
                self.central_changed |= self.central.remove(&photo);
                continue;
            }
            let mut text = format!("{SIDECAR_HEADER}\n");
            if let Some(saved) = saved {
                text.push_str(&format!("settings\t{}\n", saved_fields(saved)));
            }
            if rating > 0 {
                text.push_str(&format!("rating\t{rating}\n"));
            }
            if write_whole(&sidecar, &text) {
                self.central_changed |= self.central.remove(&photo);
                let _ = std::fs::remove_file(&legacy);
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
/// Network computers and shares the user added or opened photos from, kept in
/// `network.tsv` (one address per line, such as `\\Olfindo` or `\\Olfindo\photos`).
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