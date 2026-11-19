//! The library: folders the user adds, indexed in the background so photos can be found by
//! keyword, person or file name wherever they are.
//!
//! The index lives in the settings folder (`library-index.tsv`) and only mirrors what is in the
//! photos and their `.dvlp` files: deleting it loses nothing, it is simply built again. Each
//! photo's entry is kept while the photo file and its sidecar are unchanged (size and time), so
//! a re-index after start-up only reads what changed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui;

/// Set while a folder is being moved: finding faces and reading content wait, so the move
/// (which reads the same photos) goes faster alone.
pub static PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Paused by the user (remembered: a `faces-paused` file in the settings folder), until they
/// resume it.
static USER_PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn paused_file() -> Option<PathBuf> {
    crate::settings::settings_dir().map(|d| d.join("faces-paused"))
}

/// Whether finding faces is paused by the user.
pub fn user_paused() -> bool {
    USER_PAUSED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Pause or resume finding faces and reading content, and remember it for the next start.
pub fn set_user_paused(paused: bool) {
    USER_PAUSED.store(paused, std::sync::atomic::Ordering::Relaxed);
    if let Some(file) = paused_file() {
        if paused {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&file, "");
        } else {
            let _ = std::fs::remove_file(&file);
        }
    }
}

/// Whether reading waits now (the user paused it, or a folder is being moved).
pub fn waiting() -> bool {
    user_paused() || PAUSED.load(std::sync::atomic::Ordering::Relaxed)
}

/// How often the library is checked for changes while Develop is open.
const RECHECK: Duration = Duration::from_secs(10 * 60);
/// While reading faces and content, save what is done this often.
const SAVE_EVERY: Duration = Duration::from_secs(20);
/// Separates keywords inside one field of the index file (they cannot contain it).
const SEP: char = '\u{1f}';

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    /// What the entry was read from: the photo's size and time, and its sidecar's time.
    stamp: (u64, u64, u64),
    /// Capture date as sortable text.
    pub taken: String,
    pub rating: u8,
    pub keywords: Vec<String>,
    pub people: Vec<String>,
}

#[derive(Default)]
struct Shared {
    entries: HashMap<PathBuf, Entry>,
    indexing: bool,
    /// Photos looked at so far in the current pass, and found in total.
    seen: usize,
    found: usize,
    /// Photos in this pass whose faces and content have not been read before, and how many of
    /// them are done (the slow part: about a fifth of a second each).
    to_read: usize,
    read: usize,
    /// When reading the new photos started, for the time left.
    read_started: Option<Instant>,
    /// Photos whose people's names in their `.dvlp` differ from what the faces say (to be
    /// written by the app, which owns the sidecars).
    names_due: Vec<(PathBuf, Vec<String>)>,
}

pub struct Library {
    folders: Vec<PathBuf>,
    shared: Arc<Mutex<Shared>>,
    /// Faces and people found in the library's photos.
    pub people: Arc<Mutex<crate::people::People>>,
    /// Each photo's CLIP numbers (content search and keyword suggestions).
    pub pictures: Arc<Mutex<crate::clip::Pictures>>,
    last_pass: Option<Instant>,
    /// Shared libraries: what other computers change, as it happens (see store::watch_changes),
    /// the shared places being watched, and when a change last came in.
    changes: (std::sync::mpsc::Sender<crate::store::Change>, std::sync::mpsc::Receiver<crate::store::Change>),
    watched: Vec<PathBuf>,
    pub changed_elsewhere: Option<Instant>,
}

