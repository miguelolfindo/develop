//! An MCP (Model Context Protocol) server, so an AI assistant can help with Develop work:
//! browse photos, read and change their edits, apply presets, rate, keep versions, look at
//! a rendered preview, export and convert to DNG.
//!
//! - `develop --mcp` speaks MCP over standard input and output (one JSON-RPC message per
//!   line), for local AI apps that start the server themselves (Claude Desktop, Claude Code,
//!   LM Studio and others).
//! - `develop --mcp-http <port> [--mcp-token <token>]` serves the same over HTTP on this
//!   computer only (POST JSON-RPC to `http://127.0.0.1:<port>/mcp`), for AI services that
//!   connect over the network through a tunnel the user sets up. Every request needs the
//!   token (`Authorization: Bearer <token>`); one is made up and printed if none is given.
//!
//! Edits are read from and written to the photos' `.dvlp` sidecars, exactly as the app keeps
//! them. Previews and exports are rendered by running Develop itself with the photo's saved
//! settings (`--with-settings 1`), so they look as they do in the app.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::settings::{self, Saved};

const PROTOCOL: &str = "2025-06-18";

/// `develop --mcp`: serve MCP over stdin / stdout until stdin closes.
pub fn serve_stdio() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut server = Server::new();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle_text(&line) {
            let _ = writeln!(out, "{reply}");
            let _ = out.flush();
        }
    }
}

/// `develop --mcp-http <port>`: serve MCP over HTTP on 127.0.0.1.
pub fn serve_http(port: u16, token: Option<String>) {
    let token = token.unwrap_or_else(make_token);
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Develop MCP: cannot listen on 127.0.0.1:{port}: {e}");
            return;
        }
    };
    println!("Develop MCP server on http://127.0.0.1:{port}/mcp");
    println!("Access token: {token}");
    println!("(send it as the header  Authorization: Bearer <token>)");
    // Also kept in the settings folder, for when the console is not visible.
    if let Some(dir) = settings::settings_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("mcp-http.txt"), format!("http://127.0.0.1:{port}/mcp
Authorization: Bearer {token}
"));
    }
    let mut server = Server::new();
    for stream in listener.incoming().flatten() {
        let _ = handle_http(stream, &mut server, &token);
    }
}

fn make_token() -> String {
    // Not cryptographic quality, but unguessable enough for a token on this computer: the
    // time, the process and an address, hashed.
    use std::hash::{BuildHasher, Hasher};
    let mut text = String::new();
    for i in 0..2u8 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        h.write_u32(std::process::id());
        h.write_u8(i);
        text.push_str(&format!("{:016x}", h.finish()));
    }
    text
}

