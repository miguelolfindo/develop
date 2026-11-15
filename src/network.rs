//! Network locations for the folder tree: the computers on the network, the folders they
//! share, and (Windows) the share behind each mapped drive letter.
//!
//! Paths used in the tree:
//! - `::network`: the "Network" entry itself.
//! - A computer: `\\HOST` on Windows, `smb://host` on macOS.
//! - A share: `\\HOST\share\` on Windows, a real folder that `std::fs` reads directly. On
//!   macOS, `smb://host/share` until macOS has mounted it, then its folder under `/Volumes`.
//!
//! Everything here can take seconds on a slow network, so the folder tree calls it from a
//! background thread.

use std::path::{Path, PathBuf};
use std::time::Duration;

const ROOT: &str = "::network";

/// One entry for the tree.
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub label: String,
}

/// The "Network" entry.
pub fn root() -> PathBuf {
    PathBuf::from(ROOT)
}

pub fn is_root(path: &Path) -> bool {
    path.as_os_str() == ROOT
}

/// The computer entry for `host`.
pub fn host_path(host: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!("\\\\{host}"))
    } else {
        PathBuf::from(format!("smb://{host}"))
    }
}

/// The host of a computer entry (`\\HOST` or `smb://host`), if `path` is one.
pub fn host_of_entry(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    let rest = text.strip_prefix("\\\\").or_else(|| text.strip_prefix("smb://"))?;
    let rest = rest.trim_end_matches(['\\', '/']);
    (!rest.is_empty() && !rest.contains(['\\', '/'])).then(|| rest.to_owned())
}

/// The host part of any network path (`\\HOST\share\dir`, `smb://host/share`).
pub fn host_of(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    let rest = text.strip_prefix("\\\\").or_else(|| text.strip_prefix("smb://"))?;
    let host = rest.split(['\\', '/']).next().unwrap_or("");
    (!host.is_empty()).then(|| host.to_owned())
}

/// macOS share that has not been mounted yet (`smb://host/share`).
pub fn is_unmounted_share(path: &Path) -> bool {
    path.to_str().and_then(|t| t.strip_prefix("smb://")).is_some_and(|rest| rest.trim_end_matches('/').contains('/'))
}

/// Paths whose folders should be read in the background: anything on the network.
pub fn is_remote(path: &Path) -> bool {
    let Some(text) = path.to_str() else { return false };
    text == ROOT || text.starts_with("\\\\") || text.starts_with("smb://") || (cfg!(target_os = "macos") && text.starts_with("/Volumes/"))
}

/// Turn what someone typed ("Olfindo", "\\Olfindo\photos", "smb://nas/photos",
/// "192.168.1.20") into a host and an optional share.
pub fn parse_address(text: &str) -> Option<(String, Option<String>)> {
    let text = text.trim();
    let text = text.strip_prefix("smb://").or_else(|| text.strip_prefix("\\\\")).or_else(|| text.strip_prefix("//")).unwrap_or(text);
    let mut parts = text.split(['\\', '/']).filter(|p| !p.is_empty());
    let host = parts.next()?.to_owned();
    if host.contains(char::is_whitespace) {
        return None;
    }
    Some((host, parts.next().map(str::to_owned)))
}

/// A share's entry path from host and share name.
pub fn share_path(host: &str, share: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!("\\\\{host}\\{share}\\"))
    } else {
        PathBuf::from(format!("smb://{host}/{share}"))
    }
}

fn sort_unique(mut entries: Vec<Entry>) -> Vec<Entry> {
    entries.sort_by(|a, b| crate::filmstrip::natural_cmp(&a.label, &b.label));
    entries.dedup_by(|a, b| a.path.to_string_lossy().eq_ignore_ascii_case(&b.path.to_string_lossy()));
    entries
}

/// Run a command for at most `limit`, returning what it printed (also when it had to be
/// stopped: `dns-sd -B` never ends on its own).
#[allow(dead_code)]
fn run_for(program: &str, args: &[&str], limit: Duration) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    reader.join().ok()
}

// ---------------------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------------------

