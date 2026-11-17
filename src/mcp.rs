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
        Self { store: settings::Store::load(), presets: settings::Presets::load(), client: "assistant".into(), rounds: Default::default(), last_call: None }
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
        let text = |v: Value| Ok(vec![json!({"type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default()})]);
        match name {
            "list_photos" => {
                let folder = PathBuf::from(args["folder"].as_str().ok_or("folder is required")?);
                if !folder.is_dir() {
                    return Err(format!("{} is not a folder", folder.display()));
                }
                self.store.load_folder(&folder);
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
                // PNG, so clipping and the darkest and brightest levels are exact.
                let out = std::env::temp_dir().join(format!("develop-mcp-check-{}.png", std::process::id()));
                render(&path, &out, &[("--export-long", long_edge.to_string())])?;
                let picture = image::open(&out).map_err(|e| format!("Could not read the render: {e}"));
                let _ = std::fs::remove_file(&out);
                let rgb = picture?.to_rgb8();
                let mut report = crate::measure::measure(rgb.as_raw(), rgb.width() as usize, rgb.height() as usize);
                report["path"] = json!(path.display().to_string());
                report["settings_summary"] = settings_summary(self.store.get(&path));
                text(report)
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
        }
        Ok(list)
    }
}

const INSTRUCTIONS: &str = "Develop is a raw photo editor. Photos are files on disk; each photo's edit lives in a .dvlp file beside it. \
Your changes to a photo are saved automatically in one version per photo per request, labelled 'AI · ...' (it is updated as you keep working on the request; pass new_version: true to set_settings or apply_preset to start a separate version, e.g. for an alternative look such as black and white), and the user's own edit is kept first as 'Before AI · ...'; other versions are never changed. Use list_photos to see a folder, get_settings to read an edit, set_settings to change it (only the settings you name change), \
render_preview to look at the result (or check_photo to measure it with numbers: histogram, clipping, colour cast; use it if you cannot see images), and export to write JPEG, PNG or TIFF files. Slider scales follow Lightroom's: exposure in stops (-5..5), \
most other sliders -100..100, temperature in kelvin (0 = as shot), tint -150..150. Tell the user which AI versions you made. \
If Develop is open on the same folder, it shows changes made here when the photo is opened again.";

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
        let mut server = Server { store: settings::Store::load(), presets: settings::Presets::load(), client: "test".into(), rounds: Default::default(), last_call: None };
        let reply: Value = serde_json::from_str(&server.handle_text(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#).unwrap()).unwrap();
        assert_eq!(reply["result"]["serverInfo"]["name"], "develop");
        assert!(server.handle_text(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        let tools: Value = serde_json::from_str(&server.handle_text(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap()).unwrap();
        assert!(tools["result"]["tools"].as_array().unwrap().len() >= 14);
    }
}