fn handle_http(mut stream: std::net::TcpStream, server: &mut Server, token: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
    let mut reader = std::io::BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut length = 0usize;
    let mut authorised = false;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
            if name == "content-length" {
                length = value.parse().unwrap_or(0);
            } else if name == "authorization" {
                authorised = value.strip_prefix("Bearer ").is_some_and(|t| t.trim() == token);
            }
        }
    }
    let mut body = vec![0u8; length.min(16 << 20)];
    reader.read_exact(&mut body)?;
    let respond = |stream: &mut std::net::TcpStream, status: &str, body: &str| {
        let text = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(text.as_bytes())
    };
    let mut parts = request_line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if path.split('?').next() != Some("/mcp") {
        return respond(&mut stream, "404 Not Found", r#"{"error":"not found"}"#);
    }
    if !authorised {
        return respond(&mut stream, "401 Unauthorized", r#"{"error":"missing or wrong access token"}"#);
    }
    if method != "POST" {
        return respond(&mut stream, "405 Method Not Allowed", r#"{"error":"POST JSON-RPC messages"}"#);
    }
    match server.handle_text(&String::from_utf8_lossy(&body)) {
        Some(reply) => respond(&mut stream, "200 OK", &reply),
        None => respond(&mut stream, "202 Accepted", ""),
    }
}

struct Server {
    store: settings::Store,
    presets: settings::Presets,
    /// The connected app's name (from `initialize`), for labelling the AI's versions.
    client: String,
    /// The AI version each photo's edits in the current request go into (see `ai_edit`), and
    /// when the assistant last called a tool (a pause longer than `REQUEST_GAP` means the user's
    /// next request).
    rounds: std::collections::HashMap<PathBuf, Round>,
    last_call: Option<std::time::Instant>,
    /// Focus measurements kept between calls and runs (see `cull.rs`), read when first needed.
    focus_cache: Option<crate::cull::Cache>,
}

/// The AI edits on a photo in one request: the version they go into (found by its time stamp,
/// set when it was made or last updated) and what has been changed so far.
struct Round {
    time: u64,
    started: u64,
    changed: Vec<String>,
}

/// An assistant works through a request with tool calls in quick succession; this long without
/// any call means it has answered and the user's next message is a new request.
const REQUEST_GAP: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Versions the AI makes are named with this prefix, so they are clearly told apart from the
/// user's own.
pub const AI_PREFIX: &str = "AI · ";

impl Server {
    fn new() -> Self {
        Self { store: settings::Store::load(), presets: settings::Presets::load(), client: "assistant".into(), rounds: Default::default(), last_call: None, focus_cache: None }
    }

    /// THE RULE FOR AI EDITS, enforced here whatever the assistant is told:
    /// 1. At the start of a request's edits on a photo, if its current edit is not already kept
    ///    as a version, it is saved as "Before AI · <time>", so the user's own work can always
    ///    be brought back.
    /// 2. The change is made to the current edit.
    /// 3. The result goes into ONE AI version per photo per request, "AI · <app>: <what
    ///    changed> · <time>": made by the request's first edit to the photo, then updated by each
    ///    further edit in the same request (its name gathers everything changed). A new request
    ///    starts after `REQUEST_GAP` without any tool call, when the assistant reconnects, or for
    ///    one edit when it asks for `new_version` (an alternative look).
    /// Only the version this request made is ever updated; every other version (the user's, and
    /// earlier requests') is never changed or removed. The server has no tool that could.
    fn ai_edit(&mut self, path: &Path, summary: &str, new_version: bool, change: impl Fn(&Saved) -> Saved) {
        let current = self.store.get(path).cloned();
        let before = current.clone().unwrap_or_else(blank);
        let after = change(&before);
        let now = settings::now_seconds();
        let mut versions = self.store.versions(path).to_vec();
        // The version this round is filling, if the round is still going and the version is
        // still there (the user may have deleted it).
        let ongoing = self
            .rounds
            .get(path)
            .filter(|_| !new_version)
            .and_then(|r| versions.iter().position(|v| v.time == r.time && v.name.starts_with(AI_PREFIX)).map(|i| (i, r.started, r.changed.clone())));
        if ongoing.is_none() && current.is_some() && !versions.iter().any(|v| v.saved == before) {
            versions.push(settings::Version { name: format!("Before AI · {}", stamp(now)), time: now, saved: before });
        }
        if after == blank() {
            self.store.update(path, &after, &blank());
        } else {
            self.store.put(path, after.clone());
        }
        let (started, mut changed) = match &ongoing {
            Some((_, started, changed)) => (*started, changed.clone()),
            None => (now, Vec::new()),
        };
        for part in summary.split(", ").filter(|p| !p.is_empty()) {
            if !changed.iter().any(|c| c == part) {
                changed.push(part.to_owned());
            }
        }
        // A new time each update, so the app draws the version's thumbnail again.
        let time = match &ongoing {
            Some((i, _, _)) => now.max(versions[*i].time + 1),
            None => now,
        };
        let version = settings::Version { name: ai_label(&self.client, &changed.join(", "), &stamp(started)), time, saved: after };
        match ongoing {
            Some((i, _, _)) => versions[i] = version,
            None => versions.push(version),
        }
        self.store.set_versions(path, versions);
        self.rounds.insert(path.to_path_buf(), Round { time, started, changed });
        forget_thumbnail(path);
    }

    /// One JSON-RPC message (or a batch) in, the reply out (`None` for notifications).
    fn handle_text(&mut self, text: &str) -> Option<String> {
        let message: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => return Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("Parse error: {e}")}}).to_string()),
        };
        if let Value::Array(batch) = message {
            let replies: Vec<Value> = batch.into_iter().filter_map(|m| self.handle(m)).collect();
            return (!replies.is_empty()).then(|| Value::Array(replies).to_string());
        }
        self.handle(message).map(|v| v.to_string())
    }

    fn handle(&mut self, message: Value) -> Option<Value> {
        let id = message.get("id").cloned();
        let method = message["method"].as_str().unwrap_or_default().to_owned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        // Notifications (no id) get no reply.
        let id = id?;
        let result = match method.as_str() {
            "initialize" => Ok({
                self.rounds.clear();
                if let Some(name) = params["clientInfo"]["title"].as_str().or(params["clientInfo"]["name"].as_str()) {
                    let name = settings::clean_name(name);
                    if !name.is_empty() {
                        self.client = name;
                    }
                }
                json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL),
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "develop", "title": "Develop raw photo editor", "version": crate::update::CURRENT},
                "instructions": INSTRUCTIONS,
            })
            }),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                // A long pause since the last call: this is the user's next request.
                // (DEVELOP_MCP_REQUEST_GAP, in seconds, shortens it for testing.)
                let gap = std::env::var("DEVELOP_MCP_REQUEST_GAP").ok().and_then(|v| v.parse().ok()).map_or(REQUEST_GAP, std::time::Duration::from_secs);
                if self.last_call.is_some_and(|t| t.elapsed() > gap) {
                    self.rounds.clear();
                }
                self.last_call = Some(std::time::Instant::now());
                let name = params["name"].as_str().unwrap_or_default().to_owned();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                Ok(match self.call(&name, &args) {
                    Ok(content) => json!({"content": content, "isError": false}),
                    Err(message) => json!({"content": [{"type": "text", "text": message}], "isError": true}),
                })
            }
            _ => Err((-32601, format!("Unknown method {method}"))),
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        })
    }

    fn call(&mut self, name: &str, args: &Value) -> Result<Vec<Value>, String> {
        let text = |v: Value| Ok(vec![json!({"type": "text", "text": serde_json::to_string(&v).unwrap_or_default()})]);
        match name {
            "list_photos" => {
                let folder = PathBuf::from(args["folder"].as_str().ok_or("folder is required")?);
                if !folder.is_dir() {
                    return Err(format!("{} is not a folder", folder.display()));
                }
                self.store.load_folder_fresh(&folder);
                let min_rating = args["min_rating"].as_u64().unwrap_or(0) as u8;
                let mut photos: Vec<PathBuf> = std::fs::read_dir(&folder)
                    .map_err(|e| e.to_string())?
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| crate::filmstrip::is_raw(p) || is_picture(p))
                    .collect();
                photos.sort();
                let list: Vec<Value> = photos
                    .iter()
                    .filter(|p| self.store.rating(p) >= min_rating)
                    .map(|p| json!({
                        "path": p.display().to_string(),
                        "rating": self.store.rating(p),
                        "edited": self.store.contains(p),
                        "versions": self.store.versions(p).len(),
                    }))
                    .collect();
                text(json!({"folder": folder.display().to_string(), "count": list.len(), "photos": list}))
            }
            "get_settings" => {
                let path = self.photo(args)?;
                let saved = self.store.get(&path).cloned();
                text(json!({
                    "path": path.display().to_string(),
                    "edited": saved.is_some(),
                    "rating": self.store.rating(&path),
                    "settings": to_json(saved.as_ref().unwrap_or(&blank())),
                }))
            }
            "set_settings" => {
                let paths = self.photos(args)?;
                let changes = args.get("settings").ok_or("settings is required")?.clone();
                // Check every name and value first, so a mistake changes no photo at all.
                apply_json(&mut blank(), &changes)?;
                let summary = changes.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>().join(", ")).unwrap_or_default();
                let new_version = args["new_version"].as_bool().unwrap_or(false);
                for path in &paths {
                    self.ai_edit(path, &summary, new_version, |base| {
                        let mut saved = base.clone();
                        let _ = apply_json(&mut saved, &changes);
                        saved
                    });
                }
                self.store.flush(true);
                text(json!({"updated": paths.len(), "versions": if new_version { "each photo's result was saved as a new AI version" } else { "each photo's result is in this request's AI version (one per photo per request)" }}))
            }
            "reset_settings" => {
                let paths = self.photos(args)?;
                for path in &paths {
                    self.ai_edit(path, "reset to defaults", args["new_version"].as_bool().unwrap_or(false), |_| blank());
                }
                self.store.flush(true);
                text(json!({"reset": paths.len()}))
            }
            "list_presets" => {
                self.presets = settings::Presets::load();
                let list: Vec<Value> = self.presets.list.iter().map(|p| json!({"name": p.name, "white_balance": p.white_balance, "settings": to_json(&p.saved)})).collect();
                text(json!({"presets": list}))
            }
            "apply_preset" => {
                self.presets = settings::Presets::load();
                let name = args["preset"].as_str().ok_or("preset is required")?;
                let preset = self.presets.get(name).cloned().ok_or_else(|| format!("No preset named {name}"))?;
                let paths = self.photos(args)?;
                let summary = format!("preset {}", preset.name);
                for path in &paths {
                    self.ai_edit(path, &summary, args["new_version"].as_bool().unwrap_or(false), |base| preset.apply_to(base));
                }
                self.store.flush(true);
                text(json!({"applied": preset.name, "photos": paths.len()}))
            }
            "save_preset" => {
                let path = self.photo(args)?;
                let name = settings::clean_name(args["name"].as_str().ok_or("name is required")?);
                if name.is_empty() {
                    return Err("name is empty".into());
                }
                let saved = self.store.get(&path).cloned().unwrap_or_else(blank);
                self.presets = settings::Presets::load();
                self.presets.add(settings::Preset { name: name.clone(), white_balance: args["white_balance"].as_bool().unwrap_or(false), saved });
                text(json!({"saved": name}))
            }
            "set_rating" => {
                let paths = self.photos(args)?;
                let stars = args["stars"].as_u64().ok_or("stars (0-5) is required")?.min(5) as u8;
                for path in &paths {
                    self.store.set_rating(path, stars);
                }
                self.store.flush(true);
                text(json!({"rated": paths.len(), "stars": stars}))
            }
            "list_versions" => {
                let path = self.photo(args)?;
                let list: Vec<Value> = self.store.versions(&path).iter().map(|v| json!({"name": v.name, "saved_at": v.time, "settings": to_json(&v.saved)})).collect();
                text(json!({"path": path.display().to_string(), "versions": list}))
            }
            "save_version" => {
                let path = self.photo(args)?;
                let mut versions = self.store.versions(&path).to_vec();
                // A version the AI saves is always labelled as the AI's.
                let name = settings::clean_name(args["name"].as_str().unwrap_or(""));
                let name = if name.is_empty() { format!("Version {}", versions.len() + 1) } else { name };
                let name = settings::clean_name(&format!("{AI_PREFIX}{}", name.trim_start_matches(AI_PREFIX)));
                let saved = self.store.get(&path).cloned().unwrap_or_else(blank);
                versions.push(settings::Version { name: name.clone(), time: settings::now_seconds(), saved });
                self.store.set_versions(&path, versions);
                self.store.flush(true);
                text(json!({"saved": name}))
            }
            "restore_version" => {
                let path = self.photo(args)?;
                let name = args["name"].as_str().ok_or("name is required")?;
                let version = self.store.versions(&path).iter().find(|v| v.name.eq_ignore_ascii_case(name)).cloned().ok_or_else(|| format!("No version named {name}"))?;
                let saved = version.saved.clone();
                self.ai_edit(&path, &format!("restored {}", version.name), args["new_version"].as_bool().unwrap_or(false), move |_| saved.clone());
                self.store.flush(true);
                text(json!({"restored": version.name}))
            }
            "check_photo" => {
                let path = self.photo(args)?;
                let long_edge = args["long_edge"].as_u64().unwrap_or(800).clamp(256, 2000);
                let (mut report, _) = measure_photo(&path, long_edge, 0, false, true)?;
                report["path"] = json!(path.display().to_string());
                report["settings_summary"] = settings_summary(self.store.get(&path));
                text(report)
            }
            "check_photos" => {
                // Many photos at once, one short row each, so a model can survey a whole folder
                // without filling its context with full check_photo reports.
                let all: Vec<PathBuf> = match args["folder"].as_str() {
                    Some(folder) => {
                        let folder = PathBuf::from(folder);
                        if !folder.is_dir() {
                            return Err(format!("{} is not a folder", folder.display()));
                        }
                        self.store.load_folder_fresh(&folder);
                        let mut list: Vec<PathBuf> = std::fs::read_dir(&folder)
                            .map_err(|e| e.to_string())?
                            .filter_map(|e| e.ok().map(|e| e.path()))
                            .filter(|p| crate::filmstrip::is_raw(p) || is_picture(p))
                            .collect();
                        list.sort();
                        list
                    }
                    None => self.photos(args)?,
                };
                let start = (args["start"].as_u64().unwrap_or(0) as usize).min(all.len());
                let count = args["count"].as_u64().unwrap_or(30).clamp(1, 60) as usize;
                let long_edge = args["long_edge"].as_u64().unwrap_or(512).clamp(256, 2000);
                let focus = args["focus"].as_bool().unwrap_or(false);
                let batch = &all[start..(start + count).min(all.len())];
                // A few renders at a time; each is its own Develop process. Stop starting new ones
                // after BATCH_SECONDS so the call returns before the client's tool timeout (often 60
                // seconds); next_start then continues where it stopped.
                let started = std::time::Instant::now();
                let mut reports: Vec<Result<Value, String>> = Vec::with_capacity(batch.len());
                for (n, group) in batch.chunks(4).enumerate() {
                    if started.elapsed().as_secs() >= BATCH_SECONDS {
                        break;
                    }
                    let results: Vec<Result<Value, String>> = std::thread::scope(|s| {
                        let handles: Vec<_> = group.iter().enumerate().map(|(i, p)| s.spawn(move || measure_photo(p, long_edge, n * 4 + i + 1, focus, false).map(|r| r.0))).collect();
                        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("measuring failed".into()))).collect()
                    });
                    reports.extend(results);
                }
                let mut rows = Vec::new();
                let mut failed = Vec::new();
                let batch = &batch[..reports.len()];
                for (path, report) in batch.iter().zip(reports) {
                    let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                    match report {
                        Ok(r) => {
                            let mut row = vec![
                                json!(file),
                                json!(crate::filmstrip::capture_date(path)),
                                json!(self.store.rating(path)),
                                json!(self.store.contains(path)),
                                r["detail"].clone(),
                                r["brightness"]["mean"].clone(),
                                r["clipping_percent"]["pure_white_all_channels"].clone(),
                                r["clipping_percent"]["any_channel_at_max"].clone(),
                                r["clipping_percent"]["pure_black"].clone(),
                                r["neutral_areas"]["blue_minus_red_half"].clone(),
                                r["neutral_areas"]["green_minus_magenta"].clone(),
                                r["neutral_areas"]["share_percent"].clone(),
                            ];
                            if focus {
                                row.push(r["focus"]["sharpest_edge_px"].clone());
                                row.push(r["focus"]["median_edge_px"].clone());
                                row.push(r["focus"]["sharpest_at"].clone());
                            }
                            rows.push(Value::Array(row));
                        }
                        Err(e) => failed.push(format!("{file}: {e}")),
                    }
                }
                let next = start + batch.len();
                let mut columns = vec!["file", "taken", "rating", "edited", "detail", "mean", "white_clip_pct", "any_channel_max_pct", "black_clip_pct", "neutral_blue_minus_red", "neutral_green_minus_magenta", "neutral_share_pct"];
                if focus {
                    columns.extend(["focus_sharpest_edge_px", "focus_median_edge_px", "focus_sharpest_at"]);
                }
                text(json!({
                    "total": all.len(),
                    "start": start,
                    "next_start": if next < all.len() { json!(next) } else { Value::Null },
                    "columns": columns,
                    "rows": rows,
                    "failed": failed,
                    "how_to_read": "taken: when the photo was taken (camera clock, YYYY:MM:DD HH:MM:SS); frames seconds apart are one burst or scene, a gap of a minute or more usually means a different shot. detail: sharpness and texture at this render size, higher is sharper; compare frames of the same scene (neighbours), not different scenes. mean: brightness 0-255. neutral_*: colour cast in low-colour areas (positive blue_minus_red = too cool, raise temperature; positive green_minus_magenta = too green, raise tint), unreliable when neutral_share_pct is small. focus_* (with focus: true): edge width in pixels at full size of the sharpest area, about 2.7 at best and crisp detail 2.5-3.5, bigger is softer; compare frames of the same scene; focus_sharpest_at is where (fractions of width and height). Call again with start = next_start until it is null; use check_photo for the full report on one photo, and check_focus to see the sharpest area.",
                }))
            }
            "cull_photos" => {
                let folder = PathBuf::from(args["folder"].as_str().ok_or("folder is required")?);
                if !folder.is_dir() {
                    return Err(format!("{} is not a folder", folder.display()));
                }
                self.store.load_folder_fresh(&folder);
                let mut all: Vec<PathBuf> = std::fs::read_dir(&folder)
                    .map_err(|e| e.to_string())?
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| crate::filmstrip::is_raw(p) || is_picture(p))
                    .collect();
                all.sort();
                let rules = crate::cull::Rules {
                    gap: args["gap_seconds"].as_u64().unwrap_or(60).clamp(1, 3600) as i64,
                    sharp: args["sharp_px"].as_f64().unwrap_or(3.5).clamp(2.0, 10.0) as f32,
                    near: args["near_px"].as_f64().unwrap_or(0.5).clamp(0.0, 5.0) as f32,
                    soft: args["soft_px"].as_f64().unwrap_or(5.5).clamp(2.0, 20.0) as f32,
                };
                let cache = self.focus_cache.get_or_insert_with(crate::cull::Cache::load);
                // Measure what is not kept yet, a few at a time, for up to BATCH_SECONDS.
                let keys: Vec<String> = all.iter().map(|p| crate::cull::key(p)).collect();
                let todo: Vec<usize> = (0..all.len()).filter(|&i| cache.get(&all[i], &keys[i]).is_none()).collect();
                let started = std::time::Instant::now();
                let mut failed = Vec::new();
                let mut measured_now = 0;
                for (n, group) in todo.chunks(4).enumerate() {
                    if started.elapsed().as_secs() >= BATCH_SECONDS {
                        break;
                    }
                    let results: Vec<Result<Value, String>> = std::thread::scope(|s| {
                        let handles: Vec<_> = group
                            .iter()
                            .enumerate()
                            .map(|(i, &k)| {
                                let p = &all[k];
                                s.spawn(move || measure_photo(p, 256, n * 4 + i + 1, true, false).map(|r| r.0))
                            })
                            .collect();
                        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("measuring failed".into()))).collect()
                    });
                    for (&k, result) in group.iter().zip(results) {
                        let path = &all[k];
                        let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                        match result {
                            Ok(r) if r["focus"].is_object() => {
                                let f = &r["focus"];
                                let num = |v: &Value| v.as_f64().map(|v| v as f32);
                                let at = match (num(&f["sharpest_at"][0]), num(&f["sharpest_at"][1])) {
                                    (Some(x), Some(y)) => Some([x, y]),
                                    _ => None,
                                };
                                let measured = crate::cull::Measured {
                                    taken: crate::filmstrip::capture_date(path),
                                    sharpest: num(&f["sharpest_edge_px"]),
                                    median: num(&f["median_edge_px"]),
                                    at,
                                };
                                cache.put(path, keys[k].clone(), measured);
                                measured_now += 1;
                            }
                            Ok(_) => failed.push(format!("{file}: focus could not be measured")),
                            Err(e) => failed.push(format!("{file}: {e}")),
                        }
                    }
                }
                cache.save();
                let shots: Vec<(usize, crate::cull::Measured)> = (0..all.len()).filter_map(|i| cache.get(&all[i], &keys[i]).map(|m| (i, m.clone()))).collect();
                if shots.len() + failed.len() < all.len() {
                    return text(json!({
                        "status": "measuring",
                        "measured": shots.len(),
                        "total": all.len(),
                        "remaining": all.len() - shots.len(),
                        "measured_this_call": measured_now,
                        "failed": failed,
                        "next": "Call cull_photos again with the same folder to continue. Measurements are kept, so nothing is measured twice; you do not need to remember anything from this reply.",
                    }));
                }
                let measured: Vec<crate::cull::Measured> = shots.iter().map(|(_, m)| m.clone()).collect();
                let bursts = crate::cull::cull(&measured, &rules);
                let apply = args["apply"].as_bool().unwrap_or(false);
                let replace = args["replace_ratings"].as_bool().unwrap_or(false);
                let (mut counts, mut kept, mut changed) = ([0usize; 6], 0, 0);
                let mut rows = Vec::new();
                let name = |i: usize| all[shots[i].0].file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                for burst in &bursts {
                    for (&i, &stars) in burst.photos.iter().zip(&burst.stars) {
                        counts[stars as usize] += 1;
                        if apply {
                            let path = &all[shots[i].0];
                            let current = self.store.rating(path);
                            if current != 0 && !replace {
                                kept += 1;
                            } else if current != stars {
                                self.store.set_rating(path, stars);
                                changed += 1;
                            }
                        }
                    }
                    let first = burst.photos[0];
                    let last = *burst.photos.last().unwrap_or(&first);
                    let seconds: Vec<String> = burst.photos.iter().zip(&burst.stars).filter(|(_, s)| **s == 3).map(|(i, _)| name(*i)).collect();
                    let keeper = burst.photos.iter().zip(&burst.stars).find(|(_, s)| **s == 4).map(|(i, _)| *i);
                    rows.push(json!([
                        name(first),
                        if last == first { Value::Null } else { json!(name(last)) },
                        burst.photos.len(),
                        measured[first].taken,
                        keeper.map(name),
                        burst.best.and_then(|b| measured[b].sharpest).map(|w| (w as f64 * 100.0).round() / 100.0),
                        seconds,
                        if burst.check.is_empty() { Value::Null } else { json!(burst.check.join("; ")) },
                    ]));
                }
                if apply {
                    self.store.flush(true);
                }
                let mut report = json!({
                    "status": "done",
                    "total": all.len(),
                    "bursts": bursts.len(),
                    "suggested_stars": {"4": counts[4], "3": counts[3], "1": counts[1]},
                    "columns": ["first", "last", "frames", "taken", "keeper_4_stars", "best_edge_px", "seconds_3_stars", "check"],
                    "rows": rows,
                    "failed": failed,
                    "how_to_read": "One row per burst (frames taken within gap_seconds of the one before), in capture order; last is null for a single frame. keeper_4_stars is the burst's sharpest frame (null when even that is soft, so every frame gets 1 star); best_edge_px is its edge width; seconds_3_stars are frames nearly as sharp; every other frame gets 1 star. Edge widths are in full-size pixels: about 2.7 at best, 2.5-3.5 crisp, bigger is softer. check says why a burst is worth a look: use check_focus on its keeper to see a 100% crop of the sharpest area, and set_rating to change a suggestion.",
                });
                if apply {
                    report["applied"] = json!({"ratings_changed": changed, "already_rated_kept": kept});
                    if kept > 0 {
                        report["note"] = json!("Photos that already had a rating kept it; pass replace_ratings: true to replace those too.");
                    }
                } else {
                    report["next"] = json!("Nothing is rated yet. Call again with apply: true to save these ratings (photos that already have a rating keep it unless replace_ratings: true).");
                }
                text(report)
            }
            "auto_white_balance" => {
                // Develop finds each photo's white balance itself (see autowb.rs), four photos at a
                // time, and the result goes into the photo's AI version like any other edit.
                let paths = self.photos(args)?;
                let started = std::time::Instant::now();
                let mut results: Vec<(PathBuf, Result<Value, String>)> = Vec::new();
                for (n, group) in paths.chunks(4).enumerate() {
                    if started.elapsed().as_secs() >= BATCH_SECONDS {
                        break;
                    }
                    let found: Vec<Result<Value, String>> = std::thread::scope(|s| {
                        let handles: Vec<_> = group.iter().enumerate().map(|(i, p)| s.spawn(move || auto_wb(p, n * 4 + i + 1))).collect();
                        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("measuring failed".into()))).collect()
                    });
                    results.extend(group.iter().cloned().zip(found));
                }
                let mut rows = Vec::new();
                for (path, result) in &results {
                    let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                    match result {
                        Ok(r) if r["error"].is_null() => {
                            if r["method"] != "unchanged" {
                                let (k, tint) = (r["temperature"].as_f64().unwrap_or(0.0) as f32, r["tint"].as_f64().unwrap_or(0.0) as f32);
                                self.ai_edit(path, "auto white balance", false, |base| {
                                    let mut saved = base.clone();
                                    saved.values[0] = k;
                                    saved.tint = tint;
                                    saved
                                });
                            }
                            let mut row = r.clone();
                            row["file"] = json!(file);
                            rows.push(row);
                        }
                        Ok(r) => rows.push(json!({"file": file, "error": r["error"]})),
                        Err(e) => rows.push(json!({"file": file, "error": e})),
                    }
                }
                self.store.flush(true);
                let done = results.len();
                text(json!({
                    "done": done,
                    "remaining": paths.len() - done,
                    "photos": rows,
                    "how_to_read": "Develop set each photo's temperature (and, from neutral areas, tint) itself; exposure is unchanged. method says what it went by: skin (faces, measured as if at 20% brightness, brought to natural saturation without turning pink), neutral areas (grey and white parts made neutral) or unchanged (nothing to measure). before and after show the readings. check says why a photo is worth a look. If remaining is above 0, call again with the photos not yet done.",
                }))
            }
            "check_focus" => {
                let path = self.photo(args)?;
                let (report, crop) = measure_photo(&path, 512, 0, true, false)?;
                let mut focus = report["focus"].clone();
                focus["path"] = json!(path.display().to_string());
                let mut content = vec![json!({"type": "text", "text": serde_json::to_string(&focus).unwrap_or_default()})];
                if args["include_crop"].as_bool().unwrap_or(true) {
                    if let Some(bytes) = crop {
                        content.push(json!({"type": "image", "data": base64(&bytes), "mimeType": "image/jpeg"}));
                        content.push(json!({"type": "text", "text": "The image is a 100% crop (512 x 512 px, not resized) centred on the sharpest area of the full-size photo."}));
                    }
                }
                Ok(content)
            }
            "render_preview" => {
                let path = self.photo(args)?;
                let long_edge = args["long_edge"].as_u64().unwrap_or(1024).clamp(256, 3000);
                let out = std::env::temp_dir().join(format!("develop-mcp-preview-{}.jpg", std::process::id()));
                render(&path, &out, &[("--export-long", long_edge.to_string()), ("--export-quality", "85".into())])?;
                let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
                let _ = std::fs::remove_file(&out);
                Ok(vec![
                    json!({"type": "image", "data": base64(&bytes), "mimeType": "image/jpeg"}),
                    json!({"type": "text", "text": format!("{} rendered with its current Develop settings, {long_edge} px on the long edge.", path.display())}),
                ])
            }
            "export" => {
                let paths = self.photos(args)?;
                let folder = PathBuf::from(args["folder"].as_str().ok_or("folder is required")?);
                std::fs::create_dir_all(&folder).map_err(|e| format!("Cannot create {}: {e}", folder.display()))?;
                let format = args["format"].as_str().unwrap_or("jpeg").to_lowercase();
                let ext = match format.as_str() {
                    "jpeg" | "jpg" => "jpg",
                    "png" => "png",
                    "tiff" | "tif" => "tif",
                    other => return Err(format!("Unknown format {other} (jpeg, png or tiff)")),
                };
                let mut options = vec![("--export-quality", args["quality"].as_u64().unwrap_or(90).clamp(1, 100).to_string())];
                if let Some(edge) = args["long_edge"].as_u64() {
                    options.push(("--export-long", edge.to_string()));
                }
                let mut done = Vec::new();
                let mut failed = Vec::new();
                for path in &paths {
                    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "photo".into());
                    let out = free_name(&folder, &stem, ext);
                    match render(path, &out, &options) {
                        Ok(()) => done.push(out.display().to_string()),
                        Err(e) => failed.push(format!("{}: {e}", path.display())),
                    }
                }
                text(json!({"exported": done, "failed": failed}))
            }
            "convert_to_dng" => {
                let paths = self.photos(args)?;
                let folder = args["folder"].as_str().map(PathBuf::from);
                let embed = args["embed_original"].as_bool().unwrap_or(false);
                let mut done = Vec::new();
                let mut failed = Vec::new();
                for path in &paths {
                    let dir = folder.clone().unwrap_or_else(|| path.parent().map(Path::to_path_buf).unwrap_or_default());
                    match crate::todng::convert_one(path, &dir, embed, None) {
                        Ok(out) => done.push(out.display().to_string()),
                        Err(e) => failed.push(format!("{}: {e}", path.display())),
                    }
                }
                text(json!({"converted": done, "failed": failed}))
            }
            other => Err(format!("Unknown tool {other}")),
        }
    }

    /// The photo named by `path`, with its folder's sidecars read.
    fn photo(&mut self, args: &Value) -> Result<PathBuf, String> {
        let path = PathBuf::from(args["path"].as_str().ok_or("path is required")?);
        if !path.is_file() {
            return Err(format!("{} does not exist", path.display()));
        }
        if let Some(dir) = path.parent() {
            self.store.load_folder(dir);
        }
        // The app may have changed it since (the newer edit wins).
        self.store.reload_sidecar(&path);
        Ok(path)
    }

    /// The photos named by `paths` (or a single `path`).
    fn photos(&mut self, args: &Value) -> Result<Vec<PathBuf>, String> {
        let list: Vec<PathBuf> = match args["paths"].as_array() {
            Some(items) => items.iter().filter_map(|v| v.as_str()).map(PathBuf::from).collect(),
            None => vec![PathBuf::from(args["path"].as_str().ok_or("paths (or path) is required")?)],
        };
        if list.is_empty() {
            return Err("paths is empty".into());
        }
        for path in &list {
            if !path.is_file() {
                return Err(format!("{} does not exist", path.display()));
            }
            if let Some(dir) = path.parent() {
                self.store.load_folder(dir);
            }
            self.store.reload_sidecar(path);
        }
        Ok(list)
    }
}

