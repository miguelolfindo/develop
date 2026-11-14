//! Folder tree on the left of the window. Clicking a folder shows its photos in the
//! filmstrip. Folders are listed only when their parent is opened.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::theme;
use crate::ui::{self, Icon};

const ROW: f32 = 22.0;
const INDENT: f32 = 12.0;

pub struct Explorer {
    roots: Vec<PathBuf>,
    /// Sub-folders of each folder opened so far.
    children: HashMap<PathBuf, Vec<PathBuf>>,
    /// Folders shown expanded.
    open: HashSet<PathBuf>,
    /// The selection the tree was last opened up to.
    revealed: Option<PathBuf>,
}

fn roots() -> Vec<PathBuf> {
    if cfg!(windows) {
        ('A'..='Z').map(|letter| PathBuf::from(format!("{letter}:\\"))).filter(|drive| drive.exists()).collect()
    } else {
        let mut roots = vec![PathBuf::from("/")];
        roots.extend(std::env::var_os("HOME").map(PathBuf::from));
        roots
    }
}

/// Folders nobody browses photos in: hidden and system ones, and a few by name.
fn is_clutter(entry: &std::fs::DirEntry, name: &str) -> bool {
    const NAMES: [&str; 6] = ["Config.Msi", "msdownld.tmp", "WindowsApps", "System Volume Information", "Recovery", "PerfLogs"];
    if name.starts_with('.') || name.starts_with('$') || NAMES.iter().any(|n| n.eq_ignore_ascii_case(name)) {
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

impl Explorer {
    pub fn new() -> Self {
        Self { roots: roots(), children: HashMap::new(), open: HashSet::new(), revealed: None }
    }

    /// Draw the tree with `selected` highlighted. Returns the folder the user clicked.
    pub fn show(&mut self, ui: &mut egui::Ui, selected: Option<&Path>) -> Option<PathBuf> {
        ui.horizontal(|ui| {
            ui.set_height(24.0);
            let galley = ui.painter().layout_no_wrap("FOLDERS".to_owned(), theme::bold(11.5), theme::TEXT_HEADER);
            let (rect, _) = ui.allocate_exact_size(galley.size() + egui::vec2(4.0, 0.0), egui::Sense::hover());
            ui.painter().galley(rect.min + egui::vec2(4.0, 0.0), galley, theme::TEXT_HEADER);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui::icon_button(ui, Icon::Refresh, 22.0, ui::Look::ghost()).on_hover_text("Read the drives and folders again").clicked() {
                    self.roots = roots();
                    self.children.clear();
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
                self.open.extend(selected.ancestors().skip(1).map(Path::to_path_buf));
            }
            self.revealed = selected.map(Path::to_path_buf);
        }

        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for root in self.roots.clone() {
                self.node(ui, &root, 0, selected, reveal, &mut clicked);
            }
        });
        clicked
    }

    fn node(&mut self, ui: &mut egui::Ui, path: &Path, depth: usize, selected: Option<&Path>, reveal: bool, clicked: &mut Option<PathBuf>) {
        let label = match path.file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            None => path.display().to_string(), // a drive
        };
        let is_selected = selected == Some(path);
        let is_open = self.open.contains(path);

        // One full-width row: chevron, folder icon, name.
        let (row, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::click());
        let x = row.left() + 4.0 + depth as f32 * INDENT;
        let chevron = egui::Rect::from_center_size(egui::pos2(x + 5.0, row.center().y), egui::Vec2::splat(7.0));
        let on_chevron = response.interact_pointer_pos().is_some_and(|p| p.x < x + 14.0);
        if response.clicked() {
            if on_chevron {
                if !self.open.remove(path) {
                    self.open.insert(path.to_path_buf());
                }
            } else {
                *clicked = Some(path.to_path_buf());
                self.open.insert(path.to_path_buf());
            }
        }
        if response.double_clicked() && !on_chevron && is_open {
            self.open.remove(path);
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
            ui::draw_icon(painter, Icon::Folder, icon, if is_selected { theme::ACCENT } else { theme::TEXT_HEADER });
            // Name, cut with an ellipsis at the edge of the row.
            let mut job = egui::text::LayoutJob::single_section(label, egui::TextFormat { font_id: theme::font(12.5), color, ..Default::default() });
            job.wrap = egui::text::TextWrapping { max_width: (row.right() - 6.0 - (x + 34.0)).max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            let galley = ui.fonts(|f| f.layout_job(job));
            painter.galley(egui::pos2(x + 34.0, row.center().y - galley.size().y * 0.5), galley, color);
        }

        if self.open.contains(path) {
            let folders = self.children.entry(path.to_path_buf()).or_insert_with(|| subfolders(path)).clone();
            for folder in folders {
                self.node(ui, &folder, depth + 1, selected, reveal, clicked);
            }
        }
    }
}
