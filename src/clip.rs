//! CLIP (OpenAI, MIT licence; `models/clip_*`): pictures and words in one space, so a photo can
//! be compared with any word or phrase. Used to suggest keywords ("wedding", "car", "flower")
//! and to search the library by description ("red car at night").
//!
//! The image model turns a 224 x 224 picture into 512 numbers, the text model turns a phrase
//! into 512 numbers; how alike they are (cosine) says how well the phrase describes the picture.
//! Both run on this computer.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ort::session::Session;
use ort::value::Tensor;

const VISION: &str = "clip_vision_quantized.onnx";
const TEXT: &str = "clip_text_quantized.onnx";
const TOKENIZER: &str = "clip_tokenizer.json";
const SIDE: usize = 224;
const MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_73];
const STD: [f32; 3] = [0.268_629_54, 0.261_302_58, 0.275_777_1];
const START: i64 = 49406;
const END: i64 = 49407;
const MAX_TOKENS: usize = 77;

static VISION_SESSION: OnceLock<Result<Mutex<Session>, String>> = OnceLock::new();
static TEXT_SESSION: OnceLock<Result<Mutex<Session>, String>> = OnceLock::new();
static BPE: OnceLock<Result<Bpe, String>> = OnceLock::new();

fn open(cell: &'static OnceLock<Result<Mutex<Session>, String>>, file: &str) -> Result<&'static Mutex<Session>, String> {
    cell.get_or_init(|| {
        let path = crate::denoise::find_model(file).ok_or_else(|| format!("The CLIP model ({file}) was not found."))?;
        let environment = crate::denoise::environment()?;
        let mut builder = Session::builder(environment).map_err(|e| format!("ONNX Runtime: {e}"))?;
        builder.commit_from_file(&path).map(Mutex::new).map_err(|e| format!("ONNX Runtime: {e}"))
    })
    .as_ref()
    .map_err(|e| e.clone())
}

/// Whether the CLIP models are installed.
pub fn available() -> bool {
    crate::denoise::find_model(VISION).is_some() && crate::denoise::find_model(TEXT).is_some() && crate::denoise::find_model(TOKENIZER).is_some()
}

fn normalised(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

/// The picture's 512 numbers (length 1). `rgb` is any size; it is cut to a centred square.
pub fn image_embedding(rgb: &image::RgbImage) -> Result<Vec<f32>, String> {
    // Shortest side to 224 (bicubic), then the centre 224 x 224.
    let (w, h) = rgb.dimensions();
    let scale = SIDE as f32 / w.min(h).max(1) as f32;
    let (nw, nh) = (((w as f32 * scale).round() as u32).max(SIDE as u32), ((h as f32 * scale).round() as u32).max(SIDE as u32));
    let resized = image::imageops::resize(rgb, nw, nh, image::imageops::FilterType::CatmullRom);
    let (x0, y0) = ((nw - SIDE as u32) / 2, (nh - SIDE as u32) / 2);
    let mut input = vec![0f32; 3 * SIDE * SIDE];
    for y in 0..SIDE {
        for x in 0..SIDE {
            let p = resized.get_pixel(x0 + x as u32, y0 + y as u32);
            for c in 0..3 {
                input[c * SIDE * SIDE + y * SIDE + x] = (p[c] as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }
    }
    let tensor = Tensor::from_array(([1usize, 3, SIDE, SIDE], input.into_boxed_slice())).map_err(|e| e.to_string())?;
    let mut session = open(&VISION_SESSION, VISION)?.lock().map_err(|_| "CLIP is unavailable.".to_owned())?;
    let outputs = session.run(ort::inputs!["pixel_values" => tensor]).map_err(|e| format!("CLIP failed: {e}"))?;
    let value = outputs.get("image_embeds").ok_or("CLIP gave no image_embeds")?;
    let (_, data) = value.try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
    Ok(normalised(data.to_vec()))
}

/// The phrase's 512 numbers (length 1).
pub fn text_embedding(text: &str) -> Result<Vec<f32>, String> {
    let ids = tokens(text)?;
    let n = ids.len();
    let ids_tensor = Tensor::from_array(([1usize, n], ids.into_boxed_slice())).map_err(|e| e.to_string())?;
    let mut session = open(&TEXT_SESSION, TEXT)?.lock().map_err(|_| "CLIP is unavailable.".to_owned())?;
    let wants_mask = session.inputs().iter().any(|i| i.name() == "attention_mask");
    let outputs = if wants_mask {
        let mask = Tensor::from_array(([1usize, n], vec![1i64; n].into_boxed_slice())).map_err(|e| e.to_string())?;
        session.run(ort::inputs!["input_ids" => ids_tensor, "attention_mask" => mask])
    } else {
        session.run(ort::inputs!["input_ids" => ids_tensor])
    }
    .map_err(|e| format!("CLIP failed: {e}"))?;
    let value = outputs.get("text_embeds").ok_or("CLIP gave no text_embeds")?;
    let (_, data) = value.try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
    Ok(normalised(data.to_vec()))
}

pub fn similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// The models' inputs and outputs (for the `--dump-clip` diagnostic).
pub fn describe() -> String {
    let mut out = String::new();
    for (cell, file) in [(&VISION_SESSION, VISION), (&TEXT_SESSION, TEXT)] {
        match open(cell, file) {
            Ok(s) => {
                let s = s.lock().unwrap();
                out += &format!("{file}: inputs {:?} outputs {:?}\n", s.inputs().iter().map(|i| i.name().to_owned()).collect::<Vec<_>>(), s.outputs().iter().map(|o| o.name().to_owned()).collect::<Vec<_>>());
            }
            Err(e) => out += &format!("{file}: {e}\n"),
        }
    }
    out
}

// ---- The tokenizer: CLIP's byte-level BPE, read from `clip_tokenizer.json`. ----

struct Bpe {
    vocab: HashMap<String, i64>,
    ranks: HashMap<(String, String), usize>,
    bytes: [char; 256],
}

/// GPT-2's map from bytes to printable characters.
fn byte_chars() -> [char; 256] {
    let mut keep: Vec<u32> = (b'!' as u32..=b'~' as u32).chain(0xA1..=0xAC).chain(0xAE..=0xFF).collect();
    let mut chars: Vec<u32> = keep.clone();
    let mut extra = 0;
    for b in 0..256u32 {
        if !keep.contains(&b) {
            keep.push(b);
            chars.push(256 + extra);
            extra += 1;
        }
    }
    let mut out = [' '; 256];
    for (b, c) in keep.iter().zip(chars) {
        out[*b as usize] = char::from_u32(c).unwrap_or(' ');
    }
    out
}

fn bpe() -> Result<&'static Bpe, String> {
    BPE.get_or_init(|| {
        let path = crate::denoise::find_model(TOKENIZER).ok_or("The CLIP tokenizer was not found.")?;
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let model = &json["model"];
        let vocab: HashMap<String, i64> = model["vocab"].as_object().ok_or("No vocab")?.iter().filter_map(|(k, v)| Some((k.clone(), v.as_i64()?))).collect();
        let mut ranks = HashMap::new();
        for (i, m) in model["merges"].as_array().ok_or("No merges")?.iter().enumerate() {
            let pair = match m {
                serde_json::Value::String(s) => s.split_once(' ').map(|(a, b)| (a.to_owned(), b.to_owned())),
                serde_json::Value::Array(a) => Some((a.first().and_then(|v| v.as_str()).unwrap_or("").to_owned(), a.get(1).and_then(|v| v.as_str()).unwrap_or("").to_owned())),
                _ => None,
            };
            if let Some(pair) = pair {
                ranks.insert(pair, i);
            }
        }
        Ok(Bpe { vocab, ranks, bytes: byte_chars() })
    })
    .as_ref()
    .map_err(|e| e.clone())
}

/// Split as CLIP does: runs of letters, single digits, runs of other symbols, and the common
/// English contractions ('s 't 're 've 'm 'll 'd).
fn words(text: &str) -> Vec<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '\'' {
            let rest: String = chars[i..chars.len().min(i + 3)].iter().collect();
            let hit = ["'re", "'ve", "'ll", "'s", "'t", "'m", "'d"].iter().find(|s| rest.starts_with(**s));
            match hit {
                Some(s) => {
                    out.push(s.to_string());
                    i += s.chars().count();
                }
                None => {
                    out.push("'".to_owned());
                    i += 1;
                }
            }
        } else if c.is_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        } else if c.is_numeric() {
            out.push(c.to_string());
            i += 1;
        } else {
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() && !chars[i].is_alphanumeric() && chars[i] != '\'' {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        }
    }
    out
}

fn encode_word(b: &Bpe, word: &str) -> Vec<i64> {
    let mapped: Vec<String> = word.bytes().map(|x| b.bytes[x as usize].to_string()).collect();
    if mapped.is_empty() {
        return Vec::new();
    }
    let mut parts = mapped;
    if let Some(last) = parts.last_mut() {
        last.push_str("</w>");
    }
    loop {
        let best = (0..parts.len().saturating_sub(1)).filter_map(|i| b.ranks.get(&(parts[i].clone(), parts[i + 1].clone())).map(|r| (*r, i))).min();
        let Some((_, i)) = best else { break };
        let merged = format!("{}{}", parts[i], parts[i + 1]);
        parts.splice(i..i + 2, [merged]);
    }
    parts.iter().filter_map(|p| b.vocab.get(p).copied()).collect()
}

/// The token ids of a phrase, with the start and end markers.
pub fn tokens(text: &str) -> Result<Vec<i64>, String> {
    let b = bpe()?;
    let mut ids = vec![START];
    for w in words(text) {
        ids.extend(encode_word(b, &w));
    }
    ids.truncate(MAX_TOKENS - 1);
    ids.push(END);
    Ok(ids)
}

/// The keywords Develop suggests from, unless the user keeps their own list
/// (`keyword-vocabulary.txt` in the settings folder, one per line).
pub const DEFAULT_VOCABULARY: &[&str] = &[
    "portrait", "people", "group", "family", "children", "baby", "couple", "wedding", "bride", "groom", "party", "dinner", "restaurant", "food", "drink", "coffee", "cake",
    "landscape", "mountain", "beach", "sea", "lake", "river", "waterfall", "forest", "tree", "flower", "garden", "park", "field", "desert", "snow", "sky", "clouds", "sunset", "sunrise",
    "night", "city", "street", "building", "architecture", "church", "temple", "bridge", "interior", "home", "kitchen", "office", "market", "shop",
    "car", "motorcycle", "bicycle", "boat", "airplane", "train", "bus", "road",
    "dog", "cat", "bird", "horse", "animal", "wildlife", "insect",
    "sport", "running", "swimming", "concert", "music", "stage", "dance", "fire", "candles", "christmas", "birthday",
    "black and white", "close-up", "macro", "silhouette", "reflection", "rain", "fog",
];

/// Phrase a keyword the way CLIP was trained to read captions.
pub fn prompt(word: &str) -> String {
    format!("a photo of {}", if word.starts_with(|c: char| "aeiou".contains(c)) { format!("an {word}") } else { format!("a {word}") })
}

/// The vocabulary for suggestions: the user's list if they keep one, else the default.
pub fn vocabulary() -> Vec<String> {
    let own = crate::settings::settings_dir().and_then(|d| std::fs::read_to_string(d.join("keyword-vocabulary.txt")).ok());
    match own {
        Some(text) => text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_owned).collect(),
        None => DEFAULT_VOCABULARY.iter().map(|s| s.to_string()).collect(),
    }
}