/// How long check_photos keeps starting renders before it returns what it has.
const BATCH_SECONDS: u64 = 35;

const INSTRUCTIONS: &str = "Develop is a raw photo editor. Photos are files on disk; each photo's edit lives in a .dvlp file beside it. \
Your changes to a photo are saved automatically in one version per photo per request, labelled 'AI · ...' (it is updated as you keep working on the request; pass new_version: true to set_settings or apply_preset to start a separate version, e.g. for an alternative look such as black and white), and the user's own edit is kept first as 'Before AI · ...'; other versions are never changed. Use list_photos to see a folder, get_settings to read an edit, set_settings to change it (only the settings you name change), \
render_preview to look at the result (or check_photo to measure it with numbers: histogram, clipping, colour cast; use it if you cannot see images), check_photos to measure many photos at once with one short row each (use it for whole folders instead of check_photo on each photo; focus: true adds a full-size focus measurement), cull_photos to rate a whole folder by focus (bursts grouped by capture time; call it until status is done, then with apply: true), auto_white_balance to let Develop set photos' white balance itself (from skin, or neutral areas), check_focus to measure one photo's focus and see a 100% crop of its sharpest area, and export to write JPEG, PNG or TIFF files. Slider scales follow Lightroom's: exposure in stops (-5..5), \
most other sliders -100..100, temperature in kelvin (0 = as shot), tint -150..150. Tell the user which AI versions you made. \
If Develop is open on the same folder, it shows changes made here as they happen.";

