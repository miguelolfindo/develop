//! Filmstrip: thumbnails of the raw files in the open file's folder.
//!
//! Thumbnails come from the preview JPEG every raw file (ARW, NEF, DNG) carries, so nothing is demosaiced here.
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
/// The filmstrip's height can be dragged between these; the thumbnails follow it.
pub const MIN_HEIGHT: f32 = HEADER_HEIGHT + 1.0 + 8.0 + 56.0 + LABEL_HEIGHT + 8.0;
pub const MAX_HEIGHT: f32 = HEADER_HEIGHT + 1.0 + 8.0 + 320.0 + LABEL_HEIGHT + 8.0;
/// Thumbnail sizes in the grid view.
pub const GRID_MIN: f32 = 80.0;
pub const GRID_MAX: f32 = 320.0;
const FOOTER_HEIGHT: f32 = 30.0;

/// What one pass of drawing the thumbnails needs to know.
struct Draw<'a> {
    current_index: Option<usize>,
    scroll_to_current: bool,
    generation: u64,
    selection_paths: &'a [PathBuf],
    shown: &'a [usize],
    can_paste: bool,
    /// Side of a photo card.
    card: f32,
    grid: bool,
}
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
    /// For a version's thumbnail, its key (see settings::version_key); only `cache` is read.
    version: Option<String>,
}

type Done = (u64, usize, Option<String>, Option<egui::ColorImage>);

/// Version cards are a little narrower than photo cards and sit this far apart inside their
/// photo's group.
const VERSION_CARD: f32 = 80.0;
const GROUP_GAP: f32 = 4.0;

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
    /// Search: every word must appear in the file name, a keyword or a person's name.
    search: String,
    /// Search the whole library instead of the folder.
    library_scope: bool,
    /// The search last sent to the library (see `library_query`).
    query_sent: Option<String>,
    /// Showing library search results (photos from many folders) instead of `dir`.
    results: bool,
    /// What the results are ("Library: “dinner”", "People: Talia").
    results_label: String,
    sort: Sort,
    ascending: bool,
    /// Thumbnails of saved versions, by version key. A version without a rendered thumbnail
    /// yet shows its photo's thumbnail until the photo is opened.
    version_thumbs: std::collections::HashMap<String, Thumb>,
    /// Thumbnails fill the main window instead of a strip under the picture.
    grid: bool,
    /// Thumbnail size in the grid view.
    pub grid_card: f32,
    /// Bringing the open photo into view: where its card was drawn (on screen), then the scroll
    /// position that centres it, applied next frame. Done by hand rather than with
    /// `scroll_to_rect`, which would also scroll the other panels' lists.
    scroll_want: Option<egui::Rect>,
    scroll_goal: Option<f32>,
}

impl Filmstrip {
    pub fn grid(&self) -> bool {
        self.grid
    }

