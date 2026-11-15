//! Lens correction: a loader for Lensfun XML databases dropped into a folder, matching of
//! the open photo's lens, interpolation of the calibration to its focal length and aperture,
//! the camera's own correction data embedded in raw files, and the numbers the shader needs.
//!
//! Lensfun conventions (from its source, libs/lensfun/mod-*.cpp):
//! - Distortion and TCA use Hugin units: r = 1 at half the shorter side of a 3:2 frame of the
//!   calibration crop factor. A photo of another shape or crop is mapped through the
//!   diagonal, which is what the crop factor is defined on.
//! - Vignetting ("pa") uses r = 1 at the half diagonal of the calibration frame.
//! - The models map undistorted (output) radius to distorted (source) radius, which is the
//!   direction a shader needs; vignetting is corrected by dividing by the polynomial.
//!
//! Embedded corrections (as darktable reads them from ARW files): tags 0x7037 (distortion),
//! 0x7035 (chromatic aberration, red then blue) and 0x7032 (vignetting), each a count `nc`
//! followed by the values; knots at (i + 0.5) / (nc - 1) of the half diagonal.

use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------------------

/// Where profile corrections come from.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Source {
    /// A Lensfun profile when one matches, else the camera's embedded data.
    Auto,
    Lensfun,
    Embedded,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::Auto, Source::Lensfun, Source::Embedded];

    pub fn label(self) -> &'static str {
        match self {
            Source::Auto => "Auto (Lensfun / Embedded)",
            Source::Lensfun => "Lensfun XML",
            Source::Embedded => "Camera Embedded",
        }
    }

    fn code(self) -> u8 {
        self as u8
    }

    fn from_code(code: u8) -> Self {
        match code {
            1 => Source::Lensfun,
            2 => Source::Embedded,
            _ => Source::Auto,
        }
    }
}

/// A photo's lens-correction settings.
#[derive(Clone, PartialEq, Debug)]
pub struct LensSettings {
    /// "Enable Profile Corrections".
    pub enabled: bool,
    pub source: Source,
    /// Chosen Lensfun lens (`Lens::key`), or empty for the automatic match.
    pub profile: String,
    /// Strength of the profile's distortion and vignetting corrections, percent (0..200).
    pub profile_distortion: f32,
    pub profile_vignetting: f32,
    /// Manual distortion, -100..100; positive straightens barrel distortion.
    pub distortion: f32,
    /// Enlarge just enough that the correction never shows empty borders.
    pub constrain: bool,
    /// Lateral chromatic aberration, -100..100.
    pub red_cyan: f32,
    pub blue_yellow: f32,
    /// Axial (longitudinal) fringe removal, 0..100.
    pub purple_amount: f32,
    pub purple_hue: f32,
    pub green_amount: f32,
    pub green_hue: f32,
    /// Manual vignette, -100..100, and its midpoint, 0..100.
    pub vignette: f32,
    pub midpoint: f32,
}

impl Default for LensSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            source: Source::Auto,
            profile: String::new(),
            profile_distortion: 100.0,
            profile_vignetting: 100.0,
            distortion: 0.0,
            constrain: true,
            red_cyan: 0.0,
            blue_yellow: 0.0,
            purple_amount: 0.0,
            purple_hue: 50.0,
            green_amount: 0.0,
            green_hue: 50.0,
            vignette: 0.0,
            midpoint: 50.0,
        }
    }
}

impl LensSettings {
    /// The settings as text fields for the settings files (the profile key last).
    pub fn to_fields(&self) -> String {
        let numbers = [
            self.enabled as u8 as f32,
            self.source.code() as f32,
            self.profile_distortion,
            self.profile_vignetting,
            self.distortion,
            self.constrain as u8 as f32,
            self.red_cyan,
            self.blue_yellow,
            self.purple_amount,
            self.purple_hue,
            self.green_amount,
            self.green_hue,
            self.vignette,
            self.midpoint,
        ];
        let mut text: Vec<String> = numbers.iter().map(|v| v.to_string()).collect();
        text.push(self.profile.replace(['\t', '\n', '\r'], " "));
        text.join("\t")
    }

