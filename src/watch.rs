//! Watching the open folder for sidecars changed by another program: an AI assistant editing
//! through the MCP server, or another copy of Develop.
//!
//! The system tells Develop when a file in the folder changes (on Windows through
//! `ReadDirectoryChangesW`), so nothing is checked while nothing changes. A save often arrives
//! as several notices in a row, so a sidecar is only read again once its notices have stopped
//! for `SETTLE`. When the system reports that too much changed at once to list, the whole
//! folder is read again. Shared folders on another computer can miss notices, so there the
//! folder is checked every few seconds instead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;
use notify::Watcher;

/// How long a sidecar's notices must have stopped before it is read.
const SETTLE: Duration = Duration::from_millis(250);
/// How often a shared folder on another computer is checked.
const NETWORK_CHECK: Duration = Duration::from_secs(3);

enum Notice {
    Changed(PathBuf),
    /// Too much changed to list: read the whole folder again.
    Everything,
}

pub struct FolderWatch {
    dir: PathBuf,
    /// Kept so the watching goes on; dropping it stops it.
    _watcher: Box<dyn Watcher + Send>,
    notices: Receiver<Notice>,
    /// Photos whose sidecar changed, with the time of the latest notice.
    waiting: HashMap<PathBuf, Instant>,
    everything: Option<Instant>,
}

/// What has settled since the last call: photos whose sidecar changed, and whether the whole
/// folder should be read again.
pub struct Settled {
    pub photos: Vec<PathBuf>,
    pub everything: bool,
}

fn is_sidecar(path: &Path) -> bool {
    path.extension().is_some_and(|x| x.eq_ignore_ascii_case(crate::settings::SIDECAR_EXTENSION))
}

impl FolderWatch {
    /// Start watching `dir` (not its subfolders). None when the folder cannot be watched.
    pub fn new(dir: &Path, ctx: &egui::Context) -> Option<Self> {
        let (tx, notices) = mpsc::channel();
        let ctx = ctx.clone();
        let handler = move |result: notify::Result<notify::Event>| {
            let mut wake = false;
            match result {
                Ok(event) => {
                    if event.need_rescan() {
                        wake |= tx.send(Notice::Everything).is_ok();
                    }
                    for path in event.paths.into_iter().filter(|p| is_sidecar(p)) {
                        wake |= tx.send(Notice::Changed(path)).is_ok();
                    }
                }
                Err(_) => wake = tx.send(Notice::Everything).is_ok(),
            }
            // Only sidecar changes wake the app (not the photos or anything else in the folder).
            if wake {
                ctx.request_repaint_after(SETTLE + Duration::from_millis(20));
            }
        };
        let network = dir.to_string_lossy().starts_with(r"\\");
        let mut watcher: Box<dyn Watcher + Send> = match network {
            false => match notify::recommended_watcher(handler.clone()) {
                Ok(w) => Box::new(w),
                Err(_) => Box::new(notify::PollWatcher::new(handler, notify::Config::default().with_poll_interval(NETWORK_CHECK)).ok()?),
            },
            true => Box::new(notify::PollWatcher::new(handler, notify::Config::default().with_poll_interval(NETWORK_CHECK)).ok()?),
        };
        watcher.watch(dir, notify::RecursiveMode::NonRecursive).ok()?;
        Some(Self { dir: dir.to_path_buf(), _watcher: watcher, notices, waiting: HashMap::new(), everything: None })
    }

    /// The folder being watched.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Changes whose notices have stopped for `SETTLE`. Asks for another look when some are
    /// still settling.
    pub fn settled(&mut self, ctx: &egui::Context) -> Settled {
        let now = Instant::now();
        while let Ok(notice) = self.notices.try_recv() {
            match notice {
                Notice::Changed(path) => {
                    // The photo is the sidecar's name without `.dvlp`, in the folder as Develop
                    // names it (the system may spell the folder differently).
                    if let Some(name) = path.file_name() {
                        let photo = self.dir.join(name).with_extension("");
                        self.waiting.insert(photo, now);
                    }
                }
                Notice::Everything => self.everything = Some(now),
            }
        }
        let mut photos = Vec::new();
        self.waiting.retain(|photo, at| {
            let done = now.duration_since(*at) >= SETTLE;
            if done {
                photos.push(photo.clone());
            }
            !done
        });
        let everything = self.everything.is_some_and(|at| now.duration_since(at) >= SETTLE);
        if everything {
            self.everything = None;
        }
        if !self.waiting.is_empty() || self.everything.is_some() {
            ctx.request_repaint_after(SETTLE);
        }
        Settled { photos, everything }
    }
}
