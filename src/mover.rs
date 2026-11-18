//! Moving a folder of photos somewhere else (another drive, the network) without losing anything:
//! every file is copied, read back and checked before the originals are removed, and a move that
//! stops part-way (cancelled, network gone) can simply be run again: files already safely copied
//! are skipped. On the same drive the folder is just renamed.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use eframe::egui;

/// What a move will do, worked out before anything is touched.
#[derive(Clone, Debug)]
pub struct Plan {
    /// The folder being moved, and where it will end up (destination folder + its name).
    pub from: PathBuf,
    pub to: PathBuf,
    /// Every file under `from`, relative to it, with its size.
    pub files: Vec<(PathBuf, u64)>,
    pub bytes: u64,
    pub photos: usize,
    /// Files already at the destination with different content: the move will not start.
    pub clashes: Vec<PathBuf>,
    /// Files already at the destination with the same size (an earlier move that stopped
    /// part-way): checked, then skipped.
    pub already: usize,
    /// Free space at the destination, if known.
    pub free: Option<u64>,
    /// Where a name is taken by a different file, keep both: the moved file (with its `.dvlp`,
    /// JPEG or `.xmp` companions) gets a number, `DSC01234-2.ARW`. Off: such a move does not start.
    pub keep_both: bool,
}

/// Look at what moving `from` into the folder `into` would involve.
pub fn plan(from: &Path, into: &Path) -> Result<Plan, String> {
    let name = from.file_name().ok_or("Choose a folder, not a whole drive.")?;
    let to = into.join(name);
    if same_path(from, into) || into.starts_with(from) || to_lower(into).starts_with(&to_lower(from)) {
        return Err("A folder can't be moved into itself.".into());
    }
    if same_path(from, &to) {
        return Err("The folder is already there.".into());
    }
    let mut files = Vec::new();
    walk(from, from, &mut files).map_err(|e| format!("Could not read {}: {e}", from.display()))?;
    let bytes = files.iter().map(|(_, s)| s).sum();
    let photos = files.iter().filter(|(p, _)| crate::filmstrip::is_supported(p)).count();
    let mut clashes = Vec::new();
    let mut already = 0;
    for (rel, size) in &files {
        if let Ok(meta) = std::fs::metadata(to.join(rel)) {
            if meta.len() == *size {
                already += 1;
            } else {
                clashes.push(rel.clone());
            }
        }
    }
    Ok(Plan { from: from.to_path_buf(), to, files, bytes, photos, clashes, already, free: free_space(into), keep_both: false })
}

fn to_lower(p: &Path) -> String {
    let s = p.display().to_string();
    if cfg!(windows) { s.to_lowercase().replace('/', "\\") } else { s }
}

fn same_path(a: &Path, b: &Path) -> bool {
    to_lower(a).trim_end_matches(['\\', '/']) == to_lower(b).trim_end_matches(['\\', '/'])
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, u64)>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            walk(root, &path, out)?;
        } else if kind.is_file() {
            let rel = path.strip_prefix(root).map(Path::to_path_buf).unwrap_or_default();
            out.push((rel, entry.metadata()?.len()));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn free_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut free = 0u64;
    let ok = unsafe { windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(windows::core::PCWSTR(wide.as_ptr()), Some(&mut free), None, None) };
    ok.is_ok().then_some(free)
}

#[cfg(not(windows))]
fn free_space(dir: &Path) -> Option<u64> {
    // `df -k`: the fourth column of the second line is the space available, in KiB.
    let out = std::process::Command::new("df").arg("-k").arg(dir).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let line = text.lines().nth(1)?;
    line.split_whitespace().nth(3)?.parse::<u64>().ok().map(|k| k * 1024)
}

/// How a move is going.
pub enum Progress {
    /// Bytes done of all, and the file now being copied.
    Copying(u64, u64, String),
    Removing,
    /// Finished: how many files were copied (or were there already), and anything left behind.
    Done(Report),
    /// Stopped: what went wrong. Nothing was removed; the move can be run again.
    Failed(String),
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub renamed: bool,
    pub copied: usize,
    pub skipped: usize,
    /// Files given a number because their name was taken at the destination.
    pub numbered: usize,
    /// Files that appeared in the folder during the move, left where they were.
    pub left: Vec<PathBuf>,
    /// Files deleted by something else before their turn came (nothing to move).
    pub vanished: Vec<PathBuf>,
    /// Files changed by something else while the move ran: copied again before their original
    /// was removed.
    pub recopied: usize,
}

