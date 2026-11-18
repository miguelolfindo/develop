//! Folder tree on the left of the window. Clicking a folder shows its photos in the
//! filmstrip. Folders are listed only when their parent is opened.
//!
//! Besides the drives (and on macOS the mounted volumes) there is a "Network" entry: the
//! computers on the network, the folders each one shares, and the folders inside those.
//! Anything on the network is read in the background so a slow or sleeping computer never
//! freezes the window.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use eframe::egui;

use crate::network::{self, Entry};
use crate::settings::NetworkPlaces;
use crate::theme;
use crate::ui::{self, Icon};

const ROW: f32 = 22.0;
const INDENT: f32 = 12.0;

/// What a background read produced.
struct Loaded {
    entries: Result<Vec<Entry>, String>,
    /// macOS: where a share was mounted.
    mounted: Option<PathBuf>,
}

pub struct Explorer {
    /// The library's folders (set by the app before drawing), and a folder the user asked to
    /// add to or take out of the library with the right-click menu (taken by the app).
    pub library: Vec<PathBuf>,
    pub library_change: Option<(PathBuf, bool)>,
    /// A folder the user asked to move (Move to…), taken by the app.
    pub move_request: Option<PathBuf>,
    roots: Vec<PathBuf>,
    /// Sub-entries of each entry opened so far.
    children: HashMap<PathBuf, Vec<PathBuf>>,
    /// Names to show instead of the last path component (drives, computers, shares).
    labels: HashMap<PathBuf, String>,
    /// Entries shown expanded (see `key`).
    open: HashSet<String>,
    /// The selection the tree was last opened up to.
    revealed: Option<PathBuf>,
    /// Network reads still running, and the ones that failed.
    pending: HashMap<PathBuf, Receiver<Loaded>>,
    errors: HashMap<PathBuf, String>,
    /// macOS shares mounted so far: `smb://host/share` -> folder under /Volumes.
    mounted: HashMap<PathBuf, PathBuf>,
    /// A share clicked before it was mounted: select it once it is.
    click_when_mounted: Option<PathBuf>,
    places: NetworkPlaces,
    /// The "add a computer or share" field.
    address: String,
}

/// Comparison key for paths: case-insensitive on Windows and macOS, without a trailing
/// separator, so `\\OLFINDO\docker\` and `\\Olfindo\docker` are the same entry.
fn key(path: &Path) -> String {
    let text = path.to_string_lossy();
    let trimmed = text.trim_end_matches(['\\', '/']);
    let trimmed = if trimmed.is_empty() || trimmed.ends_with(':') { text.as_ref() } else { trimmed };
    if cfg!(any(windows, target_os = "macos")) {
        trimmed.to_lowercase()
    } else {
        trimmed.to_owned()
    }
}

/// Top-level entries, with display names for the ones that need them.
fn roots() -> (Vec<PathBuf>, Vec<(PathBuf, String)>) {
    let mut roots = Vec::new();
    let mut labels = Vec::new();
    if cfg!(windows) {
        for letter in 'A'..='Z' {
            let drive = PathBuf::from(format!("{letter}:\\"));
            if drive.exists() {
                if let Some(share) = network::drive_share(&drive) {
                    labels.push((drive.clone(), format!("{letter}:  {share}")));
                }
                roots.push(drive);
            }
        }
    } else {
        roots.push(PathBuf::from("/"));
        roots.extend(std::env::var_os("HOME").map(PathBuf::from));
        let volumes = PathBuf::from("/Volumes");
        if cfg!(target_os = "macos") && volumes.is_dir() {
            labels.push((volumes.clone(), "Volumes".to_owned()));
            roots.push(volumes);
        }
    }
    labels.push((network::root(), "Network".to_owned()));
    roots.push(network::root());
    (roots, labels)
}