/// The tools, with their input schemas.
fn tools() -> Value {
    let paths = json!({"type": "array", "items": {"type": "string"}, "description": "Full paths of the photos"});
    let path = json!({"type": "string", "description": "Full path of the photo"});
    json!([
        {"name": "list_photos", "title": "List photos",
         "description": "List the photos in a folder with their star rating and whether they have been edited.",
         "inputSchema": {"type": "object", "properties": {"folder": {"type": "string"}, "min_rating": {"type": "integer", "minimum": 0, "maximum": 5}}, "required": ["folder"]}},
        {"name": "get_settings", "title": "Get a photo's edit",
         "description": "Read a photo's Develop settings (all sliders, tone curve, colour mixer, black & white, crop) and rating.",
         "inputSchema": {"type": "object", "properties": {"path": path}, "required": ["path"]}},
        {"name": "set_settings", "title": "Change photos' edits",
         "description": format!("Change settings on one or more photos; settings not named stay as they are. Names: {}. \
curve is a list of [x, y] points 0..1; hsl_hue / hsl_saturation / hsl_luminance and mono_mix are objects keyed by colour (red, orange, yellow, green, aqua, blue, purple, magenta); crop is [left, top, right, bottom] 0..1.", SETTING_NAMES.join(", ")),
         "inputSchema": {"type": "object", "properties": {"paths": paths, "path": path, "settings": {"type": "object"}, "new_version": {"type": "boolean", "description": "Start a separate AI version (for an alternative look) instead of adding to this request's"}}, "required": ["settings"]}},
        {"name": "reset_settings", "title": "Reset photos",
         "description": "Put photos back to Develop's defaults (their versions and ratings are kept).",
         "inputSchema": {"type": "object", "properties": {"paths": paths, "path": path}}},
        {"name": "list_presets", "title": "List presets", "description": "List the user's develop presets.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "apply_preset", "title": "Apply a preset",
         "description": "Apply a develop preset to photos (tone, presence, colour, curve, mixer, detail and profile; white balance if the preset carries it).",
         "inputSchema": {"type": "object", "properties": {"preset": {"type": "string"}, "paths": paths, "path": path, "new_version": {"type": "boolean", "description": "Start a separate AI version (for an alternative look) instead of adding to this request's"}}, "required": ["preset"]}},
        {"name": "save_preset", "title": "Save a preset",
         "description": "Save a photo's current look as a develop preset.",
         "inputSchema": {"type": "object", "properties": {"path": path, "name": {"type": "string"}, "white_balance": {"type": "boolean"}}, "required": ["path", "name"]}},
        {"name": "set_rating", "title": "Rate photos", "description": "Set the star rating (0-5) of photos.",
         "inputSchema": {"type": "object", "properties": {"paths": paths, "path": path, "stars": {"type": "integer", "minimum": 0, "maximum": 5}}, "required": ["stars"]}},
        {"name": "list_versions", "title": "List versions", "description": "List a photo's saved versions.",
         "inputSchema": {"type": "object", "properties": {"path": path}, "required": ["path"]}},
        {"name": "save_version", "title": "Save a version", "description": "Save the photo's current edit as a named version.",
         "inputSchema": {"type": "object", "properties": {"path": path, "name": {"type": "string"}}, "required": ["path"]}},
        {"name": "restore_version", "title": "Restore a version", "description": "Make a saved version the photo's current edit.",
         "inputSchema": {"type": "object", "properties": {"path": path, "name": {"type": "string"}}, "required": ["path", "name"]}},
        {"name": "check_photo", "title": "Measure a photo",
         "description": "Render the photo with its current edit and measure it, for checking an edit without looking: brightness histogram (32 bins) and percentiles per channel, clipping (blown highlights, crushed shadows), tone zones, contrast, colour cast in neutral areas (with which way to move temperature and tint), saturation and the share of each Color Mixer colour, brightness in a 3x3 grid, a detail (sharpness) score, and plain-language notes. Use it before and after changes.",
         "inputSchema": {"type": "object", "properties": {"path": path, "long_edge": {"type": "integer", "minimum": 256, "maximum": 2000}}, "required": ["path"]}},
        {"name": "check_photos", "title": "Measure many photos",
         "description": "Measure a batch of photos (a folder, or a list of paths) and return one short row per photo: rating, edited, detail (sharpness) score, mean brightness, clipping and the colour cast in neutral areas. Use it to survey or compare many photos instead of calling check_photo on each. Pass focus: true to also measure focus at full size (edge width of the sharpest area; slower), for example to find the sharp photos in a shoot. Returns up to `count` photos (default 30, max 60) from `start`, fewer if measuring takes more than about 35 seconds (so the call does not time out); call again with next_start until it is null.",
         "inputSchema": {"type": "object", "properties": {"folder": {"type": "string"}, "paths": paths, "start": {"type": "integer", "minimum": 0}, "count": {"type": "integer", "minimum": 1, "maximum": 60}, "long_edge": {"type": "integer", "minimum": 256, "maximum": 2000}, "focus": {"type": "boolean"}}}},
        {"name": "cull_photos", "title": "Cull a folder by focus",
         "description": "Rate a whole folder by focus in a few calls. Groups the photos into bursts by capture time and, in each burst, suggests 4 stars for the sharpest frame (if it is reasonably sharp), 3 for frames nearly as sharp, and 1 for the rest; returns one short row per burst, with the bursts worth a look marked. Measuring takes about a second per photo, so a large folder takes several calls: while status is measuring, call again with the same folder (measurements are kept, also between sessions). With apply: true it saves the ratings (photos that already have one keep it unless replace_ratings: true). Use this instead of check_photos when culling.",
         "inputSchema": {"type": "object", "properties": {"folder": {"type": "string"}, "apply": {"type": "boolean"}, "replace_ratings": {"type": "boolean"}, "gap_seconds": {"type": "integer", "minimum": 1, "maximum": 3600, "description": "Frames taken within this many seconds of the one before are one burst (default 60)"}, "sharp_px": {"type": "number", "description": "A 3-star frame must be at most this wide (default 3.5)"}, "near_px": {"type": "number", "description": "and within this much of the keeper (default 0.5)"}, "soft_px": {"type": "number", "description": "A burst whose sharpest frame is wider than this is all rejects (default 5.5)"}}, "required": ["folder"]}},
        {"name": "auto_white_balance", "title": "Auto white balance",
         "description": "Let Develop set the white balance of photos itself, by measuring renders: from people's skin when there are faces (measured as if the faces were at 20%, brought to natural saturation without turning pink; right for firelight and tungsten), otherwise from the grey and white parts of the picture. Changes temperature (and tint when going by neutral areas) only, never exposure, and saves into the photo's AI version. Returns, per photo, the temperature set, the method, readings before and after, and anything worth a look. Several seconds per photo; for many photos it may return early with remaining above 0: call again with the rest. Prefer this to adjusting temperature by hand.",
         "inputSchema": {"type": "object", "properties": {"paths": paths, "path": path}}},
        {"name": "check_focus", "title": "Check focus",
         "description": "Measure focus on the full-size photo: the edge width in pixels of its sharpest area (about 2.7 at best; crisp detail reads 2.5-3.5; bigger is softer), across and down (a big difference can mean motion blur), where that area is, the typical edge width of the whole picture, and a verdict. Also returns a 100% crop (512 x 512 px) of the sharpest area so you can see whether the subject is in focus; pass include_crop: false if you cannot see images. For many photos use check_photos with focus: true.",
         "inputSchema": {"type": "object", "properties": {"path": path, "include_crop": {"type": "boolean"}}, "required": ["path"]}},
        {"name": "render_preview", "title": "Look at a photo",
         "description": "Render the photo with its current settings and return it as an image (default 1024 px on the long edge).",
         "inputSchema": {"type": "object", "properties": {"path": path, "long_edge": {"type": "integer", "minimum": 256, "maximum": 3000}}, "required": ["path"]}},
        {"name": "export", "title": "Export photos",
         "description": "Export photos with their current settings to a folder as JPEG, PNG or TIFF, optionally resized.",
         "inputSchema": {"type": "object", "properties": {"paths": paths, "path": path, "folder": {"type": "string"}, "format": {"type": "string", "enum": ["jpeg", "png", "tiff"]}, "long_edge": {"type": "integer"}, "quality": {"type": "integer", "minimum": 1, "maximum": 100}}, "required": ["folder"]}},
        {"name": "convert_to_dng", "title": "Convert to DNG",
         "description": "Convert raw photos to DNG (lossless), beside the originals or into a folder.",
         "inputSchema": {"type": "object", "properties": {"paths": paths, "path": path, "folder": {"type": "string"}, "embed_original": {"type": "boolean"}}}},
    ])
}

