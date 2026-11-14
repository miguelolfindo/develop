//! Filmstrip: thumbnails of the raw files in the open file's folder.
//!
//! Thumbnails come from the preview JPEG every ARW carries, so nothing is demosaiced here.
//! They are decoded on a few background threads, and only for the items scrolled into view.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::settings::Store;

/// Height thumbnails are decoded to, in pixels (enough for a high-DPI screen).
const THUMB_PX: u32 = 200;
/// Height thumbnails are drawn at, and the room for the file name under each.
/// Side of the square thumbnail cards, and the room for the file name under each.
const CARD: f32 = 96.0;
const LABEL_HEIGHT: f32 = 20.0;
const HEADER_HEIGHT: f32 = 28.0;
/// Height of the whole panel: header line, thumbnails, names, scroll bar.
pub const PANEL_HEIGHT: f32 = HEADER_HEIGHT + 1.0 + 8.0 + CARD + LABEL_HEIGHT + 8.0;
const WORKERS: usize = 4;

enum Thumb {
    /// Not asked for yet (never scrolled into view).
    None,
    Pending,
    Ready(egui::TextureHandle),
    Failed,
}

struct Item {
    path: PathBuf,
    name: String,
    /// File type, lower case ("arw", "jpg"...).
    ext: String,
    /// Capture date as sortable text; read only when sorting by date.
    date: Option<String>,
    thumb: Thumb,
}

/// How the filmstrip is ordered.
#[derive(Clone, Copy, PartialEq)]
pub enum Sort {
    CaptureDate,
    FileName,
    FileType,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::CaptureDate, Sort::FileName, Sort::FileType];

    pub fn label(self) -> &'static str {
        match self {
            Sort::CaptureDate => "Capture Date",
            Sort::FileName => "File Name",
            Sort::FileType => "File Type",
        }
    }
}

struct Job {
    generation: u64,
    index: usize,
    path: PathBuf,
    /// Saved thumbnail of the edited photo, used instead of the camera's preview if present.
    cache: Option<PathBuf>,
}

type Done = (u64, usize, Option<egui::ColorImage>);

pub struct Filmstrip {
    dir: Option<PathBuf>,
    items: Vec<Item>,
    /// Bumped whenever the folder changes, so results for the old folder are dropped.
    generation: Arc<AtomicU64>,
    jobs: Sender<Job>,
    done: Receiver<Done>,
    /// File name last highlighted; when it changes the strip scrolls to the new one.
    shown_current: Option<std::ffi::OsString>,
    /// Photos picked with Ctrl-click or Shift-click (indices into `items`).
    selected: std::collections::BTreeSet<usize>,
    /// Filter: show only photos with at least this many stars (0 = all).
    min_stars: u8,
    sort: Sort,
    ascending: bool,
}

