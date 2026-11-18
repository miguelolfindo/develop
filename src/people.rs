//! People: faces found across the library, grouped into people the user can name.
//!
//! Faces are found and fingerprinted (skin.rs, faces.rs) while the library is indexed. Each
//! face is filed under its photo's content (file name and size), not its path, so a photo moved
//! to another folder keeps its faces. Similar faces are grouped; the user names a group once and
//! the name is written into each of its photos' `.dvlp` files (as a plain word, like a keyword).
//! The fingerprints and groups stay in Develop's own library data (see store.rs: a file per photo
//! folder, a file per person) and never go into the photos' files.

use std::path::{Path, PathBuf};

/// One face in one photo.
#[derive(Clone, Debug)]
pub struct Face {
    /// The photo's content key (see `key`), and which face in it.
    pub key: String,
    pub n: u8,
    /// Where the photo was last seen.
    pub photo: PathBuf,
    /// The face's box as fractions of the picture (x, y, w, h).
    pub area: [f32; 4],
    pub print: Vec<f32>,
    /// The person (group) it belongs to.
    pub person: u32,
    /// People the user said this face is not (taken out of their set, or dragged away from
    /// them): it is never put back with them, nor suggested as theirs.
    pub not: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct Person {
    pub id: u32,
    pub name: Option<String>,
    /// The average fingerprint of its faces (length 1).
    centre: Vec<f32>,
    /// Its id in the saved files (unique across computers), and when and where it was last
    /// named.
    uid: String,
    named: (u64, String),
}

impl Person {
    fn new(id: u32, centre: Vec<f32>) -> Self {
        Self { id, name: None, centre, uid: crate::store::new_id(), named: (0, String::new()) }
    }
}

pub struct People {
    pub faces: Vec<Face>,
    pub people: Vec<Person>,
    next_id: u32,
    /// Photo content keys already looked at (with or without faces): where the photo was last
    /// seen, and on which day.
    pub seen: std::collections::HashMap<String, (PathBuf, u32)>,
    changed: bool,
    face_files: crate::store::Files,
    people_files: crate::store::Files,
}

impl Default for People {
    fn default() -> Self {
        Self { faces: Vec::new(), people: Vec::new(), next_id: 0, seen: Default::default(), changed: false, face_files: crate::store::Files::new("faces"), people_files: crate::store::Files::new("people") }
    }
}

/// A photo's content key: its file name and size, which survive moving it.
pub fn key(photo: &Path) -> String {
    let size = std::fs::metadata(photo).map(|m| m.len()).unwrap_or(0);
    let name = photo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    format!("{name}|{size}")
}

fn unhex(s: &str) -> Vec<f32> {
    let bytes: Vec<u8> = (0..s.len() / 2).filter_map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect();
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn normalised(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

/// How alike a set of faces is: the median, over its faces, of how alike each is to its three
/// closest others. 0 for fewer than three faces.
fn consistency(prints: &[&Vec<f32>]) -> f32 {
    if prints.len() < 3 {
        return 0.0;
    }
    let mut each: Vec<f32> = prints
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let mut s: Vec<f32> = prints.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, b)| crate::faces::similarity_of(a, b)).collect();
            s.sort_by(|x, y| y.total_cmp(x));
            s.iter().take(3).sum::<f32>() / s.len().min(3) as f32
        })
        .collect();
    each.sort_by(|x, y| x.total_cmp(y));
    each[each.len() / 2]
}

fn file(name: &str) -> Option<PathBuf> {
    crate::settings::settings_dir().map(|d| d.join(name))
}

/// Where a face's small picture is kept (for the People list).
pub fn face_thumb(key: &str, n: u8) -> Option<PathBuf> {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    file("thumbs").map(|d| d.join(format!("face-{hash:016x}-{n}.jpg")))
}