fn seconds(t: std::io::Result<SystemTime>) -> u64 {
    t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

/// What an entry depends on: the photo file and its sidecar.
fn stamp(photo: &Path) -> (u64, u64, u64) {
    let (size, time) = std::fs::metadata(photo).map(|m| (m.len(), seconds(m.modified()))).unwrap_or((0, 0));
    let side = std::fs::metadata(crate::settings::sidecar_path(photo)).map(|m| seconds(m.modified())).unwrap_or(0);
    (size, time, side)
}

fn folders_file() -> Option<PathBuf> {
    crate::settings::settings_dir().map(|d| d.join("library.txt"))
}

/// The library's folders, as saved.
pub fn folder_list() -> Vec<PathBuf> {
    folders_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default()
}

fn index_file() -> Option<PathBuf> {
    crate::settings::settings_dir().map(|d| d.join("library-index.tsv"))
}

/// Every photo Develop opens under `dir`, in subfolders too (hidden folders skipped).
/// The content key of each photo in the library's own index (see people::key), with where it was.
pub fn index_paths() -> HashMap<String, PathBuf> {
    let mut out = HashMap::new();
    if let Some(text) = index_file().and_then(|f| std::fs::read_to_string(f).ok()) {
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() >= 2 {
                let path = PathBuf::from(f[0]);
                if let Some(name) = path.file_name() {
                    out.insert(format!("{}|{}", name.to_string_lossy(), f[1]), path.clone());
                }
            }
        }
    }
    out
}

/// Every photo under `dir`; folders that could not be read go into `failed`.
fn photos_under(dir: &Path, out: &mut Vec<PathBuf>, failed: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        failed.push(dir.to_path_buf());
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let hidden = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
        match entry.file_type() {
            Ok(t) if t.is_dir() && !hidden => photos_under(&path, out, failed),
            Ok(t) if t.is_file() && crate::filmstrip::is_supported(&path) => out.push(path),
            _ => {}
        }
    }
}

/// Whether every word of `query` is in the entry's file name, keywords or people (ignoring case).
pub fn matches(query: &str, e: &Entry) -> bool {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return false;
    }
    let name = e.path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let tags: Vec<String> = e.keywords.iter().chain(&e.people).map(|w| w.to_lowercase()).collect();
    words.iter().all(|w| name.contains(w.as_str()) || tags.iter().any(|t| t.contains(w.as_str())))
}

impl Library {
    pub fn load() -> Self {
        Self::load_with(&|_, _| {})
    }

    /// Nothing yet: what the window has while the library is read at start.
    pub fn empty() -> Self {
        Self { folders: Vec::new(), shared: Default::default(), people: Default::default(), pictures: Default::default(), last_pass: None, changes: std::sync::mpsc::channel(), watched: Vec::new(), changed_elsewhere: None }
    }