impl Filmstrip {
    pub fn new(ctx: &egui::Context) -> Self {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel::<Done>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let generation = Arc::new(AtomicU64::new(0));
        for _ in 0..WORKERS {
            let job_rx = job_rx.clone();
            let done_tx = done_tx.clone();
            let generation = generation.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || loop {
                let job = match job_rx.lock() {
                    Ok(rx) => rx.recv(),
                    Err(_) => return,
                };
                let Ok(job) = job else { return };
                if job.generation != generation.load(Ordering::Relaxed) {
                    continue;
                }
                let image = std::panic::catch_unwind(|| {
                    job.cache.as_deref().and_then(cached_thumbnail).or_else(|| thumbnail(&job.path))
                })
                .ok()
                .flatten();
                if done_tx.send((job.generation, job.index, image)).is_err() {
                    return;
                }
                ctx.request_repaint();
            });
        }
        Self { dir: None, items: Vec::new(), generation, jobs, done, shown_current: None, selected: Default::default(), min_stars: 0, sort: Sort::CaptureDate, ascending: true }
    }

    /// Show `dir`'s raw files. Does nothing if that folder is already showing.
    pub fn set_folder(&mut self, dir: Option<&Path>) {
        let Some(dir) = dir else { return };
        let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
        if self.dir.as_deref() != Some(dir) {
            self.dir = Some(dir.to_path_buf());
            self.rescan();
        }
    }

    /// The folder being shown.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// Read the folder again (new or deleted files).
    fn rescan(&mut self) {
        let Some(dir) = &self.dir else { return };
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| is_supported(p) && p.is_file())
            .collect();
        paths.sort();
        self.items = paths
            .into_iter()
            .map(|path| Item {
                name: path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                ext: extension_of(&path),
                date: None,
                path,
                thumb: Thumb::None,
            })
            .collect();
        self.selected.clear();
        self.arrange();
    }

    /// Put the photos in the chosen order. Thumbnails already loaded are kept.
    fn arrange(&mut self) {
        use rayon::prelude::*;
        let selected = self.selected_paths();
        if self.sort == Sort::CaptureDate {
            self.items.par_iter_mut().filter(|item| item.date.is_none()).for_each(|item| item.date = Some(capture_date(&item.path)));
        }
        let by_name = |a: &Item, b: &Item| natural_cmp(&a.name, &b.name).then_with(|| a.ext.cmp(&b.ext));
        match self.sort {
            Sort::CaptureDate => self.items.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| by_name(a, b))),
            Sort::FileName => self.items.sort_by(by_name),
            Sort::FileType => self.items.sort_by(|a, b| a.ext.cmp(&b.ext).then_with(|| by_name(a, b))),
        }
        if !self.ascending {
            self.items.reverse();
        }
        // Thumbnail jobs in flight name photos by position, which has just changed.
        self.generation.fetch_add(1, Ordering::Relaxed);
        for item in &mut self.items {
            if matches!(item.thumb, Thumb::Pending) {
                item.thumb = Thumb::None;
            }
        }
        self.selected = selected.iter().filter_map(|p| self.items.iter().position(|item| item.path == *p)).collect();
        self.shown_current = None;
    }

    /// Turn finished thumbnails into textures.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let generation = self.generation.load(Ordering::Relaxed);
        while let Ok((job_generation, index, image)) = self.done.try_recv() {
            if job_generation != generation {
                continue;
            }
            if let Some(item) = self.items.get_mut(index) {
                item.thumb = match image {
                    Some(image) => {
                        Thumb::Ready(ctx.load_texture(format!("thumb{index}"), image, egui::TextureOptions::LINEAR))
                    }
                    None => Thumb::Failed,
                };
            }
        }
    }

    /// Replace a photo's thumbnail with a rendering of its current edit.
    pub fn set_thumb(&mut self, ctx: &egui::Context, photo: &Path, image: egui::ColorImage) {
        if let Some(index) = self.position(Some(photo)) {
            let texture = ctx.load_texture(format!("thumb{index}"), image, egui::TextureOptions::LINEAR);
            self.items[index].thumb = Thumb::Ready(texture);
        }
    }

    /// Load a photo's thumbnail again (after its edit was reset).
    pub fn reload(&mut self, photo: &Path) {
        if let Some(index) = self.position(Some(photo)) {
            self.items[index].thumb = Thumb::None;
        }
    }

    fn position(&self, current: Option<&Path>) -> Option<usize> {
        let name = current?.file_name()?;
        self.items.iter().position(|item| item.path.file_name() == Some(name))
    }

    fn passes(&self, index: usize, store: &Store) -> bool {
        self.min_stars == 0 || store.rating(&self.items[index].path) >= self.min_stars
    }

    /// The file `step` places after (or before) `current`, among the photos the filter shows.
    pub fn neighbour(&self, current: Option<&Path>, step: isize, store: &Store) -> Option<PathBuf> {
        let mut index = self.position(current)? as isize;
        loop {
            index += step;
            let i = usize::try_from(index).ok().filter(|&i| i < self.items.len())?;
            if self.passes(i, store) {
                return Some(self.items[i].path.clone());
            }
        }
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected.iter().filter_map(|&i| self.items.get(i)).map(|item| item.path.clone()).collect()
    }

    /// The photos a keyboard command applies to: the selection, or else the open photo.
    pub fn targets(&self, current: Option<&Path>) -> Vec<PathBuf> {
        if self.selected.is_empty() {
            current.map(Path::to_path_buf).into_iter().collect()
        } else {
            self.selected_paths()
        }
    }

    /// The photos a right-click on `photo` applies to: the whole selection when `photo` is
    /// part of a selection of several, otherwise just `photo`.
    pub fn targets_for(&self, photo: &Path) -> Vec<PathBuf> {
        match self.position(Some(photo)) {
            Some(index) if self.selected.len() > 1 && self.selected.contains(&index) => self.selected_paths(),
            _ => vec![photo.to_path_buf()],
        }
    }

    /// Draw the strip and report what the user asked for.
    /// `can_paste` says whether there are copied settings to paste.
    pub fn show(&mut self, ui: &mut egui::Ui, current: Option<&Path>, store: &Store, can_paste: bool) -> Option<Action> {
        use crate::theme;
        use crate::ui::{self, Icon, Look};
        ui.spacing_mut().item_spacing.y = 0.0;
        let Some(dir) = self.dir.clone() else {
            ui.centered_and_justified(|ui| ui.label(ui::dim("The photos of the folder you choose will appear here.", 12.5)));
            return None;
        };
        let current_index = self.position(current);
        let current_name = current.and_then(|p| p.file_name()).map(|n| n.to_os_string());
        let scroll_to_current = current_name != self.shown_current;
        self.shown_current = current_name;
        // Moving to another photo (arrow keys, Open) makes it the selection, so rating keys
        // and paste act on the photo being looked at, not on one clicked earlier.
        if scroll_to_current {
            if let Some(index) = current_index.filter(|i| !self.selected.contains(i)) {
                self.selected = std::iter::once(index).collect();
            }
        }
        let shown: Vec<usize> = (0..self.items.len()).filter(|&i| self.passes(i, store)).collect();

        // Header: where we are on the left, sort / filter / refresh on the right.
        ui.horizontal(|ui| {
            ui.set_height(HEADER_HEIGHT);
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.spacing_mut().interact_size.y = 24.0;
            let count = shown.len();
            let mut place = match current_index.and_then(|c| shown.iter().position(|&i| i == c)) {
                Some(i) => format!("{} of {count}", i + 1),
                None => format!("{count} photo{}", if count == 1 { "" } else { "s" }),
            };
            if self.min_stars > 0 {
                place.push_str(&format!(" (of {})", self.items.len()));
            }
            if self.selected.len() > 1 {
                place.push_str(&format!("  •  {} selected", self.selected.len()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui::icon_button(ui, Icon::Refresh, 22.0, Look::ghost()).on_hover_text("Read the folder again").clicked() {
                    self.rescan();
                }
                let label = match self.min_stars {
                    0 => "Filter: All".to_owned(),
                    5 => format!("Filter: {}", stars(5)),
                    n => format!("Filter: {}+", stars(n)),
                };
                ui.menu_button(label, |ui| {
                    let mut pick = |ui: &mut egui::Ui, value: u8, text: String| {
                        if ui.radio_value(&mut self.min_stars, value, text).clicked() {
                            ui.close_menu();
                        }
                    };
                    pick(ui, 0, "All photos".to_owned());
                    pick(ui, 1, format!("Rated only ({} and up)", stars(1)));
                    for n in 2..=4 {
                        pick(ui, n, format!("{} and up", stars(n)));
                    }
                    pick(ui, 5, format!("{} only", stars(5)));
                })
                .response
                .on_hover_text("Show only rated photos");
                let arrow = if self.ascending { Icon::Ascending } else { Icon::Descending };
                if ui::icon_button(ui, arrow, 24.0, Look::bordered()).on_hover_text(if self.ascending { "Ascending: click for descending" } else { "Descending: click for ascending" }).clicked() {
                    self.ascending = !self.ascending;
                    self.arrange();
                }
                let before = self.sort;
                egui::ComboBox::from_id_source("filmstrip_sort").selected_text(self.sort.label()).width(104.0).show_ui(ui, |ui| {
                    for sort in Sort::ALL {
                        ui.selectable_value(&mut self.sort, sort, sort.label());
                    }
                });
                if self.sort != before {
                    self.arrange();
                }
                ui.label(egui::RichText::new("Sort").font(theme::font(11.5)).color(theme::FADED));
                // Whatever room is left goes to the path, cut at the left edge of the controls.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let text = format!("{}  •  {place}", dir.display());
                    ui.add(egui::Label::new(egui::RichText::new(text).font(theme::font(12.0)).color(theme::TEXT_HEADER)).truncate(true));
                });
            });
        });
        ui::hairline(ui);
        ui.add_space(8.0);

        let mut action = None;
        let generation = self.generation.load(Ordering::Relaxed);
        let selection_paths = self.selected_paths();
        // The mouse wheel scrolls the strip sideways.
        ui.style_mut().always_scroll_the_only_direction = true;
        egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if shown.is_empty() {
                    ui.label(ui::dim("No photos match the filter.", 12.5));
                }
                for &index in &shown {
                    if index >= self.items.len() {
                        break; // the folder was just read again
                    }
                    let item = &mut self.items[index];
                    let (rect, response) = ui.allocate_exact_size(egui::vec2(CARD, CARD + LABEL_HEIGHT), egui::Sense::click());
                    let card = egui::Rect::from_min_size(rect.min, egui::Vec2::splat(CARD));
                    let is_current = current_index == Some(index);
                    let is_selected = self.selected.contains(&index);
                    let is_edited = store.contains(&item.path);
                    if is_current && scroll_to_current {
                        ui.scroll_to_rect(rect, Some(egui::Align::Center));
                    }
                    if ui.is_rect_visible(rect) {
                        if matches!(item.thumb, Thumb::None) {
                            let cache = if is_edited { crate::settings::thumb_file(&item.path) } else { None };
                            let job = Job { generation, index, path: item.path.clone(), cache };
                            item.thumb = if self.jobs.send(job).is_ok() { Thumb::Pending } else { Thumb::Failed };
                        }
                        let painter = ui.painter().with_clip_rect(rect.expand(2.0).intersect(ui.clip_rect()));
                        // The card: a well with a hairline, amber when it is the open photo.
                        let (fill, border) = if is_current {
                            (egui::Color32::from_rgb(0x1C, 0x1F, 0x26), egui::Stroke::new(2.0_f32, theme::ACCENT))
                        } else if is_selected {
                            (egui::Color32::from_rgb(0x1C, 0x1F, 0x26), egui::Stroke::new(1.0_f32, theme::tint(theme::ACCENT, 0.7)))
                        } else if response.hovered() {
                            (egui::Color32::from_rgb(0x17, 0x1A, 0x20), egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(40)))
                        } else {
                            (theme::WELL, theme::hairline())
                        };
                        painter.rect(card, 6.0, fill, border);
                        // The picture, whole, inside the same margin on every side.
                        let inner = card.shrink(6.0);
                        match &item.thumb {
                            Thumb::Ready(texture) => {
                                let size = texture.size_vec2();
                                let scale = (inner.width() / size.x).min(inner.height() / size.y);
                                let picture = egui::Rect::from_center_size(inner.center(), size * scale);
                                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                                painter.image(texture.id(), picture, uv, egui::Color32::WHITE);
                            }
                            Thumb::Failed => {
                                painter.text(inner.center(), egui::Align2::CENTER_CENTER, "no preview", theme::font(11.0), theme::FADED);
                            }
                            _ => {}
                        }
                        // File type, top left of the card.
                        let badge = if item.ext == "jpeg" { "JPG" } else if item.ext == "tiff" { "TIF" } else { item.ext.as_str() };
                        let badge_color = if item.ext == "arw" { theme::ACCENT } else { theme::TEAL };
                        let label = painter.layout_no_wrap(badge.to_uppercase(), theme::bold(9.0), badge_color);
                        let back = egui::Rect::from_min_size(card.min + egui::vec2(5.0, 5.0), label.size() + egui::vec2(8.0, 3.0));
                        painter.rect(back, 3.0, egui::Color32::from_rgba_unmultiplied(0x11, 0x13, 0x17, 0xE0), egui::Stroke::new(1.0_f32, theme::tint(badge_color, 0.45)));
                        painter.galley(back.min + egui::vec2(4.0, 1.5), label, badge_color);
                        if is_edited {
                            // "Edited" mark, top right: a dark disc with two little sliders.
                            let c = card.right_top() + egui::vec2(-13.0, 13.0);
                            painter.circle(c, 7.5, egui::Color32::from_rgba_unmultiplied(0x11, 0x13, 0x17, 0xE0), egui::Stroke::new(1.0_f32, theme::tint(theme::ACCENT, 0.7)));
                            for (dy, knob) in [(-2.2_f32, 1.8_f32), (2.2, -1.8)] {
                                let y = c.y + dy;
                                painter.line_segment([egui::pos2(c.x - 4.0, y), egui::pos2(c.x + 4.0, y)], egui::Stroke::new(1.1_f32, theme::ACCENT));
                                painter.circle_filled(egui::pos2(c.x + knob, y), 1.7, theme::ACCENT);
                            }
                        }
                        let rating = store.rating(&item.path);
                        if rating > 0 {
                            let galley = painter.layout_no_wrap(stars(rating), theme::font(10.0), theme::ACCENT);
                            let back = egui::Rect::from_min_size(card.left_bottom() + egui::vec2(5.0, -5.0 - galley.size().y - 2.0), galley.size() + egui::vec2(6.0, 2.0));
                            painter.rect_filled(back, 3.0, egui::Color32::from_rgba_unmultiplied(0x11, 0x13, 0x17, 0xE0));
                            painter.galley(back.min + egui::vec2(3.0, 1.0), galley, theme::ACCENT);
                        }
                        // Name under the card, never wider than the card.
                        let color = if is_current { theme::TEXT_BRIGHT } else if is_selected { theme::TEXT } else { theme::FADED };
                        let mut job = egui::text::LayoutJob::single_section(item.name.clone(), egui::TextFormat { font_id: theme::font(11.0), color, ..Default::default() });
                        job.wrap = egui::text::TextWrapping { max_width: CARD, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                        let galley = ui.fonts(|f| f.layout_job(job));
                        painter.galley(egui::pos2(card.center().x - galley.size().x * 0.5, card.bottom() + 4.0), galley, color);
                    }

                    // Click opens; Ctrl-click adds or removes from the selection; Shift-click
                    // selects everything from the open photo to here.
                    if response.clicked() {
                        let modifiers = ui.input(|i| i.modifiers);
                        if modifiers.command {
                            if self.selected.is_empty() {
                                self.selected.extend(current_index);
                            }
                            if !self.selected.remove(&index) {
                                self.selected.insert(index);
                            }
                        } else if modifiers.shift {
                            let anchor = current_index.or_else(|| self.selected.iter().next().copied()).unwrap_or(index);
                            let (from, to) = (anchor.min(index), anchor.max(index));
                            self.selected = shown.iter().copied().filter(|i| (from..=to).contains(i)).collect();
                        } else {
                            self.selected = std::iter::once(index).collect();
                            if !is_current {
                                action = Some(Action::Open(item.path.clone()));
                            }
                        }
                    }
                    let path = &self.items[index].path;
                    response.on_hover_text(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).context_menu(|ui| {
                        let targets = if is_selected && selection_paths.len() > 1 { selection_paths.clone() } else { vec![path.clone()] };
                        if let Some(picked) = edit_menu(ui, path, &targets, is_current || is_edited, can_paste) {
                            action = Some(picked);
                        }
                    });
                }
            });
        });
        action
    }
}