/// The vocabulary's text embeddings, computed once.
static VOCAB: OnceLock<Mutex<Vec<(String, Vec<f32>)>>> = OnceLock::new();

pub fn vocabulary_embeddings() -> Result<Vec<(String, Vec<f32>)>, String> {
    let cell = VOCAB.get_or_init(|| Mutex::new(Vec::new()));
    let mut v = cell.lock().map_err(|_| "CLIP is unavailable.".to_owned())?;
    if v.is_empty() {
        for word in vocabulary() {
            let e = text_embedding(&prompt(&word))?;
            v.push((word, e));
        }
    }
    Ok(v.clone())
}

/// A word is suggested when it stands out from the vocabulary's typical score for the picture
/// by `ABOVE_TYPICAL`, and is within `NEAR_BEST` of the best word; at most `MAX_SUGGESTIONS`.
/// Set on real photos: a restaurant scene gives coffee, restaurant, dinner, food; a woman hugging
/// a dog gives dog, animal; a beach portrait gives beach, portrait, sea (and a stray boat).
const ABOVE_TYPICAL: f32 = 0.022;
const NEAR_BEST: f32 = 0.03;
const MAX_SUGGESTIONS: usize = 5;

/// Keywords suggested for a picture's embedding, best first, with their scores.
pub fn suggest(image: &[f32]) -> Result<Vec<(String, f32)>, String> {
    let vocab = vocabulary_embeddings()?;
    let mut scored: Vec<(String, f32)> = vocab.iter().map(|(w, e)| (w.clone(), similarity(image, e))).collect();
    if scored.is_empty() {
        return Ok(Vec::new());
    }
    // Scores sit in a narrow band for every picture; what matters is standing out from it.
    let mut sorted: Vec<f32> = scored.iter().map(|s| s.1).collect();
    sorted.sort_by(f32::total_cmp);
    let typical = sorted[sorted.len() / 2];
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let best = scored[0].1;
    Ok(scored.into_iter().filter(|(_, s)| *s >= typical + ABOVE_TYPICAL && *s >= best - NEAR_BEST).take(MAX_SUGGESTIONS).collect())
}