/// Find the faces in a photo's preview and fingerprint them; also saves each face's small
/// picture. Slow (about a tenth of a second): call it without holding the people lock.
pub fn find_faces_in(pic: &image::RgbImage, k: &str) -> Vec<(u8, [f32; 4], Vec<f32>)> {
    let (w, h) = (pic.width() as usize, pic.height() as usize);
    let Ok(found) = crate::skin::faces(pic.as_raw(), w, h) else { return Vec::new() };
    let mut out = Vec::new();
    for (n, f) in found.iter().enumerate().filter(|(_, f)| f.w >= 24.0 && f.h >= 24.0) {
        let Ok(print) = crate::faces::fingerprint(pic.as_raw(), w, h, &f.landmarks) else { continue };
        if let Some(thumb) = face_thumb(k, n as u8) {
            let side = f.w.max(f.h) * 1.3;
            let (cx, cy) = (f.x + f.w / 2.0, f.y + f.h / 2.0);
            let (x0, y0) = ((cx - side / 2.0).max(0.0) as u32, (cy - side / 2.0).max(0.0) as u32);
            let s = (side as u32).min(pic.width().saturating_sub(x0)).min(pic.height().saturating_sub(y0)).max(1);
            let crop = image::imageops::crop_imm(pic, x0, y0, s, s).to_image();
            let small = image::imageops::thumbnail(&crop, 64, 64);
            if let Some(dir) = thumb.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = small.save(&thumb);
        }
        out.push((n as u8, [f.x / w as f32, f.y / h as f32, f.w / w as f32, f.h / h as f32], print));
    }
    out
}

impl People {
    pub fn load() -> Self {
        let mut p = People::default();
        if p.face_files.exists() || p.people_files.exists() {
            p.read_files();
        } else if file("faces.tsv").is_some_and(|f| f.is_file()) {
            // A library from before the per-folder files: read it and write it the new way. The
            // old files stay as they are, so an older Develop still finds its library.
            p.read_old();
            p.changed = true;
            p.save();
        }
        p
    }