/// Folders nobody browses photos in: hidden and system ones, and a few by name.
fn is_clutter(entry: &std::fs::DirEntry, name: &str) -> bool {
    const NAMES: [&str; 6] = ["Config.Msi", "msdownld.tmp", "WindowsApps", "System Volume Information", "Recovery", "PerfLogs"];
    // NAS system folders too: Synology and QNAP keep "#recycle", "#snapshot", "@eaDir"...
    if name.starts_with('.') || name.starts_with('$') || name.starts_with('#') || name.starts_with('@') || NAMES.iter().any(|n| n.eq_ignore_ascii_case(name)) {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const HIDDEN: u32 = 0x2;
        const SYSTEM: u32 = 0x4;
        if entry.metadata().is_ok_and(|m| m.file_attributes() & (HIDDEN | SYSTEM) != 0) {
            return true;
        }
    }
    let _ = entry;
    false
}

/// Visible sub-folders of `dir`, sorted by name.
fn subfolders(dir: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|entry| !entry.file_name().to_str().map_or(true, |name| is_clutter(entry, name)))
        .map(|entry| entry.path())
        .collect();
    folders.sort_by(|a, b| {
        let name = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        crate::filmstrip::natural_cmp(&name(a), &name(b))
    });
    folders
}

fn as_entries(folders: Vec<PathBuf>) -> Vec<Entry> {
    folders
        .into_iter()
        .map(|path| Entry { label: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string()), path })
        .collect()
}

/// The background read for a network entry.
fn load_remote(path: PathBuf, saved: Vec<String>) -> Loaded {
    if network::is_root(&path) {
        return Loaded { entries: Ok(network::computers(&saved)), mounted: None };
    }
    if let Some(host) = network::host_of_entry(&path) {
        return Loaded { entries: network::shares(&host, &saved), mounted: None };
    }
    if network::is_unmounted_share(&path) {
        return match network::mount(&path) {
            Ok(point) => Loaded { entries: Ok(as_entries(subfolders(&point))), mounted: Some(point) },
            Err(e) => Loaded { entries: Err(e), mounted: None },
        };
    }
    network::ensure_access(&path);
    if std::fs::read_dir(&path).is_err() {
        return Loaded { entries: Err("Could not open this folder (no access, or the computer is not answering).".into()), mounted: None };
    }
    Loaded { entries: Ok(as_entries(subfolders(&path))), mounted: None }
}

impl Explorer {
    pub fn new() -> Self {
        let (roots, labels) = roots();
        Self {
            library: Vec::new(),
            library_change: None,
            move_request: None,
            roots,
            children: HashMap::new(),
            labels: labels.into_iter().collect(),
            open: HashSet::new(),
            revealed: None,
            pending: HashMap::new(),
            errors: HashMap::new(),
            mounted: HashMap::new(),
            click_when_mounted: None,
            places: NetworkPlaces::load(),
            address: String::new(),
        }
    }

    fn is_open(&self, path: &Path) -> bool {
        self.open.contains(&key(path))
    }

    fn set_open(&mut self, path: &Path, open: bool) {
        if open {
            self.open.insert(key(path));
        } else {
            self.open.remove(&key(path));
        }
    }

    /// Read `path` again next time it is shown.
    /// Read `path`'s subfolders again (after a folder was moved in or out of it).
    pub fn refresh(&mut self, path: &Path) {
        self.forget(path);
    }

    fn forget(&mut self, path: &Path) {
        self.children.remove(path);
        self.errors.remove(path);
    }

    /// Start a background read of `path` unless one is running.
    fn start_load(&mut self, path: &Path) {
        if self.pending.contains_key(path) {
            return;
        }
        self.errors.remove(path);
        let (tx, rx) = mpsc::channel();
        let (job, saved) = (path.to_path_buf(), self.places.list.clone());
        std::thread::spawn(move || {
            let _ = tx.send(load_remote(job, saved));
        });
        self.pending.insert(path.to_path_buf(), rx);
    }

