//! Culling a folder by focus, for the MCP server's `cull_photos` tool.
//!
//! Photos are grouped into bursts by capture time: frames taken within `gap` seconds of the one
//! before belong to the same burst. In each burst the frame with the sharpest focus (the
//! smallest edge width, see `focus.rs`) is the keeper, frames nearly as sharp are seconds, and the
//! rest are rejects. Doing this arithmetic here, rather than in an assistant's head, is fast and
//! never miscounts; the assistant only looks at the bursts marked for a check.
//!
//! Focus measurements are kept in `focus-cache.tsv` in the settings folder, so a folder is only
//! measured once: an entry is used again while the photo file and its edit settings are
//! unchanged (a rating change does not count).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// The measurement kept for one photo.
#[derive(Clone, Debug, PartialEq)]
pub struct Measured {
    /// Capture time as Develop shows it (YYYY:MM:DD HH:MM:SS).
    pub taken: String,
    /// Edge width in pixels of the sharpest area; None when there was no detail to judge.
    pub sharpest: Option<f32>,
    pub median: Option<f32>,
    /// Where the sharpest area is, as fractions of the width and height.
    pub at: Option<[f32; 2]>,
}

/// Change this when the focus measurement changes, so old entries are measured again.
const CACHE_VERSION: &str = "focus 1";

pub struct Cache {
    file: Option<PathBuf>,
    entries: HashMap<PathBuf, (String, Measured)>,
    changed: bool,
}

/// What a photo's measurement depends on: the file (size and time) and its edit settings.
pub fn key(photo: &Path) -> String {
    let (size, time) = std::fs::metadata(photo)
        .map(|m| (m.len(), m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())))
        .unwrap_or((0, 0));
    let sidecar = std::fs::read_to_string(crate::settings::sidecar_path(photo)).unwrap_or_default();
    let settings = sidecar.lines().find(|l| l.starts_with("settings\t")).unwrap_or("");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    settings.hash(&mut hasher);
    format!("{size}-{time}-{:016x}", hasher.finish())
}

fn number(v: Option<f32>) -> String {
    v.map_or_else(String::new, |v| v.to_string())
}

impl Cache {
    pub fn load() -> Self {
        let file = crate::settings::settings_dir().map(|d| d.join("focus-cache.tsv"));
        let text = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        let mut lines = text.lines();
        let mut entries = HashMap::new();
        if lines.next() == Some(CACHE_VERSION) {
            for line in lines {
                let f: Vec<&str> = line.split('\t').collect();
                // path, key, taken, sharpest, median, x, y
                if f.len() != 7 {
                    continue;
                }
                let num = |s: &str| s.parse::<f32>().ok();
                let at = match (num(f[5]), num(f[6])) {
                    (Some(x), Some(y)) => Some([x, y]),
                    _ => None,
                };
                let measured = Measured { taken: f[2].to_owned(), sharpest: num(f[3]), median: num(f[4]), at };
                entries.insert(PathBuf::from(f[0]), (f[1].to_owned(), measured));
            }
        }
        Self { file, entries, changed: false }
    }

    /// The kept measurement of `photo`, if it is still up to date.
    pub fn get(&self, photo: &Path, key: &str) -> Option<&Measured> {
        self.entries.get(photo).filter(|(k, _)| k == key).map(|(_, m)| m)
    }

    pub fn put(&mut self, photo: &Path, key: String, measured: Measured) {
        self.entries.insert(photo.to_path_buf(), (key, measured));
        self.changed = true;
    }

    pub fn save(&mut self) {
        if !self.changed {
            return;
        }
        self.changed = false;
        let Some(file) = &self.file else { return };
        let mut lines: Vec<String> = self
            .entries
            .iter()
            .map(|(path, (key, m))| {
                let (x, y) = m.at.map_or((None, None), |[x, y]| (Some(x), Some(y)));
                format!("{}\t{key}\t{}\t{}\t{}\t{}\t{}\n", path.display(), m.taken, number(m.sharpest), number(m.median), number(x), number(y))
            })
            .collect();
        lines.sort();
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, format!("{CACHE_VERSION}\n{}", lines.concat()));
    }
}

/// Seconds since year 0 of a capture time `YYYY:MM:DD HH:MM:SS` (only differences matter).
pub fn seconds(taken: &str) -> Option<i64> {
    let n: Vec<i64> = taken.split([':', ' ', '-', 'T']).filter(|s| !s.is_empty()).take(6).map(|s| s.trim().parse().ok()).collect::<Option<Vec<_>>>()?;
    if n.len() < 6 {
        return None;
    }
    // Days from a civil date (proleptic Gregorian).
    let (y, m, d) = (n[0] - if n[1] <= 2 { 1 } else { 0 }, n[1], n[2]);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe;
    Some(days * 86_400 + n[3] * 3600 + n[4] * 60 + n[5])
}

pub struct Rules {
    /// Frames this many seconds or less after the one before are the same burst.
    pub gap: i64,
    /// A second (3 stars) must be at most this wide...
    pub sharp: f32,
    /// ...and within this much of the burst's best.
    pub near: f32,
    /// A burst's best frame wider than this is soft too: the whole burst missed.
    pub soft: f32,
}

/// One burst: its photos (indices into the culled list, in capture order), the stars suggested
/// for each, and why it should be looked at, if it should.
pub struct Burst {
    pub photos: Vec<usize>,
    pub stars: Vec<u8>,
    pub best: Option<usize>,
    pub check: Vec<String>,
}