/// What the user asked for in the filmstrip or in a right-click menu.
pub enum Action {
    Open(PathBuf),
    /// Copy this photo's settings.
    Copy(PathBuf),
    /// Paste the copied settings onto these photos.
    Paste(Vec<PathBuf>),
    /// Give these photos this many stars (0 removes the rating).
    Rate(Vec<PathBuf>, u8),
}

pub fn stars(count: u8) -> String {
    "★".repeat(count as usize)
}

/// The right-click menu shared by the filmstrip and the main picture.
pub fn edit_menu(ui: &mut egui::Ui, photo: &Path, targets: &[PathBuf], can_copy: bool, can_paste: bool) -> Option<Action> {
    let mut action = None;
    if ui.add_enabled(can_copy, egui::Button::new("Copy settings")).on_disabled_hover_text("This photo has no edits to copy").clicked() {
        action = Some(Action::Copy(photo.to_path_buf()));
        ui.close_menu();
    }
    let paste = if targets.len() > 1 { format!("Paste settings to {} photos", targets.len()) } else { "Paste settings".to_owned() };
    if ui.add_enabled(can_paste, egui::Button::new(paste)).on_disabled_hover_text("Copy a photo's settings first").clicked() {
        action = Some(Action::Paste(targets.to_vec()));
        ui.close_menu();
    }
    ui.separator();
    let rate = if targets.len() > 1 { format!("Rate {} photos", targets.len()) } else { "Rating".to_owned() };
    ui.menu_button(rate, |ui| {
        for n in (0..=5u8).rev() {
            let label = if n == 0 { "No rating".to_owned() } else { stars(n) };
            if ui.button(label).clicked() {
                action = Some(Action::Rate(targets.to_vec(), n));
                ui.close_menu();
            }
        }
    });
    action
}