/// Rank pictures for a search phrase: those it describes well enough, best first. Measured on
/// real photos, true matches scored 0.24-0.32 ("people eating noodles" 0.278 on a restaurant
/// photo, "a woman hugging a dog" 0.316) and non-matches at most 0.226, whatever the rest of
/// the library holds, so the cut is a fixed score rather than how unusual a photo is.
const SEARCH_FLOOR: f32 = 0.235;
const MAX_RESULTS: usize = 80;

pub fn rank(phrase: &[f32], pictures: &[(std::path::PathBuf, Vec<f32>)]) -> Vec<(std::path::PathBuf, f32)> {
    let mut hits: Vec<(std::path::PathBuf, f32)> = pictures.iter().map(|(p, e)| (p.clone(), similarity(phrase, e))).filter(|(_, s)| *s >= SEARCH_FLOOR).collect();
    hits.sort_by(|a, b| b.1.total_cmp(&a.1));
    hits.truncate(MAX_RESULTS);
    hits
}

/// Each photo's CLIP numbers, filed by content key (see people::key) so moved photos keep them:
/// where the photo was last seen, on which day, and its numbers. Kept in the library data, one
/// file per photo folder (see store.rs).
pub struct Pictures {
    map: HashMap<String, (std::path::PathBuf, u32, Vec<f32>)>,
    changed: bool,
    files: crate::store::Files,
}