#[cfg(windows)]
mod platform {
    use super::{host_of, host_path, share_path, sort_unique, Entry};
    use std::path::{Path, PathBuf};
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::WNet::{
        WNetAddConnection2W, WNetCloseEnum, WNetEnumResourceW, WNetGetConnectionW, WNetOpenEnumW, CONNECT_INTERACTIVE, CONNECT_PROMPT, NETRESOURCEW,
        NET_RESOURCE_SCOPE, RESOURCETYPE_ANY, RESOURCETYPE_DISK, RESOURCEUSAGE_CONTAINER, RESOURCE_CONTEXT, RESOURCE_GLOBALNET, WNET_OPEN_ENUM_USAGE,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn text_of(p: PWSTR) -> String {
        if p.is_null() {
            String::new()
        } else {
            unsafe { p.to_string() }.unwrap_or_default()
        }
    }

    /// The network resources inside `container` (the whole neighbourhood when `None`):
    /// remote name and display type (2 = computer, 3 = share).
    fn enumerate(scope: NET_RESOURCE_SCOPE, container: Option<&str>) -> windows::core::Result<Vec<(String, u32)>> {
        let mut name = container.map(wide);
        let resource = NETRESOURCEW {
            dwScope: RESOURCE_GLOBALNET,
            dwType: RESOURCETYPE_DISK,
            dwUsage: RESOURCEUSAGE_CONTAINER.0,
            lpRemoteName: name.as_mut().map_or(PWSTR::null(), |n| PWSTR(n.as_mut_ptr())),
            ..Default::default()
        };
        let mut handle = HANDLE::default();
        unsafe { WNetOpenEnumW(scope, RESOURCETYPE_DISK, WNET_OPEN_ENUM_USAGE(0), container.map(|_| &resource as *const NETRESOURCEW), &mut handle)? };
        let mut found = Vec::new();
        let mut buffer = vec![0u64; 4096]; // 32 KB, 8-byte aligned for NETRESOURCEW
        let result = loop {
            let mut count = u32::MAX;
            let mut size = (buffer.len() * 8) as u32;
            match unsafe { WNetEnumResourceW(handle, &mut count, buffer.as_mut_ptr().cast(), &mut size) } {
                Ok(()) => {
                    let items = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const NETRESOURCEW, count as usize) };
                    found.extend(items.iter().map(|r| (text_of(r.lpRemoteName), r.dwDisplayType)));
                }
                // ERROR_NO_MORE_ITEMS (259): done.
                Err(e) if (e.code().0 as u32 & 0xFFFF) == 259 => break Ok(()),
                Err(e) => break Err(e),
            }
        };
        unsafe {
            let _ = WNetCloseEnum(handle);
        }
        result.map(|_| found)
    }

    /// Ask Windows to connect to `remote`, showing its sign-in dialog when needed.
    fn connect(remote: &str, any_type: bool) -> bool {
        let mut name = wide(remote);
        let resource = NETRESOURCEW {
            dwType: if any_type { RESOURCETYPE_ANY } else { RESOURCETYPE_DISK },
            lpRemoteName: PWSTR(name.as_mut_ptr()),
            ..Default::default()
        };
        unsafe { WNetAddConnection2W(&resource, PCWSTR::null(), PCWSTR::null(), CONNECT_INTERACTIVE.0 | CONNECT_PROMPT.0) }.is_ok()
    }

    /// The share behind a mapped drive letter (`W:\` -> `\\Olfindo\docker`).
    pub fn drive_share(drive: &Path) -> Option<String> {
        let letter = drive.to_str()?.get(..2)?;
        let local = wide(letter);
        let mut buffer = vec![0u16; 1024];
        let mut length = buffer.len() as u32;
        unsafe { WNetGetConnectionW(PCWSTR(local.as_ptr()), PWSTR(buffer.as_mut_ptr()), &mut length) }.ok()?;
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..end])).filter(|s| s.starts_with("\\\\"))
    }

    pub fn computers(saved: &[String]) -> Vec<Entry> {
        let mut hosts: Vec<String> = saved.iter().filter_map(|s| super::parse_address(s).map(|(h, _)| h)).collect();
        // Servers behind mapped drives.
        for letter in 'A'..='Z' {
            if let Some(host) = drive_share(&PathBuf::from(format!("{letter}:\\"))).and_then(|s| host_of(Path::new(&s))) {
                hosts.push(host);
            }
        }
        // What Windows network discovery lists for this workgroup or domain.
        if let Ok(found) = enumerate(RESOURCE_CONTEXT, None) {
            hosts.extend(found.into_iter().filter(|(_, kind)| *kind == 2).filter_map(|(name, _)| host_of(Path::new(&name))));
        }
        sort_unique(hosts.into_iter().map(|h| Entry { path: host_path(&h), label: h.clone() }).collect())
    }

    pub fn shares(host: &str, saved: &[String]) -> Result<Vec<Entry>, String> {
        let container = format!("\\\\{host}");
        let listed = enumerate(RESOURCE_GLOBALNET, Some(&container)).or_else(|first| {
            // Often only a sign-in is missing: let Windows ask, then try again.
            if connect(&format!("{container}\\IPC$"), true) {
                enumerate(RESOURCE_GLOBALNET, Some(&container))
            } else {
                Err(first)
            }
        });
        let mut entries: Vec<Entry> = saved
            .iter()
            .filter_map(|s| super::parse_address(s))
            .filter(|(h, share)| h.eq_ignore_ascii_case(host) && share.is_some())
            .map(|(h, share)| {
                let share = share.unwrap_or_default();
                Entry { path: share_path(&h, &share), label: share }
            })
            .collect();
        match listed {
            Ok(found) => {
                for (name, _) in found {
                    let share = name.rsplit('\\').next().unwrap_or("").to_owned();
                    if !share.is_empty() && !share.ends_with('$') {
                        entries.push(Entry { path: share_path(host, &share), label: share });
                    }
                }
                Ok(sort_unique(entries))
            }
            Err(_) if !entries.is_empty() => Ok(sort_unique(entries)),
            Err(e) => Err(format!("Could not list the shared folders: {}", e.message())),
        }
    }

    /// Read access to a share or a folder in it, signing in first when Windows needs it.
    pub fn ensure_access(path: &Path) {
        if std::fs::read_dir(path).is_err() {
            if let Some(text) = path.to_str() {
                let share: Vec<&str> = text.trim_start_matches('\\').split('\\').take(2).collect();
                if share.len() == 2 {
                    connect(&format!("\\\\{}\\{}", share[0], share[1]), false);
                }
            }
        }
    }

    pub fn mount(_share: &Path) -> Result<PathBuf, String> {
        Err("Not needed on Windows".into())
    }
}