const SETTING_NAMES: [&str; 31] = [
    "temperature", "tint", "exposure", "contrast", "highlights", "shadows", "whites", "blacks",
    "texture", "clarity", "dehaze", "vibrance", "saturation", "sharpening", "sharpen_radius",
    "sharpen_detail", "sharpen_masking", "profile", "curve", "curve_highlights", "curve_lights",
    "curve_darks", "curve_shadows", "hsl_hue", "hsl_saturation", "hsl_luminance", "black_and_white",
    "mono_mix", "crop", "straighten", "lens_correction",
];

const COLOUR_KEYS: [&str; 8] = ["red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta"];

/// Contrast is stored as a factor; the slider shows Lightroom's -100..100.
fn contrast_to_slider(factor: f32) -> f32 {
    // Rounded: the factor is stored, so the slider value comes back with float noise.
    ((crate::CONTRAST_UNITS * factor.max(0.01).log2()).clamp(-100.0, 100.0) * 100.0).round() / 100.0
}

fn blank() -> Saved {
    Saved {
        values: [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        quarter_turns: 255,
        profile: String::new(),
        denoise: false,
        crop: crate::crop::FULL,
        angle: 0.0,
        sharpen: settings::SHARPEN_DEFAULT,
        tint: 0.0,
        presence: [0.0; 3],
        lens: crate::lens::LensSettings::default(),
        spots: Vec::new(),
        curve: crate::tone::identity_curve(),
        regions: [0.0; 4],
        hsl: [[0.0; 8]; 3],
        mono: false,
        mono_mix: [0.0; 8],
    }
}

/// A photo's settings as JSON, on the sliders' own scales.
fn to_json(s: &Saved) -> Value {
    let colours = |row: &[f32; 8]| -> Value { COLOUR_KEYS.iter().zip(row).map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>().into() };
    json!({
        "temperature": s.values[0], "tint": s.tint,
        "exposure": s.values[1], "contrast": contrast_to_slider(s.values[2]),
        "highlights": s.values[6], "shadows": s.values[7], "whites": s.values[5], "blacks": s.values[8],
        "texture": s.presence[0], "clarity": s.presence[1], "dehaze": s.presence[2],
        "vibrance": s.values[4], "saturation": s.values[3],
        "sharpening": s.sharpen[0], "sharpen_radius": s.sharpen[1], "sharpen_detail": s.sharpen[2], "sharpen_masking": s.sharpen[3],
        "profile": s.profile,
        "curve": s.curve, "curve_highlights": s.regions[0], "curve_lights": s.regions[1], "curve_darks": s.regions[2], "curve_shadows": s.regions[3],
        "hsl_hue": colours(&s.hsl[0]), "hsl_saturation": colours(&s.hsl[1]), "hsl_luminance": colours(&s.hsl[2]),
        "black_and_white": s.mono, "mono_mix": colours(&s.mono_mix),
        "crop": s.crop, "straighten": s.angle,
        "lens_correction": s.lens.enabled,
        "retouch_spots": s.spots.len(),
        "ai_denoise": s.denoise,
    })
}

/// Apply the named settings in `changes` to `s`, checking names and ranges.
fn apply_json(s: &mut Saved, changes: &Value) -> Result<(), String> {
    let map = changes.as_object().ok_or("settings must be an object")?;
    let num = |key: &str, v: &Value, lo: f32, hi: f32| -> Result<f32, String> {
        let x = v.as_f64().ok_or_else(|| format!("{key} must be a number"))? as f32;
        Ok(x.clamp(lo, hi))
    };
    let colours = |key: &str, v: &Value, row: &mut [f32; 8]| -> Result<(), String> {
        let m = v.as_object().ok_or_else(|| format!("{key} must be an object keyed by colour"))?;
        for (name, value) in m {
            let i = COLOUR_KEYS.iter().position(|c| c.eq_ignore_ascii_case(name)).ok_or_else(|| format!("{key}: unknown colour {name}"))?;
            row[i] = value.as_f64().ok_or_else(|| format!("{key}.{name} must be a number"))?.clamp(-100.0, 100.0) as f32;
        }
        Ok(())
    };
    for (key, v) in map {
        match key.as_str() {
            "temperature" => s.values[0] = if v.as_f64() == Some(0.0) { 0.0 } else { num(key, v, crate::color::KELVIN_MIN, crate::color::KELVIN_MAX)? },
            "tint" => s.tint = num(key, v, -150.0, 150.0)?,
            "exposure" => s.values[1] = num(key, v, -5.0, 5.0)?,
            "contrast" => s.values[2] = (num(key, v, -100.0, 100.0)? / crate::CONTRAST_UNITS).exp2(),
            "saturation" => s.values[3] = num(key, v, -100.0, 100.0)?,
            "vibrance" => s.values[4] = num(key, v, -100.0, 100.0)?,
            "whites" => s.values[5] = num(key, v, -100.0, 100.0)?,
            "highlights" => s.values[6] = num(key, v, -100.0, 100.0)?,
            "shadows" => s.values[7] = num(key, v, -100.0, 100.0)?,
            "blacks" => s.values[8] = num(key, v, -100.0, 100.0)?,
            "texture" => s.presence[0] = num(key, v, -100.0, 100.0)?,
            "clarity" => s.presence[1] = num(key, v, -100.0, 100.0)?,
            "dehaze" => s.presence[2] = num(key, v, -100.0, 100.0)?,
            "sharpening" => s.sharpen[0] = num(key, v, 0.0, 150.0)?,
            "sharpen_radius" => s.sharpen[1] = num(key, v, 0.5, 3.0)?,
            "sharpen_detail" => s.sharpen[2] = num(key, v, 0.0, 100.0)?,
            "sharpen_masking" => s.sharpen[3] = num(key, v, 0.0, 100.0)?,
            "profile" => s.profile = v.as_str().ok_or("profile must be a name")?.to_owned(),
            "curve" => {
                let points: Vec<[f32; 2]> = v
                    .as_array()
                    .ok_or("curve must be a list of [x, y] points")?
                    .iter()
                    .filter_map(|p| Some([p.get(0)?.as_f64()? as f32, p.get(1)?.as_f64()? as f32]))
                    .collect();
                s.curve = crate::tone::curve_from_field(&crate::tone::curve_to_field(&points));
            }
            "curve_highlights" => s.regions[0] = num(key, v, -100.0, 100.0)?,
            "curve_lights" => s.regions[1] = num(key, v, -100.0, 100.0)?,
            "curve_darks" => s.regions[2] = num(key, v, -100.0, 100.0)?,
            "curve_shadows" => s.regions[3] = num(key, v, -100.0, 100.0)?,
            "hsl_hue" => colours(key, v, &mut s.hsl[0])?,
            "hsl_saturation" => colours(key, v, &mut s.hsl[1])?,
            "hsl_luminance" => colours(key, v, &mut s.hsl[2])?,
            "black_and_white" => s.mono = v.as_bool().ok_or("black_and_white must be true or false")?,
            "mono_mix" => colours(key, v, &mut s.mono_mix)?,
            "crop" => {
                let c: Vec<f32> = v.as_array().ok_or("crop must be [left, top, right, bottom]")?.iter().filter_map(|x| x.as_f64().map(|x| x as f32)).collect();
                if c.len() != 4 || c[2] <= c[0] || c[3] <= c[1] {
                    return Err("crop must be [left, top, right, bottom] with right > left and bottom > top".into());
                }
                s.crop = [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0), c[3].clamp(0.0, 1.0)];
            }
            "straighten" => s.angle = num(key, v, -45.0, 45.0)?,
            "lens_correction" => s.lens.enabled = v.as_bool().ok_or("lens_correction must be true or false")?,
            other => return Err(format!("Unknown setting {other}. Known: {}", SETTING_NAMES.join(", "))),
        }
    }
    Ok(())
}

