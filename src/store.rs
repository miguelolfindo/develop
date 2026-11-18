//! Where the library's data is kept: one small file per photo folder (for each kind of data:
//! faces, content) and one file per person, under `library/` in the settings folder.
//!
//! Small files instead of one big one: a change rewrites only the files it touches, a photo folder
//! that moves or cannot be reached affects only its own file, and (later) a library shared
//! between computers can keep these files beside its photos, where two people changing different
//! folders or different people never write the same file. Each file names its photos by file name
//! only, under a `folder` line, so the files hold no other paths.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// How long Develop keeps the faces and content of a photo it no longer finds (moved away,
/// deleted, or in a folder taken out of the library) before forgetting them. Photos in folders it
/// cannot reach at all (a sleeping network drive, an unplugged disk) are kept however long.
pub const GRACE_DAYS: u32 = 30;

/// Today, as days since 1970.
pub fn today() -> u32 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| (d.as_secs() / 86_400) as u32)
}

/// Now, as seconds since 1970.
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// This computer's name, for "renamed on …" notes in a shared library.
pub fn computer() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| std::process::Command::new("hostname").output().ok().and_then(|o| String::from_utf8(o.stdout).ok()).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "this computer".to_owned())
}

/// The library data folder (local libraries: in the settings folder).
pub fn root() -> Option<PathBuf> {
    crate::settings::settings_dir().map(|d| d.join("library"))
}

/// The hidden folder a shared library keeps its data in, at its top.
pub const SHARED_DIR: &str = ".develop";

/// Where a set of library data files lives: Develop's settings folder (a local library), or a
/// shared library's `.develop` folder beside its photos, where every computer using it reads and
/// writes. In a shared place, folders are written relative to its top (`2025/01`), so the same
/// files work whether the share is `\\server\Photos` (Windows) or `/Volumes/Photos` (Mac).
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    /// The shared library's top folder, as this computer sees it; None for the local place.
    pub photos: Option<PathBuf>,
    pub data: PathBuf,
}

impl Place {
    pub fn shared(&self) -> bool {
        self.photos.is_some()
    }

    /// Whether this place keeps the data of photos in `folder`.
    pub fn holds(&self, folder: &Path) -> bool {
        match &self.photos {
            Some(top) => crate::mover::relocated(folder, top, Path::new("")).is_some(),
            None => true,
        }
    }

    /// `folder` as written in this place's files.
    pub fn written(&self, folder: &Path) -> String {
        match &self.photos {
            Some(top) => crate::mover::relocated(folder, top, Path::new("")).map(|rel| rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")).unwrap_or_default(),
            None => folder.display().to_string(),
        }
    }

    /// The folder a `folder` line names.
    pub fn folder(&self, written: &str) -> PathBuf {
        match &self.photos {
            Some(top) => written.split('/').filter(|p| !p.is_empty()).fold(top.clone(), |path, part| path.join(part)),
            None => PathBuf::from(written),
        }
    }

    /// The name of `folder`'s data file here (the same on every computer for a shared place).
    pub fn file_name(&self, folder: &Path) -> String {
        let text = self.written(folder);
        let text = if self.shared() || cfg!(windows) { text.to_lowercase() } else { text };
        format!("{:016x}.tsv", fnv(&text))
    }
}

/// Whether `folder` is the top of a shared library (it has `.develop/library.tsv`).
pub fn is_shared(folder: &Path) -> bool {
    folder.join(SHARED_DIR).join("library.tsv").is_file()
}

fn not_shared_file() -> Option<PathBuf> {
    crate::settings::settings_dir().map(|d| d.join("not-shared.txt"))
}

/// Shared libraries this computer stopped sharing (Stop sharing, on this computer only): their
/// data is kept here, locally, though the shared folder is still there for the others.
pub fn not_shared_here() -> Vec<PathBuf> {
    not_shared_file().and_then(|f| std::fs::read_to_string(f).ok()).map(|t| t.lines().filter(|l| !l.trim().is_empty()).map(PathBuf::from).collect()).unwrap_or_default()
}

fn same_folder(a: &Path, b: &Path) -> bool {
    crate::mover::relocated(a, b, b).is_some_and(|p| p == b)
}

/// Note that this computer does (or no longer does) keep `folder`'s shared library locally.
pub fn set_not_shared_here(folder: &Path, not_shared: bool) {
    let mut list: Vec<PathBuf> = not_shared_here().into_iter().filter(|f| !same_folder(f, folder)).collect();
    if not_shared {
        list.push(folder.to_path_buf());
    }
    if let Some(file) = not_shared_file() {
        let text: String = list.iter().map(|f| format!("{}\n", f.display())).collect();
        let _ = std::fs::write(file, text);
    }
}

/// Whether `folder` is a shared library this computer uses (shared, and not stopped here).
pub fn shared_here(folder: &Path) -> bool {
    is_shared(folder) && !not_shared_here().iter().any(|f| same_folder(f, folder))
}

/// Stop sharing `folder` for everyone: its `.develop` folder is renamed (kept, as a backup),
/// so no computer finds a shared library there any more.
pub fn retire_shared(folder: &Path) -> Result<PathBuf, String> {
    let from = folder.join(SHARED_DIR);
    let day = date_label();
    let mut to = folder.join(format!("{SHARED_DIR}-stopped-{day}"));
    let mut n = 2;
    while to.exists() {
        to = folder.join(format!("{SHARED_DIR}-stopped-{day}-{n}"));
        n += 1;
    }
    std::fs::rename(&from, &to).map_err(|e| format!("Could not rename {}: {e}", from.display()))?;
    Ok(to)
}

/// Today as YYYYMMDD.
fn date_label() -> String {
    let days = today() as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}{m:02}{d:02}")
}