pub struct Job {
    pub plan: Plan,
    pub rx: Receiver<Progress>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    /// Stop after the file being copied; the originals stay.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Start moving, in the background.
pub fn start(plan: Plan, ctx: &egui::Context) -> Job {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let (p, stop, ctx) = (plan.clone(), cancel.clone(), ctx.clone());
    std::thread::spawn(move || {
        let result = run(&p, &stop, &|progress| {
            let _ = tx.send(progress);
            ctx.request_repaint();
        });
        let message = match result {
            Ok(report) => {
                log(&format!("moved {} -> {} ({} files copied, {} already there{})", p.from.display(), p.to.display(), report.copied, report.skipped, if report.renamed { ", renamed" } else { "" }));
                Progress::Done(report)
            }
            Err(e) => {
                log(&format!("stopped {} -> {}: {e}", p.from.display(), p.to.display()));
                Progress::Failed(e)
            }
        };
        let _ = tx.send(message);
        ctx.request_repaint();
    });
    Job { plan, rx, cancel }
}

/// `DSC01234.ARW.dvlp` with number 2: `DSC01234-2.ARW.dvlp` (the number goes before the first
/// dot, so a photo and its companions keep matching names).
fn numbered(name: &str, n: u32) -> String {
    match name.find('.') {
        Some(i) if i > 0 => format!("{}-{n}{}", &name[..i], &name[i..]),
        _ => format!("{name}-{n}"),
    }
}

/// A photo and its companions: the same folder and the same name up to the first dot.
fn group_of(rel: &Path) -> (PathBuf, String) {
    let name = rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let base = name.split('.').next().unwrap_or(&name).to_owned();
    (rel.parent().map(Path::to_path_buf).unwrap_or_default(), base)
}

/// Whether `dst` holds exactly `src`.
fn same_file(src: &Path, dst: &Path) -> Result<bool, String> {
    match (std::fs::metadata(src), std::fs::metadata(dst)) {
        (Ok(a), Ok(b)) if a.len() == b.len() => Ok(checksum(src)? == checksum(dst)?),
        _ => Ok(false),
    }
}

fn run(plan: &Plan, stop: &AtomicBool, say: &dyn Fn(Progress)) -> Result<Report, String> {
    if !plan.clashes.is_empty() && !plan.keep_both {
        return Err("Files with the same names but different content are already at the destination.".into());
    }
    // Same drive, nothing there yet: a rename, instant and all-or-nothing.
    if !plan.to.exists() {
        if let Some(parent) = plan.to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::rename(&plan.from, &plan.to).is_ok() {
            return Ok(Report { renamed: true, copied: plan.files.len(), ..Default::default() });
        }
    }
    // Otherwise copy and check each file. First, which names are taken by different files.
    let mut report = Report::default();
    let mut taken: std::collections::HashSet<(PathBuf, String)> = std::collections::HashSet::new();
    for (rel, _) in &plan.files {
        let dst = plan.to.join(rel);
        if !plan.from.join(rel).exists() {
            continue; // gone already: see below
        }
        if dst.exists() && !same_file(&plan.from.join(rel), &dst)? {
            if !plan.keep_both {
                return Err(format!("{} is already at the destination with different content.", rel.display()));
            }
            taken.insert(group_of(rel));
        }
    }
    // Each such photo, with its companions, gets the first number under which none of them
    // would replace anything (an identical file there, from a move that stopped part-way, is
    // fine: it is kept and skipped).
    let mut number: std::collections::HashMap<(PathBuf, String), u32> = std::collections::HashMap::new();
    for group in &taken {
        let members: Vec<&PathBuf> = plan.files.iter().map(|(r, _)| r).filter(|r| &group_of(r) == group).collect();
        let mut n = 2;
        loop {
            let mut free = true;
            for m in &members {
                let name = numbered(&m.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default(), n);
                let dst = plan.to.join(&group.0).join(name);
                if dst.exists() && !same_file(&plan.from.join(m), &dst)? {
                    free = false;
                    break;
                }
            }
            if free || n > 999 {
                break;
            }
            n += 1;
        }
        number.insert(group.clone(), n);
    }
    let mut done: u64 = 0;
    // Each file copied (or found there), where it went, and how the original looked then.
    let mut copied: Vec<(&PathBuf, PathBuf, Option<(u64, std::time::SystemTime)>)> = Vec::new();
    let look = |p: &Path| std::fs::metadata(p).ok().and_then(|m| Some((m.len(), m.modified().ok()?)));
    for (rel, size) in &plan.files {
        if stop.load(Ordering::Relaxed) {
            return Err("Cancelled. Nothing was removed; run the move again to carry on.".into());
        }
        say(Progress::Copying(done, plan.bytes, rel.display().to_string()));
        let src = plan.from.join(rel);
        if !src.exists() {
            // Deleted by something else since the move was planned: nothing to move.
            report.vanished.push(rel.clone());
            done += size;
            continue;
        }
        let before = look(&src);
        let dst = match number.get(&group_of(rel)) {
            Some(&n) => {
                report.numbered += 1;
                plan.to.join(rel.parent().unwrap_or(Path::new(""))).join(numbered(&rel.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default(), n))
            }
            None => plan.to.join(rel),
        };
        if dst.exists() {
            // There already (checked above to be the same file): keep it.
            report.skipped += 1;
            copied.push((rel, dst, before));
            done += size;
            continue;
        }
        copy_checked(&src, &dst).map_err(|e| format!("{}: {e}", rel.display()))?;
        report.copied += 1;
        copied.push((rel, dst, before));
        done += size;
    }
    // Every file is safely at the destination: remove the originals. One changed since it was
    // copied (an edit saved meanwhile) is copied again first, so the newest arrives.
    say(Progress::Removing);
    for (rel, dst, before) in copied {
        let src = plan.from.join(rel);
        let now = look(&src);
        if now.is_none() {
            continue; // gone meanwhile; its copy is at the destination
        }
        if now != before {
            if copy_checked(&src, &dst).is_err() {
                continue; // keep the original
            }
            report.recopied += 1;
        }
        let _ = std::fs::remove_file(&src);
    }
    remove_empty_dirs(&plan.from);
    if plan.from.exists() {
        let mut left = Vec::new();
        let _ = walk(&plan.from, &plan.from, &mut left);
        report.left = left.into_iter().map(|(p, _)| p).collect();
    }
    Ok(report)
}

/// Copy `src` to `dst` under a temporary name, keep its modified time, read it back and compare
/// checksums, and only then give it its name.
fn copy_checked(src: &Path, dst: &Path) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("could not make the folder: {e}"))?;
    }
    let part = dst.with_file_name(format!("{}.part", dst.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
    let mut input = std::fs::File::open(src).map_err(|e| format!("could not read: {e}"))?;
    let mut output = std::fs::File::create(&part).map_err(|e| format!("could not write: {e}"))?;
    let mut sum = crc32fast::Hasher::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = input.read(&mut buffer).map_err(|e| format!("could not read: {e}"))?;
        if n == 0 {
            break;
        }
        sum.update(&buffer[..n]);
        output.write_all(&buffer[..n]).map_err(|e| format!("could not write: {e}"))?;
    }
    output.sync_all().map_err(|e| format!("could not finish writing: {e}"))?;
    if let Ok(modified) = std::fs::metadata(src).and_then(|m| m.modified()) {
        let _ = output.set_modified(modified);
    }
    drop(output);
    let expected = sum.finalize();
    if checksum(&part)? != expected {
        let _ = std::fs::remove_file(&part);
        return Err("the copy did not match the original (nothing removed)".into());
    }
    std::fs::rename(&part, dst).map_err(|e| format!("could not name the copy: {e}"))
}