/// "17 Nov 14:32 UTC": when an AI version was made.
fn stamp(seconds: u64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (_, m, d) = crate::civil((seconds / 86_400) as i64);
    let t = seconds % 86_400;
    format!("{d} {} {:02}:{:02} UTC", MONTHS[(m - 1) as usize], t / 3600, t / 60 % 60)
}

/// "AI · Claude: exposure, shadows · 17 Nov 14:32 UTC", fitted to the 60 characters a version
/// name may have (the app and what changed are shortened; the prefix and time always stay).
fn ai_label(client: &str, summary: &str, stamp: &str) -> String {
    let tail = format!(" · {stamp}");
    let room = 60usize.saturating_sub(AI_PREFIX.chars().count() + tail.chars().count());
    let mut body: String = format!("{client}: {summary}").chars().take(room).collect();
    if format!("{client}: {summary}").chars().count() > room && room > 1 {
        body = body.chars().take(room - 1).collect::<String>() + "…";
    }
    format!("{AI_PREFIX}{body}{tail}")
}

/// The main settings of an edit in one line, beside the measurements, so the numbers can be
/// related to what was changed.
fn settings_summary(saved: Option<&Saved>) -> Value {
    let Some(s) = saved else { return json!("unedited (Develop's defaults)") };
    let mut parts = Vec::new();
    let names = [(1, "exposure"), (6, "highlights"), (7, "shadows"), (5, "whites"), (8, "blacks"), (4, "vibrance"), (3, "saturation")];
    for (i, name) in names {
        if s.values[i].abs() > 1e-3 {
            parts.push(format!("{name} {}", round_setting(s.values[i])));
        }
    }
    let contrast = contrast_to_slider(s.values[2]);
    if contrast.abs() > 0.05 {
        parts.insert(1, format!("contrast {}", round_setting(contrast)));
    }
    if s.values[0] > 0.0 {
        parts.insert(0, format!("temperature {} K", s.values[0].round()));
    }
    if s.tint.abs() > 1e-3 {
        parts.insert(1.min(parts.len()), format!("tint {}", round_setting(s.tint)));
    }
    for (v, name) in s.presence.iter().zip(["texture", "clarity", "dehaze"]) {
        if v.abs() > 1e-3 {
            parts.push(format!("{name} {}", round_setting(*v)));
        }
    }
    if s.mono {
        parts.push("black & white".into());
    }
    if parts.is_empty() {
        parts.push("only non-slider changes (crop, curve, mixer, retouch...)".into());
    }
    json!(parts.join(", "))
}