    /// Read what `to_fields` wrote; anything missing keeps its default.
    pub fn from_fields<'a>(fields: &mut impl Iterator<Item = &'a str>) -> Self {
        let mut s = Self::default();
        let mut next = || fields.next().and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite());
        let values: Vec<Option<f32>> = (0..14).map(|_| next()).collect();
        let get = |i: usize, default: f32, lo: f32, hi: f32| values[i].map_or(default, |v| v.clamp(lo, hi));
        if values[0].is_none() {
            return s;
        }
        s.enabled = get(0, 0.0, 0.0, 1.0) > 0.5;
        s.source = Source::from_code(get(1, 0.0, 0.0, 2.0) as u8);
        s.profile_distortion = get(2, 100.0, 0.0, 200.0);
        s.profile_vignetting = get(3, 100.0, 0.0, 200.0);
        s.distortion = get(4, 0.0, -100.0, 100.0);
        s.constrain = get(5, 1.0, 0.0, 1.0) > 0.5;
        s.red_cyan = get(6, 0.0, -100.0, 100.0);
        s.blue_yellow = get(7, 0.0, -100.0, 100.0);
        s.purple_amount = get(8, 0.0, 0.0, 100.0);
        s.purple_hue = get(9, 50.0, 0.0, 100.0);
        s.green_amount = get(10, 0.0, 0.0, 100.0);
        s.green_hue = get(11, 50.0, 0.0, 100.0);
        s.vignette = get(12, 0.0, -100.0, 100.0);
        s.midpoint = get(13, 50.0, 0.0, 100.0);
        drop(next);
        s.profile = fields.next().unwrap_or("").trim().to_owned();
        s
    }
}

// ---------------------------------------------------------------------------------------
// What the photo says about itself
// ---------------------------------------------------------------------------------------

/// Camera and lens details from the photo's EXIF, for matching.
#[derive(Clone, Debug, Default)]
pub struct Exif {
    pub make: String,
    pub model: String,
    pub lens_make: String,
    pub lens_model: String,
    /// Millimetres and f-number; 0 when unknown.
    pub focal: f32,
    pub fnumber: f32,
}

/// The camera's own correction tables from a raw file, already converted to factors.
#[derive(Clone, Debug)]
pub struct Embedded {
    /// Per knot: distortion factor, red and blue CA factors, vignetting falloff.
    pub knots: Vec<[f32; 4]>,
}

impl Embedded {
    /// Build from the raw tag values (each without its leading count).
    pub fn from_tags(distortion: &[i32], ca: &[i32], vignetting: &[i32]) -> Option<Self> {
        let nc = distortion.len();
        if !(2..=16).contains(&nc) || ca.len() != 2 * nc || vignetting.len() != nc {
            return None;
        }
        let knots = (0..nc)
            .map(|i| {
                [
                    distortion[i] as f32 * 2f32.powi(-14) + 1.0,
                    ca[i] as f32 * 2f32.powi(-21) + 1.0,
                    ca[nc + i] as f32 * 2f32.powi(-21) + 1.0,
                    2f32.powf(0.5 - 2f32.powf(vignetting[i] as f32 * 2f32.powi(-13) - 1.0)),
                ]
            })
            .collect();
        Some(Self { knots })
    }

    /// Linear interpolation across the knots; `component` 0..4 as in `knots`.
    fn value(&self, r: f32, component: usize) -> f32 {
        let n = self.knots.len();
        let pos = (r * (n - 1) as f32 - 0.5).clamp(0.0, (n - 1) as f32);
        let i = (pos.floor() as usize).min(n - 2);
        let t = pos - i as f32;
        self.knots[i][component] + (self.knots[i + 1][component] - self.knots[i][component]) * t
    }
}

// ---------------------------------------------------------------------------------------
// The database
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Camera {
    pub maker: String,
    pub model: String,
    pub mount: String,
    pub crop: f32,
}

