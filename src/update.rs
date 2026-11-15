//! Update check: on launch, ask GitHub for the latest Develop release; when it is newer than
//! this build, offer to download its installer and install it.
//!
//! - Windows: the setup .exe is downloaded and started, and Develop closes so the installer
//!   can replace it.
//! - macOS: the .dmg is downloaded and mounted, Develop.app is replaced in place, and the new
//!   version starts once this one has quit.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use eframe::egui;

use crate::theme;

/// Where releases are published.
pub const REPOSITORY: &str = "miguelolfindo/develop";

/// This build's version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// The version to compare against: this build's, unless DEVELOP_PRETEND_VERSION is set (for
/// trying the update dialog).
pub fn current() -> String {
    std::env::var("DEVELOP_PRETEND_VERSION").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| CURRENT.to_owned())
}

#[derive(Clone, Debug)]
pub struct Release {
    /// "0.2.2"
    pub version: String,
    /// The release notes (Markdown).
    pub notes: String,
    /// The installer for this system.
    pub asset_name: String,
    pub asset_url: String,
    pub asset_size: u64,
    /// The release page, for when there is no installer for this system.
    pub page: String,
}

/// Download progress, sent from the download thread.
pub enum Progress {
    /// Fraction done, 0..1 (or negative when the size is unknown).
    Downloading(f32),
    Ready(PathBuf),
    Failed(String),
}

/// "v0.2.10" -> [0, 2, 10]
fn parse_version(text: &str) -> Vec<u32> {
    text.trim().trim_start_matches(['v', 'V']).split(['.', '-', '+']).map_while(|p| p.parse::<u32>().ok()).collect()
}

/// True when `candidate` is a later version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (a, b) = (parse_version(candidate), parse_version(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// The installer for this system among a release's files.
fn pick_asset(name: &str) -> bool {
    let name = name.to_lowercase();
    if cfg!(windows) {
        name.ends_with("-setup.exe") && name.contains("x64")
    } else if cfg!(target_os = "macos") {
        name.ends_with(".dmg") && (name.contains("aarch64") || name.contains("arm64"))
    } else {
        false
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(20))).build().into()
}

/// Ask GitHub for the latest release. `Ok(None)` when this build is up to date.
pub fn check() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let mut response = agent()
        .get(&url)
        .header("User-Agent", &format!("Develop/{CURRENT}"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("Could not reach GitHub: {e}"))?;
    let text = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let tag = json["tag_name"].as_str().unwrap_or_default();
    if tag.is_empty() || !is_newer(tag, &current()) || json["draft"].as_bool() == Some(true) || json["prerelease"].as_bool() == Some(true) {
        return Ok(None);
    }
    let asset = json["assets"].as_array().and_then(|assets| assets.iter().find(|a| a["name"].as_str().is_some_and(pick_asset)));
    Ok(Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_owned(),
        notes: json["body"].as_str().unwrap_or_default().replace("\r\n", "\n"),
        asset_name: asset.and_then(|a| a["name"].as_str()).unwrap_or_default().to_owned(),
        asset_url: asset.and_then(|a| a["browser_download_url"].as_str()).unwrap_or_default().to_owned(),
        asset_size: asset.and_then(|a| a["size"].as_u64()).unwrap_or(0),
        page: json["html_url"].as_str().unwrap_or_default().to_owned(),
    }))
}

/// Download the release's installer into a temporary folder, reporting progress.
pub fn download(release: &Release, progress: &Sender<Progress>) {
    let result = (|| -> Result<PathBuf, String> {
        let dir = std::env::temp_dir().join("develop-update");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(&release.asset_name);
        let response = ureq::get(&release.asset_url)
            .header("User-Agent", &format!("Develop/{CURRENT}"))
            .call()
            .map_err(|e| format!("Download failed: {e}"))?;
        let total = response.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(release.asset_size);
        let mut reader = response.into_body().into_reader();
        let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; 1 << 16];
        let (mut done, mut last) = (0u64, 0u64);
        loop {
            let n = reader.read(&mut buffer).map_err(|e| format!("Download failed: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            if done - last > (1 << 20) {
                last = done;
                let _ = progress.send(Progress::Downloading(if total > 0 { done as f32 / total as f32 } else { -1.0 }));
            }
        }
        file.flush().map_err(|e| e.to_string())?;
        if total > 0 && done != total {
            return Err(format!("Download incomplete ({done} of {total} bytes)"));
        }
        Ok(path)
    })();
    let _ = progress.send(match result {
        Ok(path) => Progress::Ready(path),
        Err(e) => Progress::Failed(e),
    });
}