// ---------------------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod platform {
    use super::{host_path, run_for, share_path, sort_unique, Entry};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    pub fn drive_share(_drive: &Path) -> Option<String> {
        None
    }

    /// SMB shares macOS has mounted: (host, share, mount point).
    fn mounted() -> Vec<(String, String, PathBuf)> {
        let text = run_for("/sbin/mount", &["-t", "smbfs"], Duration::from_secs(3)).unwrap_or_default();
        text.lines()
            .filter_map(|line| {
                // //user@host/share on /Volumes/share (smbfs, ...)
                let (source, rest) = line.split_once(" on ")?;
                let point = rest.rsplit_once(" (")?.0;
                let source = source.trim_start_matches('/');
                let source = source.rsplit_once('@').map_or(source, |(_, s)| s);
                let (host, share) = source.split_once('/')?;
                let share = share.replace("%20", " ");
                Some((host.to_owned(), share, PathBuf::from(point)))
            })
            .collect()
    }

    /// Computers that advertise file sharing over Bonjour, with their host names.
    fn bonjour() -> Vec<(String, String)> {
        let text = run_for("/usr/bin/dns-sd", &["-B", "_smb._tcp", "local."], Duration::from_millis(2500)).unwrap_or_default();
        let names: Vec<String> = text
            .lines()
            .filter(|l| l.contains(" Add "))
            .filter_map(|l| l.split_once("_smb._tcp.").map(|(_, name)| name.trim().to_owned()))
            .filter(|n| !n.is_empty())
            .collect();
        names
            .into_iter()
            .map(|name| {
                let resolved = run_for("/usr/bin/dns-sd", &["-L", &name, "_smb._tcp", "local."], Duration::from_millis(1500)).unwrap_or_default();
                let host = resolved
                    .lines()
                    .find_map(|l| l.split_once("can be reached at ").map(|(_, r)| r.split(':').next().unwrap_or("").trim_end_matches('.').to_owned()))
                    .filter(|h| !h.is_empty())
                    .unwrap_or_else(|| format!("{}.local", name.replace(' ', "-")));
                (name, host)
            })
            .collect()
    }

    pub fn computers(saved: &[String]) -> Vec<Entry> {
        let mut entries: Vec<Entry> = saved
            .iter()
            .filter_map(|s| super::parse_address(s))
            .map(|(h, _)| Entry { path: host_path(&h), label: h.clone() })
            .collect();
        entries.extend(mounted().into_iter().map(|(h, _, _)| Entry { path: host_path(&h), label: h.clone() }));
        entries.extend(bonjour().into_iter().map(|(name, host)| Entry { path: host_path(&host), label: name }));
        sort_unique(entries)
    }

    pub fn shares(host: &str, saved: &[String]) -> Result<Vec<Entry>, String> {
        let mut entries: Vec<Entry> = saved
            .iter()
            .filter_map(|s| super::parse_address(s))
            .filter(|(h, share)| h.eq_ignore_ascii_case(host) && share.is_some())
            .map(|(h, share)| {
                let share = share.unwrap_or_default();
                Entry { path: share_path(&h, &share), label: share }
            })
            .collect();
        // As guest, so it can never stop and ask for a password in a terminal.
        let listed = run_for("/usr/bin/smbutil", &["view", "-g", &format!("//{host}")], Duration::from_secs(6)).unwrap_or_default();
        for line in listed.lines() {
            let columns: Vec<&str> = line.split_whitespace().collect();
            if let Some(pos) = columns.iter().position(|c| *c == "Disk") {
                let share = columns[..pos].join(" ");
                if !share.is_empty() && !share.ends_with('$') {
                    entries.push(Entry { path: share_path(host, &share), label: share });
                }
            }
        }
        // Shares already mounted open straight away.
        for (h, share, point) in mounted() {
            if h.eq_ignore_ascii_case(host) {
                entries.retain(|e| !e.label.eq_ignore_ascii_case(&share));
                entries.push(Entry { path: point, label: share });
            }
        }
        if entries.is_empty() {
            Err("No shared folders found without signing in. Use Connect to sign in.".into())
        } else {
            Ok(sort_unique(entries))
        }
    }

    pub fn ensure_access(_path: &Path) {}

    /// Have macOS mount `smb://host/share` (Finder asks for a password if needed) and wait
    /// for it to appear under /Volumes.
    pub fn mount(share: &Path) -> Result<PathBuf, String> {
        let text = share.to_str().ok_or("Bad address")?;
        let (host, name) = text.trim_start_matches("smb://").split_once('/').ok_or("Bad address")?;
        let find = || mounted().into_iter().find(|(h, s, _)| h.eq_ignore_ascii_case(host) && s.eq_ignore_ascii_case(name.trim_end_matches('/'))).map(|(_, _, p)| p);
        if let Some(point) = find() {
            return Ok(point);
        }
        let url = format!("smb://{host}/{}", name.replace(' ', "%20"));
        let _ = std::process::Command::new("/usr/bin/open").arg(&url).spawn();
        for _ in 0..240 {
            std::thread::sleep(Duration::from_millis(500));
            if let Some(point) = find() {
                return Ok(point);
            }
        }
        Err(format!("{url} was not mounted"))
    }

    /// Let Finder show its "Connect to Server" sign-in for `host`.
    pub fn connect_dialog(host: &str) {
        let _ = std::process::Command::new("/usr/bin/open").arg(format!("smb://{host}")).spawn();
    }
}

// ---------------------------------------------------------------------------------------
// Other systems: only what was added by hand.
// ---------------------------------------------------------------------------------------

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::{host_path, Entry};
    use std::path::{Path, PathBuf};
    pub fn drive_share(_drive: &Path) -> Option<String> {
        None
    }
    pub fn computers(saved: &[String]) -> Vec<Entry> {
        saved.iter().filter_map(|s| super::parse_address(s)).map(|(h, _)| Entry { path: host_path(&h), label: h.clone() }).collect()
    }
    pub fn shares(_host: &str, _saved: &[String]) -> Result<Vec<Entry>, String> {
        Err("Browsing network shares is not supported on this system".into())
    }
    pub fn ensure_access(_path: &Path) {}
    pub fn mount(_share: &Path) -> Result<PathBuf, String> {
        Err("Not supported on this system".into())
    }
}

pub use platform::{computers, drive_share, ensure_access, mount, shares};

/// Open the system's sign-in for `host` (macOS Finder "Connect to Server"); on Windows the
/// sign-in dialog appears by itself when a share is opened.
pub fn connect_dialog(host: &str) {
    #[cfg(target_os = "macos")]
    platform::connect_dialog(host);
    let _ = host;
}