    pub fn set_grid(&mut self, on: bool) {
        if self.grid != on {
            self.grid = on;
            // Bring the open photo into view in whichever layout is now showing.
            self.shown_current = None;
            self.scroll_goal = None;
        }
    }

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
                let image = std::panic::catch_unwind(|| match job.version {
                    Some(_) => job.cache.as_deref().and_then(cached_thumbnail),
                    None => job.cache.as_deref().and_then(cached_thumbnail).or_else(|| thumbnail(&job.path)),
                })
                .ok()
                .flatten();
                if done_tx.send((job.generation, job.index, job.version, image)).is_err() {
                    return;
                }
                ctx.request_repaint();
            });
        }
        Self { dir: None, items: Vec::new(), generation, jobs, done, shown_current: None, selected: Default::default(), min_stars: 0, search: String::new(), library_scope: false, query_sent: None, results: false, results_label: String::new(), sort: Sort::CaptureDate, ascending: true, version_thumbs: Default::default(), grid: false, grid_card: 160.0, scroll_want: None, scroll_goal: None }
    }

    /// Start with this search (`--search` on the command line).
    /// (`library:<words>` searches the library.)
    pub fn set_search(&mut self, text: &str) {
        match text.strip_prefix("library:") {
            Some(words) => {
                self.search = words.to_owned();
                self.library_scope = true;
            }
            None => self.search = text.to_owned(),
        }
    }

    /// Show `dir`'s raw files. Does nothing if that folder is already showing.
    pub fn set_folder(&mut self, dir: Option<&Path>) {
        let Some(dir) = dir else { return };
        let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
        if self.dir.as_deref() != Some(dir) || self.results {
            self.dir = Some(dir.to_path_buf());
            self.results = false;
            self.library_scope = false;
            self.query_sent = None;
            self.rescan();
        }
    }

    /// The search to run on the library, when it changed since last asked (None otherwise).
    /// An empty search means: back to the folder.
    pub fn library_query(&mut self) -> Option<String> {
        if !self.library_scope {
            if self.results {
                self.results = false;
                self.query_sent = None;
                self.rescan();
            }
            return None;
        }
        let q = self.search.trim().to_owned();
        if self.query_sent.as_deref() == Some(q.as_str()) {
            return None;
        }
        self.query_sent = Some(q.clone());
        if q.is_empty() && self.results {
            self.results = false;
            self.rescan();
            return None;
        }
        (!q.is_empty()).then_some(q)
    }

    /// Ask for the library search again (the index changed).
    pub fn requery(&mut self) {
        self.query_sent = None;
    }

    /// Whether library search results are showing.
    pub fn showing_results(&self) -> bool {
        self.results
    }

    /// Whether `photo` is one of the photos showing.
    pub fn shows(&self, photo: &Path) -> bool {
        self.items.iter().any(|i| i.path == photo)
    }

    /// The star filter.
    pub fn min_stars(&self) -> u8 {
        self.min_stars
    }

    /// Show library search results, or a person's photos, under `label`.
    pub fn show_results(&mut self, paths: Vec<PathBuf>, label: String) {
        self.results = true;
        self.results_label = label;
        // Stays until the scope is switched back to the folder or the search changes.
        self.library_scope = true;
        self.query_sent = Some(self.search.trim().to_owned());
        self.items = paths
            .into_iter()
            .map(|path| Item { name: path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), ext: extension_of(&path), date: None, path, thumb: Thumb::None })
            .collect();
        self.selected.clear();
        self.arrange();
    }

    /// Folders of the photos showing (for reading their sidecars).
    pub fn result_dirs(&self) -> Vec<PathBuf> {
        if !self.results {
            return Vec::new();
        }
        let mut dirs: Vec<PathBuf> = self.items.iter().filter_map(|i| i.path.parent().map(Path::to_path_buf)).collect();
        dirs.sort();
        dirs.dedup();
        dirs
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
        while let Ok((job_generation, index, version, image)) = self.done.try_recv() {
            if let Some(key) = version {
                let thumb = match image {
                    Some(image) => Thumb::Ready(ctx.load_texture(format!("version {key}"), image, egui::TextureOptions::LINEAR)),
                    None => Thumb::Failed,
                };
                self.version_thumbs.insert(key, thumb);
                continue;
            }
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

    /// Use a freshly rendered thumbnail for a version.
    pub fn set_version_thumb(&mut self, ctx: &egui::Context, key: &str, image: egui::ColorImage) {
        let texture = ctx.load_texture(format!("version {key}"), image, egui::TextureOptions::LINEAR);
        self.version_thumbs.insert(key.to_owned(), Thumb::Ready(texture));
    }

    /// True when a version's thumbnail has been rendered (or is being read from disk).
    pub fn has_version_thumb(&self, key: &str) -> bool {
        matches!(self.version_thumbs.get(key), Some(Thumb::Ready(_)) | Some(Thumb::Pending))
    }

    /// Load a photo's thumbnail again (after its edit was reset).
    pub fn reload(&mut self, photo: &Path) {
        if let Some(index) = self.position(Some(photo)) {
            self.items[index].thumb = Thumb::None;
        }
    }

    fn position(&self, current: Option<&Path>) -> Option<usize> {
        // Library results come from many folders, where file names repeat.
        if self.results {
            let current = current?;
            return self.items.iter().position(|item| item.path == current);
        }
        let name = current?.file_name()?;
        self.items.iter().position(|item| item.path.file_name() == Some(name))
    }

    fn passes(&self, index: usize, store: &Store) -> bool {
        let path = &self.items[index].path;
        // Library results already match the search.
        (self.min_stars == 0 || store.rating(path) >= self.min_stars) && (self.results || self.library_scope || matches_search(&self.search, path, store))
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
            if self.min_stars > 0 || !self.search.trim().is_empty() {
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
                if !self.search.is_empty() && ui::button(ui, "×", egui::vec2(22.0, 22.0), Look::ghost(), theme::font(13.0), None).on_hover_text("Clear the search").clicked() {
                    self.search.clear();
                }
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search keywords, people, file names").desired_width(190.0))
                    .on_hover_text("Shows the photos whose keywords, people or file name contain every word you type");
                let scope = if self.library_scope { "Library" } else { "This folder" };
                if ui::button(ui, scope, egui::vec2(0.0, 22.0), if self.library_scope { Look::teal() } else { Look::bordered() }, theme::font(11.5), None)
                    .on_hover_text("Search this folder, or every folder in the library (add folders to the library in the left panel)")
                    .clicked()
                {
                    self.library_scope = !self.library_scope;
                    self.query_sent = None;
                }
                // Whatever room is left goes to the path, cut at the left edge of the controls.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let (look, tip) = if self.grid {
                        (Look::teal(), "Back to the photo, with the filmstrip under it (or double-click a thumbnail)")
                    } else {
                        (Look::bordered(), "Show the thumbnails full screen, in a grid")
                    };
                    if ui::button(ui, "Grid", egui::vec2(0.0, 22.0), look, theme::font(11.5), Some(Icon::Grid)).on_hover_text(tip).clicked() {
                        let on = !self.grid;
                        self.set_grid(on);
                    }
                    let text = match self.results {
                        true => format!("{}  •  {place}", self.results_label),
                        false => format!("{}  •  {place}", dir.display()),
                    };
                    ui.add(egui::Label::new(egui::RichText::new(text).font(theme::font(12.0)).color(theme::TEXT_HEADER)).truncate(true));
                });
            });
        });
        ui::hairline(ui);
        ui.add_space(8.0);

        let mut action = None;
        let generation = self.generation.load(Ordering::Relaxed);
        let selection_paths = self.selected_paths();
        if self.grid {
            // Grid: rows of thumbnails filling the window, the size slider at the bottom right.
            let room = egui::vec2(ui.available_width(), (ui.available_height() - FOOTER_HEIGHT).max(0.0));
            ui.allocate_ui(room, |ui| {
                let mut area = egui::ScrollArea::vertical().id_source("grid").auto_shrink([false, false]);
                if let Some(y) = self.scroll_goal.take() {
                    area = area.vertical_scroll_offset(y);
                }
                let out = area.show(ui, |ui| {
                    let origin = ui.max_rect().min;
                    // The slider sets about how big the thumbnails are; each row then stretches
                    // them a little so it runs from the left edge to the right edge. With fewer
                    // photos than fit in a row they keep the slider's size.
                    const GAP: f32 = 10.0;
                    let wish = self.grid_card.clamp(GRID_MIN, GRID_MAX);
                    let width = ui.available_width();
                    let columns = (((width + GAP) / (wish + GAP)).floor() as usize).max(1);
                    let card = if shown.len() >= columns {
                        // A hair under the exact size, so rounding never pushes the last one
                        // onto the next row.
                        ((width - GAP * (columns - 1) as f32) / columns as f32 - 0.05).max(GRID_MIN.min(width))
                    } else {
                        wish
                    };
                    let d = Draw { current_index, scroll_to_current, generation, selection_paths: &selection_paths, shown: &shown, can_paste, card, grid: true };
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(GAP, 12.0);
                        if shown.is_empty() {
                            ui.label(ui::dim("No photos match the filter.", 12.5));
                        }
                        for &index in &shown {
                            if index >= self.items.len() {
                                break; // the folder was just read again
                            }
                            self.draw_group(ui, index, &d, store, &mut action);
                        }
                    });
                    origin
                });
                if let Some(rect) = self.scroll_want.take() {
                    self.scroll_goal = Some((rect.center().y - out.inner.y - out.inner_rect.height() * 0.5).max(0.0));
                    ui.ctx().request_repaint();
                }
            });
            ui.horizontal(|ui| {
                ui.set_height(FOOTER_HEIGHT);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().slider_width = 160.0;
                    ui.add(egui::Slider::new(&mut self.grid_card, GRID_MIN..=GRID_MAX).show_value(false)).on_hover_text("Thumbnail size");
                    ui.label(egui::RichText::new("Size").font(theme::font(11.5)).color(theme::FADED));
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.label(ui::dim("Double-click a photo to open it", 11.5));
                    });
                });
            });
            return action;
        }
        // Strip: one row, as tall as the panel allows.
        let card = (ui.available_height() - LABEL_HEIGHT - 8.0).clamp(48.0, 400.0);
        let d = Draw { current_index, scroll_to_current, generation, selection_paths: &selection_paths, shown: &shown, can_paste, card, grid: false };
        // The mouse wheel scrolls the strip sideways.
        ui.style_mut().always_scroll_the_only_direction = true;
        let mut area = egui::ScrollArea::horizontal().id_source("strip").auto_shrink([false, false]);
        if let Some(x) = self.scroll_goal.take() {
            area = area.horizontal_scroll_offset(x);
        }
        let out = area.show(ui, |ui| {
            let origin = ui.max_rect().min;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if shown.is_empty() {
                    ui.label(ui::dim("No photos match the filter.", 12.5));
                }
                for &index in &shown {
                    if index >= self.items.len() {
                        break; // the folder was just read again
                    }
                    self.draw_group(ui, index, &d, store, &mut action);
                }
            });
            origin
        });
        if let Some(rect) = self.scroll_want.take() {
            self.scroll_goal = Some((rect.center().x - out.inner.x - out.inner_rect.width() * 0.5).max(0.0));
            ui.ctx().request_repaint();
        }
        action
    }

    /// One photo's card, and its versions' cards beside it, kept together on one row.
    fn draw_group(&mut self, ui: &mut egui::Ui, index: usize, d: &Draw, store: &Store, action: &mut Option<Action>) {
        let versions = if d.grid { 0 } else { store.versions(&self.items[index].path).len() };
        let vw = d.card * VERSION_CARD / CARD;
        let width = d.card + versions as f32 * (GROUP_GAP + vw);
        ui.allocate_ui_with_layout(egui::vec2(width, d.card + LABEL_HEIGHT), egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            self.draw_cards(ui, index, d, store, action, vw);
        });
    }

    fn draw_cards(&mut self, ui: &mut egui::Ui, index: usize, d: &Draw, store: &Store, action: &mut Option<Action>, vw: f32) {
        use crate::theme;
        let versions: Vec<crate::settings::Version> = store.versions(&self.items[index].path).to_vec();
        // A photo with versions and its version cards share one frame, drawn
        // behind them once their positions are known.
        let group_back = (!versions.is_empty() && !d.grid).then(|| ui.painter().add(egui::Shape::Noop));
        let item = &mut self.items[index];
        let (rect, response) = ui.allocate_exact_size(egui::vec2(d.card, d.card + LABEL_HEIGHT), egui::Sense::click());
        let card = egui::Rect::from_min_size(rect.min, egui::Vec2::splat(d.card));
        let is_current = d.current_index == Some(index);
        let is_selected = self.selected.contains(&index);
        let is_edited = store.contains(&item.path);
        if is_current && d.scroll_to_current {
            self.scroll_want = Some(rect);
        }
        if ui.is_rect_visible(rect) {
            if matches!(item.thumb, Thumb::None) {
                let cache = if is_edited { crate::settings::thumb_file(&item.path) } else { None };
                let job = Job { generation: d.generation, index, path: item.path.clone(), cache, version: None };
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
            let badge_color = if item.ext != "dng" && is_raw(&item.path) { theme::ACCENT } else { theme::TEAL };
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
            if !versions.is_empty() {
                // Versions mark, bottom right: two stacked frames and the count.
                let galley = painter.layout_no_wrap(format!("{}", versions.len() + 1), theme::bold(9.5), theme::TEAL);
                let size = egui::vec2(galley.size().x + 20.0, galley.size().y + 4.0);
                let back = egui::Rect::from_min_size(card.right_bottom() - size - egui::vec2(5.0, 5.0), size);
                painter.rect(back, 3.0, egui::Color32::from_rgba_unmultiplied(0x11, 0x13, 0x17, 0xE0), egui::Stroke::new(1.0_f32, theme::tint(theme::TEAL, 0.5)));
                let icon = egui::Rect::from_min_size(back.min + egui::vec2(4.0, 3.5), egui::vec2(8.0, 6.0));
                painter.rect_stroke(icon.translate(egui::vec2(2.5, -1.5)), 1.0, egui::Stroke::new(1.0_f32, theme::tint(theme::TEAL, 0.6)));
                painter.rect(icon, 1.0, egui::Color32::from_rgb(0x11, 0x13, 0x17), egui::Stroke::new(1.0_f32, theme::TEAL));
                painter.galley(egui::pos2(back.min.x + 15.0, back.min.y + 2.0), galley, theme::TEAL);
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
            job.wrap = egui::text::TextWrapping { max_width: d.card, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            let galley = ui.fonts(|f| f.layout_job(job));
            painter.galley(egui::pos2(card.center().x - galley.size().x * 0.5, card.bottom() + 4.0), galley, color);
        }

        // Click opens; Ctrl-click adds or removes from the selection; Shift-click
        // selects everything from the open photo to here.
        if response.clicked() {
            let modifiers = ui.input(|i| i.modifiers);
            if modifiers.command {
                if self.selected.is_empty() {
                    self.selected.extend(d.current_index);
                }
                if !self.selected.remove(&index) {
                    self.selected.insert(index);
                }
            } else if modifiers.shift {
                let anchor = d.current_index.or_else(|| self.selected.iter().next().copied()).unwrap_or(index);
                let (from, to) = (anchor.min(index), anchor.max(index));
                self.selected = d.shown.iter().copied().filter(|i| (from..=to).contains(i)).collect();
            } else {
                self.selected = std::iter::once(index).collect();
                if !is_current {
                    *action = Some(Action::Open(item.path.clone()));
                }
            }
        }
        if d.grid && response.double_clicked() {
            if !is_current {
                *action = Some(Action::Open(item.path.clone()));
            }
            self.set_grid(false);
        }
        let path = &self.items[index].path;
        let hover = match versions.len() {
            0 => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            n => format!("{}  ·  the current edit, and {n} saved version{}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), if n == 1 { "" } else { "s" }),
        };
        response.on_hover_text(hover).context_menu(|ui| {
            let targets = if is_selected && d.selection_paths.len() > 1 { d.selection_paths.to_vec() } else { vec![path.clone()] };
            if let Some(picked) = edit_menu(ui, path, &targets, is_current || is_edited, d.can_paste) {
                *action = Some(picked);
            }
        });

        // The photo's saved versions, as smaller cards linked to it in one frame.
        if let Some(back) = group_back {
            let photo = self.items[index].path.clone();
            let main_thumb = match &self.items[index].thumb {
                Thumb::Ready(t) => Some(t.clone()),
                _ => None,
            };
            let mut group = rect;
            for (vi, version) in versions.iter().enumerate() {
                ui.add_space(GROUP_GAP - ui.spacing().item_spacing.x);
                let (vrect, vresponse) = ui.allocate_exact_size(egui::vec2(vw, d.card + LABEL_HEIGHT), egui::Sense::click());
                group = group.union(vrect);
                if !ui.is_rect_visible(vrect) {
                    continue;
                }
                let key = crate::settings::version_key(&photo, version);
                if !self.version_thumbs.contains_key(&key) {
                    // Read a thumbnail rendered earlier; there is none until the
                    // photo has been opened once since the version was saved.
                    let file = crate::settings::version_thumb_file(&key).filter(|f| f.is_file());
                    let state = match file {
                        Some(file) => {
                            let job = Job { generation: d.generation, index, path: photo.clone(), cache: Some(file), version: Some(key.clone()) };
                            if self.jobs.send(job).is_ok() { Thumb::Pending } else { Thumb::Failed }
                        }
                        None => Thumb::Failed,
                    };
                    self.version_thumbs.insert(key.clone(), state);
                }
                let texture = match self.version_thumbs.get(&key) {
                    Some(Thumb::Ready(t)) => Some((t.clone(), true)),
                    _ => main_thumb.clone().map(|t| (t, false)),
                };
                let painter = ui.painter().with_clip_rect(vrect.expand(2.0).intersect(ui.clip_rect()));
                let vcard = egui::Rect::from_min_size(vrect.min + egui::vec2(0.0, 6.0), egui::vec2(vw, d.card - 12.0));
                let (fill, border) = if vresponse.hovered() {
                    (egui::Color32::from_rgb(0x17, 0x1A, 0x20), egui::Stroke::new(1.0_f32, theme::tint(theme::TEAL, 0.8)))
                } else {
                    (theme::WELL, egui::Stroke::new(1.0_f32, theme::tint(theme::TEAL, 0.35)))
                };
                painter.rect(vcard, 5.0, fill, border);
                let inner = vcard.shrink(5.0);
                if let Some((texture, rendered)) = texture {
                    let size = texture.size_vec2();
                    let scale = (inner.width() / size.x).min(inner.height() / size.y);
                    let picture = egui::Rect::from_center_size(inner.center(), size * scale);
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    // Not rendered yet: the photo's own thumbnail, dimmed.
                    let tint = if rendered { egui::Color32::WHITE } else { egui::Color32::from_gray(110) };
                    painter.image(texture.id(), picture, uv, tint);
                }
                // "V1", "V2"... top left, in the versions' colour.
                let label = painter.layout_no_wrap(format!("V{}", vi + 1), theme::bold(9.0), theme::TEAL);
                let badge = egui::Rect::from_min_size(vcard.min + egui::vec2(4.0, 4.0), label.size() + egui::vec2(8.0, 3.0));
                painter.rect(badge, 3.0, egui::Color32::from_rgba_unmultiplied(0x11, 0x13, 0x17, 0xE0), egui::Stroke::new(1.0_f32, theme::tint(theme::TEAL, 0.45)));
                painter.galley(badge.min + egui::vec2(4.0, 1.5), label, theme::TEAL);
                // The version's name under it.
                let mut job = egui::text::LayoutJob::single_section(version.name.clone(), egui::TextFormat { font_id: theme::font(10.5), color: theme::tint(theme::TEAL, 0.9), ..Default::default() });
                job.wrap = egui::text::TextWrapping { max_width: vw, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                let galley = ui.fonts(|f| f.layout_job(job));
                painter.galley(egui::pos2(vcard.center().x - galley.size().x * 0.5, vrect.min.y + d.card + 4.0), galley, theme::TEAL);
                if vresponse.clicked() {
                    *action = Some(Action::OpenVersion(photo.clone(), vi));
                }
                vresponse.on_hover_text(format!("{}  ·  version {} of {}: {}\nClick to open the photo at this version", photo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), vi + 1, versions.len(), version.name)).context_menu(|ui| {
                    if ui.button("Open this version").clicked() {
                        *action = Some(Action::OpenVersion(photo.clone(), vi));
                        ui.close_menu();
                    }
                    if ui.button("Delete this version").clicked() {
                        *action = Some(Action::DeleteVersion(photo.clone(), vi));
                        ui.close_menu();
                    }
                });
            }
            // The shared frame: a tinted panel behind the photo and its versions,
            // with a line along the bottom joining them.
            let frame = group.expand2(egui::vec2(3.0, 2.0));
            ui.painter().set(back, egui::Shape::rect_filled(frame, 8.0, egui::Color32::from_rgba_unmultiplied(0x2A, 0x9D, 0x8F, 18)));
            let y = rect.min.y + d.card + 1.5;
            ui.painter().line_segment([egui::pos2(rect.center().x, y), egui::pos2(group.max.x - vw * 0.5, y)], egui::Stroke::new(1.5_f32, theme::tint(theme::TEAL, 0.5)));
        }
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
    /// Export these photos, each with its own settings.
    Export(Vec<PathBuf>),
    /// Convert these raw photos to DNG; `true` also embeds the original raw file.
    ToDng(Vec<PathBuf>, bool),
    /// Open this photo at its saved version number n (0-based).
    OpenVersion(PathBuf, usize),
    /// Delete this photo's saved version number n.
    DeleteVersion(PathBuf, usize),
}

/// Whether every word of `search` appears (ignoring case) in the photo's file name, one of its
/// keywords or one of its people's names. An empty search matches everything.
pub fn matches_search(search: &str, path: &Path, store: &Store) -> bool {
    let words: Vec<String> = search.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let tags: Vec<String> = store.keywords(path).iter().chain(store.people(path)).map(|w| w.to_lowercase()).collect();
    words.iter().all(|w| name.contains(w.as_str()) || tags.iter().any(|t| t.contains(w.as_str())))
}

pub fn stars(count: u8) -> String {
    "★".repeat(count as usize)
}

/// The right-click menu shared by the filmstrip and the main picture.
pub fn edit_menu(ui: &mut egui::Ui, photo: &Path, targets: &[PathBuf], can_copy: bool, can_paste: bool) -> Option<Action> {
    let mut action = None;
    let export = if targets.len() > 1 { format!("Export {} photos…", targets.len()) } else { "Export…".to_owned() };
    if ui.button(export).clicked() {
        action = Some(Action::Export(targets.to_vec()));
        ui.close_menu();
    }
    let any_raw = targets.iter().any(|p| is_raw(p) && extension_of(p) != "dng");
    let to_dng = if targets.len() > 1 { format!("Convert {} photos to DNG…", targets.len()) } else { "Convert to DNG…".to_owned() };
    if ui
        .add_enabled(any_raw, egui::Button::new(to_dng))
        .on_hover_text("The original raw data in Adobe's open raw format, losslessly compressed. Develop's edits come along in a .dvlp file beside it.")
        .on_disabled_hover_text("Only camera raw files can be converted")
        .clicked()
    {
        action = Some(Action::ToDng(targets.to_vec(), false));
        ui.close_menu();
    }
    if ui
        .add_enabled(any_raw, egui::Button::new("Convert to DNG, embedding the original…"))
        .on_hover_text("As Convert to DNG, with the original raw file stored inside the DNG so it can be extracted again (larger files)")
        .on_disabled_hover_text("Only camera raw files can be converted")
        .clicked()
    {
        action = Some(Action::ToDng(targets.to_vec(), true));
        ui.close_menu();
    }
    ui.separator();
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
/// Finished pictures the app opens, lower case (raw formats come from the decoder: see
/// `raw_extensions`).
pub const PICTURE_EXTENSIONS: [&str; 7] = ["jpg", "jpeg", "png", "tif", "tiff", "heic", "heif"];

/// Every camera raw format the decoder (rawler) reads, lower case, plus DNG.
pub fn raw_extensions() -> Vec<String> {
    let mut list: Vec<String> = rawler::decoders::supported_extensions().iter().map(|e| e.to_lowercase()).collect();
    if !list.iter().any(|e| e == "dng") {
        list.push("dng".to_owned());
    }
    list
}

/// Everything the app opens: raw formats and finished pictures, lower case.
pub fn all_extensions() -> Vec<String> {
    let mut list = raw_extensions();
    list.extend(PICTURE_EXTENSIONS.iter().map(|e| e.to_string()));
    list
}

pub fn extension_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn is_supported(path: &Path) -> bool {
    is_raw(path) || PICTURE_EXTENSIONS.contains(&extension_of(path).as_str())
}

/// True for the camera raw formats (as opposed to finished pictures such as JPEG).
pub fn is_raw(path: &Path) -> bool {
    let ext = extension_of(path);
    ext == "dng" || rawler::decoders::supported_extensions().iter().any(|e| e.eq_ignore_ascii_case(&ext))
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

/// What a TIFF-structured file (ARW, NEF, DNG, TIFF, or the EXIF block of a JPEG) says about
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
    /// Fujifilm RAF RawExposureBias, in stops: how far the camera underexposed the raw
    /// (about -0.72 normally, a stop more in DR200 and so on).
    pub raw_exposure_bias: Option<f32>,
    /// DNG LinearizationTable: stored value -> linear value.
    pub linearization: Option<Vec<u16>>,
    /// Camera and lens, for lens-profile matching.
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens_make: Option<String>,
    pub lens_model: Option<String>,
    pub focal: Option<f32>,
    pub fnumber: Option<f32>,
    /// The camera's embedded lens corrections (tags 0x7037, 0x7035, 0x7032), each starting with
    /// its count.
    pub embedded_distortion: Vec<i32>,
    pub embedded_ca: Vec<i32>,
    pub embedded_vignetting: Vec<i32>,
}

/// Walk the directories of a TIFF structure that starts at `base` in `r`.
fn scan_tiff<R: Read + Seek>(r: &mut R, base: u64) -> Option<TiffInfo> {
    scan_tiff_as(r, base, false)
}

/// `scan_tiff`, treating the first directory as an EXIF directory when `root_is_exif` (Canon
/// CR3 keeps the EXIF directory in a TIFF structure of its own).
fn scan_tiff_as<R: Read + Seek>(r: &mut R, base: u64, root_is_exif: bool) -> Option<TiffInfo> {
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
    // 42 for TIFF; Panasonic RW2 uses 0x55 and Olympus ORF "RO" / "RS" with the same layout.
    if !matches!(u16_at(&head[2..]), 42 | 0x55 | 0x4F52 | 0x5352) {
        return None;
    }
    let mut info = TiffInfo { orientation: 1, ..Default::default() };
    // Directories still to read: (offset, is the EXIF directory).
    let mut queue = vec![(u32_at(&head[4..]) as u64, root_is_exif)];
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
                // Panasonic RW2: the embedded full JPEG preview (JpgFromRaw).
                0x002E if !is_exif && n > 1024 => info.jpegs.push((value, n as u64)),
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
                            0x7037 => info.embedded_distortion = values,
                            0x7035 => info.embedded_ca = values,
                            _ => info.embedded_vignetting = values,
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

/// Fill whatever `info` lacks from `other` (a second source of EXIF for the same photo).
fn fill_missing(info: &mut TiffInfo, other: TiffInfo) {
    info.date = info.date.take().or(other.date);
    info.make = info.make.take().or(other.make);
    info.model = info.model.take().or(other.model);
    info.lens_make = info.lens_make.take().or(other.lens_make);
    info.lens_model = info.lens_model.take().or(other.lens_model);
    info.focal = info.focal.or(other.focal);
    info.fnumber = info.fnumber.or(other.fnumber);
    if info.orientation == 1 {
        info.orientation = other.orientation;
    }
}

/// The EXIF of a JPEG held in memory (its offsets are relative to the EXIF block).
fn exif_of_jpeg(bytes: &[u8]) -> Option<TiffInfo> {
    let at = bytes.windows(6).take(256 * 1024).position(|w| w == b"Exif\0\0")? + 6;
    let mut info = scan_tiff(&mut std::io::Cursor::new(bytes), at as u64)?;
    info.jpegs.clear();
    Some(info)
}

/// Read `len` bytes of `file` at `offset`.
fn read_range(file: &mut File, offset: u64, len: usize) -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; len];
    file.seek(SeekFrom::Start(offset)).ok()?;
    file.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

/// Fujifilm RAF: a fixed header points to an embedded JPEG, which carries the EXIF.
fn raf_info(file: &mut File) -> Option<TiffInfo> {
    let head = read_range(file, 0, 100)?;
    if !head.starts_with(b"FUJIFILMCCD-RAW") {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]) as u64;
    let (offset, len) = (be(84), be(88));
    if len == 0 || len > (64 << 20) {
        return None;
    }
    let jpeg = read_range(file, offset, len as usize)?;
    let mut info = exif_of_jpeg(&jpeg).unwrap_or(TiffInfo { orientation: 1, ..Default::default() });
    info.jpegs = vec![(offset, len)];
    // The raw header directory: entries of (tag, size, data), big-endian.
    let (dir_offset, dir_len) = (be(92), be(96));
    if dir_len >= 4 && dir_len < (1 << 20) {
        if let Some(dir) = read_range(file, dir_offset, dir_len as usize) {
            let be16 = |i: usize| u16::from_be_bytes([dir[i], dir[i + 1]]);
            let count = u32::from_be_bytes([dir[0], dir[1], dir[2], dir[3]]);
            let mut p = 4usize;
            for _ in 0..count.min(512) {
                if p + 4 > dir.len() {
                    break;
                }
                let (tag, size) = (be16(p), be16(p + 2) as usize);
                if tag == 0x9650 && size >= 4 && p + 8 <= dir.len() {
                    let (num, den) = (be16(p + 4) as i16 as f32, be16(p + 6) as f32);
                    if den > 0.0 {
                        info.raw_exposure_bias = Some(num / den);
                    }
                }
                p += 4 + size;
            }
        }
    }
    Some(info)
}

/// Canon CR3 (an ISO media file): the CMT1 box holds IFD0 (camera, orientation), CMT2 the
/// EXIF directory (date, lens, focal length), and the PRVW box a preview JPEG.
fn cr3_info(file: &mut File) -> Option<TiffInfo> {
    let mut head = vec![0u8; 4 << 20];
    file.seek(SeekFrom::Start(0)).ok()?;
    let n = file.read(&mut head).ok()?;
    head.truncate(n);
    if head.get(4..8) != Some(b"ftyp") {
        return None;
    }
    let find = |tag: &[u8]| head.windows(4).position(|w| w == tag).filter(|&p| p >= 4);
    let box_end = |p: usize| p - 4 + u32::from_be_bytes([head[p - 4], head[p - 3], head[p - 2], head[p - 1]]) as usize;
    let mut cursor = std::io::Cursor::new(&head);
    let mut info = find(b"CMT1").and_then(|p| scan_tiff_as(&mut cursor, (p + 4) as u64, false)).unwrap_or(TiffInfo { orientation: 1, ..Default::default() });
    info.jpegs.clear();
    if let Some(exif) = find(b"CMT2").and_then(|p| scan_tiff_as(&mut cursor, (p + 4) as u64, true)) {
        fill_missing(&mut info, exif);
    }
    if let Some(p) = find(b"PRVW") {
        let end = box_end(p).min(head.len());
        if let Some(soi) = head[p..end.min(p + 64)].windows(2).position(|w| w == [0xFF, 0xD8]) {
            let start = p + soi;
            info.jpegs.push((start as u64, (end - start) as u64));
        }
    }
    Some(info)
}

/// Read a file's TIFF / EXIF information: directly for ARW, NEF, CR2, RW2, DNG and TIFF,
/// from the embedded JPEG for RAF, from the metadata boxes for CR3, and from the EXIF block
/// for JPEG. `None` for files without any (PNG, or a JPEG with no EXIF).
pub fn tiff_info(path: &Path) -> Option<TiffInfo> {
    let mut file = File::open(path).ok()?;
    match extension_of(path).as_str() {
        "raf" => raf_info(&mut file),
        "cr3" => cr3_info(&mut file),
        "rw2" => {
            // Some of the EXIF (the lens, for one) is only in the embedded JPEG preview.
            let mut info = scan_tiff(&mut file, 0)?;
            if info.lens_model.is_none() {
                let largest = info.jpegs.iter().copied().max_by_key(|&(_, len)| len);
                if let Some(exif) = largest.and_then(|(o, l)| read_range(&mut file, o, l as usize)).and_then(|j| exif_of_jpeg(&j)) {
                    fill_missing(&mut info, exif);
                }
            }
            Some(info)
        }
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

/// When the picture was taken, for people to read ("18 Nov 2026, 21:42"), from the EXIF
/// capture date; None when the file doesn't record one.
pub fn taken_text(path: &Path) -> Option<String> {
    let date = tiff_info(path).and_then(|info| info.date).filter(|d| d.len() >= 16 && !d.starts_with("0000"))?;
    let num = |r: std::ops::Range<usize>| date.get(r).and_then(|s| s.parse::<usize>().ok());
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let name = months.get(month.checked_sub(1)?)?;
    Some(format!("{day} {name} {year}, {}", date.get(11..16)?))
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
    // Formats without a TIFF-style preview (Minolta MRW, Sigma X3F, old Canon CRW...): ask
    // the decoder for the preview it knows how to find.
    decoder_preview(path).map(finish)
}

/// A picture of the photo for analysis (faces, content), long side at most `max_side`, upright:
/// the largest preview inside a raw file, or the picture itself. Fast: no raw decoding.
pub fn preview(path: &Path, max_side: u32) -> Option<image::RgbImage> {
    let info = tiff_info(path);
    let orientation = info.as_ref().map_or(1, |i| i.orientation);
    let finish = |decoded: image::DynamicImage| upright(decoded.thumbnail(max_side, max_side), orientation).to_rgb8();
    if !is_raw(path) {
        return crate::heic::open_picture(path).ok().map(finish);
    }
    let mut file = File::open(path).ok()?;
    let mut jpegs = info?.jpegs;
    jpegs.sort_by_key(|&(_, len)| std::cmp::Reverse(len));
    for (offset, len) in jpegs {
        let mut bytes = vec![0u8; len as usize];
        if file.seek(SeekFrom::Start(offset)).is_err() || file.read_exact(&mut bytes).is_err() {
            continue;
        }
        if let Ok(decoded) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg) {
            return Some(finish(decoded));
        }
    }
    decoder_preview(path).map(finish)
}

/// The embedded preview of a raw file, as the decoder finds it.
fn decoder_preview(path: &Path) -> Option<image::DynamicImage> {
    let source = rawler::rawsource::RawSource::new(path).ok()?;
    let decoder = rawler::get_decoder(&source).ok()?;
    let params = rawler::decoders::RawDecodeParams::default();
    decoder
        .preview_image(&source, &params)
        .ok()
        .flatten()
        .or_else(|| decoder.thumbnail_image(&source, &params).ok().flatten())
}