#[derive(Clone, Copy, Debug)]
struct DistCal {
    focal: f32,
    /// 1 ptlens (a, b, c), 2 poly3 (k1), 3 poly5 (k1, k2).
    model: u8,
    k: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
struct TcaCal {
    focal: f32,
    /// v, c, b for red and blue: scale = v + c r + b r².
    red: [f32; 3],
    blue: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
struct VigCal {
    focal: f32,
    aperture: f32,
    distance: f32,
    k: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct Lens {
    pub maker: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub crop: f32,
    /// The XML file it came from (just the name).
    pub file: String,
    focal_range: (f32, f32),
    dist: Vec<DistCal>,
    tca: Vec<TcaCal>,
    vig: Vec<VigCal>,
}

impl Lens {
    /// Identifies the entry in saved settings.
    pub fn key(&self) -> String {
        format!("{}|{}|{}", self.file, self.maker, self.model)
    }

    /// Short description of what the profile corrects.
    pub fn contents(&self) -> String {
        let mut parts = Vec::new();
        if !self.dist.is_empty() {
            parts.push("distortion");
        }
        if !self.tca.is_empty() {
            parts.push("chromatic aberration");
        }
        if !self.vig.is_empty() {
            parts.push("vignetting");
        }
        if parts.is_empty() {
            "no calibration".to_owned()
        } else {
            parts.join(", ")
        }
    }
}

#[derive(Default)]
pub struct Database {
    pub cameras: Vec<Camera>,
    pub lenses: Vec<Lens>,
    /// XML files read, and files that could not be read (with why).
    pub files: usize,
    pub problems: Vec<String>,
    /// The folders searched; the last one is where new files are best put.
    pub folders: Vec<PathBuf>,
}

/// Lensfun folders, in the order searched: `lensfun` beside the program (or a folder above
/// it, during development), then the user's application-data folder.
pub fn folders() -> Vec<PathBuf> {
    let mut list = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(found) = exe.ancestors().skip(1).take(4).map(|d| d.join("lensfun")).find(|d| d.is_dir()) {
            list.push(found);
        } else if let Some(dir) = exe.parent() {
            list.push(dir.join("lensfun"));
        }
    }
    let data = if cfg!(windows) {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Develop").join("lensfun"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Develop/lensfun"))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/develop/lensfun"))
    };
    list.extend(data);
    list
}

/// Lower-case letters and digits only, for comparing names written differently
/// ("FE 24-70mm F2.8 GM" and "FE 24-70mm f/2.8 GM" both become "fe2470mmf28gm").
pub fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// True when two maker names refer to the same company ("NIKON", "Nikon Corporation"), or
/// when the photo's maker is unknown.
fn maker_agrees(photo: &str, profile: &str) -> bool {
    let first = |s: &str| normalize(s.split(|c: char| !c.is_alphanumeric()).find(|w| !w.is_empty()).unwrap_or(""));
    let (a, b) = (first(photo), first(profile));
    a.is_empty() || b.is_empty() || a == b || a.contains(&b) || b.contains(&a)
}

/// `name` without a leading maker name: the whole maker or its first word, so "Nikon Z 6_2"
/// by "Nikon Corporation" becomes "z62".
fn strip_maker(name: &str, maker: &str) -> String {
    let (n, m) = (normalize(name), normalize(maker));
    let first = normalize(maker.split(|c: char| !c.is_alphanumeric()).find(|w| !w.is_empty()).unwrap_or(""));
    for prefix in [m.as_str(), first.as_str()] {
        if !prefix.is_empty() && n.len() > prefix.len() && n.starts_with(prefix) {
            return n[prefix.len()..].to_owned();
        }
    }
    n
}

fn number(node: roxmltree::Node, attribute: &str) -> Option<f32> {
    node.attribute(attribute).and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite())
}

/// The text of the first `tag` child without a `lang` attribute (or the first at all).
fn child_text(node: roxmltree::Node, tag: &str) -> Option<String> {
    let mut matches = node.children().filter(|c| c.is_element() && c.tag_name().name() == tag);
    let all: Vec<roxmltree::Node> = matches.by_ref().collect();
    all.iter()
        .find(|c| c.attribute("lang").is_none())
        .or_else(|| all.first())
        .and_then(|c| c.text())
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
}

