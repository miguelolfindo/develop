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
    /// The faces files and the people files of each place the library keeps data (the local
    /// place first, then each shared library; see store.rs).
    face_files: Vec<crate::store::Files>,
    people_files: Vec<crate::store::Files>,
    /// The People list and each person's best face, worked out once for the state they were
    /// worked out from (see `generation`): with tens of thousands of faces, working them out
    /// takes a while, and the window asks for them on every frame.
    cache: std::cell::RefCell<Option<Summary>>,
}

/// One entry of the People list: id, name, photos, the face that stands for them.
pub type ListEntry = (u32, Option<String>, usize, Option<(String, u8)>);

struct Summary {
    generation: u64,
    /// When it was worked out, from how many faces, and the state of those faces and the
    /// names then: while faces are being found, new ones alone don't make it worked out again
    /// more than every couple of seconds.
    made: std::time::Instant,
    faces: usize,
    core: u64,
    list: Vec<ListEntry>,
    best: std::collections::HashMap<u32, usize>,
}

impl Default for People {
    fn default() -> Self {
        Self { faces: Vec::new(), people: Vec::new(), next_id: 0, seen: Default::default(), changed: false, face_files: vec![crate::store::Files::new("faces")], people_files: vec![crate::store::Files::new("people")], cache: Default::default() }
    }
}

/// A person file's contents.
struct PersonFile {
    uid: String,
    name: Option<String>,
    centre: Vec<f32>,
    named: (u64, String),
}