    /// Take in finished background reads. Returns a share that was clicked before macOS had
    /// mounted it, once it has been.
    fn poll(&mut self, ctx: &egui::Context) -> Option<PathBuf> {
        let mut clicked = None;
        let done: Vec<(PathBuf, Loaded)> = self
            .pending
            .iter()
            .filter_map(|(path, rx)| match rx.try_recv() {
                Ok(loaded) => Some((path.clone(), loaded)),
                Err(mpsc::TryRecvError::Disconnected) => Some((path.clone(), Loaded { entries: Err("The lookup stopped.".into()), mounted: None })),
                Err(mpsc::TryRecvError::Empty) => None,
            })
            .collect();
        for (path, loaded) in done {
            self.pending.remove(&path);
            if let Some(point) = loaded.mounted {
                if self.click_when_mounted.as_deref() == Some(path.as_path()) {
                    self.click_when_mounted = None;
                    clicked = Some(point.clone());
                }
                self.mounted.insert(path.clone(), point);
            }
            match loaded.entries {
                Ok(entries) => {
                    let paths = entries.iter().map(|e| e.path.clone()).collect();
                    for e in entries {
                        self.labels.insert(e.path, e.label);
                    }
                    self.children.insert(path, paths);
                }
                Err(message) => {
                    if self.click_when_mounted.as_deref() == Some(path.as_path()) {
                        self.click_when_mounted = None;
                    }
                    self.errors.insert(path, message);
                }
            }
        }
        if !self.pending.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
        clicked
    }

    /// A photo opened from `\\HOST\share\...`: remember the computer and open the tree to it.
    fn reveal_network(&mut self, selected: &Path) {
        let Some(host) = network::host_of(selected) else { return };
        let text = selected.to_string_lossy();
        if !(text.starts_with("\\\\") || text.starts_with("smb://")) {
            return;
        }
        if self.places.add(&format!("\\\\{host}")) {
            self.forget(&network::root());
        }
        self.set_open(&network::root(), true);
        self.set_open(&network::host_path(&host), true);
    }