fn parse_file(path: &Path, db: &mut Database) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let options = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(&text, options).map_err(|e| e.to_string())?;
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for node in doc.root_element().children().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "camera" => {
                let (Some(maker), Some(model)) = (child_text(node, "maker"), child_text(node, "model")) else { continue };
                db.cameras.push(Camera {
                    maker,
                    model,
                    mount: child_text(node, "mount").unwrap_or_default(),
                    crop: child_text(node, "cropfactor").and_then(|c| c.parse().ok()).filter(|c: &f32| *c > 0.0).unwrap_or(1.0),
                });
            }
            "lens" => {
                let (Some(maker), Some(model)) = (child_text(node, "maker"), child_text(node, "model")) else { continue };
                let mut lens = Lens {
                    maker,
                    model,
                    mounts: node.children().filter(|c| c.has_tag_name("mount")).filter_map(|c| c.text()).map(|t| t.trim().to_owned()).collect(),
                    crop: child_text(node, "cropfactor").and_then(|c| c.parse().ok()).filter(|c: &f32| *c > 0.0).unwrap_or(1.0),
                    file: file.clone(),
                    focal_range: (0.0, 0.0),
                    dist: Vec::new(),
                    tca: Vec::new(),
                    vig: Vec::new(),
                };
                if let Some(focal) = node.children().find(|c| c.has_tag_name("focal")) {
                    let value = number(focal, "value");
                    lens.focal_range = (number(focal, "min").or(value).unwrap_or(0.0), number(focal, "max").or(value).unwrap_or(0.0));
                }
                for calibration in node.children().filter(|c| c.has_tag_name("calibration")) {
                    for entry in calibration.children().filter(|c| c.is_element()) {
                        let Some(focal) = number(entry, "focal") else { continue };
                        let model = entry.attribute("model").unwrap_or("");
                        let get = |a: &str, default: f32| number(entry, a).unwrap_or(default);
                        match (entry.tag_name().name(), model) {
                            ("distortion", "ptlens") => lens.dist.push(DistCal { focal, model: 1, k: [get("a", 0.0), get("b", 0.0), get("c", 0.0)] }),
                            ("distortion", "poly3") => lens.dist.push(DistCal { focal, model: 2, k: [get("k1", 0.0), 0.0, 0.0] }),
                            ("distortion", "poly5") => lens.dist.push(DistCal { focal, model: 3, k: [get("k1", 0.0), get("k2", 0.0), 0.0] }),
                            ("tca", "poly3") => lens.tca.push(TcaCal {
                                focal,
                                red: [get("vr", 1.0), get("cr", 0.0), get("br", 0.0)],
                                blue: [get("vb", 1.0), get("cb", 0.0), get("bb", 0.0)],
                            }),
                            ("tca", "linear") => lens.tca.push(TcaCal { focal, red: [get("kr", 1.0), 0.0, 0.0], blue: [get("kb", 1.0), 0.0, 0.0] }),
                            ("vignetting", "pa") => lens.vig.push(VigCal {
                                focal,
                                aperture: get("aperture", 0.0),
                                distance: get("distance", 1000.0),
                                k: [get("k1", 0.0), get("k2", 0.0), get("k3", 0.0)],
                            }),
                            _ => {}
                        }
                    }
                }
                if lens.focal_range.1 <= 0.0 {
                    let focals = lens.dist.iter().map(|d| d.focal).chain(lens.tca.iter().map(|t| t.focal)).chain(lens.vig.iter().map(|v| v.focal));
                    let (lo, hi) = focals.fold((f32::MAX, 0.0f32), |(lo, hi), f| (lo.min(f), hi.max(f)));
                    if hi > 0.0 {
                        lens.focal_range = (lo, hi);
                    }
                }
                db.lenses.push(lens);
            }
            _ => {}
        }
    }
    Ok(())
}

