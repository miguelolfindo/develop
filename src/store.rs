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

/// The file name for a photo folder's data.
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
    /// The text last read or written for each file, by a hash of it.
    known: HashMap<String, u64>,
    /// For per-folder files: a summary of what each holds (see `save_folders`).
    summaries: HashMap<String, u64>,
}

impl Files {
    pub fn new(kind: &str) -> Self {
        Self { dir: root().map(|r| r.join(kind)), known: HashMap::new(), summaries: HashMap::new() }
    }

    /// What the per-folder files hold now, as just read (so the first save rewrites nothing).
    pub fn set_summaries(&mut self, summaries: &HashMap<PathBuf, u64>) {
        self.summaries = summaries.iter().map(|(f, h)| (folder_file(f), *h)).collect();
    }

    /// Save per-folder files efficiently: `summaries` holds a cheap hash of what each folder's
    /// file should hold; only folders whose hash changed are built (by `text`) and written, and
    /// files of folders no longer there are removed.
    pub fn save_folders(&mut self, summaries: &HashMap<PathBuf, u64>, text: impl Fn(&Path) -> String) {
        let Some(dir) = self.dir.clone() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let mut wanted = std::collections::HashSet::new();
        for (folder, hash) in summaries {
            let name = folder_file(folder);
            wanted.insert(name.clone());
            if self.summaries.get(&name) == Some(hash) {
                continue;
            }
            let tmp = dir.join(format!("{name}.tmp"));
            if std::fs::write(&tmp, text(folder)).is_ok() && std::fs::rename(&tmp, dir.join(&name)).is_ok() {
                self.summaries.insert(name, *hash);
            }
        }
        let gone: Vec<String> = self.summaries.keys().filter(|n| !wanted.contains(*n)).cloned().collect();
        for name in gone {
            let _ = std::fs::remove_file(dir.join(&name));
            self.summaries.remove(&name);
        }
    }

    /// Whether this kind has any files yet.
    pub fn exists(&self) -> bool {
        self.dir.as_ref().is_some_and(|d| d.is_dir())
    }

    /// Every file's text.
    pub fn read_all(&mut self) -> Vec<String> {
        let Some(dir) = &self.dir else { return Vec::new() };
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut out = Vec::new();
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_some_and(|x| x == "tsv") {
                if let (Some(name), Ok(text)) = (path.file_name().map(|n| n.to_string_lossy().into_owned()), std::fs::read_to_string(&path)) {
                    self.known.insert(name, fnv(&text));
                    out.push(text);
                }
            }
        }
        out
    }

    /// Make the files exactly `files` (name -> text), touching only those that differ.
    pub fn write_all(&mut self, files: HashMap<String, String>) {
        let Some(dir) = self.dir.clone() else { return };
        let _ = std::fs::create_dir_all(&dir);
        for (name, text) in &files {
            let hash = fnv(text);
            if self.known.get(name) == Some(&hash) {
                continue;
            }
            // Write beside, then swap in, so a crash never leaves half a file.
            let tmp = dir.join(format!("{name}.tmp"));
            if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, dir.join(name)).is_ok() {
                self.known.insert(name.clone(), hash);
            }
        }
        let gone: Vec<String> = self.known.keys().filter(|n| !files.contains_key(*n)).cloned().collect();
        for name in gone {
            let _ = std::fs::remove_file(dir.join(&name));
            self.known.remove(&name);
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
    fn ids_differ() {
        assert_ne!(new_id(), new_id());
        assert_eq!(new_id().len(), 32);
    }
}