/// Remove this computer's own (local) data files for folders under `top`: before using a
/// shared library again, whose data is the one to go by.
pub fn drop_local_under(top: &Path) {
    let Some(local) = root() else { return };
    for kind in ["faces", "content"] {
        let Ok(entries) = std::fs::read_dir(local.join(kind)) else { continue };
        for e in entries.flatten() {
            let first = std::fs::read_to_string(e.path()).ok().and_then(|t| t.lines().next().map(str::to_owned)).unwrap_or_default();
            if let Some(folder) = first.strip_prefix("folder\t") {
                if crate::mover::relocated(Path::new(folder), top, top).is_some() {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}

/// Every place library data lives: the local one first, then each shared library among the
/// library's folders that can be reached now (and that this computer has not stopped sharing).
pub fn places() -> Vec<Place> {
    let mut out: Vec<Place> = root().map(|data| Place { photos: None, data }).into_iter().collect();
    for folder in crate::library::folder_list() {
        if shared_here(&folder) {
            out.push(Place { data: folder.join(SHARED_DIR), photos: Some(folder) });
        }
    }
    out
}

/// The place that keeps `folder`'s data: the shared library it is in (the deepest), else the
/// local place (index 0).
pub fn place_of(places: &[Place], folder: &Path) -> usize {
    places.iter().enumerate().filter(|(_, p)| p.shared() && p.holds(folder)).max_by_key(|(_, p)| p.photos.as_ref().map_or(0, |t| t.components().count())).map_or(0, |(i, _)| i)
}

/// This running copy of Develop, in a shared library's change log (to skip its own changes).
pub fn instance() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(new_id)
}

/// Note in a shared place's `changes.log` that this copy of Develop wrote (or removed) `file`
/// of `kind`, so the other computers read it again.
pub fn note_change(place: &Place, kind: &str, file: &str) {
    if !place.shared() {
        return;
    }
    use std::io::Write;
    let log = place.data.join("changes.log");
    // Kept short: once big, it starts again (readers then reload everything once).
    if std::fs::metadata(&log).is_ok_and(|m| m.len() > 1 << 20) {
        let _ = std::fs::rename(&log, place.data.join("changes.old"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
        let _ = writeln!(f, "{}\t{kind}/{file}", instance());
    }
}

/// What other computers changed in a shared place, from its `changes.log`: checked every few
/// seconds (one small request), by a background thread per shared place.
pub enum Change {
    /// A file of a kind ("faces", "content", "people") was written or removed.
    File(PathBuf, String, String),
    /// The log started again: read everything in this place again.
    All(PathBuf),
}

pub fn watch_changes(place: &Place, tx: std::sync::mpsc::Sender<Change>, wake: impl Fn() + Send + 'static) {
    let data = place.data.clone();
    std::thread::spawn(move || {
        use std::io::{Read, Seek, SeekFrom};
        let log = data.join("changes.log");
        let mut offset = std::fs::metadata(&log).map_or(0, |m| m.len());
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let len = std::fs::metadata(&log).map_or(0, |m| m.len());
            if len < offset {
                offset = 0;
                if tx.send(Change::All(data.clone())).is_err() {
                    return;
                }
                wake();
            }
            if len <= offset {
                continue;
            }
            let Ok(mut f) = std::fs::File::open(&log) else { continue };
            let mut text = String::new();
            if f.seek(SeekFrom::Start(offset)).is_err() || f.read_to_string(&mut text).is_err() {
                continue;
            }
            // Only whole lines; a line still being written is read next time.
            let Some(end) = text.rfind('\n') else { continue };
            offset += end as u64 + 1;
            let mut any = false;
            for line in text[..end].lines() {
                let Some((who, what)) = line.split_once('\t') else { continue };
                if who == instance() {
                    continue;
                }
                if let Some((kind, file)) = what.split_once('/') {
                    if tx.send(Change::File(data.clone(), kind.to_owned(), file.to_owned())).is_err() {
                        return;
                    }
                    any = true;
                }
            }
            if any {
                wake();
            }
        }
    });
}

/// Make `folder` the top of a shared library: its data will live in `folder/.develop` (hidden),
/// where every computer that adds `folder` to its library reads and writes it.
pub fn make_shared(folder: &Path) -> Result<(), String> {
    let data = folder.join(SHARED_DIR);
    for sub in ["faces", "content", "people", "claims"] {
        std::fs::create_dir_all(data.join(sub)).map_err(|e| format!("Could not create {}: {e}", data.display()))?;
    }
    let text = format!("library\t{}\ncreated\t{}\t{}\n", new_id(), now(), computer());
    std::fs::write(data.join("library.tsv"), text).map_err(|e| format!("Could not write in {}: {e}", data.display()))?;
    // Hidden on Windows too (a dot is enough on a Mac).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("attrib").arg("+h").arg(&data).creation_flags(0x0800_0000).status();
    }
    Ok(())
}

/// How long a computer's claim on a folder (reading its new photos) holds before others may
/// take over (it renews it while reading).
const CLAIM_SECONDS: u64 = 15 * 60;

/// Claim `folder` of a shared place for reading its new photos' faces and content, so two
/// computers never do the same work. False when another computer is at it (its claim is
/// recent): leave the folder to it; its results arrive through the change log.
pub fn claim(place: &Place, folder: &Path) -> bool {
    if !place.shared() {
        return true;
    }
    let file = place.data.join("claims").join(place.file_name(folder));
    if let Ok(text) = std::fs::read_to_string(&file) {
        let f: Vec<&str> = text.trim().split('\t').collect();
        if let [who, _, at] = f.as_slice() {
            if *who != instance() && now().saturating_sub(at.parse().unwrap_or(0)) < CLAIM_SECONDS {
                return false;
            }
        }
    }
    let _ = std::fs::create_dir_all(place.data.join("claims"));
    std::fs::write(&file, format!("{}\t{}\t{}\n", instance(), computer(), now())).is_ok()
}

/// Give up this computer's claims in a shared place (its reading is done).
pub fn release_claims(place: &Place) {
    let Ok(entries) = std::fs::read_dir(place.data.join("claims")) else { return };
    for e in entries.flatten() {
        if std::fs::read_to_string(e.path()).is_ok_and(|t| t.starts_with(instance())) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// The record a line of a data file is about, for merging: the folder line, a photo (`P`, `C`)
/// or one face of a photo (`F`).
fn record_id(line: &str) -> String {
    let f: Vec<&str> = line.splitn(4, '\t').collect();
    match f.as_slice() {
        ["folder", ..] => "folder".to_owned(),
        ["F", key, n, ..] => format!("F\t{key}\t{n}"),
        [kind, key, ..] => format!("{kind}\t{key}"),
        _ => line.to_owned(),
    }
}

/// Merge a data file changed by another computer since this one last read it: per record,
/// what only one side changed is taken from that side; where both changed the same record,
/// this computer's (newer: it is being saved now) wins. `base` is the text both started from.
pub fn merge(base: &str, mine: &str, theirs: &str) -> String {
    let index = |text: &str| -> (Vec<String>, HashMap<String, String>) {
        let mut order = Vec::new();
        let mut map = HashMap::new();
        for line in text.lines().filter(|l| !l.is_empty()) {
            let id = record_id(line);
            if map.insert(id.clone(), line.to_owned()).is_none() {
                order.push(id);
            }
        }
        (order, map)
    };
    let (_, b) = index(base);
    let (mine_order, m) = index(mine);
    let (theirs_order, th) = index(theirs);
    let mut ids = mine_order;
    for id in theirs_order {
        if !m.contains_key(&id) {
            ids.push(id);
        }
    }
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        let (bb, mm, tt) = (b.get(&id), m.get(&id), th.get(&id));
        let pick = if mm == bb { tt } else { mm };
        if let Some(line) = pick {
            out.push(line.clone());
        }
    }
    // The folder line first, then the records sorted (as the writers sort them).
    out.sort_by(|a, b| (record_id(a) != "folder").cmp(&(record_id(b) != "folder")).then(a.cmp(b)));
    let mut text = out.join("\n");
    text.push('\n');
    text
}

fn fnv(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    mix(&mut hash, text);
    hash
}

/// Add `text` to a running hash (for summaries of what a folder's file holds).
pub fn mix(hash: &mut u64, text: &str) {
    for byte in text.bytes().chain(std::iter::once(0xff)) {
        *hash = (*hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
}

pub const HASH_START: u64 = 0xcbf2_9ce4_8422_2325;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Numbers as text, compactly: half precision (plenty for fingerprints and CLIP numbers, which
/// are compared by angle), in base64. A third the size of plain hex floats.
pub fn pack(v: &[f32]) -> String {
    let bytes: Vec<u8> = v.iter().flat_map(|x| half::f16::from_f32(*x).to_le_bytes()).collect();
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The numbers `pack` wrote.
pub fn unpack(s: &str) -> Vec<f32> {
    let mut bytes = Vec::with_capacity(s.len() * 3 / 4);
    let mut n = 0u32;
    let mut have = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        n = n << 6 | v as u32;
        have += 6;
        if have >= 8 {
            have -= 8;
            bytes.push((n >> have & 0xff) as u8);
        }
    }
    bytes.chunks_exact(2).map(|b| half::f16::from_le_bytes([b[0], b[1]]).to_f32()).collect()
}

/// The file name for a photo folder's data (local place; see `Place::file_name`).
pub fn folder_file(folder: &Path) -> String {
    let text = folder.display().to_string();
    let text = if cfg!(windows) { text.to_lowercase() } else { text };
    format!("{:016x}.tsv", fnv(&text))
}

/// A new unique id (for a person), 32 hex digits.
pub fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut out = String::new();
    for part in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(now());
        h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        h.write_u64(n);
        h.write_u64(part);
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// One kind of data files (a subfolder of the library data): read them all, and write back only
/// those whose text changed, removing the ones no longer needed.
pub struct Files {
    dir: Option<PathBuf>,
    /// Where these files live, and what kind they are ("faces", "content", "people").
    pub place: Place,
    kind: String,
    /// The text last read or written for each file, by a hash of it.
    known: HashMap<String, u64>,
    /// In a shared place: that text itself, to merge another computer's changes against.
    base: HashMap<String, String>,
    /// For per-folder files: a summary of what each holds (see `save_folders`).
    summaries: HashMap<String, u64>,
}

impl Files {
    pub fn new(kind: &str) -> Self {
        let place = Place { photos: None, data: root().unwrap_or_default() };
        Self::at(&place, kind)
    }

    /// The files of `kind` in `place`.
    pub fn at(place: &Place, kind: &str) -> Self {
        let dir = (!place.data.as_os_str().is_empty()).then(|| place.data.join(kind));
        Self { dir, place: place.clone(), kind: kind.to_owned(), known: HashMap::new(), base: HashMap::new(), summaries: HashMap::new() }
    }

    /// What the per-folder files hold now, as just read (so the first save rewrites nothing).
    pub fn set_summaries(&mut self, summaries: &HashMap<PathBuf, u64>) {
        // Only for files that are really there (read or written by this copy): a folder whose
        // data has no file here yet (a library just shared) must be written.
        self.summaries = summaries.iter().map(|(f, h)| (self.place.file_name(f), *h)).filter(|(name, _)| self.known.contains_key(name)).collect();
    }

    fn remember(&mut self, name: &str, text: &str) {
        self.known.insert(name.to_owned(), fnv(text));
        if self.place.shared() {
            self.base.insert(name.to_owned(), text.to_owned());
        }
    }

    /// Whether `name` on disk is not what this copy last read or wrote (another computer
    /// changed it); then its text.
    pub fn changed_on_disk(&self, name: &str) -> Option<String> {
        let dir = self.dir.as_ref()?;
        let text = std::fs::read_to_string(dir.join(name)).ok()?;
        (self.known.get(name) != Some(&fnv(&text))).then_some(text)
    }

    /// Read one file again (another computer changed it); None if it is gone.
    pub fn reload(&mut self, name: &str) -> Option<String> {
        let dir = self.dir.clone()?;
        match std::fs::read_to_string(dir.join(name)) {
            Ok(text) => {
                self.remember(name, &text);
                Some(text)
            }
            Err(_) => {
                self.known.remove(name);
                self.base.remove(name);
                None
            }
        }
    }

    /// Save per-folder files efficiently: `summaries` holds a cheap hash of what each folder's
    /// file should hold; only folders whose hash changed are built (by `text`) and written, and
    /// files of folders no longer there are removed. In a shared place, a file another computer
    /// changed meanwhile is merged with it, not overwritten: the merged texts are returned, to be
    /// taken into memory.
    pub fn save_folders(&mut self, summaries: &HashMap<PathBuf, u64>, text: impl Fn(&Path) -> String) -> Vec<String> {
        let mut merged = Vec::new();
        let Some(dir) = self.dir.clone() else { return merged };
        let _ = std::fs::create_dir_all(&dir);
        let mut wanted = std::collections::HashSet::new();
        for (folder, hash) in summaries {
            let name = self.place.file_name(folder);
            wanted.insert(name.clone());
            if self.summaries.get(&name) == Some(hash) {
                continue;
            }
            let mut body = text(folder);
            if self.place.shared() {
                if let Some(theirs) = self.changed_on_disk(&name) {
                    body = merge(self.base.get(&name).map(String::as_str).unwrap_or(""), &body, &theirs);
                    merged.push(body.clone());
                }
            }
            let tmp = dir.join(format!("{name}.{}.tmp", instance()));
            if std::fs::write(&tmp, &body).is_ok() && std::fs::rename(&tmp, dir.join(&name)).is_ok() {
                self.summaries.insert(name.clone(), *hash);
                self.remember(&name, &body);
                note_change(&self.place, &self.kind, &name);
            } else {
                let _ = std::fs::remove_file(&tmp);
            }
        }
        // Files here for folders whose data now lives elsewhere, or that are gone: removed.
        let mut gone: Vec<String> = self.summaries.keys().chain(self.known.keys()).filter(|n| !wanted.contains(*n)).cloned().collect();
        gone.sort();
        gone.dedup();
        for name in gone {
            // In a shared place, not if another computer has written it since.
            if !(self.place.shared() && self.changed_on_disk(&name).is_some()) {
                let _ = std::fs::remove_file(dir.join(&name));
                note_change(&self.place, &self.kind, &name);
            }
            self.summaries.remove(&name);
            self.known.remove(&name);
            self.base.remove(&name);
        }
        merged
    }

    /// Whether this kind has any files yet.
    pub fn exists(&self) -> bool {
        self.dir.as_ref().is_some_and(|d| d.is_dir())
    }

    /// Every file's text, by file name (read many at a time: over the network, one after
    /// another would be slow).
    pub fn read_all(&mut self) -> Vec<(String, String)> {
        use rayon::prelude::*;
        let Some(dir) = &self.dir else { return Vec::new() };
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        let paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "tsv")).collect();
        let out: Vec<(String, String)> = paths
            .par_iter()
            .filter_map(|path| Some((path.file_name()?.to_string_lossy().into_owned(), std::fs::read_to_string(path).ok()?)))
            .collect();
        for (name, text) in &out {
            self.remember(name, text);
        }
        out
    }

    /// Make the files exactly `files` (name -> text), touching only those that differ. In a
    /// shared place, a file another computer wrote since this one last read it is not removed.
    pub fn write_all(&mut self, files: HashMap<String, String>) {
        let Some(dir) = self.dir.clone() else { return };
        let _ = std::fs::create_dir_all(&dir);
        for (name, text) in &files {
            let hash = fnv(text);
            if self.known.get(name) == Some(&hash) {
                continue;
            }
            // Write beside, then swap in, so a crash never leaves half a file.
            let tmp = dir.join(format!("{name}.{}.tmp", instance()));
            if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, dir.join(name)).is_ok() {
                self.remember(name, text);
                note_change(&self.place, &self.kind, name);
            } else {
                let _ = std::fs::remove_file(&tmp);
            }
        }
        let gone: Vec<String> = self.known.keys().filter(|n| !files.contains_key(*n)).cloned().collect();
        for name in gone {
            if !(self.place.shared() && self.changed_on_disk(&name).is_some()) {
                let _ = std::fs::remove_file(dir.join(&name));
                note_change(&self.place, &self.kind, &name);
            }
            self.known.remove(&name);
            self.base.remove(&name);
        }
    }
}


/// Whether `photo` is in one of the folders that could not be read this time.
pub fn unreachable(photo: &Path, folders: &[PathBuf]) -> bool {
    folders.iter().any(|f| photo.starts_with(f))
}

/// Whether to forget a photo not found this time: not if its folder could not be read, and not
/// until it has been missing for `GRACE_DAYS`.
pub fn forget(photo: &Path, last_seen: u32, unreachable_folders: &[PathBuf], today: u32) -> bool {
    !unreachable(photo, unreachable_folders) && today.saturating_sub(last_seen) > GRACE_DAYS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_photos_are_kept_for_a_while_and_unreachable_ones_always() {
        let photo = Path::new("/nas/2025/a.arw");
        assert!(!forget(photo, 100, &[], 100 + GRACE_DAYS), "within the grace period");
        assert!(forget(photo, 100, &[], 101 + GRACE_DAYS), "after it");
        assert!(!forget(photo, 100, &[PathBuf::from("/nas")], 1000), "folder unreachable");
    }

    #[test]
    fn numbers_pack_small_and_come_back_close() {
        let v: Vec<f32> = (0..128).map(|i| ((i as f32) * 0.37).sin() * 0.2).collect();
        let s = pack(&v);
        assert_eq!(s.len(), 344, "256 bytes in base64");
        let back = unpack(&s);
        assert_eq!(back.len(), v.len());
        assert!(v.iter().zip(&back).all(|(a, b)| (a - b).abs() < 2e-4));
    }

    #[test]
    fn merging_keeps_both_computers_changes() {
        let base = "folder\t2025/01\nP\ta|1\ta.ARW\t100\nF\ta|1\t0\tbox\tprint\tP1\t\n";
        // Here: the face was moved to person P2. There: a new photo was read.
        let mine = "folder\t2025/01\nP\ta|1\ta.ARW\t100\nF\ta|1\t0\tbox\tprint\tP2\t\n";
        let theirs = "folder\t2025/01\nP\ta|1\ta.ARW\t100\nF\ta|1\t0\tbox\tprint\tP1\t\nP\tb|2\tb.ARW\t100\n";
        let m = merge(base, mine, theirs);
        assert!(m.contains("F\ta|1\t0\tbox\tprint\tP2"), "my change kept: {m}");
        assert!(m.contains("P\tb|2\tb.ARW"), "their new photo kept: {m}");
        assert!(m.starts_with("folder\t"));
        // They removed a photo I did not touch: it goes.
        let theirs2 = "folder\t2025/01\n";
        let m2 = merge(base, base, theirs2);
        assert!(!m2.contains("P\ta|1"), "{m2}");
    }

    #[test]
    fn shared_places_name_folders_the_same_on_every_computer() {
        let win = Place { photos: Some(PathBuf::from(r"\\server\Photos")), data: PathBuf::new() };
        let mac = Place { photos: Some(PathBuf::from("/Volumes/Photos")), data: PathBuf::new() };
        let a = PathBuf::from(r"\\server\Photos").join("2025").join("01");
        let b = PathBuf::from("/Volumes/Photos").join("2025").join("01");
        assert_eq!(win.written(&a), "2025/01");
        assert_eq!(mac.written(&b), "2025/01");
        assert_eq!(win.file_name(&a), mac.file_name(&b));
        assert_eq!(win.folder("2025/01"), a);
        assert!(win.holds(&a));
        assert!(!win.holds(Path::new(r"O:\Photo WIP")));
    }

    #[test]
    fn ids_differ() {
        assert_ne!(new_id(), new_id());
        assert_eq!(new_id().len(), 32);
    }
}