    /// Draw the tree with `selected` highlighted. Returns the folder the user clicked.
    pub fn show(&mut self, ui: &mut egui::Ui, selected: Option<&Path>) -> Option<PathBuf> {
        let mut clicked = self.poll(ui.ctx());
        ui.horizontal(|ui| {
            ui.set_height(24.0);
            let galley = ui.painter().layout_no_wrap("FOLDERS".to_owned(), theme::bold(11.5), theme::TEXT_HEADER);
            let (rect, _) = ui.allocate_exact_size(galley.size() + egui::vec2(4.0, 0.0), egui::Sense::hover());
            ui.painter().galley(rect.min + egui::vec2(4.0, 0.0), galley, theme::TEXT_HEADER);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui::icon_button(ui, Icon::Refresh, 22.0, ui::Look::ghost()).on_hover_text("Read the drives, network and folders again").clicked() {
                    let (roots, labels) = roots();
                    self.roots = roots;
                    self.labels.extend(labels);
                    self.children.clear();
                    self.errors.clear();
                }
            });
        });
        ui::hairline(ui);
        ui.add_space(4.0);

        // When the selection changes (a folder clicked, a file opened elsewhere, or the last
        // session restored), open the tree down to it and scroll it into view.
        let reveal = selected != self.revealed.as_deref();
        if reveal {
            if let Some(selected) = selected {
                for ancestor in selected.ancestors().skip(1) {
                    self.open.insert(key(ancestor));
                }
                self.reveal_network(selected);
            }
            self.revealed = selected.map(Path::to_path_buf);
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for root in self.roots.clone() {
                self.node(ui, &root, 0, selected, reveal, &mut clicked);
            }
        });
        clicked
    }

    /// One-line message under an entry (searching, an error), with an optional button.
    fn note(ui: &mut egui::Ui, depth: usize, text: &str, color: egui::Color32, button: Option<&str>) -> bool {
        let mut pressed = false;
        ui.horizontal(|ui| {
            ui.set_height(ROW);
            ui.add_space(4.0 + depth as f32 * INDENT + 20.0);
            // The button first (from the right), then the message in what is left, cut with
            // "…": a long message must never widen the panel.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(label) = button {
                    pressed = ui.small_button(label).clicked();
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(text).font(theme::font(11.5)).color(color)).truncate(true)).on_hover_text(text);
                });
            });
        });
        pressed
    }

    /// The field for adding a computer or share by address, shown under "Network".
    fn address_row(&mut self, ui: &mut egui::Ui, depth: usize) {
        let hint = if cfg!(windows) { "\\\\computer or \\\\computer\\share" } else { "computer or smb://computer/share" };
        ui.horizontal(|ui| {
            ui.set_height(ROW + 2.0);
            ui.add_space(4.0 + depth as f32 * INDENT + 20.0);
            // The button first (from the right), then the box in exactly the room left, so the
            // row never comes out wider than the panel (which would widen the panel, again and
            // again).
            let (add, entered) = ui
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let add = ui.small_button("Add").on_hover_text("Add this computer or shared folder to Network").clicked();
                    let size = egui::vec2(ui.available_width().max(40.0), ROW);
                    let field = ui.add_sized(size, egui::TextEdit::singleline(&mut self.address).hint_text(hint));
                    (add, field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                })
                .inner;
            if (add || entered) && !self.address.trim().is_empty() {
                if let Some((host, share)) = network::parse_address(&self.address) {
                    let address = match (&share, cfg!(windows)) {
                        (Some(s), true) => format!("\\\\{host}\\{s}"),
                        (None, true) => format!("\\\\{host}"),
                        (Some(s), false) => format!("smb://{host}/{s}"),
                        (None, false) => format!("smb://{host}"),
                    };
                    self.places.add(&address);
                    let computer = network::host_path(&host);
                    self.forget(&network::root());
                    self.forget(&computer);
                    self.set_open(&computer, true);
                    self.address.clear();
                }
            }
        });
    }

    fn node(&mut self, ui: &mut egui::Ui, path: &Path, depth: usize, selected: Option<&Path>, reveal: bool, clicked: &mut Option<PathBuf>) {
        let label = match self.labels.get(path) {
            Some(label) => label.clone(),
            None => match path.file_name() {
                Some(name) => name.to_string_lossy().into_owned(),
                None => path.display().to_string(), // a drive
            },
        };
        let is_network = network::is_root(path);
        let computer = network::host_of_entry(path);
        // Entries that are places to open, not folders to show: Network and the computers.
        let container_only = is_network || computer.is_some();
        let real = self.mounted.get(path).cloned();
        let is_selected = selected.is_some_and(|s| key(s) == key(path) || real.as_deref().is_some_and(|r| key(r) == key(s)));
        let is_open = self.is_open(path);

        // One full-width row: chevron, folder icon, name.
        let (row, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::click());
        let x = row.left() + 4.0 + depth as f32 * INDENT;
        let chevron = egui::Rect::from_center_size(egui::pos2(x + 5.0, row.center().y), egui::Vec2::splat(7.0));
        let on_chevron = response.interact_pointer_pos().is_some_and(|p| p.x < x + 14.0);
        if response.clicked() {
            if on_chevron || container_only {
                self.set_open(path, !is_open);
            } else if network::is_unmounted_share(path) {
                match real.clone() {
                    Some(point) => *clicked = Some(point),
                    None => self.click_when_mounted = Some(path.to_path_buf()),
                }
                self.set_open(path, true);
            } else {
                *clicked = Some(path.to_path_buf());
                self.set_open(path, true);
            }
        }
        if response.double_clicked() && !on_chevron && is_open && !container_only {
            self.set_open(path, false);
        }
        // A real folder (not Network or a computer, nor a share not opened yet) can go into the
        // library.
        let folder = if container_only { None } else if network::is_unmounted_share(path) { real.clone() } else { Some(path.to_path_buf()) };
        let in_library = folder.as_ref().is_some_and(|f| self.library.iter().any(|l| key(l) == key(f)));
        let networked = is_network || computer.is_some() || network::is_remote(path);
        if networked || folder.is_some() {
            let saved_host = computer.as_ref().filter(|h| self.places.list.iter().any(|l| network::parse_address(l).is_some_and(|(x, _)| x.eq_ignore_ascii_case(h))));
            let mut forget = false;
            let mut remove = None;
            let mut change = None;
            response.context_menu(|ui| {
                if let Some(f) = &folder {
                    if f.parent().is_some() && ui.button("Move to…").on_hover_text("Move this folder, with everything in it, somewhere else: every file is copied and checked before the original is removed").clicked() {
                        self.move_request = Some(f.clone());
                        ui.close_menu();
                    }
                    let (text, add) = if in_library { ("Remove from Library", false) } else { ("Add to Library", true) };
                    if ui.button(text).on_hover_text("The library is the folders searched together, with their faces and keywords (Faces & Metadata)").clicked() {
                        change = Some((f.clone(), add));
                        ui.close_menu();
                    }
                }
                if !networked {
                    return;
                }
                if ui.button("Read again").clicked() {
                    forget = true;
                    ui.close_menu();
                }
                if let Some(host) = saved_host {
                    if ui.button("Remove from Network").clicked() {
                        remove = Some(host.clone());
                        ui.close_menu();
                    }
                }
            });
            if change.is_some() {
                self.library_change = change;
            }
            if forget {
                self.forget(path);
            }
            if let Some(host) = remove {
                self.places.remove_host(&host);
                self.forget(&network::root());
            }
        }
        if is_selected && reveal {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        if ui.is_rect_visible(row) {
            let painter = ui.painter();
            let color = if is_selected {
                painter.rect_filled(row, 4.0, theme::tint(theme::ACCENT, 0.14));
                theme::ACCENT
            } else {
                if response.hovered() {
                    painter.rect_filled(row, 4.0, theme::ELEVATED);
                }
                theme::TEXT
            };
            ui::draw_icon(painter, if is_open { Icon::ChevronDown } else { Icon::ChevronRight }, chevron, theme::FADED);
            let icon = egui::Rect::from_center_size(egui::pos2(x + 22.0, row.center().y), egui::vec2(13.0, 12.0));
            ui::draw_icon(painter, Icon::Folder, icon, if is_selected { theme::ACCENT } else if container_only { theme::TEAL } else { theme::TEXT_HEADER });
            // Name, cut with an ellipsis at the edge of the row.
            let mut job = egui::text::LayoutJob::single_section(label, egui::TextFormat { font_id: theme::font(12.5), color, ..Default::default() });
            job.wrap = egui::text::TextWrapping { max_width: (row.right() - 6.0 - (x + 34.0)).max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            let galley = ui.fonts(|f| f.layout_job(job));
            painter.galley(egui::pos2(x + 34.0, row.center().y - galley.size().y * 0.5), galley, color);
        }

        if !self.is_open(path) {
            return;
        }
        if is_network {
            self.address_row(ui, depth + 1);
        }
        let entries = match self.children.get(path) {
            Some(list) => list.clone(),
            None if network::is_remote(path) || container_only => {
                if let Some(message) = self.errors.get(path).cloned() {
                    let action = if computer.is_some() && cfg!(target_os = "macos") { "Connect…" } else { "Retry" };
                    if Self::note(ui, depth + 1, &message, ui.visuals().error_fg_color, Some(action)) {
                        if let (Some(host), true) = (&computer, cfg!(target_os = "macos")) {
                            network::connect_dialog(host);
                        }
                        self.start_load(path);
                    }
                } else {
                    self.start_load(path);
                    let text = if is_network { "Looking for computers…" } else if computer.is_some() { "Asking for its shared folders…" } else { "Reading…" };
                    Self::note(ui, depth + 1, text, theme::FADED, None);
                }
                return;
            }
            None => {
                let folders = subfolders(path);
                self.children.insert(path.to_path_buf(), folders.clone());
                folders
            }
        };
        if entries.is_empty() && container_only {
            let text = if is_network { "No computers found yet. Add one by address above." } else { "No shared folders." };
            Self::note(ui, depth + 1, text, theme::FADED, None);
        }
        for entry in entries {
            self.node(ui, &entry, depth + 1, selected, reveal, clicked);
        }
    }
}