fn round_setting(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

fn is_picture(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "tif" | "tiff" | "heic")
}

/// The app's thumbnail of a photo shows its old edit; drop it so it is drawn again.
fn forget_thumbnail(path: &Path) {
    if let Some(file) = settings::thumb_file(path) {
        let _ = std::fs::remove_file(file);
    }
}

fn free_name(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.{ext}"));
        n += 1;
    }
    path
}

/// Render `photo` with its saved settings into `out` by running Develop itself.
/// Render a photo with its edit (PNG, so clipping and the darkest and brightest levels are exact)
/// and measure it. With `focus`, the same render also measures focus on the full-size picture
/// (see `focus.rs`): the report gains `focus`, and the 100% crop of the sharpest area is returned
/// as JPEG bytes. `slot` keeps the temporary files of parallel renders apart.
/// Run Develop's own auto white balance on a photo with its edit (see autowb.rs).
fn auto_wb(photo: &Path, slot: usize) -> Result<Value, String> {
    let out = std::env::temp_dir().join(format!("develop-mcp-autowb-{}-{slot}.json", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let status = std::process::Command::new(exe)
        .arg(photo)
        .args(["--with-settings", "1", "--auto-wb"])
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("Could not start Develop: {e}"))?;
    let text = std::fs::read_to_string(&out).map_err(|_| format!("Measuring {} failed ({status})", photo.display()))?;
    let _ = std::fs::remove_file(&out);
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn measure_photo(path: &Path, long_edge: u64, slot: usize, focus: bool, skin: bool) -> Result<(Value, Option<Vec<u8>>), String> {
    let out = std::env::temp_dir().join(format!("develop-mcp-check-{}-{slot}.png", std::process::id()));
    let focus_out = std::env::temp_dir().join(format!("develop-mcp-focus-{}-{slot}.json", std::process::id()));
    let info_out = std::env::temp_dir().join(format!("develop-mcp-info-{}-{slot}.json", std::process::id()));
    let crop_out = crate::focus::crop_path(&focus_out);
    let _ = std::fs::remove_file(&focus_out);
    let _ = std::fs::remove_file(&info_out);
    let mut options = vec![("--export-long", long_edge.to_string()), ("--info-out", info_out.display().to_string())];
    if focus {
        options.push(("--focus-out", focus_out.display().to_string()));
    }
    let rendered = render(path, &out, &options);
    let picture = rendered.and_then(|_| image::open(&out).map_err(|e| format!("Could not read the render: {e}")));
    let _ = std::fs::remove_file(&out);
    let rgb = picture?.to_rgb8();
    let mut report = crate::measure::measure(rgb.as_raw(), rgb.width() as usize, rgb.height() as usize);
    // With `skin`, the skin tone of any faces (see skin.rs), for white balance without neutrals.
    // Skin colour is judged with the faces at 20%: if they are not, the photo is rendered again
    // with its exposure shifted for the measurement only (the edit is not changed).
    if skin {
        let mut reading = crate::skin::measure(rgb.as_raw(), rgb.width() as usize, rgb.height() as usize);
        let mut shift = 0.0;
        for _ in 0..2 {
            let Some(step) = reading["exposure_to_target_stops"].as_f64() else { break };
            if step.abs() < 0.1 {
                break;
            }
            shift = (shift + step).clamp(-5.0, 5.0);
            let out = std::env::temp_dir().join(format!("develop-mcp-skin-{}-{slot}.png", std::process::id()));
            let shifted = render(path, &out, &[("--export-long", long_edge.to_string()), ("--exposure-shift", format!("{shift:.2}"))])
                .and_then(|_| image::open(&out).map_err(|e| e.to_string()));
            let _ = std::fs::remove_file(&out);
            let Ok(shifted) = shifted else { break };
            let shifted = shifted.to_rgb8();
            reading = crate::skin::measure(shifted.as_raw(), shifted.width() as usize, shifted.height() as usize);
        }
        if let Some(obj) = reading.as_object_mut() {
            obj.remove("exposure_to_target_stops");
            obj.insert("measured_with_exposure_shift_stops".into(), json!((shift * 100.0_f64).round() / 100.0));
        }
        report["skin"] = reading;
    }
    // The photo's own white balance, so a temperature change can be made relative to it.
    if let Some(info) = std::fs::read_to_string(&info_out).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        report["white_balance"] = json!({
            "as_shot_temperature": info["as_shot_temperature"],
            "temperature_now": info["temperature"],
            "how_to_read": "Kelvin, as in Lightroom: a higher temperature makes the picture warmer (more yellow), a lower one cooler (bluer). temperature in set_settings is absolute: to warm the photo by 300 K, set as_shot_temperature + 300 (or temperature_now + 300 to build on the current edit); 0 means as shot. tint is relative to the as-shot white balance already (0 = as shot; positive = more magenta, negative = greener). JPEG and other finished pictures start at about 6500 K.",
        });
    }
    let _ = std::fs::remove_file(&info_out);
    let mut crop = None;
    if focus {
        report["focus"] = std::fs::read_to_string(&focus_out)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| json!("focus could not be measured"));
        crop = std::fs::read(&crop_out).ok();
        let _ = std::fs::remove_file(&focus_out);
        let _ = std::fs::remove_file(&crop_out);
    }
    Ok((report, crop))
}