/// Start installing the downloaded file. On success the caller closes Develop.
pub fn install(installer: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::process::Command::new(installer).spawn().map_err(|e| format!("Could not start the installer: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        // The app bundle this copy runs from (…/Develop.app/Contents/MacOS/develop).
        let bundle = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("/Applications/Develop.app"));
        let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "'\\''"));
        let mount = std::env::temp_dir().join("develop-update-mount");
        // Wait for this copy to quit, swap the bundle, clean up and start the new one.
        let script = format!(
            "while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; \
             mkdir -p {mount} && hdiutil attach -nobrowse -readonly -mountpoint {mount} {dmg} >/dev/null && \
             src=$(ls -d {mount}/*.app | head -1) && [ -n \"$src\" ] && \
             rm -rf {bundle} && ditto \"$src\" {bundle}; \
             hdiutil detach {mount} >/dev/null 2>&1; \
             xattr -cr {bundle} 2>/dev/null; open {bundle}",
            pid = std::process::id(),
            mount = quote(&mount),
            dmg = quote(installer),
            bundle = quote(&bundle),
        );
        std::process::Command::new("/bin/sh").arg("-c").arg(script).spawn().map_err(|e| format!("Could not start the update: {e}"))?;
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = installer;
        Err("Automatic updates are not available on this system".into())
    }
}

/// Draw Markdown release notes: headings, bullet and numbered lists, code blocks, links and
/// bold text (shown plainly). Enough for what the release notes use.
pub fn show_notes(ui: &mut egui::Ui, markdown: &str) {
    let plain = |text: &str| -> String {
        // [label](url) -> label; **bold** and `code` markers dropped.
        let mut out = String::new();
        let mut rest = text;
        while let Some(start) = rest.find('[') {
            match (rest[start..].find("]("), rest[start..].find(')')) {
                (Some(mid), Some(end)) if mid < end => {
                    out.push_str(&rest[..start]);
                    out.push_str(&rest[start + 1..start + mid]);
                    rest = &rest[start + end + 1..];
                }
                _ => break,
            }
        }
        out.push_str(rest);
        out.replace("**", "").replace('`', "")
    };
    // A list item being collected (its marker, indent and text) or a plain paragraph.
    let mut item: Option<(String, f32, String)> = None;
    let mut paragraph = String::new();
    let flush = |ui: &mut egui::Ui, item: &mut Option<(String, f32, String)>, paragraph: &mut String| {
        if let Some((mark, indent, text)) = item.take() {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(4.0 + indent * 6.0);
                ui.label(egui::RichText::new(mark).font(theme::font(12.5)).color(theme::FADED));
                ui.label(egui::RichText::new(text).font(theme::font(12.5)).color(theme::TEXT));
            });
        }
        if !paragraph.is_empty() {
            ui.label(egui::RichText::new(paragraph.trim()).font(theme::font(12.5)).color(theme::TEXT));
            paragraph.clear();
        }
    };
    let mut in_code = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            flush(ui, &mut item, &mut paragraph);
            in_code = !in_code;
            continue;
        }
        if in_code {
            ui.label(egui::RichText::new(line).monospace().color(theme::TEXT));
            continue;
        }
        if trimmed.is_empty() {
            flush(ui, &mut item, &mut paragraph);
            ui.add_space(4.0);
            continue;
        }
        let heading = trimmed.chars().take_while(|c| *c == '#').count();
        if heading > 0 && trimmed[heading..].starts_with(' ') {
            flush(ui, &mut item, &mut paragraph);
            let size = match heading {
                1 | 2 => 15.0,
                3 => 13.5,
                _ => 12.5,
            };
            ui.add_space(4.0);
            ui.label(egui::RichText::new(plain(trimmed[heading..].trim())).font(theme::bold(size)).color(theme::TEXT_HEADER));
            continue;
        }
        let indent = (line.len() - line.trim_start().len()) as f32;
        let bullet = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* "));
        let numbered = trimmed.split_once(". ").filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        if let Some(text) = bullet {
            flush(ui, &mut item, &mut paragraph);
            item = Some(("•".to_owned(), indent, plain(text)));
            continue;
        }
        if let Some((n, text)) = numbered {
            flush(ui, &mut item, &mut paragraph);
            item = Some((format!("{n}."), indent, plain(text)));
            continue;
        }
        // A continuation line of the list item or paragraph above.
        match &mut item {
            Some((_, _, text)) => {
                text.push(' ');
                text.push_str(&plain(trimmed));
            }
            None => {
                if !paragraph.is_empty() {
                    paragraph.push(' ');
                }
                paragraph.push_str(&plain(trimmed));
            }
        }
    }
    flush(ui, &mut item, &mut paragraph);
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn versions() {
        assert!(is_newer("v0.2.2", "0.2.1"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(!is_newer("v0.2.1", "0.2.1"));
        assert!(!is_newer("0.2.0", "0.2.1"));
    }
}