impl Default for Pictures {
    fn default() -> Self {
        Self { map: HashMap::new(), changed: false, files: crate::store::Files::new("content") }
    }
}

fn unhex(s: &str) -> Vec<f32> {
    let bytes: Vec<u8> = (0..s.len() / 2).filter_map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect();
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

impl Pictures {
    pub fn load() -> Self {
        let mut p = Pictures::default();
        let today = crate::store::today();
        if p.files.exists() {
            for text in p.files.read_all() {
                let mut folder = std::path::PathBuf::new();
                for line in text.lines() {
                    let f: Vec<&str> = line.split('\t').collect();
                    match f.as_slice() {
                        ["folder", path] => folder = std::path::PathBuf::from(path),
                        ["C", key, name, day, numbers] => {
                            p.map.insert((*key).to_owned(), (folder.join(name), day.parse().unwrap_or(today), crate::store::unpack(numbers)));
                        }
                        _ => {}
                    }
                }
            }
            let summaries = p.summaries().into_iter().map(|(f, (h, _))| (f, h)).collect();
            p.files.set_summaries(&summaries);
        } else if let Some(text) = crate::settings::settings_dir().and_then(|d| std::fs::read_to_string(d.join("clip.tsv")).ok()) {
            // The single file of 0.4.x: read it and write it the new way (the old file stays, for
            // an older Develop).
            for line in text.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() == 3 {
                    p.map.insert(f[0].to_owned(), (std::path::PathBuf::from(f[1]), today, unhex(f[2])));
                }
            }
            p.changed = true;
            p.save();
        }
        p
    }

    pub fn save(&mut self) {
        if !self.changed {
            return;
        }
        self.changed = false;
        // Only folders whose summary changed are written (the numbers never change once read).
        let mut files = std::mem::replace(&mut self.files, crate::store::Files::new("content"));
        let summaries = self.summaries();
        let hashes: HashMap<std::path::PathBuf, u64> = summaries.iter().map(|(f, (h, _))| (f.clone(), *h)).collect();
        let text = |folder: &std::path::Path| -> String {
            let mut text = format!("folder\t{}\n", folder.display());
            for k in summaries.get(folder).map(|(_, keys)| keys.as_slice()).unwrap_or(&[]) {
                let (photo, day, e) = &self.map[*k];
                text.push_str(&format!("C\t{k}\t{}\t{day}\t{}\n", photo.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), crate::store::pack(e)));
            }
            text
        };
        files.save_folders(&hashes, text);
        self.files = files;
    }

    /// Per photo folder: a cheap summary of what its file holds, and its photos' keys.
    fn summaries(&self) -> HashMap<std::path::PathBuf, (u64, Vec<&str>)> {
        let mut out: HashMap<std::path::PathBuf, (u64, Vec<&str>)> = HashMap::new();
        let mut keys: Vec<&String> = self.map.keys().collect();
        keys.sort();
        for k in keys {
            let (photo, day, _) = &self.map[k];
            let Some(folder) = photo.parent() else { continue };
            let entry = out.entry(folder.to_path_buf()).or_insert((crate::store::HASH_START, Vec::new()));
            crate::store::mix(&mut entry.0, &format!("{k}|{}|{day}", photo.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
            entry.1.push(k.as_str());
        }
        out
    }

    /// Whether `photo`'s content has its numbers already (and note where it is now).
    pub fn known(&mut self, photo: &std::path::Path, k: &str) -> bool {
        match self.map.get_mut(k) {
            Some((p, day, _)) => {
                let today = crate::store::today();
                if p != photo || *day != today {
                    *p = photo.to_path_buf();
                    *day = today;
                    self.changed = true;
                }
                true
            }
            None => false,
        }
    }

    pub fn insert(&mut self, photo: &std::path::Path, k: &str, e: Vec<f32>) {
        self.map.insert(k.to_owned(), (photo.to_path_buf(), crate::store::today(), e));
        self.changed = true;
    }

    pub fn get(&self, k: &str) -> Option<&Vec<f32>> {
        self.map.get(k).map(|(_, _, e)| e)
    }

    /// Forget photos no longer in the library: only once missing for a while, and never those in
    /// folders that could not be read.
    pub fn prune(&mut self, keep: &std::collections::HashSet<String>, unreachable: &[std::path::PathBuf]) {
        let before = self.map.len();
        let today = crate::store::today();
        self.map.retain(|k, (photo, day, _)| keep.contains(k) || !crate::store::forget(photo, *day, unreachable, today));
        self.changed |= self.map.len() != before;
    }

    /// Every photo with its numbers, by where it was last seen.
    pub fn all(&self) -> Vec<(std::path::PathBuf, Vec<f32>)> {
        self.map.values().map(|(p, _, e)| (p.clone(), e.clone())).collect()
    }
}

/// The CLIP numbers of one photo, from its preview (for a photo outside the library).
pub fn embed_photo(photo: &std::path::Path) -> Result<Vec<f32>, String> {
    let pic = crate::filmstrip::preview(photo, 448).ok_or("The photo has no preview to read.")?;
    image_embedding(&pic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_like_clip() {
        assert_eq!(words("A red car, at night!"), vec!["a", "red", "car", ",", "at", "night", "!"]);
        assert_eq!(words("it's 2026"), vec!["it", "'s", "2", "0", "2", "6"]);
    }

    #[test]
    fn byte_map_keeps_printable_ascii() {
        let m = byte_chars();
        assert_eq!(m[b'a' as usize], 'a');
        assert_eq!(m[b'!' as usize], '!');
        assert_ne!(m[b' ' as usize], ' ');
    }
}