fn parse_person(text: &str) -> Option<PersonFile> {
    let mut p = PersonFile { uid: String::new(), name: None, centre: Vec::new(), named: (0, String::new()) };
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f.as_slice() {
            ["person", u] => p.uid = (*u).to_owned(),
            ["name", n] => p.name = (!n.is_empty()).then(|| (*n).to_owned()),
            ["centre", c] => p.centre = crate::store::unpack(c),
            ["named", at, by] => p.named = (at.parse().unwrap_or(0), (*by).to_owned()),
            _ => {}
        }
    }
    (!p.uid.is_empty()).then_some(p)
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
        let places = crate::store::places();
        p.face_files = places.iter().map(|pl| crate::store::Files::at(pl, "faces")).collect();
        p.people_files = places.iter().map(|pl| crate::store::Files::at(pl, "people")).collect();
        if p.face_files.iter().any(|f| f.exists()) || p.people_files.iter().any(|f| f.exists()) {
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
        // People: from every place; where two places differ, the newest naming wins.
        let mut found: std::collections::HashMap<String, PersonFile> = std::collections::HashMap::new();
        for files in &mut self.people_files {
            for (_, text) in files.read_all() {
                if let Some(pf) = parse_person(&text) {
                    if found.get(&pf.uid).map_or(true, |had| pf.named.0 > had.named.0) {
                        found.insert(pf.uid.clone(), pf);
                    }
                }
            }
        }
        let mut ids: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for (_, pf) in found {
            if pf.centre.is_empty() {
                continue;
            }
            let id = self.next_id;
            self.next_id += 1;
            ids.insert(pf.uid.clone(), id);
            self.people.push(Person { id, name: pf.name, centre: pf.centre, uid: pf.uid, named: pf.named });
        }
        // Faces: each place's folder files.
        for i in 0..self.face_files.len() {
            let place = self.face_files[i].place.clone();
            for (_, text) in self.face_files[i].read_all() {
                self.take_folder_text(&place, &text, &mut ids, false);
            }
        }
        // Unnamed groups' average faces, from their faces.
        let rebuild: Vec<u32> = self.people.iter().filter(|p| p.centre.is_empty()).map(|p| p.id).collect();
        for id in rebuild {
            self.recentre(id);
        }
        self.people.retain(|p| !p.centre.is_empty() && p.centre.iter().any(|v| *v != 0.0) || p.name.is_some());
        self.reset_summaries();
    }

    /// Take in one folder's faces file. With `replace`, what was known for that folder goes
    /// first (the file was changed by another computer).
    fn take_folder_text(&mut self, place: &crate::store::Place, text: &str, ids: &mut std::collections::HashMap<String, u32>, replace: bool) {
        let today = crate::store::today();
        let mut folder = PathBuf::new();
        let mut where_: std::collections::HashMap<String, PathBuf> = std::collections::HashMap::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["folder", path] => {
                    folder = place.folder(path);
                    if replace {
                        self.seen.retain(|_, (photo, _)| photo.parent() != Some(folder.as_path()));
                        self.faces.retain(|x| x.photo.parent() != Some(folder.as_path()));
                    }
                }
                ["P", key, name, day] => {
                    let photo = folder.join(name);
                    where_.insert((*key).to_owned(), photo.clone());
                    self.seen.insert((*key).to_owned(), (photo, day.parse().unwrap_or(today)));
                    if replace {
                        // The photo's faces come with this file.
                        self.faces.retain(|x| x.key != *key);
                    }
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

    /// The ids of the people known now, by their file ids.
    fn ids(&self) -> std::collections::HashMap<String, u32> {
        self.people.iter().map(|p| (p.uid.clone(), p.id)).collect()
    }

    /// Tell each place's files what their folders hold now (after reading).
    fn reset_summaries(&mut self) {
        let all: std::collections::HashMap<PathBuf, u64> = self.summaries().into_iter().map(|(f, (h, _))| (f, h)).collect();
        let places: Vec<crate::store::Place> = self.face_files.iter().map(|f| f.place.clone()).collect();
        for (i, files) in self.face_files.iter_mut().enumerate() {
            let mine: std::collections::HashMap<PathBuf, u64> = all.iter().filter(|(f, _)| crate::store::place_of(&places, f) == i).map(|(f, h)| (f.clone(), *h)).collect();
            files.set_summaries(&mine);
        }
    }

    /// Another computer changed a file in a shared place (its `.develop` folder is `data`):
    /// read it again. Returns whether anything here changed.
    pub fn reload(&mut self, data: &Path, kind: &str, name: &str) -> bool {
        match kind {
            "people" => {
                let Some(i) = self.people_files.iter().position(|f| f.place.data == data) else { return false };
                let Some(text) = self.people_files[i].reload(name) else { return false };
                let Some(pf) = parse_person(&text) else { return false };
                match self.people.iter_mut().find(|p| p.uid == pf.uid) {
                    Some(p) if pf.named.0 > p.named.0 => {
                        p.name = pf.name;
                        p.named = pf.named;
                        true
                    }
                    Some(_) => false,
                    None => {
                        let id = self.next_id;
                        self.next_id += 1;
                        self.people.push(Person { id, name: pf.name, centre: pf.centre, uid: pf.uid, named: pf.named });
                        true
                    }
                }
            }
            "faces" => {
                let Some(i) = self.face_files.iter().position(|f| f.place.data == data) else { return false };
                let place = self.face_files[i].place.clone();
                let mut ids = self.ids();
                match self.face_files[i].reload(name) {
                    Some(text) => self.take_folder_text(&place, &text, &mut ids, true),
                    None => {
                        // Removed there: forget the folder it was for.
                        let gone: Vec<String> = self.seen.iter().filter(|(_, (p, _))| p.parent().is_some_and(|f| place.holds(f) && place.file_name(f) == name)).map(|(k, _)| k.clone()).collect();
                        for k in &gone {
                            self.seen.remove(k);
                        }
                        self.faces.retain(|f| !gone.contains(&f.key));
                    }
                }
                let unnamed: Vec<u32> = self.people.iter().filter(|p| p.name.is_none()).map(|p| p.id).collect();
                for id in unnamed {
                    self.recentre(id);
                }
                let used: std::collections::HashSet<u32> = self.faces.iter().map(|f| f.person).collect();
                self.people.retain(|p| used.contains(&p.id) || p.name.is_some());
                self.reset_summaries();
                true
            }
            _ => false,
        }
    }

    /// Keep the data in these places from now on (a library stopped being shared here), and
    /// write everything there: what was in the shared folder is copied to this computer.
    pub fn use_places(&mut self, places: &[crate::store::Place]) {
        self.face_files = places.iter().map(|pl| crate::store::Files::at(pl, "faces")).collect();
        self.people_files = places.iter().map(|pl| crate::store::Files::at(pl, "people")).collect();
        self.save_all();
    }

    /// Write everything again, each folder to the place that keeps it now (after a library
    /// became shared: its data moves from this computer to the shared folder).
    pub fn save_all(&mut self) {
        self.changed = true;
        self.save();
    }

    /// Where each named person was last named (computer, seconds since 1970): for "named on …".
    pub fn named_on(&self, id: u32) -> Option<(String, u64)> {
        self.people.iter().find(|p| p.id == id).filter(|p| !p.named.1.is_empty()).map(|p| (p.named.1.clone(), p.named.0))
    }

    /// Read everything in a shared place again (its change log started over).
    pub fn reload_all(&mut self, data: &Path) {
        let names: Vec<String> = match self.face_files.iter_mut().find(|f| f.place.data == data) {
            Some(files) => files.read_all().into_iter().map(|(n, _)| n).collect(),
            None => return,
        };
        for n in names {
            self.reload(data, "faces", &n);
        }
        let people: Vec<String> = match self.people_files.iter_mut().find(|f| f.place.data == data) {
            Some(files) => files.read_all().into_iter().map(|(n, _)| n).collect(),
            None => return,
        };
        for n in people {
            self.reload(data, "people", &n);
        }
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
        // A name given on another computer since this one read it is newer: take it first.
        for files in &self.people_files {
            if !files.place.shared() {
                continue;
            }
            for p in self.people.iter_mut().filter(|p| p.name.is_some()) {
                if let Some(theirs) = files.changed_on_disk(&format!("{}.tsv", p.uid)).as_deref().and_then(parse_person) {
                    if theirs.named.0 > p.named.0 {
                        p.name = theirs.name;
                        p.named = theirs.named;
                    }
                }
            }
        }
        // One file per named person, in every place (the shared library, and this computer's
        // copy for when it can't be reached); unnamed groups are rebuilt from their faces.
        let mut people = std::collections::HashMap::new();
        for p in self.people.iter().filter(|p| p.name.is_some()) {
            let text = format!("person\t{}\nname\t{}\ncentre\t{}\nnamed\t{}\t{}\n", p.uid, p.name.as_deref().unwrap_or(""), crate::store::pack(&p.centre), p.named.0, p.named.1);
            people.insert(format!("{}.tsv", p.uid), text);
        }
        for files in &mut self.people_files {
            files.write_all(people.clone());
        }
        // One file per photo folder, in the place that keeps it: each photo looked at, then its
        // faces. Only folders whose summary changed are written; a shared file another computer
        // changed meanwhile is merged with it.
        let mut all = std::mem::take(&mut self.face_files);
        let places: Vec<crate::store::Place> = all.iter().map(|f| f.place.clone()).collect();
        let mut merged: Vec<(crate::store::Place, String)> = Vec::new();
        {
            let summaries = self.summaries();
            let mut by_key: std::collections::HashMap<&str, Vec<&Face>> = std::collections::HashMap::new();
            for f in &self.faces {
                by_key.entry(f.key.as_str()).or_default().push(f);
            }
            let uid: std::collections::HashMap<u32, &str> = self.people.iter().map(|p| (p.id, p.uid.as_str())).collect();
            for (i, files) in all.iter_mut().enumerate() {
                let place = files.place.clone();
                let hashes: std::collections::HashMap<PathBuf, u64> = summaries.iter().filter(|(f, _)| crate::store::place_of(&places, f) == i).map(|(f, (h, _))| (f.clone(), *h)).collect();
                let text = |folder: &Path| -> String {
                    let mut text = format!("folder\t{}\n", place.written(folder));
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
                merged.extend(files.save_folders(&hashes, text).into_iter().map(|m| (place.clone(), m)));
            }
        }
        self.face_files = all;
        // What another computer changed meanwhile, merged in: take it into memory too.
        if !merged.is_empty() {
            let mut ids = self.ids();
            for (place, text) in merged {
                self.take_folder_text(&place, &text, &mut ids, true);
            }
            self.reset_summaries();
        }
    }

    /// Whether `photo`'s content was looked at already. Notes where the photo is now, if it
    /// moved.
    pub fn known(&mut self, photo: &Path, k: &str) -> bool {
        let today = crate::store::today();
        let Some((was, day)) = self.seen.get_mut(k) else { return false };
        // The day it was last seen is refreshed weekly (it only matters after 30 days missing),
        // so a library pass does not rewrite every folder's file every day.
        if was.as_path() != photo || *day + 7 <= today {
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
    /// Faces and person groups, for measuring.
    pub fn sizes(&self) -> String {
        format!("{} faces, {} person groups", self.faces.len(), self.people.len())
    }

    /// A number that changes whenever the faces, who they belong to, or the people do: cheap
    /// (no face comparisons), so it can be checked on every frame.
    pub fn generation(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |x: u64| {
            h ^= x;
            h = h.wrapping_mul(0x0100_0000_01b3);
        };
        mix(self.faces.len() as u64);
        for f in &self.faces {
            mix(f.person as u64 | (f.not.len() as u64) << 32);
        }
        mix(self.people.len() as u64);
        for p in &self.people {
            mix(p.id as u64);
            mix(p.centre.first().map_or(0, |c| c.to_bits() as u64));
            for b in p.name.as_deref().unwrap_or("").bytes() {
                mix(b as u64);
            }
        }
        h
    }

    /// Who the first `n` faces belong to, and the names: what the user changes.
    fn core(&self, n: usize) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |x: u64| {
            h ^= x;
            h = h.wrapping_mul(0x0100_0000_01b3);
        };
        for f in self.faces.iter().take(n) {
            mix(f.person as u64 | (f.not.len() as u64) << 32);
        }
        for p in self.people.iter().filter(|p| p.name.is_some()) {
            mix(p.id as u64);
            for b in p.name.as_deref().unwrap_or("").bytes() {
                mix(b as u64);
            }
        }
        h
    }

    fn summary(&self) -> std::cell::Ref<'_, Summary> {
        let generation = self.generation();
        let fresh = match self.cache.borrow().as_ref() {
            None => false,
            Some(s) if s.generation == generation => true,
            // Only new faces since (being found now): good enough for a couple of seconds.
            Some(s) => self.faces.len() > s.faces && s.made.elapsed() < std::time::Duration::from_secs(2) && self.core(s.faces) == s.core,
        };
        if !fresh {
            *self.cache.borrow_mut() = Some(self.summarise(generation));
        }
        std::cell::Ref::map(self.cache.borrow(), |s| s.as_ref().expect("just made"))
    }

    /// The list and best faces, in one pass over the faces.
    fn summarise(&self, generation: u64) -> Summary {
        let centre: std::collections::HashMap<u32, &[f32]> = self.people.iter().map(|p| (p.id, p.centre.as_slice())).collect();
        let mut photos: std::collections::HashMap<u32, Vec<&Path>> = std::collections::HashMap::new();
        let mut best: std::collections::HashMap<u32, (usize, f32)> = std::collections::HashMap::new();
        for (i, f) in self.faces.iter().enumerate() {
            photos.entry(f.person).or_default().push(f.photo.as_path());
            if let Some(c) = centre.get(&f.person) {
                let s = crate::faces::similarity_of(&f.print, c);
                let e = best.entry(f.person).or_insert((i, s));
                if s > e.1 {
                    *e = (i, s);
                }
            }
        }
        let mut list: Vec<ListEntry> = self
            .people
            .iter()
            .map(|p| {
                let count = photos.get_mut(&p.id).map_or(0, |v| {
                    v.sort();
                    v.dedup();
                    v.len()
                });
                let face = best.get(&p.id).map(|&(i, _)| (self.faces[i].key.clone(), self.faces[i].n));
                (p.id, p.name.clone(), count, face)
            })
            .filter(|(_, name, count, _)| name.is_some() || *count >= 2)
            .collect();
        list.sort_by(|a, b| b.1.is_some().cmp(&a.1.is_some()).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));
        Summary { generation, made: std::time::Instant::now(), faces: self.faces.len(), core: self.core(self.faces.len()), list, best: best.into_iter().map(|(id, (i, _))| (id, i)).collect() }
    }

    /// The People list: named people first, then groups by how many photos they are in.
    pub fn list(&self) -> Vec<ListEntry> {
        self.summary().list.clone()
    }

    /// The generation the People list shown now was worked out for (it may lag new faces by
    /// a couple of seconds while faces are being found): for keeping other views in step.
    pub fn shown_generation(&self) -> u64 {
        self.summary().generation
    }

    /// Whether anyone is in the People list.
    pub fn list_is_empty(&self) -> bool {
        self.summary().list.is_empty()
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

    /// A folder was moved from `from` to `to`: its photos are there now.
    pub fn relocate(&mut self, from: &Path, to: &Path) {
        for (photo, _) in self.seen.values_mut() {
            if let Some(p) = crate::mover::relocated(photo, from, to) {
                *photo = p;
                self.changed = true;
            }
        }
        for f in &mut self.faces {
            if let Some(p) = crate::mover::relocated(&f.photo, from, to) {
                f.photo = p;
            }
        }
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
        let i = *self.summary().best.get(&id)?;
        self.faces.get(i).cloned()
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
    /// The names of the people in each of these photos, in one pass over the faces.
    pub fn names_for(&self, photos: &[PathBuf]) -> Vec<(PathBuf, Vec<String>)> {
        let wanted: std::collections::HashSet<&Path> = photos.iter().map(PathBuf::as_path).collect();
        let name: std::collections::HashMap<u32, &str> = self.people.iter().filter_map(|p| Some((p.id, p.name.as_deref()?))).collect();
        let mut found: std::collections::HashMap<&Path, Vec<String>> = photos.iter().map(|p| (p.as_path(), Vec::new())).collect();
        for f in &self.faces {
            if wanted.contains(f.photo.as_path()) {
                if let (Some(n), Some(v)) = (name.get(&f.person), found.get_mut(f.photo.as_path())) {
                    v.push((*n).to_owned());
                }
            }
        }
        photos
            .iter()
            .map(|p| {
                let mut names = found.remove(p.as_path()).unwrap_or_default();
                names.sort();
                names.dedup();
                (p.clone(), names)
            })
            .collect()
    }

    #[cfg(test)]
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
        // Many photos at once give the same answers.
        let photos: Vec<PathBuf> = ["y", "z", "w", "nowhere"].iter().map(PathBuf::from).collect();
        for (photo, names) in p.names_for(&photos) {
            assert_eq!(names, p.names_in(&photo), "{}", photo.display());
        }
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
