//! People: faces found across the library, grouped into people the user can name.
//!
//! Faces are found and fingerprinted (skin.rs, faces.rs) while the library is indexed. Each
//! face is filed under its photo's content (file name and size), not its path, so a photo moved
//! to another folder keeps its faces. Similar faces are grouped; the user names a group once and
//! the name is written into each of its photos' `.dvlp` files (as a plain word, like a keyword).
//! The fingerprints and groups stay in the settings folder (`faces.tsv`, `people.tsv`) and never
//! go into the photos' files.

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
}

#[derive(Clone, Debug)]
pub struct Person {
    pub id: u32,
    pub name: Option<String>,
    /// The average fingerprint of its faces (length 1).
    centre: Vec<f32>,
}

#[derive(Default)]
pub struct People {
    pub faces: Vec<Face>,
    pub people: Vec<Person>,
    next_id: u32,
    /// Photo content keys already looked at (with or without faces).
    pub seen: std::collections::HashSet<String>,
    changed: bool,
}

/// A photo's content key: its file name and size, which survive moving it.
pub fn key(photo: &Path) -> String {
    let size = std::fs::metadata(photo).map(|m| m.len()).unwrap_or(0);
    let name = photo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    format!("{name}|{size}")
}

fn hex(v: &[f32]) -> String {
    v.iter().flat_map(|f| f.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
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
        if let Some(text) = file("people.tsv").and_then(|f| std::fs::read_to_string(f).ok()) {
            for line in text.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() == 3 {
                    let id = f[0].parse().unwrap_or(0);
                    p.next_id = p.next_id.max(id + 1);
                    let name = (!f[1].is_empty()).then(|| f[1].to_owned());
                    p.people.push(Person { id, name, centre: unhex(f[2]) });
                }
            }
        }
        if let Some(text) = file("faces.tsv").and_then(|f| std::fs::read_to_string(f).ok()) {
            for line in text.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() == 1 && !f[0].is_empty() {
                    p.seen.insert(f[0].to_owned()); // a photo without faces
                } else if f.len() == 6 {
                    let area: Vec<f32> = f[3].split(',').filter_map(|v| v.parse().ok()).collect();
                    if area.len() == 4 {
                        p.seen.insert(f[0].to_owned());
                        p.faces.push(Face { key: f[0].to_owned(), n: f[1].parse().unwrap_or(0), photo: PathBuf::from(f[2]), area: [area[0], area[1], area[2], area[3]], print: unhex(f[4]), person: f[5].parse().unwrap_or(0) });
                    }
                }
            }
        }
        p
    }

    pub fn save(&mut self) {
        if !self.changed {
            return;
        }
        self.changed = false;
        let Some(dir) = crate::settings::settings_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let people: String = self.people.iter().map(|p| format!("{}\t{}\t{}\n", p.id, p.name.as_deref().unwrap_or(""), hex(&p.centre))).collect();
        let _ = std::fs::write(dir.join("people.tsv"), people);
        let with_faces: std::collections::HashSet<&str> = self.faces.iter().map(|f| f.key.as_str()).collect();
        let mut lines: Vec<String> = self
            .faces
            .iter()
            .map(|f| format!("{}\t{}\t{}\t{:.4},{:.4},{:.4},{:.4}\t{}\t{}\n", f.key, f.n, f.photo.display(), f.area[0], f.area[1], f.area[2], f.area[3], hex(&f.print), f.person))
            .collect();
        lines.extend(self.seen.iter().filter(|k| !with_faces.contains(k.as_str())).map(|k| format!("{k}\n")));
        lines.sort();
        let _ = std::fs::write(dir.join("faces.tsv"), lines.concat());
    }

    /// Whether `photo`'s content was looked at already. Notes where the photo is now, if it
    /// moved.
    pub fn known(&mut self, photo: &Path, k: &str) -> bool {
        if !self.seen.contains(k) {
            return false;
        }
        for f in self.faces.iter_mut().filter(|f| f.key == k && f.photo != photo) {
            f.photo = photo.to_path_buf();
            self.changed = true;
        }
        true
    }

    /// File faces found by `find_faces`.
    pub fn add_faces(&mut self, photo: &Path, k: &str, found: Vec<(u8, [f32; 4], Vec<f32>)>) {
        self.seen.insert(k.to_owned());
        self.changed = true;
        for (n, area, print) in found {
            let person = self.assign(&print);
            self.faces.push(Face { key: k.to_owned(), n, photo: photo.to_path_buf(), area, print, person });
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
        self.people.push(Person { id, name: None, centre: print.to_vec() });
        id
    }

    /// Forget photos no longer in the library (`keep` holds the content keys still there).
    pub fn prune(&mut self, keep: &std::collections::HashSet<String>) {
        let before = self.faces.len() + self.seen.len();
        self.faces.retain(|f| keep.contains(&f.key));
        self.seen.retain(|k| keep.contains(k));
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
            for f in self.faces.iter_mut().filter(|f| f.person == id) {
                f.person = other;
            }
            self.people.retain(|p| p.id != id);
            self.recentre(other);
            touched.extend(self.photos_of(other));
        } else if let Some(p) = self.people.iter_mut().find(|p| p.id == id) {
            p.name = (!name.is_empty()).then_some(name);
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
            self.people.push(Person { id: nid, name: None, centre: self.faces[i].print.clone() });
            self.faces[i].person = nid;
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
            .filter(|f| f.person != id && !named.contains(&f.person) && !mine.contains(f.photo.as_path()))
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
        p.faces.push(Face { key: "x|1".into(), n: 0, photo: "x".into(), area: [0.0; 4], print: a.clone(), person: ia });
        let ia2 = p.assign(&a2);
        p.faces.push(Face { key: "y|1".into(), n: 0, photo: "y".into(), area: [0.0; 4], print: a2, person: ia2 });
        let ib = p.assign(&b);
        p.faces.push(Face { key: "z|1".into(), n: 0, photo: "z".into(), area: [0.0; 4], print: b, person: ib });
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
        p.faces.push(Face { key: "x|1".into(), n: 0, photo: "x".into(), area: [0.0; 4], print: a, person: ia });
        let ib = p.assign(&b);
        p.faces.push(Face { key: "y|1".into(), n: 0, photo: "y".into(), area: [0.0; 4], print: b, person: ib });
        p.rename(ia, "Talia");
        let touched = p.move_photo(ib, Path::new("y"), ia);
        assert_eq!(touched, vec![PathBuf::from("y")]);
        assert_eq!(p.names_in(Path::new("y")), vec!["Talia".to_owned()]);
        assert!(!p.people.iter().any(|x| x.id == ib), "the emptied unnamed group goes");
        assert_eq!(p.faces_of(ia).len(), 2);
    }

    #[test]
    fn fingerprints_survive_saving_as_text() {
        let v = print(3.0);
        let back = unhex(&hex(&v));
        assert_eq!(v, back);
    }
}