impl Database {
    /// Read every `.xml` file in the Lensfun folders.
    pub fn load() -> Self {
        let mut db = Database { folders: folders(), ..Default::default() };
        for folder in db.folders.clone() {
            let Ok(entries) = std::fs::read_dir(&folder) else { continue };
            let mut files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xml")))
                .collect();
            files.sort();
            for file in files {
                match parse_file(&file, &mut db) {
                    Ok(()) => db.files += 1,
                    Err(e) => db.problems.push(format!("{}: {e}", file.display())),
                }
            }
        }
        db.lenses.sort_by(|a, b| a.maker.to_lowercase().cmp(&b.maker.to_lowercase()).then_with(|| crate::filmstrip::natural_cmp(&a.model, &b.model)));
        db
    }

    /// The folder to open for adding files: the first that exists, else the data folder.
    pub fn folder_to_open(&self) -> Option<PathBuf> {
        self.folders.iter().find(|d| d.is_dir()).cloned().or_else(|| self.folders.last().cloned())
    }

    pub fn find_key(&self, key: &str) -> Option<usize> {
        self.lenses.iter().position(|l| l.key() == key)
    }

    pub fn makers(&self) -> Vec<String> {
        let mut makers: Vec<String> = self.lenses.iter().map(|l| l.maker.clone()).collect();
        makers.dedup();
        makers
    }

    pub fn find_camera(&self, make: &str, model: &str) -> Option<&Camera> {
        if model.is_empty() {
            return None;
        }
        let want = strip_maker(model, make);
        let make_n = normalize(make);
        self.cameras.iter().find(|c| {
            let maker = normalize(&c.maker);
            (make_n.is_empty() || maker.contains(&make_n) || make_n.contains(&maker)) && strip_maker(&c.model, &c.maker) == want
        })
    }

    /// The lens profile that fits the photo best, if any.
    pub fn auto_match(&self, exif: &Exif) -> Option<usize> {
        let camera = self.find_camera(&exif.make, &exif.model);
        let fits_focal = |lens: &Lens| exif.focal <= 0.0 || lens.focal_range.1 <= 0.0 || (exif.focal >= lens.focal_range.0 * 0.97 && exif.focal <= lens.focal_range.1 * 1.03);
        // Interchangeable lenses: by name (and mount).
        if !exif.lens_model.trim().is_empty() {
            let maker = if exif.lens_make.is_empty() { exif.make.as_str() } else { exif.lens_make.as_str() };
            let want = strip_maker(&exif.lens_model, maker);
            let mut best: Option<(i32, usize)> = None;
            for (i, lens) in self.lenses.iter().enumerate() {
                let have = strip_maker(&lens.model, &lens.maker);
                let mut score = if have == want {
                    100
                } else if want.len() >= 4 && have.len() >= 4 && (have.contains(&want) || want.contains(&have)) {
                    // One name inside the other, unless the difference is a version suffix.
                    let (long, short) = if have.len() > want.len() { (&have, &want) } else { (&want, &have) };
                    let rest = long.replacen(short.as_str(), "", 1);
                    if ["ii", "iii", "iv", "v", "2", "3"].contains(&rest.as_str()) {
                        0
                    } else {
                        40 - rest.len() as i32
                    }
                } else {
                    0
                };
                if score <= 0 {
                    continue;
                }
                // A partial name match must come from the same maker ("EF 50mm f/1.8 II" by
                // Canon is not the Pentacon 50mm f/1.8).
                if score < 100 && !maker_agrees(maker, &lens.maker) {
                    continue;
                }
                if let Some(camera) = camera {
                    if lens.mounts.iter().any(|m| m == &camera.mount) {
                        score += 10;
                    }
                }
                if !fits_focal(lens) {
                    score -= 30;
                }
                if score > 0 && best.map_or(true, |(s, _)| score > s) {
                    best = Some((score, i));
                }
            }
            if best.is_some() {
                return best.map(|(_, i)| i);
            }
        }
        // Fixed-lens cameras and phones: the lens on the camera's own mount, by focal length.
        // Only when that mount has a single lens: on an interchangeable-lens body that did not
        // record its lens, any guess would be wrong.
        let camera = camera?;
        let on_mount = self.lenses.iter().filter(|l| l.mounts.iter().any(|m| m == &camera.mount)).count();
        if on_mount != 1 {
            return None;
        }
        self.lenses
            .iter()
            .enumerate()
            .filter(|(_, l)| l.mounts.iter().any(|m| m == &camera.mount) && fits_focal(l))
            .min_by(|(_, a), (_, b)| {
                let off = |l: &Lens| ((l.focal_range.0 + l.focal_range.1) * 0.5 - exif.focal).abs();
                off(a).total_cmp(&off(b))
            })
            .map(|(i, _)| i)
    }
}