    fn read_files(&mut self) {
        let mut ids: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for text in self.people_files.read_all() {
            let (mut uid, mut name, mut centre, mut named) = (String::new(), None, Vec::new(), (0, String::new()));
            for line in text.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                match f.as_slice() {
                    ["person", u] => uid = (*u).to_owned(),
                    ["name", n] => name = (!n.is_empty()).then(|| (*n).to_owned()),
                    ["centre", c] => centre = crate::store::unpack(c),
                    ["named", at, by] => named = (at.parse().unwrap_or(0), (*by).to_owned()),
                    _ => {}
                }
            }
            if uid.is_empty() || centre.is_empty() {
                continue;
            }
            let id = self.next_id;
            self.next_id += 1;
            ids.insert(uid.clone(), id);
            self.people.push(Person { id, name, centre, uid, named });
        }
        let today = crate::store::today();
        for text in self.face_files.read_all() {
            let mut folder = PathBuf::new();
            let mut where_: std::collections::HashMap<String, PathBuf> = std::collections::HashMap::new();
            for line in text.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                match f.as_slice() {
                    ["folder", path] => folder = PathBuf::from(path),
                    ["P", key, name, day] => {
                        let photo = folder.join(name);
                        where_.insert((*key).to_owned(), photo.clone());
                        self.seen.insert((*key).to_owned(), (photo, day.parse().unwrap_or(today)));
                    }
                    ["F", key, n, area, print, person, not] => {
                        let area: Vec<f32> = area.split(',').filter_map(|v| v.parse().ok()).collect();
                        let Some(photo) = where_.get(*key) else { continue };
                        if area.len() != 4 || person.is_empty() {
                            continue;
                        }
                        // An unnamed group has no file of its own: it is rebuilt from its faces.
                        let person = match ids.get(*person) {
                            Some(&id) => id,
                            None => {
                                let id = self.next_id;
                                self.next_id += 1;
                                ids.insert((*person).to_owned(), id);
                                let mut group = Person::new(id, Vec::new());
                                group.uid = (*person).to_owned();
                                self.people.push(group);
                                id
                            }
                        };
                        let not = not.split(',').filter_map(|u| ids.get(u).copied()).collect();
                        self.faces.push(Face { key: (*key).to_owned(), n: n.parse().unwrap_or(0), photo: photo.clone(), area: [area[0], area[1], area[2], area[3]], print: crate::store::unpack(print), person, not });
                    }
                    _ => {}
                }
            }
        }
        // Unnamed groups' average faces, from their faces.
        let rebuild: Vec<u32> = self.people.iter().filter(|p| p.centre.is_empty()).map(|p| p.id).collect();
        for id in rebuild {
            self.recentre(id);
        }
        self.people.retain(|p| !p.centre.is_empty() && p.centre.iter().any(|v| *v != 0.0) || p.name.is_some());
        let summaries = self.summaries().into_iter().map(|(f, (h, _))| (f, h)).collect();
        self.face_files.set_summaries(&summaries);
    }

    /// Per photo folder: a cheap summary of what its file holds (photos, days, faces' people and
    /// boxes; fingerprints never change once found), and its photos' keys.
    fn summaries(&self) -> std::collections::HashMap<PathBuf, (u64, Vec<&str>)> {
        let mut by_key: std::collections::HashMap<&str, Vec<&Face>> = std::collections::HashMap::new();
        for f in &self.faces {
            by_key.entry(f.key.as_str()).or_default().push(f);
        }
        let uid: std::collections::HashMap<u32, &str> = self.people.iter().map(|p| (p.id, p.uid.as_str())).collect();
        let mut out: std::collections::HashMap<PathBuf, (u64, Vec<&str>)> = std::collections::HashMap::new();
        let mut keys: Vec<&String> = self.seen.keys().collect();
        keys.sort();
        for k in keys {
            let (photo, day) = &self.seen[k];
            let Some(folder) = photo.parent() else { continue };
            let entry = out.entry(folder.to_path_buf()).or_insert((crate::store::HASH_START, Vec::new()));
            crate::store::mix(&mut entry.0, &format!("{k}|{}|{day}", photo.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
            for f in by_key.get(k.as_str()).map(Vec::as_slice).unwrap_or(&[]) {
                let not: Vec<&str> = f.not.iter().filter_map(|n| uid.get(n).copied()).collect();
                crate::store::mix(&mut entry.0, &format!("{}|{}|{:?}|{}", f.n, uid.get(&f.person).unwrap_or(&""), f.area, not.join(",")));
            }
            entry.1.push(k.as_str());
        }
        out
    }

    /// The single-file store of 0.4.x (`people.tsv`, `faces.tsv`).
    fn read_old(&mut self) {
        let today = crate::store::today();
        if let Some(text) = file("people.tsv").and_then(|f| std::fs::read_to_string(f).ok()) {
            for line in text.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() == 3 {
                    let id = f[0].parse().unwrap_or(0);
                    self.next_id = self.next_id.max(id + 1);
                    let mut person = Person::new(id, unhex(f[2]));
                    person.name = (!f[1].is_empty()).then(|| f[1].to_owned());
                    self.people.push(person);
                }
            }
        }
        if let Some(text) = file("faces.tsv").and_then(|f| std::fs::read_to_string(f).ok()) {
            let mut no_faces = Vec::new();
            for line in text.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() == 1 && !f[0].is_empty() {
                    no_faces.push(f[0].to_owned()); // a photo without faces (no path in this format)
                } else if f.len() == 6 || f.len() == 7 {
                    let area: Vec<f32> = f[3].split(',').filter_map(|v| v.parse().ok()).collect();
                    if area.len() == 4 {
                        let photo = PathBuf::from(f[2]);
                        self.seen.insert(f[0].to_owned(), (photo.clone(), today));
                        self.faces.push(Face { key: f[0].to_owned(), n: f[1].parse().unwrap_or(0), photo, area: [area[0], area[1], area[2], area[3]], print: unhex(f[4]), person: f[5].parse().unwrap_or(0), not: f.get(6).map(|s| s.split(',').filter_map(|v| v.parse().ok()).collect()).unwrap_or_default() });
                    }
                }
            }
            // Photos without faces had no path then; the library's own index has it.
            if !no_faces.is_empty() {
                let places = crate::library::index_paths();
                for k in no_faces {
                    if let Some(photo) = places.get(&k) {
                        self.seen.insert(k, (photo.clone(), today));
                    }
                }
            }
        }
    }

    pub fn save(&mut self) {
        if !self.changed {
            return;
        }
        self.changed = false;
        // One file per named person (only those that changed are written); unnamed groups are
        // rebuilt from their faces, so they need none.
        let mut people = std::collections::HashMap::new();
        for p in self.people.iter().filter(|p| p.name.is_some()) {
            let text = format!("person\t{}\nname\t{}\ncentre\t{}\nnamed\t{}\t{}\n", p.uid, p.name.as_deref().unwrap_or(""), crate::store::pack(&p.centre), p.named.0, p.named.1);
            people.insert(format!("{}.tsv", p.uid), text);
        }
        self.people_files.write_all(people);
        // One file per photo folder: each photo looked at, then its faces. Only folders whose
        // summary changed are written.
        let mut files = std::mem::replace(&mut self.face_files, crate::store::Files::new("faces"));
        let summaries = self.summaries();
        let hashes: std::collections::HashMap<PathBuf, u64> = summaries.iter().map(|(f, (h, _))| (f.clone(), *h)).collect();
        let mut by_key: std::collections::HashMap<&str, Vec<&Face>> = std::collections::HashMap::new();
        for f in &self.faces {
            by_key.entry(f.key.as_str()).or_default().push(f);
        }
        let uid: std::collections::HashMap<u32, &str> = self.people.iter().map(|p| (p.id, p.uid.as_str())).collect();
        let text = |folder: &Path| -> String {
            let mut text = format!("folder\t{}\n", folder.display());
            for k in summaries.get(folder).map(|(_, keys)| keys.as_slice()).unwrap_or(&[]) {
                let (photo, day) = &self.seen[*k];
                text.push_str(&format!("P\t{k}\t{}\t{day}\n", photo.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
                for f in by_key.get(k).map(Vec::as_slice).unwrap_or(&[]) {
                    let Some(person) = uid.get(&f.person) else { continue };
                    let not: Vec<&str> = f.not.iter().filter_map(|n| uid.get(n).copied()).collect();
                    text.push_str(&format!("F\t{}\t{}\t{:.4},{:.4},{:.4},{:.4}\t{}\t{person}\t{}\n", f.key, f.n, f.area[0], f.area[1], f.area[2], f.area[3], crate::store::pack(&f.print), not.join(",")));
                }
            }
            text
        };
        files.save_folders(&hashes, text);
        self.face_files = files;
    }

    /// Whether `photo`'s content was looked at already. Notes where the photo is now, if it
    /// moved.
    pub fn known(&mut self, photo: &Path, k: &str) -> bool {
        let today = crate::store::today();
        let Some((was, day)) = self.seen.get_mut(k) else { return false };
        if was.as_path() != photo || *day != today {
            *was = photo.to_path_buf();
            *day = today;
            self.changed = true;
        }
        for f in self.faces.iter_mut().filter(|f| f.key == k && f.photo != photo) {
            f.photo = photo.to_path_buf();
        }
        true
    }

    /// File faces found by `find_faces`.
    pub fn add_faces(&mut self, photo: &Path, k: &str, found: Vec<(u8, [f32; 4], Vec<f32>)>) {
        self.seen.insert(k.to_owned(), (photo.to_path_buf(), crate::store::today()));
        self.changed = true;
        for (n, area, print) in found {
            let person = self.assign(&print);
            self.faces.push(Face { key: k.to_owned(), n, photo: photo.to_path_buf(), area, print, person, not: Vec::new() });
        }
    }

    /// The person a new face belongs to: the most similar one if similar enough, else a new one.
    fn assign(&mut self, print: &[f32]) -> u32 {
        let best = self.people.iter().map(|p| (p.id, crate::faces::similarity_of(&p.centre, print))).max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((id, s)) = best {
            if s >= crate::faces::SAME_PERSON_GROUP {
                // Move the centre a little toward the new face.
                let members = self.faces.iter().filter(|f| f.person == id).count() as f32;
                if let Some(p) = self.people.iter_mut().find(|p| p.id == id) {
                    let mixed: Vec<f32> = p.centre.iter().zip(print).map(|(c, x)| c * members + x).collect();
                    p.centre = normalised(mixed);
                }
                return id;
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.people.push(Person::new(id, print.to_vec()));
        id
    }

    /// Forget photos no longer in the library (`keep` holds the content keys found this time):
    /// only once missing for a while, and never those in folders that could not be read.
    pub fn prune(&mut self, keep: &std::collections::HashSet<String>, unreachable: &[PathBuf]) {
        let before = self.faces.len() + self.seen.len();
        let today = crate::store::today();
        self.seen.retain(|k, (photo, day)| keep.contains(k) || !crate::store::forget(photo, *day, unreachable, today));
        let seen = &self.seen;
        self.faces.retain(|f| seen.contains_key(&f.key));
        let used: std::collections::HashSet<u32> = self.faces.iter().map(|f| f.person).collect();
        self.people.retain(|p| used.contains(&p.id) || p.name.is_some());
        if self.faces.len() + self.seen.len() != before {
            self.changed = true;
        }
    }

    /// People to show: named ones first, then groups of at least two faces, largest first.
    /// Each with its number of photos and a face to show.
    pub fn list(&self) -> Vec<(u32, Option<String>, usize, Option<(String, u8)>)> {
        let mut out: Vec<(u32, Option<String>, usize, Option<(String, u8)>)> = self
            .people
            .iter()
            .map(|p| {
                let faces: Vec<&Face> = self.faces.iter().filter(|f| f.person == p.id).collect();
                let mut photos: Vec<&Path> = faces.iter().map(|f| f.photo.as_path()).collect();
                photos.sort();
                photos.dedup();
                // The face most like the person's centre stands for them.
                let face = faces.iter().max_by(|a, b| crate::faces::similarity_of(&a.print, &p.centre).total_cmp(&crate::faces::similarity_of(&b.print, &p.centre))).map(|f| (f.key.clone(), f.n));
                (p.id, p.name.clone(), photos.len(), face)
            })
            .filter(|(_, name, count, _)| name.is_some() || *count >= 2)
            .collect();
        out.sort_by(|a, b| b.1.is_some().cmp(&a.1.is_some()).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));
        out
    }

    /// The photos a person is in, by capture order of their paths.
    pub fn photos_of(&self, id: u32) -> Vec<PathBuf> {
        let mut photos: Vec<PathBuf> = self.faces.iter().filter(|f| f.person == id).map(|f| f.photo.clone()).collect();
        photos.sort();
        photos.dedup();
        photos
    }

    /// Name a person. A name another person already has merges the two. Returns the photos
    /// whose people names change (their `.dvlp` files must be updated).
    pub fn rename(&mut self, id: u32, name: &str) -> Vec<PathBuf> {
        let name = crate::settings::clean_name(name);
        let mut touched = self.photos_of(id);
        if let Some(other) = self.people.iter().find(|p| p.id != id && p.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&name))).map(|p| p.id) {
            // Merge `id` into the existing person.
            for f in self.faces.iter_mut() {
                if f.person == id {
                    f.person = other;
                }
                for n in f.not.iter_mut().filter(|n| **n == id) {
                    *n = other;
                }
            }
            self.people.retain(|p| p.id != id);
            self.recentre(other);
            touched.extend(self.photos_of(other));
        } else if let Some(p) = self.people.iter_mut().find(|p| p.id == id) {
            p.name = (!name.is_empty()).then_some(name);
            p.named = (crate::store::now(), crate::store::computer());
        }
        self.changed = true;
        touched.sort();
        touched.dedup();
        touched
    }

    /// Take `photo`'s faces out of person `id` (a wrong match); they become people of their own.
    pub fn not_this_person(&mut self, id: u32, photo: &Path) -> Vec<PathBuf> {
        let moved: Vec<usize> = self.faces.iter().enumerate().filter(|(_, f)| f.person == id && f.photo == photo).map(|(i, _)| i).collect();
        for i in moved {
            let nid = self.next_id;
            self.next_id += 1;
            self.people.push(Person::new(nid, self.faces[i].print.clone()));
            self.faces[i].person = nid;
            if !self.faces[i].not.contains(&id) {
                self.faces[i].not.push(id);
            }
        }
        self.recentre(id);
        self.changed = true;
        vec![photo.to_path_buf()]
    }

    /// Move `photo`'s faces from person `from` to person `to` (dragged onto the right person).
    /// An unnamed group left empty goes away. Returns the photos whose names change.
    pub fn move_photo(&mut self, from: u32, photo: &Path, to: u32) -> Vec<PathBuf> {
        if from == to || !self.people.iter().any(|p| p.id == to) {
            return Vec::new();
        }
        let mut moved = false;
        for f in self.faces.iter_mut().filter(|f| f.person == from && f.photo == photo) {
            f.person = to;
            f.not.retain(|n| *n != to);
            if !f.not.contains(&from) {
                f.not.push(from);
            }
            moved = true;
        }
        if !moved {
            return Vec::new();
        }
        self.recentre(from);
        self.recentre(to);
        if !self.faces.iter().any(|f| f.person == from) {
            self.people.retain(|p| p.id != from || p.name.is_some());
        }
        self.changed = true;
        vec![photo.to_path_buf()]
    }

    /// Check the unnamed faces against the named people again (each named person gains faces as
    /// the user names and corrects), and move the clear matches to them: a face must closely match
    /// that person's own faces and clearly better than anyone else's; never to someone the face
    /// was said not to be.
    /// Borderline faces stay where they are (they show as possible matches). Returns how many
    /// faces moved, and the photos whose names change.
    pub fn regroup(&mut self) -> (usize, Vec<PathBuf>, Vec<String>) {
        // How alike a face must be to a person's closest faces, and how much more than to anyone
        // else. Measured: the same person 0.93-0.98 in similar light, two different women
        // 0.37-0.41.
        const NEAR: f32 = 0.55;
        const MARGIN: f32 = 0.08;
        // Compared with a person's actual faces (the mean of the three closest), not their
        // average face: an average over many varied faces looks a little like everyone.
        // Only people whose own faces are consistent take new ones. A set mixing people, or
        // holding the backs of heads or animals, matches loose faces too easily. Measured on a
        // library of 6,000 photos: real people 0.79-0.92, mixed sets 0.50-0.65.
        const CONSISTENT: f32 = 0.75;
        let named_ids: Vec<u32> = self.people.iter().filter(|p| p.name.is_some()).map(|p| p.id).collect();
        if named_ids.is_empty() {
            return (0, Vec::new(), Vec::new());
        }
        let all: Vec<(u32, Vec<&Vec<f32>>)> = named_ids.iter().map(|id| (*id, self.faces.iter().filter(|f| f.person == *id).map(|f| &f.print).collect())).collect();
        let mut skipped = Vec::new();
        let members: Vec<(u32, Vec<&Vec<f32>>)> = all
            .into_iter()
            .filter(|(id, faces)| {
                let ok = consistency(faces) >= CONSISTENT;
                if !ok {
                    skipped.extend(self.name_of(*id));
                }
                ok
            })
            .collect();
        let unnamed: std::collections::HashSet<u32> = self.people.iter().filter(|p| p.name.is_none()).map(|p| p.id).collect();
        let mut moves: Vec<(usize, u32)> = Vec::new();
        for (i, f) in self.faces.iter().enumerate() {
            if !unnamed.contains(&f.person) {
                continue;
            }
            let mut scores: Vec<(u32, f32)> = members
                .iter()
                .filter(|(id, faces)| !f.not.contains(id) && !faces.is_empty())
                .map(|(id, faces)| {
                    let mut s: Vec<f32> = faces.iter().map(|p| crate::faces::similarity_of(p, &f.print)).collect();
                    s.sort_by(|a, b| b.total_cmp(a));
                    let k = s.len().min(3);
                    (*id, s[..k].iter().sum::<f32>() / k as f32)
                })
                .collect();
            scores.sort_by(|a, b| b.1.total_cmp(&a.1));
            let Some(&(id, top)) = scores.first() else { continue };
            let second = scores.get(1).map_or(0.0, |s| s.1);
            if top >= NEAR && top - second >= MARGIN {
                moves.push((i, id));
            }
        }
        skipped.sort();
        if moves.is_empty() {
            return (0, Vec::new(), skipped);
        }
        let mut touched: Vec<PathBuf> = Vec::new();
        let mut changed_people = std::collections::HashSet::new();
        for &(i, to) in &moves {
            changed_people.insert(self.faces[i].person);
            changed_people.insert(to);
            self.faces[i].person = to;
            touched.push(self.faces[i].photo.clone());
        }
        for id in changed_people {
            self.recentre(id);
        }
        let used: std::collections::HashSet<u32> = self.faces.iter().map(|f| f.person).collect();
        self.people.retain(|p| used.contains(&p.id) || p.name.is_some());
        self.changed = true;
        touched.sort();
        touched.dedup();
        (moves.len(), touched, skipped)
    }

    /// A person's faces, one per photo (the first if a photo has two), in path order.
    pub fn faces_of(&self, id: u32) -> Vec<Face> {
        let mut faces: Vec<Face> = self.faces.iter().filter(|f| f.person == id).cloned().collect();
        faces.sort_by(|a, b| a.photo.cmp(&b.photo).then(a.n.cmp(&b.n)));
        faces.dedup_by(|a, b| a.photo == b.photo);
        faces
    }

    /// The face that stands for a person: the one most like their average.
    pub fn best_face(&self, id: u32) -> Option<Face> {
        let p = self.people.iter().find(|p| p.id == id)?;
        self.faces.iter().filter(|f| f.person == id).max_by(|a, b| crate::faces::similarity_of(&a.print, &p.centre).total_cmp(&crate::faces::similarity_of(&b.print, &p.centre))).cloned()
    }

    /// Faces elsewhere that look somewhat like person `id` but not enough to have joined them:
    /// from unnamed groups, one per photo, most alike first, with how alike (0..1).
    pub fn maybe(&self, id: u32) -> Vec<(Face, f32)> {
        let Some(me) = self.people.iter().find(|p| p.id == id) else { return Vec::new() };
        let named: std::collections::HashSet<u32> = self.people.iter().filter(|p| p.name.is_some()).map(|p| p.id).collect();
        let mine: std::collections::HashSet<&Path> = self.faces.iter().filter(|f| f.person == id).map(|f| f.photo.as_path()).collect();
        let mut out: Vec<(Face, f32)> = self
            .faces
            .iter()
            .filter(|f| f.person != id && !named.contains(&f.person) && !mine.contains(f.photo.as_path()) && !f.not.contains(&id))
            .map(|f| (f, crate::faces::similarity_of(&f.print, &me.centre)))
            .filter(|(_, s)| *s >= crate::faces::MAYBE_SAME_PERSON)
            .map(|(f, s)| (f.clone(), s))
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut seen = std::collections::HashSet::new();
        out.retain(|(f, _)| seen.insert(f.photo.clone()));
        out.truncate(60);
        out
    }

    /// Named people who look somewhat like person `id` (possible matches), most alike first.
    pub fn similar_named(&self, id: u32) -> Vec<String> {
        let Some(me) = self.people.iter().find(|p| p.id == id) else { return Vec::new() };
        let mut out: Vec<(String, f32)> = self
            .people
            .iter()
            .filter(|p| p.id != id)
            .filter_map(|p| p.name.clone().map(|n| (n, crate::faces::similarity_of(&p.centre, &me.centre))))
            .filter(|(_, s)| *s >= crate::faces::MAYBE_SAME_PERSON)
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        out.into_iter().map(|(n, _)| n).collect()
    }

    pub fn name_of(&self, id: u32) -> Option<String> {
        self.people.iter().find(|p| p.id == id).and_then(|p| p.name.clone())
    }

    fn recentre(&mut self, id: u32) {
        let mut sum = vec![0f32; 128];
        for f in self.faces.iter().filter(|f| f.person == id) {
            for (s, x) in sum.iter_mut().zip(&f.print) {
                *s += x;
            }
        }
        if let Some(p) = self.people.iter_mut().find(|p| p.id == id) {
            p.centre = normalised(sum);
        }
    }

    /// Names of the named people in `photo`.
    pub fn names_in(&self, photo: &Path) -> Vec<String> {
        let ids: Vec<u32> = self.faces.iter().filter(|f| f.photo == photo).map(|f| f.person).collect();
        let mut names: Vec<String> = self.people.iter().filter(|p| ids.contains(&p.id)).filter_map(|p| p.name.clone()).collect();
        names.sort();
        names.dedup();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn print(seed: f32) -> Vec<f32> {
        normalised((0..128).map(|i| ((i as f32 + 1.0) * seed).sin()).collect())
    }

    #[test]
    fn similar_faces_group_and_names_merge() {
        let mut p = People::default();
        let a = print(1.0);
        let mut a2 = a.clone();
        a2[0] += 0.05;
        let a2 = normalised(a2);
        let b = print(7.3);
        let ia = p.assign(&a);
        p.faces.push(Face { key: "x|1".into(), n: 0, photo: "x".into(), area: [0.0; 4], print: a.clone(), person: ia, not: Vec::new() });
        let ia2 = p.assign(&a2);
        p.faces.push(Face { key: "y|1".into(), n: 0, photo: "y".into(), area: [0.0; 4], print: a2, person: ia2, not: Vec::new() });
        let ib = p.assign(&b);
        p.faces.push(Face { key: "z|1".into(), n: 0, photo: "z".into(), area: [0.0; 4], print: b, person: ib, not: Vec::new() });
        assert_eq!(ia, ia2, "the same face groups");
        assert_ne!(ia, ib, "a different face does not");
        p.rename(ia, "Talia");
        assert_eq!(p.names_in(Path::new("y")), vec!["Talia".to_owned()]);
        // Naming the other group the same merges them.
        let touched = p.rename(ib, "talia");
        assert!(touched.contains(&PathBuf::from("z")));
        assert_eq!(p.names_in(Path::new("z")), vec!["Talia".to_owned()]);
        assert_eq!(p.list().len(), 1);
    }

    #[test]
    fn a_photo_moves_to_the_right_person() {
        let mut p = People::default();
        let (a, b) = (print(1.0), print(7.3));
        let ia = p.assign(&a);
        p.faces.push(Face { key: "x|1".into(), n: 0, photo: "x".into(), area: [0.0; 4], print: a, person: ia, not: Vec::new() });
        let ib = p.assign(&b);
        p.faces.push(Face { key: "y|1".into(), n: 0, photo: "y".into(), area: [0.0; 4], print: b, person: ib, not: Vec::new() });
        p.rename(ia, "Talia");
        let touched = p.move_photo(ib, Path::new("y"), ia);
        assert_eq!(touched, vec![PathBuf::from("y")]);
        assert_eq!(p.names_in(Path::new("y")), vec!["Talia".to_owned()]);
        assert!(!p.people.iter().any(|x| x.id == ib), "the emptied unnamed group goes");
        assert_eq!(p.faces_of(ia).len(), 2);
    }

    #[test]
    fn regroup_moves_clear_matches_but_not_rejected_faces() {
        let mut p = People::default();
        let (a, b) = (print(1.0), print(7.3));
        // Talia: three faces.
        let ia = p.assign(&a);
        for k in ["x|1", "x|2", "x|3"] {
            p.faces.push(Face { key: k.into(), n: 0, photo: "x".into(), area: [0.0; 4], print: a.clone(), person: ia, not: Vec::new() });
        }
        p.rename(ia, "Talia");
        // Two faces like hers that ended up in groups of their own, and one unlike her.
        let mut near = a.clone();
        near[3] += 0.05;
        let near = normalised(near);
        for (k, pr) in [("y|1", near.clone()), ("z|1", near.clone()), ("w|1", b.clone())] {
            let nid = p.next_id;
            p.next_id += 1;
            p.people.push(Person::new(nid, pr.clone()));
            p.faces.push(Face { key: k.into(), n: 0, photo: k[..1].into(), area: [0.0; 4], print: pr, person: nid, not: Vec::new() });
        }
        // The user said z is not Talia.
        let zi = p.faces.iter().position(|f| f.key == "z|1").unwrap();
        p.faces[zi].not.push(ia);
        let (moved, touched, skipped) = p.regroup();
        assert!(skipped.is_empty());
        assert_eq!(moved, 1, "only y joins Talia");
        assert_eq!(touched, vec![PathBuf::from("y")]);
        assert_eq!(p.names_in(Path::new("y")), vec!["Talia".to_owned()]);
        assert!(p.names_in(Path::new("z")).is_empty(), "a face said not to be her stays out");
        assert!(p.names_in(Path::new("w")).is_empty(), "an unlike face stays out");
        // And "not her" survives saving as text: the 7th column.
        assert!(p.maybe(ia).iter().all(|(f, _)| f.key != "z|1"));
    }

    #[test]
    fn fingerprints_survive_saving_as_text() {
        let v = print(3.0);
        let back = crate::store::unpack(&crate::store::pack(&v));
        assert!(crate::faces::similarity_of(&v, &back) > 0.9999);
    }
}