/// A thumbnail saved earlier for an edited photo.
fn cached_thumbnail(file: &Path) -> Option<egui::ColorImage> {
    let rgb = image::open(file).ok()?.to_rgb8();
    Some(egui::ColorImage::from_rgb([rgb.width() as usize, rgb.height() as usize], rgb.as_raw()))
}

/// File types the app opens, lower case.
pub const EXTENSIONS: [&str; 9] = ["arw", "dng", "jpg", "jpeg", "png", "tif", "tiff", "heic", "heif"];

pub fn extension_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn is_supported(path: &Path) -> bool {
    EXTENSIONS.contains(&extension_of(path).as_str())
}

/// True for the camera raw formats (as opposed to finished pictures such as JPEG).
pub fn is_raw(path: &Path) -> bool {
    matches!(extension_of(path).as_str(), "arw" | "dng")
}

/// Compare names the way people read them: "IMG_9" before "IMG_10".
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let strip = |s: &[u8]| {
                let zeros = s.iter().take_while(|&&c| c == b'0').count();
                s[zeros.min(s.len().saturating_sub(1))..].to_vec()
            };
            let (x, y) = (strip(&a[si..i]), strip(&b[sj..j]));
            let order = x.len().cmp(&y.len()).then_with(|| x.cmp(&y));
            if order != Ordering::Equal {
                return order;
            }
        } else {
            let order = a[i].to_ascii_lowercase().cmp(&b[j].to_ascii_lowercase());
            if order != Ordering::Equal {
                return order;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

/// What a TIFF-structured file (ARW, DNG, TIFF, or the EXIF block of a JPEG) says about
/// itself, as far as the app needs.
#[derive(Default)]
pub struct TiffInfo {
    /// Embedded JPEG previews: offset and length.
    jpegs: Vec<(u64, u64)>,
    /// EXIF orientation (1 = as is, 3, 6, 8 = rotated).
    pub orientation: u16,
    /// Capture date, "YYYY:MM:DD HH:MM:SS".
    pub date: Option<String>,
    /// DNG BaselineExposure, in stops.
    pub baseline_exposure: Option<f32>,
    /// DNG LinearizationTable: stored value -> linear value.
    pub linearization: Option<Vec<u16>>,
    /// Camera and lens, for lens-profile matching.
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens_make: Option<String>,
    pub lens_model: Option<String>,
    pub focal: Option<f32>,
    pub fnumber: Option<f32>,
    /// Sony's embedded lens corrections (tags 0x7037, 0x7035, 0x7032), each starting with
    /// its count.
    pub sony_distortion: Vec<i32>,
    pub sony_ca: Vec<i32>,
    pub sony_vignetting: Vec<i32>,
}

/// Walk the directories of a TIFF structure that starts at `base` in `r`.
fn scan_tiff<R: Read + Seek>(r: &mut R, base: u64) -> Option<TiffInfo> {
    let mut head = [0u8; 8];
    r.seek(SeekFrom::Start(base)).ok()?;
    r.read_exact(&mut head).ok()?;
    let little = match &head[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |b: &[u8]| if little { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) };
    let u32_at = |b: &[u8]| {
        let four = [b[0], b[1], b[2], b[3]];
        if little {
            u32::from_le_bytes(four)
        } else {
            u32::from_be_bytes(four)
        }
    };
    if u16_at(&head[2..]) != 42 {
        return None;
    }
    let mut info = TiffInfo { orientation: 1, ..Default::default() };
    // Directories still to read: (offset, is the EXIF directory).
    let mut queue = vec![(u32_at(&head[4..]) as u64, false)];
    let mut date_time = None;
    let mut visited = 0;
    while let Some((ifd, is_exif)) = queue.pop() {
        visited += 1;
        if ifd == 0 || visited > 16 || r.seek(SeekFrom::Start(base + ifd)).is_err() {
            continue;
        }
        let mut count = [0u8; 2];
        if r.read_exact(&mut count).is_err() {
            continue;
        }
        let count = u16_at(&count) as usize;
        let mut entries = vec![0u8; count * 12 + 4];
        if r.read_exact(&mut entries).is_err() {
            continue;
        }
        let read_at = |r: &mut R, offset: u64, len: usize| -> Option<Vec<u8>> {
            let mut bytes = vec![0u8; len.min(1 << 17)];
            r.seek(SeekFrom::Start(base + offset)).ok()?;
            r.read_exact(&mut bytes).ok()?;
            Some(bytes)
        };
        let (mut jpeg_start, mut jpeg_len, mut strip_start, mut strip_len, mut compression) = (0u64, 0u64, 0u64, 0u64, 0u64);
        for entry in entries[..count * 12].chunks_exact(12) {
            let (tag, kind, n) = (u16_at(entry), u16_at(&entry[2..]), u32_at(&entry[4..]) as usize);
            let value = if kind == 3 { u16_at(&entry[8..]) as u64 } else { u32_at(&entry[8..]) as u64 };
            let text = |r: &mut R| -> Option<String> {
                let bytes = if n <= 4 { entry[8..8 + n].to_vec() } else { read_at(r, value, n)? };
                Some(String::from_utf8_lossy(&bytes).trim_end_matches('\0').trim().to_owned()).filter(|s| !s.is_empty())
            };
            match tag {
                0x0201 => jpeg_start = value,
                0x0202 => jpeg_len = value,
                0x0103 => compression = value,
                0x0111 if n == 1 => strip_start = value,
                0x0117 if n == 1 => strip_len = value,
                0x0112 if !is_exif && info.orientation == 1 => info.orientation = value as u16,
                0x0132 if !is_exif => date_time = text(r),
                0x9003 if is_exif => info.date = text(r),
                0x8769 => queue.push((value, true)),
                0x014A => {
                    // Sub-directories (DNG keeps its previews and the raw data there).
                    if n == 1 {
                        queue.push((value, false));
                    } else if let Some(bytes) = read_at(r, value, n.min(8) * 4) {
                        queue.extend(bytes.chunks_exact(4).map(|b| (u32_at(b) as u64, false)));
                    }
                }
                0x010F if !is_exif && info.make.is_none() => info.make = text(r),
                0x0110 if !is_exif && info.model.is_none() => info.model = text(r),
                0xA433 => info.lens_make = text(r),
                0xA434 => info.lens_model = text(r),
                0x920A | 0x829D if kind == 5 || kind == 10 => {
                    if let Some(b) = read_at(r, value, 8) {
                        let (num, den) = (u32_at(&b) as f32, u32_at(&b[4..]) as f32);
                        if den > 0.0 {
                            if tag == 0x920A {
                                info.focal = Some(num / den);
                            } else {
                                info.fnumber = Some(num / den);
                            }
                        }
                    }
                }
                0x7037 | 0x7035 | 0x7032 if matches!(kind, 3 | 8 | 4 | 9) && (1..=64).contains(&n) => {
                    let size = if kind == 3 || kind == 8 { 2 } else { 4 };
                    let bytes = if n * size <= 4 { Some(entry[8..8 + n * size].to_vec()) } else { read_at(r, value, n * size) };
                    if let Some(bytes) = bytes {
                        let values: Vec<i32> = bytes
                            .chunks_exact(size)
                            .map(|b| match kind {
                                3 => u16_at(b) as i32,
                                8 => u16_at(b) as i16 as i32,
                                4 => u32_at(b) as i32,
                                _ => u32_at(b) as i32,
                            })
                            .collect();
                        match tag {
                            0x7037 => info.sony_distortion = values,
                            0x7035 => info.sony_ca = values,
                            _ => info.sony_vignetting = values,
                        }
                    }
                }
                0xC618 if kind == 3 && n >= 2 && info.linearization.is_none() => {
                    if let Some(bytes) = read_at(r, value, n * 2) {
                        info.linearization = Some(bytes.chunks_exact(2).map(u16_at).collect());
                    }
                }
                0xC62A => {
                    if let Some(b) = read_at(r, value, 8) {
                        let (num, den) = (u32_at(&b) as i32, u32_at(&b[4..]) as i32);
                        if den != 0 {
                            info.baseline_exposure = Some(num as f32 / den as f32);
                        }
                    }
                }
                _ => {}
            }
        }
        if jpeg_start > 0 && jpeg_len > 0 && jpeg_len < (64 << 20) {
            info.jpegs.push((jpeg_start, jpeg_len));
        } else if compression == 7 && strip_start > 0 && strip_len > 0 && strip_len < (64 << 20) {
            info.jpegs.push((strip_start, strip_len));
        }
        if !is_exif {
            queue.push((u32_at(&entries[count * 12..]) as u64, false));
        }
    }
    info.date = info.date.or(date_time);
    // Best preview first: the smallest that is still a real picture (not a postage stamp).
    info.jpegs.sort_by_key(|&(_, len)| (len < 30_000, len));
    Some(info)
}

/// Read a file's TIFF / EXIF information: directly for ARW, DNG and TIFF, from the EXIF
/// block for JPEG. `None` for files without any (PNG, or a JPEG with no EXIF).
pub fn tiff_info(path: &Path) -> Option<TiffInfo> {
    let mut file = File::open(path).ok()?;
    match extension_of(path).as_str() {
        "jpg" | "jpeg" => {
            let mut head = vec![0u8; 256 * 1024];
            let n = file.read(&mut head).ok()?;
            head.truncate(n);
            let at = head.windows(6).position(|w| w == b"Exif\0\0")? + 6;
            let mut info = scan_tiff(&mut std::io::Cursor::new(&head), at as u64)?;
            info.jpegs.clear(); // offsets would be relative to the EXIF block; not needed
            Some(info)
        }
        "png" | "heic" | "heif" => None,
        _ => scan_tiff(&mut file, 0),
    }
}

/// The EXIF orientation recorded in a file (1 = as is, 3, 6, 8 = rotated).
pub fn exif_orientation(path: &Path) -> Option<u16> {
    tiff_info(path).map(|info| info.orientation)
}

/// When the picture was taken, as sortable text: the EXIF capture date, or else the
/// file's modified time in the same form.
pub fn capture_date(path: &Path) -> String {
    if let Some(date) = tiff_info(path).and_then(|info| info.date).filter(|d| d.len() >= 19 && !d.starts_with("0000")) {
        return date;
    }
    let seconds = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    // Days since 1970 to a calendar date (proleptic Gregorian).
    let (days, rem) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!("{year:04}:{month:02}:{day:02} {:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn upright(image: image::DynamicImage, orientation: u16) -> image::DynamicImage {
    match orientation {
        3 => image.rotate180(),
        6 => image.rotate90(),
        8 => image.rotate270(),
        _ => image,
    }
}

/// A small picture of a file for the filmstrip, turned upright: the preview JPEG inside a
/// raw file, or the picture itself scaled down for JPEG, PNG and TIFF.
fn thumbnail(path: &Path) -> Option<egui::ColorImage> {
    let info = tiff_info(path);
    let orientation = info.as_ref().map_or(1, |i| i.orientation);
    let finish = |decoded: image::DynamicImage| {
        let rgb = upright(decoded.thumbnail(THUMB_PX * 3, THUMB_PX), orientation).to_rgb8();
        egui::ColorImage::from_rgb([rgb.width() as usize, rgb.height() as usize], rgb.as_raw())
    };
    if !is_raw(path) {
        return crate::heic::open_picture(path).ok().map(finish);
    }
    let mut file = File::open(path).ok()?;
    for (offset, len) in info?.jpegs {
        let mut bytes = vec![0u8; len as usize];
        if file.seek(SeekFrom::Start(offset)).is_err() || file.read_exact(&mut bytes).is_err() {
            continue;
        }
        if let Ok(decoded) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg) {
            return Some(finish(decoded));
        }
    }
    None
}