// ---------------------------------------------------------------------------------------
// Interpolation
// ---------------------------------------------------------------------------------------

/// A calibration evaluated at one focal length and aperture.
#[derive(Clone, Debug, Default)]
pub struct Calibration {
    /// 0 none, 1 ptlens, 2 poly3, 3 poly5.
    pub dist_model: u8,
    pub dist: [f32; 3],
    pub tca: Option<([f32; 3], [f32; 3])>,
    pub vig: Option<[f32; 3]>,
}

/// The entries just below and above `focal` (or the nearest one twice) and the blend between.
fn bracket<T: Copy>(items: &[T], focal_of: impl Fn(&T) -> f32, focal: f32) -> Option<(T, T, f32)> {
    let mut sorted: Vec<T> = items.to_vec();
    sorted.sort_by(|a, b| focal_of(a).total_cmp(&focal_of(b)));
    let first = *sorted.first()?;
    if focal <= focal_of(&first) {
        return Some((first, first, 0.0));
    }
    for pair in sorted.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if focal <= focal_of(&b) {
            let span = focal_of(&b) - focal_of(&a);
            let t = if span > 0.0 { (focal - focal_of(&a)) / span } else { 0.0 };
            return Some((a, b, t));
        }
    }
    let last = *sorted.last()?;
    Some((last, last, 0.0))
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl Lens {
    /// The calibration at `focal` mm and f/`aperture` (0 = unknown: the widest calibrated).
    pub fn calibrate(&self, focal: f32, aperture: f32) -> Calibration {
        let focal = if focal > 0.0 { focal } else { self.focal_range.0.max(1.0) };
        let mut out = Calibration::default();

        // Distortion: same model only; coefficients scale roughly with 1/focal, as Lensfun
        // interpolates them.
        if let Some(model) = self.dist.first().map(|d| d.model) {
            let same: Vec<DistCal> = self.dist.iter().copied().filter(|d| d.model == model).collect();
            if let Some((a, b, t)) = bracket(&same, |d| d.focal, focal) {
                out.dist_model = model;
                for i in 0..3 {
                    out.dist[i] = lerp(a.k[i] * a.focal, b.k[i] * b.focal, t) / focal;
                }
            }
        }

        if let Some((a, b, t)) = bracket(&self.tca, |c| c.focal, focal) {
            let blend = |x: [f32; 3], y: [f32; 3], fx: f32, fy: f32| [lerp(x[0], y[0], t), lerp(x[1] * fx, y[1] * fy, t) / focal, lerp(x[2] * fx, y[2] * fy, t) / focal];
            out.tca = Some((blend(a.red, b.red, a.focal, b.focal), blend(a.blue, b.blue, a.focal, b.focal)));
        }

        // Vignetting: bilinear across the nearest calibrated focal lengths and apertures,
        // using the farthest focus distance at each.
        if !self.vig.is_empty() {
            let mut focals: Vec<f32> = self.vig.iter().map(|v| v.focal).collect();
            focals.sort_by(f32::total_cmp);
            focals.dedup();
            let at_focal = |f: f32| -> Option<[f32; 3]> {
                let here: Vec<VigCal> = self.vig.iter().copied().filter(|v| v.focal == f).collect();
                let far = here.iter().map(|v| v.distance).fold(0.0f32, f32::max);
                let set: Vec<VigCal> = here.into_iter().filter(|v| v.distance == far).collect();
                let wanted = if aperture > 0.0 { aperture } else { set.iter().map(|v| v.aperture).fold(f32::MAX, f32::min) };
                let (a, b, t) = bracket(&set, |v| v.aperture, wanted)?;
                Some([lerp(a.k[0], b.k[0], t), lerp(a.k[1], b.k[1], t), lerp(a.k[2], b.k[2], t)])
            };
            if let Some((f0, f1, t)) = bracket(&focals, |f| *f, focal) {
                if let (Some(a), Some(b)) = (at_focal(f0), at_focal(f1)) {
                    out.vig = Some([lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]);
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------------
// Shader parameters
// ---------------------------------------------------------------------------------------

/// Everything the shader needs for lens correction and the geometry it implies.
#[derive(Clone, Copy, Debug)]
pub struct Gpu {
    /// x model (0 none, 1 ptlens, 2 poly3, 3 poly5, 4 embedded knots), yzw coefficients.
    pub dist: [f32; 4],
    /// x Hugin-radius scale, y vignetting-radius scale (crop ratio), z manual distortion,
    /// w enlargement (constrain crop).
    pub geo: [f32; 4],
    /// x profile distortion strength, y profile vignetting strength, z embedded knot count.
    pub strength: [f32; 4],
    /// v, c, b and manual scale for red, then blue.
    pub tca_r: [f32; 4],
    pub tca_b: [f32; 4],
    /// k1, k2, k3, w model (0 none, 1 pa, 2 embedded).
    pub vig: [f32; 4],
    /// x manual amount (-1..1), y midpoint (0..1).
    pub vig2: [f32; 4],
    /// Purple amount, purple hue, green amount, green hue (all 0..1).
    pub fringe: [f32; 4],
    pub knots: [[f32; 4]; 16],
}

impl Default for Gpu {
    fn default() -> Self {
        Self {
            dist: [0.0; 4],
            geo: [1.0, 1.0, 0.0, 1.0],
            strength: [1.0, 1.0, 0.0, 0.0],
            tca_r: [1.0, 0.0, 0.0, 0.0],
            tca_b: [1.0, 0.0, 0.0, 0.0],
            vig: [0.0; 4],
            vig2: [0.0, 0.5, 0.0, 0.0],
            fringe: [0.0, 0.5, 0.0, 0.5],
            knots: [[1.0; 4]; 16],
        }
    }
}

/// What is being corrected, for the panel.
pub enum Active {
    None,
    Lensfun(usize),
    Embedded,
}

impl Gpu {
    /// Build the shader parameters. `profile` is the Lensfun lens in use (with the crop
    /// factor of the camera, when known), `embedded` the photo's own tables; `aspect` is the
    /// stored picture's width / height.
    pub fn new(settings: &LensSettings, active: &Active, db: &Database, exif: &Exif, embedded: Option<&Embedded>, aspect: f32) -> Self {
        let mut gpu = Gpu::default();
        let long_side = aspect.max(1.0 / aspect.max(1.0e-3));
        // Hugin radius: half the shorter side of a 3:2 frame, matched through the diagonal.
        let shape_to_short = if aspect < 1.0 { 1.0 / aspect } else { 1.0 };
        let diagonal_match = 1.5f32.hypot(1.0) / long_side.hypot(1.0);
        gpu.geo[0] = diagonal_match * shape_to_short;
        if settings.enabled {
            gpu.strength = [settings.profile_distortion / 100.0, settings.profile_vignetting / 100.0, 0.0, 0.0];
            match active {
                Active::Lensfun(i) => {
                    let lens = &db.lenses[*i];
                    let camera_crop = db.find_camera(&exif.make, &exif.model).map_or(lens.crop, |c| c.crop);
                    let crop_ratio = lens.crop / camera_crop.max(0.01);
                    gpu.geo[0] *= crop_ratio;
                    gpu.geo[1] = crop_ratio;
                    let cal = lens.calibrate(exif.focal, exif.fnumber);
                    if cal.dist_model > 0 {
                        gpu.dist = [cal.dist_model as f32, cal.dist[0], cal.dist[1], cal.dist[2]];
                    }
                    if let Some((red, blue)) = cal.tca {
                        gpu.tca_r[..3].copy_from_slice(&red);
                        gpu.tca_b[..3].copy_from_slice(&blue);
                    }
                    if let Some(k) = cal.vig {
                        gpu.vig = [k[0], k[1], k[2], 1.0];
                    }
                }
                Active::Embedded => {
                    if let Some(embedded) = embedded {
                        let n = embedded.knots.len().min(16);
                        gpu.knots[..n].copy_from_slice(&embedded.knots[..n]);
                        gpu.strength[2] = n as f32;
                        gpu.dist[0] = 4.0;
                        gpu.vig[3] = 2.0;
                    }
                }
                Active::None => {}
            }
        }
        gpu.geo[2] = settings.distortion / 100.0 * 0.12;
        gpu.tca_r[3] = settings.red_cyan / 100.0 * 0.002;
        gpu.tca_b[3] = settings.blue_yellow / 100.0 * 0.002;
        gpu.vig2 = [settings.vignette / 100.0, settings.midpoint / 100.0, 0.0, 0.0];
        gpu.fringe = [settings.purple_amount / 100.0, settings.purple_hue / 100.0, settings.green_amount / 100.0, settings.green_hue / 100.0];
        if settings.constrain {
            gpu.geo[3] = gpu.constrain_scale(aspect);
        }
        gpu
    }

    fn knot(&self, r: f32, component: usize) -> f32 {
        let n = self.strength[2] as usize;
        if n < 2 {
            return 1.0;
        }
        Embedded { knots: self.knots[..n].to_vec() }.value(r, component)
    }

    /// Where an output point (centred texture coordinates, before enlargement) is sampled
    /// from, for green, red and blue. Mirrors `lens_coords` in shader.wgsl.
    fn warp(&self, c: [f32; 2], aspect: f32, zoom: f32) -> [[f32; 2]; 3] {
        let (cx, cy) = (c[0] / zoom, c[1] / zoom);
        let len = (cx * aspect).hypot(cy);
        let half_diag = aspect.hypot(1.0);
        let rh = len * 2.0 * self.geo[0];
        let rd = len / half_diag * 2.0;
        let [model, k1, k2, k3] = self.dist;
        let mut s = match model as i32 {
            1 => k1 * rh.powi(3) + k2 * rh * rh + k3 * rh + 1.0 - k1 - k2 - k3,
            2 => 1.0 - k1 + k1 * rh * rh,
            3 => 1.0 + k1 * rh * rh + k2 * rh.powi(4),
            4 => self.knot(rd, 0),
            _ => 1.0,
        };
        s = 1.0 + self.strength[0] * (s - 1.0);
        s *= 1.0 - self.geo[2] * rd * rd;
        let src = [cx * s, cy * s];
        let rs = (src[0] * aspect).hypot(src[1]) * 2.0 * self.geo[0];
        let mut sr = self.tca_r[0] + self.tca_r[1] * rs + self.tca_r[2] * rs * rs;
        let mut sb = self.tca_b[0] + self.tca_b[1] * rs + self.tca_b[2] * rs * rs;
        if model as i32 == 4 {
            sr = self.knot(rd, 1);
            sb = self.knot(rd, 2);
        }
        sr *= 1.0 + self.tca_r[3];
        sb *= 1.0 + self.tca_b[3];
        [src, [src[0] * sr, src[1] * sr], [src[0] * sb, src[1] * sb]]
    }

    /// The smallest enlargement at which no edge point samples outside the picture.
    fn constrain_scale(&self, aspect: f32) -> f32 {
        let fits = |zoom: f32| {
            (0..=32).all(|i| {
                let t = i as f32 / 32.0 - 0.5;
                [[t, -0.5], [t, 0.5], [-0.5, t], [0.5, t]].into_iter().all(|edge| {
                    self.warp(edge, aspect, zoom).iter().all(|p| p[0].abs() <= 0.5 + 1.0e-4 && p[1].abs() <= 0.5 + 1.0e-4)
                })
            })
        };
        if fits(1.0) {
            return 1.0;
        }
        let (mut lo, mut hi) = (1.0f32, 3.0f32);
        if !fits(hi) {
            return hi;
        }
        for _ in 0..24 {
            let mid = (lo + hi) * 0.5;
            if fits(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        hi
    }
}