/// Group `shots` (any order) into bursts and suggest stars: 4 for each burst's sharpest frame
/// (unless even that is soft), 3 for frames nearly as sharp, 1 for the rest. Photos without a
/// focus measurement get 1 and are marked for a check.
pub fn cull(shots: &[Measured], rules: &Rules) -> Vec<Burst> {
    let mut order: Vec<usize> = (0..shots.len()).collect();
    order.sort_by_key(|&i| (seconds(&shots[i].taken).unwrap_or(i64::MAX), i));
    let mut bursts: Vec<Vec<usize>> = Vec::new();
    let mut last: Option<i64> = None;
    for i in order {
        let t = seconds(&shots[i].taken);
        let same = matches!((last, t), (Some(a), Some(b)) if b - a <= rules.gap);
        match bursts.last_mut() {
            Some(burst) if same => burst.push(i),
            _ => bursts.push(vec![i]),
        }
        last = t;
    }
    bursts
        .into_iter()
        .map(|photos| {
            let width = |i: usize| shots[i].sharpest.unwrap_or(f32::INFINITY);
            let best = photos.iter().copied().filter(|&i| shots[i].sharpest.is_some()).min_by(|&a, &b| width(a).total_cmp(&width(b)));
            let mut check = Vec::new();
            let stars: Vec<u8> = photos
                .iter()
                .map(|&i| match best {
                    Some(b) if i == b => {
                        if width(b) <= rules.soft {
                            4
                        } else {
                            1
                        }
                    }
                    Some(b) if width(b) <= rules.soft && width(i) <= rules.sharp && width(i) - width(b) <= rules.near => 3,
                    _ => 1,
                })
                .collect();
            if let Some(b) = best {
                let w = width(b);
                if w > rules.sharp && w <= rules.soft {
                    check.push(format!("best is only slightly sharp ({w:.2} px)"));
                } else if w > rules.soft && w <= rules.soft + 1.0 {
                    check.push(format!("all rejected, but the best is close to the limit ({w:.2} px)"));
                }
                if let Some([x, y]) = shots[b].at {
                    if !(0.08..=0.92).contains(&x) || !(0.08..=0.92).contains(&y) {
                        check.push("the sharpest area is near the edge of the frame; it may not be the subject".into());
                    }
                }
                // A near tie: which frame is best is a matter of looking.
                let runner = photos.iter().copied().filter(|&i| i != b).map(width).fold(f32::INFINITY, f32::min);
                if w <= rules.soft && runner - w < 0.15 {
                    check.push("two frames are almost equally sharp".into());
                }
            }
            if photos.iter().any(|&i| shots[i].sharpest.is_none()) {
                check.push("some frames had no detail to measure".into());
            }
            Burst { photos, stars, best, check }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(taken: &str, sharpest: f32) -> Measured {
        Measured { taken: taken.into(), sharpest: Some(sharpest), median: Some(sharpest + 2.0), at: Some([0.5, 0.5]) }
    }

    const RULES: Rules = Rules { gap: 60, sharp: 3.5, near: 0.5, soft: 5.5 };

    #[test]
    fn capture_times_are_comparable() {
        let a = seconds("2026:09:01 23:59:50").unwrap();
        let b = seconds("2026:09:02 00:00:10").unwrap();
        assert_eq!(b - a, 20);
        assert_eq!(seconds("2026:03:01 00:00:00").unwrap() - seconds("2026:02:28 00:00:00").unwrap(), 86_400);
        assert_eq!(seconds("not a date"), None);
    }

    #[test]
    fn bursts_split_on_gaps_and_get_stars() {
        let shots = vec![
            shot("2026:09:01 17:18:12", 2.9),  // alone
            shot("2026:09:01 17:21:34", 5.29), // burst 2: best, slightly sharp
            shot("2026:09:01 17:21:36", 9.4),
            shot("2026:09:01 17:24:46", 4.6), // burst 3, chained by short gaps
            shot("2026:09:01 17:24:58", 3.0),
            shot("2026:09:01 17:25:03", 3.3), // within 0.5 and under 3.5: second
            shot("2026:09:01 17:25:21", 3.6), // within 0.6: reject
            shot("2026:09:01 17:36:28", 7.0), // burst 4: all soft
            shot("2026:09:01 17:36:29", 6.2),
        ];
        let bursts = cull(&shots, &RULES);
        assert_eq!(bursts.len(), 4);
        assert_eq!(bursts[0].stars, vec![4]);
        assert_eq!(bursts[1].stars, vec![4, 1]);
        assert!(bursts[1].check.iter().any(|c| c.contains("slightly sharp")));
        assert_eq!(bursts[2].photos, vec![3, 4, 5, 6]);
        assert_eq!(bursts[2].stars, vec![1, 4, 3, 1]);
        assert_eq!(bursts[3].stars, vec![1, 1]);
        assert!(bursts[3].check.iter().any(|c| c.contains("close to the limit")));
    }

    #[test]
    fn photos_without_a_measurement_are_flagged() {
        let mut flat = shot("2026:09:01 10:00:00", 0.0);
        flat.sharpest = None;
        let bursts = cull(&[flat, shot("2026:09:01 10:00:01", 3.0)], &RULES);
        assert_eq!(bursts[0].stars, vec![1, 4]);
        assert!(!bursts[0].check.is_empty());
    }
}