    /// Read the library, saying how far along it is (0..1) and what it is reading.
    pub fn load_with(report: &dyn Fn(f32, &str)) -> Self {
        report(0.0, "the photo index");
        USER_PAUSED.store(paused_file().is_some_and(|f| f.is_file()), std::sync::atomic::Ordering::Relaxed);
        let folders: Vec<PathBuf> = folders_file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|t| t.lines().filter(|l| !l.trim().is_empty()).map(PathBuf::from).collect())
            .unwrap_or_default();
        let t = Instant::now();
        let mut entries = HashMap::new();
        if let Some(text) = index_file().and_then(|f| std::fs::read_to_string(f).ok()) {
            for line in text.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() != 9 {
                    continue;
                }
                let num = |s: &str| s.parse::<u64>().unwrap_or(0);
                let words = |s: &str| s.split(SEP).filter(|w| !w.is_empty()).map(str::to_owned).collect::<Vec<_>>();
                let path = PathBuf::from(f[0]);
                entries.insert(path.clone(), Entry { path, stamp: (num(f[1]), num(f[2]), num(f[3])), taken: f[4].to_owned(), rating: f[5].parse().unwrap_or(0), keywords: words(f[6]), people: words(f[7]) });
                let _ = f[8];
            }
        }
        crate::timing("start: library index read", t);
        // (How long each part takes in a big library sets how far along each one is.)
        report(0.05, "people");
        let t = Instant::now();
        let people = crate::people::People::load();
        crate::timing("start: people read", t);
        report(0.5, "keywords and search");
        let t = Instant::now();
        let pictures = crate::clip::Pictures::load();
        report(1.0, "done");
        crate::timing("start: content (search) read", t);
        Self { folders, shared: Arc::new(Mutex::new(Shared { entries, ..Default::default() })), people: Arc::new(Mutex::new(people)), pictures: Arc::new(Mutex::new(pictures)), last_pass: None, changes: std::sync::mpsc::channel(), watched: Vec::new(), changed_elsewhere: None }
    }

    pub fn folders(&self) -> &[PathBuf] {
        &self.folders
    }

    fn save_folders(&self) {
        let Some(file) = folders_file() else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text: String = self.folders.iter().map(|f| format!("{}\n", f.display())).collect();
        let _ = std::fs::write(file, text);
    }

    /// Whether `folder` (a library folder) is shared with other computers, as used here.
    pub fn is_shared(&self, folder: &Path) -> bool {
        crate::store::shared_here(folder)
    }

    /// Whether `folder` is a shared library this computer stopped sharing (it keeps its own copy).
    pub fn stopped_here(&self, folder: &Path) -> bool {
        crate::store::is_shared(folder) && !crate::store::shared_here(folder)
    }

    /// Stop sharing `folder`: this computer copies the library's data to itself and goes back to
    /// a local library. With `everyone`, the shared folder's `.develop` is also renamed (kept as
    /// a backup), so no computer uses it any more. Returns what to tell the user.
    pub fn stop_sharing(&mut self, folder: &Path, everyone: bool, ctx: &egui::Context) -> Result<String, String> {
        if self.status().0 {
            return Err("Wait until the library has finished reading its photos, then try again.".into());
        }
        crate::store::set_not_shared_here(folder, true);
        let places = crate::store::places();
        if let Ok(mut p) = self.people.lock() {
            p.use_places(&places);
        }
        if let Ok(mut p) = self.pictures.lock() {
            p.use_places(&places);
        }
        let text = if everyone {
            let kept = crate::store::retire_shared(folder)?;
            crate::store::set_not_shared_here(folder, false);
            format!("{} is no longer shared. Its people and search data are on this computer now; the shared data was kept as {}.", folder.display(), kept.display())
        } else {
            format!("This computer no longer shares {}: it keeps its own copy of the people and search data. The other computers still share it.", folder.display())
        };
        self.reindex(ctx);
        Ok(text)
    }

    /// Use a shared library again on this computer (after Stop sharing on this computer only):
    /// the shared data is the one to go by; this computer's own copy for it is set aside.
    pub fn share_again(&mut self, folder: &Path, ctx: &egui::Context) -> Result<(), String> {
        if self.status().0 {
            return Err("Wait until the library has finished reading its photos, then try again.".into());
        }
        crate::store::set_not_shared_here(folder, false);
        crate::store::drop_local_under(folder);
        self.people = Arc::new(Mutex::new(crate::people::People::load()));
        self.pictures = Arc::new(Mutex::new(crate::clip::Pictures::load()));
        self.reindex(ctx);
        Ok(())
    }

    /// Share the library folder `folder`: its faces, people and content data move into
    /// `folder/.develop`, where other computers that add `folder` to their library use them.
    pub fn share(&mut self, folder: &Path, ctx: &egui::Context) -> Result<(), String> {
        if self.status().0 {
            return Err("Wait until the library has finished reading its photos, then try again.".into());
        }
        crate::store::make_shared(folder)?;
        // Read again with the new place, and write everything: what belongs to the shared
        // folder goes there (and leaves this computer's own files).
        let mut people = crate::people::People::load();
        people.save_all();
        let mut pictures = crate::clip::Pictures::load();
        pictures.save_all();
        self.people = Arc::new(Mutex::new(people));
        self.pictures = Arc::new(Mutex::new(pictures));
        self.reindex(ctx);
        Ok(())
    }

    /// Capture dates the index knows for photos in `dir` (unchanged since they were indexed).
    pub fn dates_in(&self, dir: &Path) -> HashMap<PathBuf, String> {
        let Ok(s) = self.shared.lock() else { return HashMap::new() };
        s.entries.values().filter(|e| e.path.parent() == Some(dir) && !e.taken.is_empty()).map(|e| (e.path.clone(), e.taken.clone())).collect()
    }

    /// A folder was moved from `from` to `to` (Move to…): faces, content and the index follow it
    /// at once. A library folder that was moved is replaced by its new place; a folder moved out
    /// of the library into a place outside it adds that place. Returns whether `to` was added.
    pub fn relocate(&mut self, from: &Path, to: &Path, ctx: &egui::Context) -> bool {
        if let Ok(mut p) = self.people.lock() {
            p.relocate(from, to);
            p.save();
        }
        if let Ok(mut p) = self.pictures.lock() {
            p.relocate(from, to);
            p.save();
        }
        if let Ok(mut s) = self.shared.lock() {
            let moved: Vec<PathBuf> = s.entries.keys().filter(|p| crate::mover::relocated(p, from, to).is_some()).cloned().collect();
            for old in moved {
                if let (Some(mut e), Some(new)) = (s.entries.remove(&old), crate::mover::relocated(&old, from, to)) {
                    e.path = new.clone();
                    s.entries.insert(new, e);
                }
            }
        }
        let was_in = self.folders.iter().any(|f| crate::mover::relocated(from, f, f).is_some());
        let mut added = false;
        let mut changed = false;
        for f in &mut self.folders {
            if let Some(new) = crate::mover::relocated(f, from, to) {
                *f = new;
                changed = true;
            }
        }
        if was_in && !self.folders.iter().any(|f| crate::mover::relocated(to, f, f).is_some()) {
            self.folders.push(to.to_path_buf());
            added = true;
            changed = true;
        }
        if changed {
            self.folders.sort();
            self.save_folders();
        }
        self.reindex(ctx);
        added
    }

    pub fn add_folder(&mut self, folder: PathBuf, ctx: &egui::Context) {
        // A folder inside one already in the library adds nothing.
        if self.folders.iter().any(|f| folder.starts_with(f)) {
            return;
        }
        // One that contains folders already in the library replaces them.
        self.folders.retain(|f| !f.starts_with(&folder));
        self.folders.push(folder);
        self.folders.sort();
        self.save_folders();
        self.reindex(ctx);
    }

    pub fn remove_folder(&mut self, folder: &Path, ctx: &egui::Context) {
        self.folders.retain(|f| f != folder);
        self.save_folders();
        self.reindex(ctx);
    }

    /// Whether `photo` is in one of the library's folders.
    pub fn contains(&self, photo: &Path) -> bool {
        self.folders.iter().any(|f| photo.starts_with(f))
    }

    /// The slow part of a pass under way: (new photos read, new photos to read, seconds left),
    /// while faces and content are being read for photos not seen before.
    pub fn reading(&self) -> Option<(usize, usize, f32)> {
        let s = self.shared.lock().ok()?;
        if !(s.indexing && s.to_read > s.read) {
            return None;
        }
        // At the speed so far, once a few are done; about half a second each before that.
        let each = match s.read_started {
            Some(t) if s.read >= 3 => t.elapsed().as_secs_f32() / s.read as f32,
            _ => 0.5,
        };
        Some((s.read, s.to_read, each * (s.to_read - s.read) as f32))
    }

    /// Save the faces and content read so far (when quitting in the middle of a pass); the rest
    /// is read on the next pass.
    pub fn save_progress(&self) {
        if let Ok(mut p) = self.people.lock() {
            p.save();
        }
        if let Ok(mut p) = self.pictures.lock() {
            p.save();
        }
    }

    /// Indexing state: (running, photos looked at, photos found, photos indexed).
    pub fn status(&self) -> (bool, usize, usize, usize) {
        let s = self.shared.lock().map(|s| (s.indexing, s.seen, s.found, s.entries.len()));
        s.unwrap_or((false, 0, 0, 0))
    }

    /// Start a pass over the library in the background, unless one is running.
    pub fn reindex(&mut self, ctx: &egui::Context) {
        {
            let Ok(mut s) = self.shared.lock() else { return };
            if s.indexing {
                return;
            }
            s.indexing = true;
            s.seen = 0;
            s.found = 0;
            s.to_read = 0;
            s.read = 0;
        }
        self.last_pass = Some(Instant::now());
        let (shared, folders, ctx, people, pictures) = (self.shared.clone(), self.folders.clone(), ctx.clone(), self.people.clone(), self.pictures.clone());
        let use_clip = crate::clip::available();
        std::thread::spawn(move || {
            let mut photos = Vec::new();
            // Folders that could not be read (a network drive asleep, a disk unplugged): their
            // photos are kept as they are, not forgotten.
            let mut unreachable = Vec::new();
            for folder in &folders {
                photos_under(folder, &mut photos, &mut unreachable);
            }
            // Which photos are new (faces or content not read yet): counted first, for progress.
            let keyed: Vec<(PathBuf, String)> = photos.into_iter().map(|p| {
                let k = crate::people::key(&p);
                (p, k)
            }).collect();
            let new = keyed
                .iter()
                .filter(|(p, k)| !people.lock().map(|mut x| x.known(p, k)).unwrap_or(true) || (use_clip && !pictures.lock().map(|mut x| x.known(p, k)).unwrap_or(true)))
                .count();
            if let Ok(mut s) = shared.lock() {
                s.found = keyed.len();
                s.to_read = new;
                s.read_started = Some(Instant::now());
            }
            let mut read = 0;
            let mut saved = Instant::now();
            let known: HashMap<PathBuf, Entry> = shared.lock().map(|s| s.entries.clone()).unwrap_or_default();
            let mut fresh = HashMap::with_capacity(keyed.len());
            let mut keys = std::collections::HashSet::new();
            // In a shared library, a folder is read by one computer at a time (see store::claim).
            let places = crate::store::places();
            let mut claims: HashMap<PathBuf, (bool, Instant)> = HashMap::new();
            for (n, (photo, k)) in keyed.into_iter().enumerate() {
                // Faces: found once per photo content (moving a photo keeps them).
                keys.insert(k.clone());
                let faces_known = people.lock().map(|mut p| p.known(&photo, &k)).unwrap_or(true);
                let clip_known = !use_clip || pictures.lock().map(|mut p| p.known(&photo, &k)).unwrap_or(true);
                let dir = photo.parent().map(Path::to_path_buf).unwrap_or_default();
                let mine = if faces_known && clip_known {
                    true
                } else {
                    let place = &places[crate::store::place_of(&places, &dir)];
                    let entry = claims.entry(dir.clone()).or_insert_with(|| (crate::store::claim(place, &dir), Instant::now()));
                    // Renew a claim held for a long read.
                    if entry.0 && entry.1.elapsed() > Duration::from_secs(5 * 60) {
                        *entry = (crate::store::claim(place, &dir), Instant::now());
                    }
                    entry.0
                };
                if !mine {
                    // Another computer is reading this folder: its results come through the
                    // shared library's change log.
                    if let Ok(mut s) = shared.lock() {
                        s.to_read = s.to_read.saturating_sub(1);
                    }
                }
                if (!faces_known || !clip_known) && mine {
                    // Wait while paused (by the user, or while a folder is moved); the time spent
                    // waiting doesn't count towards the estimate of the time left.
                    if waiting() {
                        let since = Instant::now();
                        while waiting() {
                            std::thread::sleep(Duration::from_millis(500));
                        }
                        if let Ok(mut s) = shared.lock() {
                            s.read_started = s.read_started.map(|t| t + since.elapsed());
                        }
                    }
                    // One preview for both.
                    let pic = crate::filmstrip::preview(&photo, 1600);
                    if !faces_known {
                        let found = pic.as_ref().map(|p| crate::people::find_faces_in(p, &k)).unwrap_or_default();
                        if let Ok(mut p) = people.lock() {
                            p.add_faces(&photo, &k, found);
                        }
                    }
                    if !clip_known {
                        if let Some(Ok(e)) = pic.as_ref().map(crate::clip::image_embedding) {
                            if let Ok(mut p) = pictures.lock() {
                                p.insert(&photo, &k, e);
                            }
                        }
                    }
                    read += 1;
                    if let Ok(mut s) = shared.lock() {
                        s.read = read;
                    }
                    ctx.request_repaint();
                    // Keep what is done, so quitting part-way loses little.
                    if saved.elapsed() >= SAVE_EVERY {
                        if let Ok(mut p) = people.lock() {
                            p.save();
                        }
                        if let Ok(mut p) = pictures.lock() {
                            p.save();
                        }
                        saved = Instant::now();
                    }
                }
                let st = stamp(&photo);
                let entry = match known.get(&photo) {
                    Some(e) if e.stamp == st => e.clone(),
                    _ => {
                        let (rating, keywords, people) = crate::settings::read_tags(&photo);
                        Entry { taken: crate::filmstrip::capture_date(&photo), path: photo.clone(), stamp: st, rating, keywords, people }
                    }
                };
                fresh.insert(photo, entry);
                if n % 200 == 0 {
                    if let Ok(mut s) = shared.lock() {
                        s.seen = n;
                    }
                    ctx.request_repaint();
                }
            }
            let mut lines: Vec<String> = fresh
                .values()
                .map(|e| {
                    let j = |w: &[String]| w.join(&SEP.to_string());
                    format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t\n", e.path.display(), e.stamp.0, e.stamp.1, e.stamp.2, e.taken, e.rating, j(&e.keywords), j(&e.people))
                })
                .collect();
            lines.sort();
            if let Some(file) = index_file() {
                if let Some(dir) = file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(file, lines.concat());
            }
            // Named people found in photos whose .dvlp does not say so yet (or no longer).
            let mut due = Vec::new();
            for place in places.iter().filter(|p| p.shared()) {
                crate::store::release_claims(place);
            }
            if let Ok(mut p) = pictures.lock() {
                p.prune(&keys, &unreachable);
                p.save();
            }
            // Names worked out for every photo in one pass over the faces (photo by photo it
            // would hold the people store for minutes in a big library, freezing the window).
            let all: Vec<PathBuf> = fresh.keys().cloned().collect();
            let names = people.lock().map(|mut p| {
                p.prune(&keys, &unreachable);
                p.save();
                p.names_for(&all)
            });
            for (photo, names) in names.unwrap_or_default() {
                if let Some(e) = fresh.get(&photo) {
                    let mut have = e.people.clone();
                    have.sort();
                    have.dedup();
                    if names != have {
                        due.push((photo, names));
                    }
                }
            }
            if let Ok(mut s) = shared.lock() {
                s.entries = fresh;
                s.indexing = false;
                s.names_due = due;
            }
            ctx.request_repaint();
        });
    }

    /// Re-index at start-up and every `RECHECK` while open.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if self.folders.is_empty() {
            return;
        }
        // Shared libraries: watch each for other computers' changes, and take them in.
        for place in crate::store::places().into_iter().filter(|p| p.shared()) {
            if !self.watched.contains(&place.data) {
                let c = ctx.clone();
                crate::store::watch_changes(&place, self.changes.0.clone(), move || c.request_repaint());
                self.watched.push(place.data.clone());
            }
        }
        let mut any = false;
        while let Ok(change) = self.changes.1.try_recv() {
            any = true;
            match change {
                crate::store::Change::File(data, kind, name) => match kind.as_str() {
                    "content" => {
                        if let Ok(mut p) = self.pictures.lock() {
                            p.reload(&data, &name);
                        }
                    }
                    _ => {
                        if let Ok(mut p) = self.people.lock() {
                            p.reload(&data, &kind, &name);
                        }
                    }
                },
                crate::store::Change::All(data) => {
                    if let Ok(mut p) = self.people.lock() {
                        p.reload_all(&data);
                    }
                    if let Ok(mut p) = self.pictures.lock() {
                        p.reload_all(&data);
                    }
                }
            }
        }
        if any {
            self.changed_elsewhere = Some(Instant::now());
            ctx.request_repaint();
        }
        if self.last_pass.map_or(true, |t| t.elapsed() >= RECHECK) {
            self.reindex(ctx);
        }
        ctx.request_repaint_after(RECHECK);
    }

    /// Update one photo's entry from what Develop has just remembered for it (a keyword added
    /// in the app shows in library searches straight away, before the next pass).
    pub fn note(&self, photo: &Path, rating: u8, keywords: &[String], people: &[String]) {
        if !self.contains(photo) {
            return;
        }
        if let Ok(mut s) = self.shared.lock() {
            let entry = s.entries.entry(photo.to_path_buf()).or_insert_with(|| Entry {
                path: photo.to_path_buf(),
                stamp: (0, 0, 0),
                taken: crate::filmstrip::capture_date(photo),
                rating: 0,
                keywords: Vec::new(),
                people: Vec::new(),
            });
            entry.rating = rating;
            entry.keywords = keywords.to_vec();
            entry.people = people.to_vec();
            entry.stamp = (0, 0, 0); // read it again on the next pass
        }
    }

    /// Note the names of the people in a photo in the index (its other tags as they were).
    pub fn note_people(&self, photo: &Path, people: &[String]) {
        if let Ok(mut s) = self.shared.lock() {
            if let Some(entry) = s.entries.get_mut(photo) {
                entry.people = people.to_vec();
                entry.stamp = (0, 0, 0); // read it again on the next pass
            }
        }
    }

    /// People's names the app should write into photos' `.dvlp` files (taken once).
    pub fn take_names_due(&self) -> Vec<(PathBuf, Vec<String>)> {
        self.shared.lock().map(|mut s| std::mem::take(&mut s.names_due)).unwrap_or_default()
    }

    /// The photos showing a person, oldest first (from the index's capture dates).
    pub fn photos_of(&self, id: u32) -> Vec<PathBuf> {
        let photos = self.people.lock().map(|p| p.photos_of(id)).unwrap_or_default();
        let Ok(s) = self.shared.lock() else { return photos };
        let mut dated: Vec<(String, PathBuf)> = photos.into_iter().map(|p| (s.entries.get(&p).map(|e| e.taken.clone()).unwrap_or_default(), p)).collect();
        dated.sort();
        dated.into_iter().map(|(_, p)| p).collect()
    }

    /// Photos matching `query`: those whose keywords, people or file name contain every word
    /// (oldest first), then those whose content fits the description best (by CLIP, best
    /// first). Returns the photos and how many matched by words.
    pub fn search(&self, query: &str, min_stars: u8) -> (Vec<PathBuf>, usize) {
        let Ok(s) = self.shared.lock() else { return (Vec::new(), 0) };
        let mut hits: Vec<&Entry> = s.entries.values().filter(|e| e.rating >= min_stars && matches(query, e)).collect();
        hits.sort_by(|a, b| a.taken.cmp(&b.taken).then_with(|| a.path.cmp(&b.path)));
        let mut out: Vec<PathBuf> = hits.into_iter().map(|e| e.path.clone()).collect();
        let by_words = out.len();
        if crate::clip::available() {
            if let Ok(phrase) = crate::clip::text_embedding(query) {
                let pictures: Vec<(PathBuf, Vec<f32>)> = self
                    .pictures
                    .lock()
                    .map(|p| p.all())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(p, _)| s.entries.get(p).is_some_and(|e| e.rating >= min_stars))
                    .collect();
                for (p, _) in crate::clip::rank(&phrase, &pictures) {
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
        (out, by_words)
    }

    /// Suggested keywords for a photo (from the cached CLIP numbers, or worked out now), without
    /// the ones it already has.
    pub fn suggestions(&self, photo: &Path, have: &[String]) -> Vec<(String, f32)> {
        if !crate::clip::available() {
            return Vec::new();
        }
        let k = crate::people::key(photo);
        let cached = self.pictures.lock().ok().and_then(|p| p.get(&k).cloned());
        let Some(e) = cached.or_else(|| crate::clip::embed_photo(photo).ok()) else { return Vec::new() };
        crate::clip::suggest(&e).unwrap_or_default().into_iter().filter(|(w, _)| !have.iter().any(|h| h.eq_ignore_ascii_case(w))).collect()
    }

    /// Bring entries up to date with their sidecars now (ratings, keywords, people), without
    /// a full pass: for the MCP server, which reads the index written by the app.
    pub fn refresh_tags(&self) {
        let Ok(mut s) = self.shared.lock() else { return };
        for e in s.entries.values_mut() {
            let st = stamp(&e.path);
            if st != e.stamp {
                let (rating, keywords, people) = crate::settings::read_tags(&e.path);
                e.rating = rating;
                e.keywords = keywords;
                e.people = people;
                e.stamp = st;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, keywords: &[&str], people: &[&str]) -> Entry {
        Entry { path: PathBuf::from(path), stamp: (0, 0, 0), taken: String::new(), rating: 0, keywords: keywords.iter().map(|s| s.to_string()).collect(), people: people.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn every_word_must_match_somewhere() {
        let e = entry(r"O:\Photos\2026\DSC05166.JPG", &["dinner", "Restaurant"], &["Talia"]);
        assert!(matches("dinner", &e));
        assert!(matches("DINNER talia", &e), "case and people");
        assert!(matches("rest", &e), "part of a word");
        assert!(matches("dsc0516", &e), "file name");
        assert!(!matches("dinner beach", &e));
        assert!(!matches("", &e));
    }
}