fn render(photo: &Path, out: &Path, options: &[(&str, String)]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = std::process::Command::new(exe);
    command.arg(photo).arg("--export").arg(out).arg("--with-settings").arg("1");
    for (name, value) in options {
        command.arg(name).arg(value);
    }
    let status = command.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map_err(|e| format!("Could not start Develop: {e}"))?;
    if !status.success() || !out.is_file() {
        return Err(format!("Rendering {} failed", photo.display()));
    }
    Ok(())
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[test]
    fn settings_round_trip() {
        let mut s = blank();
        apply_json(&mut s, &json!({"exposure": 0.7, "contrast": 25, "hsl_saturation": {"blue": -40}, "black_and_white": true, "curve": [[0, 0], [0.5, 0.6], [1, 1]]})).unwrap();
        let j = to_json(&s);
        assert!((j["exposure"].as_f64().unwrap() - 0.7).abs() < 1e-6);
        assert!((j["contrast"].as_f64().unwrap() - 25.0).abs() < 1e-3);
        assert_eq!(j["hsl_saturation"]["blue"].as_f64().unwrap(), -40.0);
        assert_eq!(j["black_and_white"], json!(true));
        assert!(apply_json(&mut s, &json!({"nonsense": 1})).is_err());
    }

    #[test]
    fn ai_labels_fit() {
        let label = ai_label("A very long assistant application name", "exposure, shadows, highlights, whites, blacks", "17 Nov 14:32 UTC");
        assert!(label.starts_with(AI_PREFIX));
        assert!(label.ends_with(" · 17 Nov 14:32 UTC"));
        assert!(label.chars().count() <= 60);
        assert_eq!(settings::clean_name(&label), label);
        assert_eq!(stamp(1_763_390_000), "17 Nov 14:33 UTC");
    }

    #[test]
    fn initialize_and_list() {
        let mut server = Server { store: settings::Store::load(), presets: settings::Presets::load(), client: "test".into(), rounds: Default::default(), last_call: None, focus_cache: None };
        let reply: Value = serde_json::from_str(&server.handle_text(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#).unwrap()).unwrap();
        assert_eq!(reply["result"]["serverInfo"]["name"], "develop");
        assert!(server.handle_text(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        let tools: Value = serde_json::from_str(&server.handle_text(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap()).unwrap();
        assert!(tools["result"]["tools"].as_array().unwrap().len() >= 14);
    }
}