fn checksum(path: &Path) -> Result<u32, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let mut sum = crc32fast::Hasher::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buffer).map_err(|e| format!("could not read {}: {e}", path.display()))?;
        if n == 0 {
            return Ok(sum.finalize());
        }
        sum.update(&buffer[..n]);
    }
}

/// Remove `dir` and the folders under it that are empty (deepest first).
fn remove_empty_dirs(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                remove_empty_dirs(&e.path());
            }
        }
    }
    let _ = std::fs::remove_dir(dir);
}

/// Note a move in `moves.log` in the settings folder.
fn log(line: &str) {
    if let Some(dir) = crate::settings::settings_dir() {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("moves.log")) {
            let _ = writeln!(f, "{}\t{line}", crate::store::now());
        }
    }
}

/// `path` with `from` at its start replaced by `to` (ignoring case on Windows), if it is under
/// `from`.
pub fn relocated(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    let (p, f) = (to_lower(path), to_lower(from));
    let f = f.trim_end_matches(['\\', '/']);
    // Under it: the same path, or it followed by a separator (not "2025" matching "20250").
    if !p.starts_with(f) || !(p.len() == f.len() || p[f.len()..].starts_with(['\\', '/'])) {
        return None;
    }
    let rest = &path.display().to_string()[f.len()..];
    let rest = rest.trim_start_matches(['\\', '/']);
    Some(if rest.is_empty() { to.to_path_buf() } else { to.join(rest) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("develop-mover-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_folder_moves_with_its_files_and_subfolders() {
        let base = scratch("move");
        let from = base.join("2025");
        std::fs::create_dir_all(from.join("09")).unwrap();
        std::fs::write(from.join("09").join("a.ARW"), vec![7u8; 300_000]).unwrap();
        std::fs::write(from.join("09").join("a.ARW.dvlp"), "rating\t4\n").unwrap();
        std::fs::write(from.join("note.txt"), "x").unwrap();
        let into = base.join("nas");
        std::fs::create_dir_all(&into).unwrap();
        let p = plan(&from, &into).unwrap();
        assert_eq!(p.files.len(), 3);
        assert_eq!(p.photos, 1);
        let report = run(&p, &AtomicBool::new(false), &|_| {}).unwrap();
        assert!(report.left.is_empty());
        assert!(!from.exists(), "the original is gone");
        assert_eq!(std::fs::read(into.join("2025").join("09").join("a.ARW")).unwrap().len(), 300_000);
        assert_eq!(std::fs::read_to_string(into.join("2025").join("09").join("a.ARW.dvlp")).unwrap(), "rating\t4\n");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn copying_checks_and_a_rerun_skips_what_is_there() {
        let base = scratch("copy");
        let from = base.join("2024");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("b.ARW"), vec![3u8; 50_000]).unwrap();
        let dst = base.join("nas").join("2024").join("b.ARW");
        copy_checked(&from.join("b.ARW"), &dst).unwrap();
        assert_eq!(checksum(&dst).unwrap(), checksum(&from.join("b.ARW")).unwrap());
        // A second copy of the folder finds it there and skips it.
        let p = plan(&from, &base.join("nas")).unwrap();
        assert_eq!(p.already, 1);
        assert!(p.clashes.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_copy_path_copies_checks_then_removes() {
        // The destination folder exists already, so it can't be a rename: every file is copied.
        let base = scratch("copypath");
        let from = base.join("2022");
        std::fs::create_dir_all(from.join("a").join("b")).unwrap();
        for (i, rel) in ["x.ARW", "a/y.ARW", "a/b/z.JPG", "a/b/z.JPG.dvlp"].iter().enumerate() {
            std::fs::write(from.join(rel), vec![i as u8; 70_000 + i]).unwrap();
        }
        let to = base.join("nas").join("2022");
        std::fs::create_dir_all(&to).unwrap();
        let p = plan(&from, &base.join("nas")).unwrap();
        let report = run(&p, &AtomicBool::new(false), &|_| {}).unwrap();
        assert!(!report.renamed);
        assert_eq!(report.copied, 4);
        assert!(!from.exists(), "originals removed after checking");
        assert_eq!(std::fs::read(to.join("a").join("b").join("z.JPG.dvlp")).unwrap(), vec![3u8; 70_003]);
        assert!(std::fs::read_dir(&to).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".part")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn keep_both_numbers_a_photo_and_its_companions() {
        let base = scratch("keepboth");
        let from = base.join("2020");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("DSC1.ARW"), vec![1u8; 500]).unwrap();
        std::fs::write(from.join("DSC1.ARW.dvlp"), "rating\t5\n").unwrap();
        std::fs::write(from.join("DSC2.ARW"), vec![2u8; 500]).unwrap();
        let to = base.join("nas").join("2020");
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(to.join("DSC1.ARW"), vec![9u8; 600]).unwrap(); // someone else's DSC1
        std::fs::write(to.join("DSC1-2.ARW"), vec![8u8; 600]).unwrap(); // and a -2 too
        let mut p = plan(&from, &base.join("nas")).unwrap();
        assert_eq!(p.clashes.len(), 1);
        assert!(run(&p, &AtomicBool::new(false), &|_| {}).is_err(), "off: refuses");
        p.keep_both = true;
        let report = run(&p, &AtomicBool::new(false), &|_| {}).unwrap();
        assert_eq!(report.numbered, 2, "the photo and its .dvlp");
        assert_eq!(std::fs::read(to.join("DSC1-3.ARW")).unwrap(), vec![1u8; 500]);
        assert_eq!(std::fs::read_to_string(to.join("DSC1-3.ARW.dvlp")).unwrap(), "rating\t5\n");
        assert_eq!(std::fs::read(to.join("DSC1.ARW")).unwrap(), vec![9u8; 600], "theirs untouched");
        assert_eq!(std::fs::read(to.join("DSC2.ARW")).unwrap(), vec![2u8; 500]);
        assert!(!from.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_file_deleted_before_its_turn_is_skipped() {
        let base = scratch("vanish");
        let from = base.join("2019");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("a.JPG"), vec![1u8; 100]).unwrap();
        std::fs::write(from.join("a.JPG.dvlp"), "x").unwrap();
        std::fs::create_dir_all(base.join("nas").join("2019")).unwrap();
        let p = plan(&from, &base.join("nas")).unwrap();
        std::fs::remove_file(from.join("a.JPG.dvlp")).unwrap(); // deleted after planning
        let report = run(&p, &AtomicBool::new(false), &|_| {}).unwrap();
        assert_eq!(report.vanished, vec![PathBuf::from("a.JPG.dvlp")]);
        assert_eq!(std::fs::read(base.join("nas").join("2019").join("a.JPG")).unwrap().len(), 100);
        assert!(!from.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cancelling_removes_nothing() {
        let base = scratch("cancel");
        let from = base.join("2021");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("a.ARW"), vec![1u8; 1000]).unwrap();
        std::fs::create_dir_all(base.join("nas").join("2021")).unwrap();
        let p = plan(&from, &base.join("nas")).unwrap();
        assert!(run(&p, &AtomicBool::new(true), &|_| {}).is_err());
        assert!(from.join("a.ARW").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn different_files_at_the_destination_stop_the_move() {
        let base = scratch("clash");
        let from = base.join("2023");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("c.ARW"), vec![1u8; 10]).unwrap();
        std::fs::create_dir_all(base.join("nas").join("2023")).unwrap();
        std::fs::write(base.join("nas").join("2023").join("c.ARW"), vec![2u8; 20]).unwrap();
        let p = plan(&from, &base.join("nas")).unwrap();
        assert_eq!(p.clashes.len(), 1);
        assert!(run(&p, &AtomicBool::new(false), &|_| {}).is_err());
        assert!(from.join("c.ARW").exists(), "nothing removed");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn paths_follow_a_move() {
        let from = Path::new(r"O:\Photo WIP\2025");
        let to = Path::new(r"\\olfindo\Photos\2025");
        let moved = relocated(Path::new(r"O:\Photo WIP\2025\09\a.ARW"), from, to).unwrap();
        assert_eq!(moved, PathBuf::from(r"\\olfindo\Photos\2025").join(r"09\a.ARW"));
        assert!(relocated(Path::new(r"O:\Photo WIP\2024\a.ARW"), from, to).is_none());
        assert!(relocated(Path::new(r"O:\Photo WIP\20250\a.ARW"), from, to).is_none());
    }
}
