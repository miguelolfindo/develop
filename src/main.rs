//! Develop: a GPU raw photo processor.
//!
//! Stage 1 (raw.rs, background thread, once per file): decode -> normalise -> demosaic -> GPU texture.
//! Stage 2 (shader.wgsl, every frame): white balance, exposure, tone, contrast, saturation, sRGB.
//! Moving a slider only rewrites a ~150-byte uniform buffer.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod color;
mod autowb;
mod crop;
mod cull;
mod dcp;
mod denoise;
mod explorer;
mod export;
mod filmstrip;
mod focus;
mod headless;
mod heic;
mod lens;
mod mcp;
mod measure;
mod network;
mod pipeline;
mod profile;
mod raw;
mod scopes;
mod settings;
mod skin;
mod spots;
mod theme;
mod tone;
mod todng;
mod ui;
mod update;
mod watch;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Instant;

use eframe::{egui, egui_wgpu, wgpu};

use color::{CameraColor, KELVIN_DEFAULT, KELVIN_MAX, KELVIN_MIN};
use pipeline::{ImageCallback, ImageRenderer, Uniforms};
use raw::DecodedImage;

fn main() -> eframe::Result<()> {
    // Diagnostic: `app --dump-dcp <profile.dcp> <out.txt>` writes a summary of a camera profile.
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "--dump-dcp" {
        let text = match dcp::load(std::path::Path::new(&args[2])) {
            Ok(profile) => dcp::describe(&profile),
            Err(e) => e,
        };
        let _ = std::fs::write(&args[3], text);
        return Ok(());
    }

    // `app --mcp`: an MCP server on stdin / stdout for AI assistants (see mcp.rs).
    // `app --mcp-http <port> [--mcp-token <token>]`: the same over HTTP on this computer.
    if args.len() >= 2 && args[1] == "--mcp" {
        mcp::serve_stdio();
        return Ok(());
    }
    if args.len() >= 3 && args[1] == "--mcp-http" {
        #[cfg(windows)]
        unsafe {
            // A windowed program has no console: use the terminal it was started from.
            let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
        }
        let port = args[2].parse().unwrap_or(8765);
        let token = args.iter().position(|a| a == "--mcp-token").and_then(|i| args.get(i + 1)).cloned();
        mcp::serve_http(port, token);
        return Ok(());
    }

    // `app --to-dng <folder> [--embed] <raw files...>`: convert to DNG and quit (scripted use
    // and tests). Prints one line per file.
    if args.len() >= 4 && args[1] == "--to-dng" {
        let dir = PathBuf::from(&args[2]);
        let embed = args.iter().any(|a| a == "--embed");
        for file in args[3..].iter().filter(|a| *a != "--embed") {
            match todng::convert_one(std::path::Path::new(file), &dir, embed, None) {
                Ok(out) => println!("{file} -> {}
    {}", out.display(), todng::describe(std::path::Path::new(file))),
                Err(e) => println!("{file}: {e}"),
            }
        }
        return Ok(());
    }
    // Diagnostic: `app --dump-network <out.txt>` lists what the folder tree's Network entry
    // finds: mapped drives, computers and each computer's shares.
    if args.len() >= 3 && args[1] == "--dump-network" {
        let saved = settings::NetworkPlaces::load().list;
        let mut text = String::new();
        for letter in 'A'..='Z' {
            let drive = PathBuf::from(format!("{letter}:\\"));
            if let Some(share) = network::drive_share(&drive) {
                text += &format!("drive {letter}: -> {share}\n");
            }
        }
        for computer in network::computers(&saved) {
            text += &format!("computer {} ({})\n", computer.label, computer.path.display());
            if let Some(host) = network::host_of_entry(&computer.path) {
                match network::shares(&host, &saved) {
                    Ok(shares) => {
                        for share in shares {
                            let folders = std::fs::read_dir(&share.path).map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).count());
                            text += &format!("    share {} ({}) -> {}\n", share.label, share.path.display(), folders.map_or_else(|e| format!("not readable: {e}"), |n| format!("{n} folders")));
                        }
                    }
                    Err(e) => text += &format!("    {e}\n"),
                }
            }
        }
        let _ = std::fs::write(&args[2], text);
        return Ok(());
    }
    if args.len() >= 4 && args[1] == "--dump-raw" {
        let _ = std::fs::write(&args[3], raw::describe(std::path::Path::new(&args[2])));
        return Ok(());
    }

    // `app <photo> --export <file> ...` and `--auto-wb <file>` (how the MCP server renders and
    // measures): no window at all, so
    // nothing flashes up on screen while an assistant works. Falls back to a window if the GPU
    // cannot be reached without one.
    let scripted = args.len() >= 4 && (args[2] == "--export" || args.iter().any(|a| a == "--auto-wb"));
    if scripted && headless::run(std::env::args_os().nth(1).map(PathBuf::from)) {
        return Ok(());
    }

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("Develop")
            .with_icon(app_icon())
            .with_inner_size([1600.0, 1000.0])
            .with_min_inner_size([900.0, 600.0])
            .with_drag_and_drop(true),
        wgpu_options: egui_wgpu::WgpuConfiguration {
            // A 61 MP frame is 9504 px wide; wgpu's default limit is 8192. Ask for what the GPU has.
            device_descriptor: Arc::new(|adapter| {
                let base = if adapter.get_info().backend == wgpu::Backend::Gl {
                    wgpu::Limits::downlevel_webgl2_defaults()
                } else {
                    wgpu::Limits::default()
                };
                wgpu::DeviceDescriptor {
                    label: Some("develop device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits {
                        max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d,
                        ..base
                    },
                }
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    eframe::run_native(
        "Develop",
        options,
        Box::new(|cc| Box::new(App::new(cc, std::env::args_os().nth(1).map(PathBuf::from)))),
    )
}

/// The window icon (also the taskbar icon while the app runs).
fn app_icon() -> egui::IconData {
    match image::load_from_memory(include_bytes!("../assets/icon-256.png")) {
        Ok(icon) => {
            let rgba = icon.to_rgba8();
            egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
        }
        Err(_) => egui::IconData::default(),
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Adjust {
    kelvin: f32,
    exposure: f32,
    contrast: f32,
    saturation: f32,
    vibrance: f32,
    whites: f32,
    highlights: f32,
    shadows: f32,
    blacks: f32,
}

impl Default for Adjust {
    fn default() -> Self {
        Self {
            kelvin: KELVIN_DEFAULT,
            exposure: 0.0,
            contrast: 1.0,
            saturation: 0.0,
            vibrance: 0.0,
            whites: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            blacks: 0.0,
        }
    }
}

impl Adjust {
    fn to_array(self) -> [f32; settings::VALUES] {
        [
            self.kelvin,
            self.exposure,
            self.contrast,
            self.saturation,
            self.vibrance,
            self.whites,
            self.highlights,
            self.shadows,
            self.blacks,
        ]
    }

    fn from_array(v: [f32; settings::VALUES]) -> Self {
        Self {
            kelvin: v[0].clamp(KELVIN_MIN, KELVIN_MAX),
            exposure: v[1].clamp(-5.0, 5.0),
            contrast: v[2].clamp(0.5, 2.0),
            saturation: v[3].clamp(-100.0, 100.0),
            vibrance: v[4].clamp(-100.0, 100.0),
            whites: v[5].clamp(-100.0, 100.0),
            highlights: v[6].clamp(-100.0, 100.0),
            shadows: v[7].clamp(-100.0, 100.0),
            blacks: v[8].clamp(-100.0, 100.0),
        }
    }
}

/// The base rendering applied underneath the sliders.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Profile {
    /// Scene-linear straight to the display: flat and dark, nothing added.
    Linear,
    /// Built-in base tone curve fitted to a raw converter's default look.
    Standard,
    /// An Adobe camera profile found on this computer (index into `App::dcps`).
    Dcp(usize),
}

/// Baseline exposure (stops) Adobe applies to this camera before the profile's tone curve.
/// It is not stored in the raw file or in the .dcp, so it is a constant here: +0.35 matches
/// Lightroom's sky tones on the reference file and is exactly what the camera-matching
/// profiles cancel with their -0.35 offset. `--baseline` overrides it for testing.
const BASELINE_EXPOSURE: f32 = 0.35;

/// Unsharp-mask strength per unit of the Amount slider, fitted to Lightroom exports.
const SHARPEN_SCALE: f32 = 0.0295;

/// Extra baseline exposure (stops) that Lightroom gives some cameras' raw files beyond the
/// common one (Adobe keeps a per-model value). Measured on Lightroom exports of blank
/// renders; models not listed get 0, which matched the Sony, Nikon, Canon and Fujifilm files
/// tried so far.
const MODEL_BASELINE: [(&str, &str, f32); 3] = [("panasonic", "dc-g9", 0.33), ("panasonic", "dc-gh6", -0.27), ("panasonic", "dc-s5", -0.3)];

fn maker_baseline(make: &str, model: &str) -> f32 {
    let (make, model) = (make.to_lowercase(), model.to_lowercase());
    MODEL_BASELINE.iter().find(|(mk, md, _)| make.starts_with(mk) && model == *md).map_or(0.0, |&(_, _, ev)| ev)
}

/// Open the system file browser at `path`: the file selected, or the folder itself.
fn reveal_in_folder(path: &Path) {
    let result = if cfg!(windows) {
        if path.is_file() {
            std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn()
        } else {
            std::process::Command::new("explorer").arg(path).spawn()
        }
    } else if cfg!(target_os = "macos") {
        if path.is_file() {
            std::process::Command::new("open").arg("-R").arg(path).spawn()
        } else {
            std::process::Command::new("open").arg(path).spawn()
        }
    } else {
        std::process::Command::new("xdg-open").arg(path.parent().filter(|_| path.is_file()).unwrap_or(path)).spawn()
    };
    let _ = result;
}

/// Display levels assumed before a photo has been measured (Uniforms::scene).
const SCENE_DEFAULT: [f32; 4] = [0.2, 0.9, 0.3, 0.0];

/// Contrast slider units per doubling of the contrast factor: Lightroom's Contrast +100
/// matches a factor of 2^0.5 (fitted to Lightroom exports of DLZ_5063.NEF).
const CONTRAST_UNITS: f32 = 200.0;

/// Contrast that turns Adobe Standard into an approximation of Lightroom's "Adobe Color":
/// fitted to shadow and sky patches of Lightroom exports of the reference file.
const ADOBE_COLOR_CONTRAST: f32 = 1.18;

/// Adobe camera profiles installed for `camera` (maker and model, as Adobe names them), Adobe
/// Standard first, then the camera-matching ones.
fn find_camera_profiles(camera: &str) -> Vec<(String, dcp::Dcp, f32)> {
    let mut roots = Vec::new();
    if let Some(program_data) = std::env::var_os("ProgramData") {
        roots.push(PathBuf::from(program_data).join("Adobe/CameraRaw/CameraProfiles"));
    }
    roots.push(PathBuf::from("/Library/Application Support/Adobe/CameraRaw/CameraProfiles"));

    let mut files = Vec::new();
    for root in roots {
        files.push(root.join("Adobe Standard").join(format!("{camera} Adobe Standard.dcp")));
        if let Ok(dir) = std::fs::read_dir(root.join("Camera").join(camera)) {
            let mut matching: Vec<PathBuf> = dir
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dcp")))
                .collect();
            matching.sort();
            files.extend(matching);
        }
    }

    let mut found = Vec::new();
    for file in files {
        if let Ok(profile) = dcp::load(&file) {
            let label = if profile.name.is_empty() {
                file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
            } else {
                profile.name.clone()
            };
            // Lightroom's default "Adobe Color" is Adobe Standard plus a look that is not a
            // loadable file. Offer an approximation of it first: the same profile with the
            // extra contrast measured from Lightroom exports.
            if label == "Adobe Standard" {
                found.push(("Adobe Color (approx.)".to_owned(), profile.clone(), ADOBE_COLOR_CONTRAST));
            }
            found.push((label, profile, 1.0));
        }
    }
    found
}

struct Loaded {
    name: String,
    /// Longer details, shown on hover.
    info: String,
    /// For the header and the telemetry bar.
    camera: String,
    ext: String,
    full_size: (u32, u32),
    bits: u8,
    seconds: f32,
    /// Camera and lens for lens correction, and the camera's own embedded corrections.
    exif: lens::Exif,
    embedded: Option<lens::Embedded>,
    width: f32,
    height: f32,
    color: CameraColor,
    /// The raw file on disk.
    path: PathBuf,
    /// Shot details to write into exports.
    photo: export::PhotoMeta,
}

/// An export of several photos: each is opened in turn, exported with its own remembered
/// settings, and the photo that was open is brought back at the end.
struct Batch {
    queue: std::collections::VecDeque<PathBuf>,
    dir: PathBuf,
    total: usize,
    done: usize,
    failed: Vec<String>,
    /// The photo being worked on, and whether its export has been started.
    current: Option<PathBuf>,
    exporting: bool,
    return_to: Option<PathBuf>,
    /// Close the app when finished (command-line batch).
    quit: bool,
}

/// `stem.ext` in `dir`, or `stem-2.ext`, `stem-3.ext`... if that name is taken.
fn free_name(dir: &std::path::Path, stem: &str, ext: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.{ext}"));
        n += 1;
    }
    path
}

/// A drag in progress in crop mode: which part of the frame (none = rotating), and how
/// things stood when it began.
#[derive(Clone, Copy)]
struct CropDrag {
    handle: Option<crop::Handle>,
    start: crop::Rect,
    origin: egui::Pos2,
    angle: f32,
}

type LoadResult = Result<(DecodedImage, PathBuf, f32), String>;

/// One photo's edit history: the settings before each change (undo), the ones undone
/// (redo), and the settings last recorded.
#[derive(Default)]
struct History {
    undo: Vec<settings::Saved>,
    redo: Vec<settings::Saved>,
    committed: Option<settings::Saved>,
}

/// Smallest and largest spot radius, as fractions of the picture height (the Size slider
/// shows thousandths).
const SPOT_MIN: f32 = 0.002;
const SPOT_MAX: f32 = 0.2;

/// How long the note "Changed by another program" shows.
const OUTSIDE_NOTE_TIME: std::time::Duration = std::time::Duration::from_secs(10);
/// Steps of undo kept per photo.
const HISTORY_STEPS: usize = 200;

struct App {
    /// Undo / redo, per photo, for as long as the app is open.
    history: std::collections::HashMap<PathBuf, History>,
    /// Update check on launch, the newer release offered, and its download.
    update_check: Option<Receiver<Result<Option<update::Release>, String>>>,
    update_offer: Option<update::Release>,
    update_download: Option<Receiver<update::Progress>>,
    update_fraction: f32,
    update_status: Option<String>,
    /// The open photo's display levels with the tone sliders at zero (`Uniforms::scene`), and
    /// the settings they were measured for.
    scene: std::cell::Cell<[f32; 4]>,
    scene2: std::cell::Cell<[f32; 4]>,
    scene_for: std::cell::RefCell<String>,
    render_state: egui_wgpu::RenderState,
    max_texture: u32,
    adjust: Adjust,
    loaded: Option<Loaded>,
    loading: Option<(String, Receiver<LoadResult>)>,
    /// The file being decoded, so the filmstrip can highlight it straight away.
    loading_path: Option<PathBuf>,
    /// Thumbnails of the open file's folder, along the bottom of the window.
    filmstrip: filmstrip::Filmstrip,
    /// Each photo's remembered settings, and the open photo's defaults (what it would show
    /// with nothing remembered). `remember` is off for command-line test runs.
    store: settings::Store,
    defaults: Option<settings::Saved>,
    remember: bool,
    /// Watches the filmstrip's folder for sidecars changed by another program (an AI
    /// assistant through the MCP server, another copy of Develop).
    folder_watch: Option<watch::FolderWatch>,
    /// Photos changed by another program, waiting to be taken in (the open photo waits while
    /// a slider is being dragged or a number typed).
    outside: std::collections::HashSet<PathBuf>,
    /// When the open photo was last changed by another program, for the note under its name.
    outside_note: Option<(PathBuf, Instant)>,
    /// The settings the open photo's filmstrip thumbnail currently shows.
    thumb_shows: Option<settings::Saved>,
    /// Settings copied with "Copy settings", in the form they are stored.
    clipboard: Option<settings::Saved>,
    /// What a right-click menu on the picture asked for, handled on the next frame.
    menu_action: Option<filmstrip::Action>,
    /// Last-used export settings on disk, the last export folder and the named presets.
    presets: export::Presets,
    preset_name: String,
    batch: Option<Batch>,
    /// Folder tree on the left, and the folder and photo to come back to on the next start.
    /// AI denoise: whether the open photo has it, a switch waiting to be applied on the next
    /// load (the checkbox, a paste, or `--denoise 1`), and a request to load the photo again.
    denoise: bool,
    denoise_override: Option<bool>,
    reload: bool,
    /// Zoom and position to restore after a reload (so a denoise switch can be compared).
    keep_view: Option<(Option<f32>, egui::Vec2)>,
    /// Whether the open file is a Bayer raw the AI denoiser can work on.
    can_denoise: bool,
    /// What kind of file is open (see `DecodedImage::input_mode`), and a DNG's own baseline
    /// exposure in stops.
    input_mode: u8,
    file_baseline: Option<f32>,
    /// Extra baseline exposure for the open photo's camera maker (see maker_baseline).
    maker_baseline: f32,
    /// Side panels shown ([ and ] toggle them), the A/B split and where its divider is
    /// (0..1 across the canvas), the clipping warning, and the small app icon in the top bar.
    show_left: bool,
    show_right: bool,
    ab_split: bool,
    ab_pos: f32,
    clipping: bool,
    /// Presence (texture, clarity, dehaze) and lens correction of the open photo, the Lensfun
    /// profiles found, the profile matched to the open photo, and the maker picked in the panel.
    presence: [f32; 3],
    /// Tone curve (point curve and parametric regions) and Color Mixer of the open photo, the
    /// mixer tab shown (0 hue, 1 saturation, 2 luminance), and the curve point being dragged.
    curve: Vec<[f32; 2]>,
    regions: [f32; 4],
    hsl: [[f32; 8]; 3],
    hsl_tab: usize,
    /// Black & white, and its mix (how bright each colour becomes, -100..100).
    mono: bool,
    mono_mix: [f32; 8],
    /// Develop presets, the name typed for a new one, and whether it takes white balance.
    develop_presets: settings::Presets,
    develop_preset_name: String,
    preset_wb: bool,
    /// The name typed for a new version, and a version shown as "before" in the A/B split.
    version_name: String,
    compare: Option<settings::Saved>,
    /// A version to switch to once its photo has opened (clicked in the filmstrip).
    pending_version: Option<(PathBuf, usize)>,
    curve_drag: Option<usize>,
    /// The white balance eyedropper is waiting for a click on the picture (W).
    wb_pick: bool,
    /// The last Auto white balance's result, shown under the buttons.
    auto_wb_note: Option<String>,
    /// `--auto-wb <file>`: find the white balance, write the result there as JSON, and quit
    /// (with `--with-settings 1`; for the MCP server).
    pending_auto_wb: Option<PathBuf>,
    /// Scroll the edit panel to the Tone Curve and Color Mixer on the next frame (`--show-curve`,
    /// for testing).
    curve_reveal: bool,
    /// Open the Presets and Versions sections on the next frame (`--open-sections`, testing).
    sections_reveal: bool,
    /// Command-line runs: apply the photo's saved settings (`--with-settings 1`, used by the
    /// MCP server's previews and exports) without remembering anything.
    with_settings: bool,
    lens: lens::LensSettings,
    lens_db: lens::Database,
    lens_auto: Option<usize>,
    lens_make_pick: Option<String>,
    icon: Option<egui::TextureHandle>,
    /// Crop of the open photo (on the picture as stored), the crop the user last asked for
    /// (rotating shrinks the real one to fit; rotating back restores this), and the
    /// straighten angle in degrees.
    /// Sharpening: amount (0-150), radius (px), detail (0-100), masking (0-100).
    sharpen: [f32; 4],
    /// Tint, -150 (green) to +150 (magenta), relative to the as-shot white balance.
    tint: f32,
    /// Strength per unit of Amount; fitted to Lightroom exports (`--sharpen-scale` overrides).
    sharpen_scale: f32,
    crop: crop::Rect,
    crop_wish: crop::Rect,
    angle: f32,
    crop_mode: bool,
    /// Locked ratio as (width, height), and its name in the Ratio list.
    crop_ratio: Option<(f32, f32)>,
    crop_ratio_label: String,
    crop_drag: Option<CropDrag>,
    crop_presets: settings::CropPresets,
    new_ratio: (f32, f32),
    new_ratio_name: String,
    explorer: explorer::Explorer,
    session: settings::Session,
    error: Option<String>,
    /// Screen pixels per image pixel; `None` = fit to window.
    zoom: Option<f32>,
    /// Image point (0..1, in display orientation) shown at the centre of the viewport.
    center: egui::Vec2,
    quarter_turns: u8,
    profile: Profile,
    /// Adobe camera profiles available for the open file's camera: (label, profile).
    dcps: Vec<(String, dcp::Dcp, f32)>,
    baseline_exposure: f32,
    /// What the shader is currently set up to render, and the (profile, temperature,
    /// baseline) it was built for.
    render: Option<profile::ProfileRender>,
    render_key: Option<(Profile, i32, u32)>,
    /// Export settings (kept while the app runs), whether the dialog is showing, the export
    /// in progress, and the outcome of the last one.
    export: export::Settings,
    export_open: bool,
    export_job: Option<Receiver<Result<String, String>>>,
    export_status: Option<Result<String, String>>,
    /// `--export <file>`: export as soon as the picture is ready, then quit.
    pending_export: Option<PathBuf>,
    quit_after_export: bool,
    /// Show the "Export complete" window for the export that just finished, and the file or
    /// folder it went to.
    export_popup: bool,
    export_last: Option<PathBuf>,
    /// A Convert to DNG run: its events, how far it is (done, total, current file), and the
    /// switch that stops it.
    dng_job: Option<Receiver<todng::Event>>,
    dng_progress: (usize, usize, String),
    dng_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// The export dialog's DNG choice (original raw data instead of a rendered picture), and
    /// whether to embed the original raw file in it.
    export_dng: bool,
    export_dng_embed: bool,
    /// Spot removal: the open photo's spots, the tool switched on (Q), the spot selected, the
    /// circle being dragged (spot, source rather than target, grab offset), circles hidden
    /// (H), the brush (size as a fraction of the picture height, feather, opacity, clone),
    /// and a small copy of the photo for choosing sources.
    spots: Vec<spots::Spot>,
    spot_tool: bool,
    spot_selected: Option<usize>,
    spot_drag: Option<(usize, bool, [f32; 2])>,
    /// The stroke being painted (texture coordinates).
    spot_paint: Option<Vec<[f32; 2]>>,
    /// Healing gains worked out for the Heal spots, by spot index (= atlas slot); see
    /// `sync_heal`.
    heal_grids: std::cell::RefCell<Vec<Option<spots::HealGrid>>>,
    /// Stroke outlines already traced (see `spot_edges`).
    spot_outlines: std::cell::RefCell<std::collections::HashMap<u64, std::rc::Rc<Vec<[[f32; 2]; 2]>>>>,
    spot_hide: bool,
    spot_brush: spots::Spot,
    spot_preview: spots::Preview,
    /// Open the Spot Removal section on the next frame (the tool was just switched on).
    spot_reveal: bool,
    /// `--name value` pairs from the command line, applied to the first file opened.
    overrides: Vec<(String, f32)>,
    /// `--focus-out <file>`: with `--export`, also measure focus on the full-size picture and
    /// write the report there (for the MCP server).
    focus_out: Option<PathBuf>,
    /// `--info-out <file>`: with `--export`, also write the photo's own (as-shot) white balance
    /// and the temperature used, as JSON (for the MCP server).
    info_out: Option<PathBuf>,
    /// `--exposure-shift <stops>`: added to the exposure after the saved edit is applied, for a
    /// measuring render only (the MCP server judges skin with the faces at 20%).
    exposure_shift: f32,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        let render_state = cc.wgpu_render_state.clone().expect("eframe must run with the wgpu renderer");
        Self::with_gpu(&cc.egui_ctx, render_state, initial)
    }

    /// The app on a GPU connection: a window's (see `new`), or one made without a window for
    /// exports (see headless.rs).
    fn with_gpu(egui_ctx: &egui::Context, render_state: egui_wgpu::RenderState, initial: Option<PathBuf>) -> Self {
        // Settings from versions that kept them under another folder name.
        settings::migrate_legacy_settings();
        theme::apply(egui_ctx);
        let renderer = ImageRenderer::new(&render_state.device, &render_state.queue, render_state.target_format);
        render_state.renderer.write().callback_resources.insert(renderer);
        let scopes = scopes::Scopes::new(&render_state.device, render_state.target_format);
        render_state.renderer.write().callback_resources.insert(scopes);
        let max_texture = render_state.device.limits().max_texture_dimension_2d;

        let mut app = Self {
            history: std::collections::HashMap::new(),
            update_check: None,
            update_offer: None,
            update_download: None,
            update_fraction: 0.0,
            update_status: None,
            scene: std::cell::Cell::new(SCENE_DEFAULT),
            scene2: std::cell::Cell::new([0.8, 0.0, 0.0, 0.0]),
            scene_for: std::cell::RefCell::new(String::new()),
            render_state,
            max_texture,
            adjust: Adjust::default(),
            loaded: None,
            loading: None,
            loading_path: None,
            filmstrip: filmstrip::Filmstrip::new(egui_ctx),
            store: settings::Store::load(),
            defaults: None,
            // `app file.ARW --something ...` is a test or batch run: start from defaults and
            // leave the remembered settings alone.
            remember: {
                let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
                args.len() <= 2 || (args.len() == 4 && args[2] == "--export-dialog")
            },
            folder_watch: None,
            outside: Default::default(),
            outside_note: None,
            thumb_shows: None,
            clipboard: None,
            menu_action: None,
            presets: export::Presets::disabled(),
            preset_name: String::new(),
            batch: None,
            denoise: false,
            denoise_override: {
                let args: Vec<String> = std::env::args().collect();
                args.iter().position(|a| a == "--denoise").and_then(|i| args.get(i + 1)).map(|v| v == "1")
            },
            reload: false,
            keep_view: None,
            can_denoise: true,
            input_mode: 0,
            file_baseline: None,
            maker_baseline: 0.0,
            show_left: true,
            show_right: true,
            ab_split: false,
            ab_pos: 0.5,
            clipping: false,
            presence: [0.0; 3],
            curve: tone::identity_curve(),
            regions: [0.0; 4],
            hsl: [[0.0; 8]; 3],
            hsl_tab: 1,
            mono: false,
            mono_mix: [0.0; 8],
            develop_presets: settings::Presets::load(),
            develop_preset_name: String::new(),
            preset_wb: false,
            version_name: String::new(),
            compare: None,
            pending_version: None,
            curve_drag: None,
            wb_pick: false,
            auto_wb_note: None,
            pending_auto_wb: {
                let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
                args.iter().position(|a| a == "--auto-wb").and_then(|i| args.get(i + 1)).map(PathBuf::from)
            },
            curve_reveal: false,
            sections_reveal: false,
            with_settings: false,
            lens: lens::LensSettings::default(),
            lens_db: lens::Database::load(),
            lens_auto: None,
            lens_make_pick: None,
            icon: image::load_from_memory(include_bytes!("../assets/icon-256.png")).ok().map(|icon| {
                let small = icon.resize(72, 72, image::imageops::FilterType::Lanczos3).to_rgba8();
                let pixels = egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw());
                egui_ctx.load_texture("app icon", pixels, egui::TextureOptions::LINEAR)
            }),
            sharpen: settings::SHARPEN_DEFAULT,
            tint: 0.0,
            sharpen_scale: SHARPEN_SCALE,
            crop: crop::FULL,
            crop_wish: crop::FULL,
            angle: 0.0,
            crop_mode: false,
            crop_ratio: None,
            crop_ratio_label: "Free".to_owned(),
            crop_drag: None,
            crop_presets: settings::CropPresets::load(),
            new_ratio: (4.0, 5.0),
            new_ratio_name: String::new(),
            explorer: explorer::Explorer::new(),
            session: settings::Session::load(),
            error: None,
            zoom: None,
            center: egui::vec2(0.5, 0.5),
            quarter_turns: 0,
            profile: Profile::Standard,
            dcps: Vec::new(),
            baseline_exposure: BASELINE_EXPOSURE,
            render: None,
            render_key: None,
            export: export::Settings::default(),
            export_open: false,
            export_job: None,
            export_status: None,
            pending_export: {
                let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
                args.iter().position(|a| a == "--export").and_then(|i| args.get(i + 1)).map(PathBuf::from)
            },
            quit_after_export: false,
            export_popup: false,
            export_last: None,
            dng_job: None,
            dng_progress: (0, 0, String::new()),
            dng_cancel: None,
            export_dng: false,
            export_dng_embed: false,
            spots: Vec::new(),
            spot_tool: false,
            spot_selected: None,
            spot_drag: None,
            spot_paint: None,
            spot_outlines: Default::default(),
            heal_grids: Default::default(),
            spot_hide: false,
            spot_brush: spots::Spot { x: 0.0, y: 0.0, sx: 0.0, sy: 0.0, path: Vec::new(), radius: 0.012, feather: 0.5, opacity: 1.0, clone: false },
            spot_preview: spots::Preview::default(),
            spot_reveal: false,
            overrides: {
                let args: Vec<String> = std::env::args().skip(2).collect();
                args.chunks_exact(2).filter_map(|p| Some((p[0].clone(), p[1].parse().ok()?))).collect()
            },
            focus_out: {
                let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
                args.iter().position(|a| a == "--focus-out").and_then(|i| args.get(i + 1)).map(PathBuf::from)
            },
            exposure_shift: {
                let args: Vec<String> = std::env::args().collect();
                args.iter().position(|a| a == "--exposure-shift").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(0.0)
            },
            info_out: {
                let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
                args.iter().position(|a| a == "--info-out").and_then(|i| args.get(i + 1)).map(PathBuf::from)
            },
        };
        if app.remember {
            let (presets, last) = export::Presets::load(settings::export_file());
            app.presets = presets;
            if let Some(last) = last {
                app.export = last;
            }
        }
        // Test / scripting: `app first.ARW [--export-long 800 ...] --batch-export <folder> more.ARW ...`
        // exports the first file and every further raw file named after the folder, then quits.
        let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
        if let (Some(i), Some(first)) = (args.iter().position(|a| a == "--batch-export"), &initial) {
            if let Some(dir) = args.get(i + 1) {
                let mut queue = std::collections::VecDeque::from([first.clone()]);
                queue.extend(
                    args[i + 2..].iter().map(PathBuf::from).filter(|p| filmstrip::is_raw(p)),
                );
                app.batch = Some(Batch {
                    total: queue.len(),
                    queue,
                    dir: PathBuf::from(dir),
                    done: 0,
                    failed: Vec::new(),
                    current: None,
                    exporting: false,
                    return_to: None,
                    quit: true,
                });
            }
        }
        if let Some(path) = initial {
            app.start_loading(path, egui_ctx);
        } else if app.remember {
            // Started without a file: come back to the photo and folder of the last session.
            if let Some(photo) = app.session.photo.clone().filter(|p| p.is_file()) {
                app.start_loading(photo, egui_ctx);
            }
            if let Some(folder) = app.session.folder.clone().filter(|p| p.is_dir()) {
                app.filmstrip.set_folder(Some(&folder));
            }
        }
        // Look for a newer release in the background, except in scripted runs
        // (`app file.ARW --export ...`) and when DEVELOP_NO_UPDATE_CHECK is set.
        let scripted = std::env::args().skip(1).any(|a| a.starts_with("--"));
        if !scripted && std::env::var_os("DEVELOP_NO_UPDATE_CHECK").is_none() {
            let (tx, rx) = mpsc::channel();
            let ctx = egui_ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(update::check());
                ctx.request_repaint();
            });
            app.update_check = Some(rx);
        }
        app
    }

    /// The "Update available" dialog: the new version's release notes, and downloading and
    /// installing it.
    fn update_dialog(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.update_check {
            if let Ok(result) = rx.try_recv() {
                self.update_check = None;
                if let Ok(Some(release)) = result {
                    if settings::skipped_update().as_deref() != Some(release.version.as_str()) {
                        self.update_offer = Some(release);
                    }
                }
            }
        }
        if let Some(rx) = &self.update_download {
            let mut finished = None;
            while let Ok(progress) = rx.try_recv() {
                match progress {
                    update::Progress::Downloading(fraction) => self.update_fraction = fraction,
                    other => finished = Some(other),
                }
            }
            match finished {
                Some(update::Progress::Ready(path)) => {
                    self.update_download = None;
                    match update::install(&path) {
                        // The installer (Windows) or the swap script (macOS) takes over once
                        // Develop has closed.
                        Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                        Err(message) => self.update_status = Some(message),
                    }
                }
                Some(update::Progress::Failed(message)) => {
                    self.update_download = None;
                    self.update_status = Some(message);
                }
                _ => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            }
        }
        let Some(release) = self.update_offer.clone() else { return };
        let (mut start, mut later, mut skip) = (false, false, false);
        egui::Window::new("Update available")
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(540.0)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(format!("Develop {} is available", release.version)).font(theme::bold(16.0)).color(theme::TEXT));
                ui.label(egui::RichText::new(format!("You have version {}. What's new:", update::current())).font(theme::font(12.5)).color(theme::FADED));
                ui.add_space(6.0);
                ui::hairline(ui);
                egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.add_space(4.0);
                    update::show_notes(ui, &release.notes);
                });
                ui::hairline(ui);
                ui.add_space(8.0);
                if let Some(status) = &self.update_status {
                    ui.label(egui::RichText::new(status).font(theme::font(12.0)).color(ui.visuals().error_fg_color));
                    ui.add_space(4.0);
                }
                if self.update_download.is_some() {
                    let bar = if self.update_fraction >= 0.0 { egui::ProgressBar::new(self.update_fraction).show_percentage() } else { egui::ProgressBar::new(0.0).animate(true) };
                    ui.add(bar.text(format!("Downloading {}...", release.asset_name)));
                } else {
                    ui.horizontal(|ui| {
                        if ui::button(ui, "Download and install", egui::vec2(0.0, 30.0), ui::Look::primary(), theme::bold(13.0), None)
                            .on_hover_text("Downloads the installer, then closes Develop to install it")
                            .clicked()
                        {
                            start = true;
                        }
                        if ui::button(ui, "Later", egui::vec2(0.0, 30.0), ui::Look::ghost(), theme::font(13.0), None).clicked() {
                            later = true;
                        }
                        if ui::button(ui, "Skip this version", egui::vec2(0.0, 30.0), ui::Look::ghost(), theme::font(13.0), None).clicked() {
                            skip = true;
                        }
                    });
                }
            });
        if start {
            self.update_status = None;
            self.update_fraction = 0.0;
            let (tx, rx) = mpsc::channel();
            let (job, repaint) = (release.clone(), ctx.clone());
            std::thread::spawn(move || {
                update::download(&job, &tx);
                repaint.request_repaint();
            });
            self.update_download = Some(rx);
        }
        if skip {
            settings::skip_update(&release.version);
        }
        if later || skip {
            self.update_offer = None;
            self.update_status = None;
        }
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .set_title("Open photo")
            .add_filter("Photos", &filmstrip::all_extensions().iter().flat_map(|e| [e.clone(), e.to_uppercase()]).collect::<Vec<String>>())
            .add_filter("All files", &["*"])
            .pick_file();
        if let Some(path) = picked {
            self.start_loading(path, ctx);
        }
    }

    /// Stage 1 runs on its own thread so the window stays responsive while 61 MP is demosaiced.
    fn start_loading(&mut self, path: PathBuf, ctx: &egui::Context) {
        if self.loading.is_some() {
            return;
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.filmstrip.set_folder(path.parent());
        self.loading_path = Some(path.clone());
        self.store.flush(true);
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let max_texture = self.max_texture;
        if self.remember {
            if let Some(dir) = path.parent() {
                self.store.load_folder(dir);
            }
        }
        let remembered = self.remember && self.store.get(&path).is_some_and(|s| s.denoise);
        let denoise = self.denoise_override.take().unwrap_or(remembered);
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = std::panic::catch_unwind(|| raw::load(&path, max_texture, denoise))
                .unwrap_or_else(|_| Err("The decoder crashed on this file.".to_owned()))
                .map(|image| (image, path, started.elapsed().as_secs_f32()));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.error = None;
        self.loading = Some((name, rx));
    }

    fn poll_loader(&mut self) {
        let Some((_, rx)) = &self.loading else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("The loader stopped unexpectedly.".to_owned()),
        };
        self.loading = None;
        self.loading_path = None;

        match result {
            Ok((image, path, seconds)) => {
                {
                    let rs = &self.render_state;
                    let mut renderer = rs.renderer.write();
                    if let Some(res) = renderer.callback_resources.get_mut::<ImageRenderer>() {
                        res.upload(&rs.device, &rs.queue, &image.mips);
                    }
                }
                let color = CameraColor::new(image.xyz_to_cam, image.as_shot_wb);
                let shown = &image.mips[0];
                let reduced = if shown.width != image.full_width {
                    format!(" (shown at {} × {}: GPU texture limit)", shown.width, shown.height)
                } else {
                    String::new()
                };
                self.adjust = Adjust { kelvin: color.as_shot_kelvin, ..Adjust::default() };
                self.reset_crop();
                self.crop_drag = None;
                self.sharpen = settings::SHARPEN_DEFAULT;
                self.tint = 0.0;
                self.presence = [0.0; 3];
                self.curve = tone::identity_curve();
                self.regions = [0.0; 4];
                self.hsl = [[0.0; 8]; 3];
                self.mono = false;
                self.mono_mix = [0.0; 8];
                self.compare = None;
                self.curve_drag = None;
                self.spots.clear();
                self.spot_selected = None;
                self.spot_drag = None;
                self.spot_preview = spots::Preview::from_mips(&image.mips, 2400);
                self.heal_grids.borrow_mut().clear();
                self.lens = lens::LensSettings::default();
                self.lens_make_pick = None;
                self.lens_auto = self.lens_db.auto_match(&image.exif);
                self.input_mode = image.input_mode;
                self.file_baseline = image.baseline_exposure;
                self.maker_baseline = maker_baseline(&image.exif.make, &image.exif.model) + image.exposure_compensation;
                // Bayer raws use one RawNIND model, linear DNGs (phones) the other.
                self.can_denoise = match image.input_mode {
                    0 => denoise::model_path(denoise::Model::Bayer).is_some(),
                    1 => denoise::model_path(denoise::Model::Linear).is_some(),
                    _ => false,
                };
                if image.input_mode == 2 {
                    self.sharpen[0] = 0.0; // a finished picture has been sharpened already
                }
                // Use Adobe's own profile for this camera when it is installed; keep the
                // user's choice if the same profile exists for the new file's camera.
                let previous = match self.profile {
                    Profile::Dcp(i) => self.dcps.get(i).map(|(label, _, _)| label.clone()),
                    _ => None,
                };
                let first_file = self.loaded.is_none();
                self.dcps = find_camera_profiles(&image.camera);
                self.profile = match (self.profile, previous) {
                    (Profile::Dcp(_), Some(label)) => match self.dcps.iter().position(|(l, _, _)| *l == label) {
                        Some(i) => Profile::Dcp(i),
                        None => Profile::Standard,
                    },
                    (Profile::Standard, _) if first_file && !self.dcps.is_empty() => Profile::Dcp(0),
                    (other, _) => other,
                };
                self.render_key = None;
                // Optional start-up values: `app file.ARW --highlights -100 --exposure -1 ...`
                // `--profile`: 0 = Linear, 1 = Standard, 2 = first Adobe profile, 3 = second...
                self.zoom = None;
                self.center = egui::vec2(0.5, 0.5);
                if let Some((zoom, center)) = self.keep_view.take() {
                    self.zoom = zoom;
                    self.center = center;
                }
                for (name, value) in std::mem::take(&mut self.overrides) {
                    let a = &mut self.adjust;
                    match name.as_str() {
                        "--profile" => {
                            self.profile = match value as i32 {
                                0 => Profile::Linear,
                                n if n >= 2 && ((n - 2) as usize) < self.dcps.len() => Profile::Dcp((n - 2) as usize),
                                _ => Profile::Standard,
                            }
                        }
                        "--baseline" => self.baseline_exposure = value.clamp(-3.0, 3.0),
                        // Export settings for `--export <file>` (format comes from the name).
                        "--export-dialog" => self.export_open = value > 0.5,
                        "--export-quality" => self.export.quality = value.clamp(1.0, 100.0) as u8,
                        "--export-bits" => self.export.sixteen_bit = value >= 16.0,
                        "--export-long" => {
                            self.export.resize = export::Resize::LongEdge;
                            self.export.resize_value = value;
                        }
                        "--export-percent" => {
                            self.export.resize = export::Resize::Percent;
                            self.export.resize_value = value;
                        }
                        "--export-metadata" => {
                            self.export.metadata = match value as i32 {
                                0 => export::Metadata::None,
                                1 => export::Metadata::CopyrightOnly,
                                _ => export::Metadata::All,
                            }
                        }
                        // View, for testing: zoom in screen pixels per image pixel, centre 0..1.
                        "--zoom" => self.zoom = Some(value.clamp(0.01, 16.0)),
                        "--cx" => self.center.x = value.clamp(0.0, 1.0),
                        "--cy" => self.center.y = value.clamp(0.0, 1.0),
                        "--kelvin" => a.kelvin = value.clamp(KELVIN_MIN, KELVIN_MAX),
                        "--exposure" => a.exposure = value.clamp(-5.0, 5.0),
                        "--contrast" => a.contrast = value.clamp(0.5, 2.0),
                        "--saturation" => a.saturation = value.clamp(-100.0, 100.0),
                        "--vibrance" => a.vibrance = value.clamp(-100.0, 100.0),
                        "--whites" => a.whites = value.clamp(-100.0, 100.0),
                        "--highlights" => a.highlights = value.clamp(-100.0, 100.0),
                        "--shadows" => a.shadows = value.clamp(-100.0, 100.0),
                        "--blacks" => a.blacks = value.clamp(-100.0, 100.0),
                        // Crop, for testing: straighten angle, the four sides (0..1), crop mode.
                        "--tint" => self.tint = value.clamp(-150.0, 150.0),
                        // Presence and lens correction, for testing.
                        "--texture" => self.presence[0] = value.clamp(-100.0, 100.0),
                        "--clarity" => self.presence[1] = value.clamp(-100.0, 100.0),
                        "--dehaze" => self.presence[2] = value.clamp(-100.0, 100.0),
                        // Tone curve and Color Mixer, for testing: regions, a point at the middle
                        // (`--curve-mid 0.6` puts 0.5 at 0.6), and `--hsl-s-blue -100` and so on.
                        "--curve-highlights" => self.regions[0] = value.clamp(-100.0, 100.0),
                        "--curve-lights" => self.regions[1] = value.clamp(-100.0, 100.0),
                        "--curve-darks" => self.regions[2] = value.clamp(-100.0, 100.0),
                        "--curve-shadows" => self.regions[3] = value.clamp(-100.0, 100.0),
                        "--curve-mid" => self.curve = vec![[0.0, 0.0], [0.5, value.clamp(0.0, 1.0)], [1.0, 1.0]],
                        // `--curve-x 0.25 --curve-y 0.4 ...`: a point curve point by point (0..1);
                        // the first point replaces the straight curve.
                        "--curve-x" => {
                            if self.curve == tone::identity_curve() {
                                self.curve.clear();
                            }
                            self.curve.push([value.clamp(0.0, 1.0), f32::NAN]);
                        }
                        "--curve-y" => {
                            if let Some(p) = self.curve.iter_mut().find(|p| p[1].is_nan()) {
                                p[1] = value.clamp(0.0, 1.0);
                            }
                            self.curve.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
                        }
                        other if other.starts_with("--hsl-") => {
                            let mut parts = other.trim_start_matches("--hsl-").splitn(2, '-');
                            let kind = match parts.next() {
                                Some("h") => 0,
                                Some("s") => 1,
                                _ => 2,
                            };
                            if let Some(i) = parts.next().and_then(|name| tone::COLOURS.iter().position(|c| c.0.eq_ignore_ascii_case(name))) {
                                self.hsl[kind][i] = value.clamp(-100.0, 100.0);
                            }
                        }
                        "--lens" => self.lens.enabled = value > 0.5,
                        "--lens-source" => self.lens.source = [lens::Source::Auto, lens::Source::Lensfun, lens::Source::Embedded][(value as usize).min(2)],
                        "--lens-distortion" => self.lens.distortion = value.clamp(-100.0, 100.0),
                        "--lens-profile-distortion" => self.lens.profile_distortion = value.clamp(0.0, 200.0),
                        "--lens-profile-vignetting" => self.lens.profile_vignetting = value.clamp(0.0, 200.0),
                        "--lens-constrain" => self.lens.constrain = value > 0.5,
                        "--lens-vignette" => self.lens.vignette = value.clamp(-100.0, 100.0),
                        "--red-cyan" => self.lens.red_cyan = value.clamp(-100.0, 100.0),
                        "--purple" => self.lens.purple_amount = value.clamp(0.0, 100.0),
                        "--sharpen" => self.sharpen[0] = value.clamp(0.0, 150.0),
                        "--sharpen-radius" => self.sharpen[1] = value.clamp(0.5, 3.0),
                        "--sharpen-detail" => self.sharpen[2] = value.clamp(0.0, 100.0),
                        "--sharpen-masking" => self.sharpen[3] = value.clamp(0.0, 100.0),
                        "--sharpen-scale" => self.sharpen_scale = value,
                        "--angle" => self.angle = value.clamp(-45.0, 45.0),
                        "--crop-l" => self.crop[0] = value,
                        "--crop-t" => self.crop[1] = value,
                        "--crop-r" => self.crop[2] = value,
                        "--crop-b" => self.crop[3] = value,
                        "--crop-mode" => self.crop_mode = value > 0.5,
                        // Spots, for testing: `--spot-x 0.4 --spot-y 0.6 --spot-r 0.01` adds a
                        // healed spot there with an automatic source (`--spot-clone 1` first
                        // for clone); `--spot-px 0.5 --spot-py 0.6` before `--spot-r` makes it
                        // a stroke through more points; `--spot-tool 1` shows the tool.
                        "--spot-x" => self.spot_brush.x = value,
                        "--spot-y" => self.spot_brush.y = value,
                        "--spot-px" => self.spot_brush.path.push([value, self.spot_brush.y]),
                        "--spot-py" => {
                            if let Some(p) = self.spot_brush.path.last_mut() {
                                p[1] = value;
                            }
                        }
                        "--spot-clone" => self.spot_brush.clone = value > 0.5,
                        "--spot-feather" => self.spot_brush.feather = value.clamp(0.0, 1.0),
                        "--spot-r" => {
                            let mut spot = std::mem::take(&mut self.spot_brush.path);
                            spot.insert(0, [self.spot_brush.x, self.spot_brush.y]);
                            let radius = value.clamp(SPOT_MIN, SPOT_MAX);
                            let [dx, dy] = self.spot_preview.find_offset(&spot, radius);
                            let (x, y) = (spot[0][0], spot[0][1]);
                            self.spots.push(spots::Spot { x, y, sx: x + dx, sy: y + dy, path: spot[1..].to_vec(), radius, ..self.spot_brush.clone() });
                            self.spot_selected = Some(self.spots.len() - 1);
                        }
                        "--spot-tool" => self.set_spot_tool(value > 0.5),
                        "--wb-pick" => self.wb_pick = value > 0.5,
                        "--show-curve" => self.curve_reveal = value > 0.5,
                        // Black & white, for testing: `--mono 1 --mono-yellow 60`.
                        "--mono" => self.mono = value > 0.5,
                        other if other.starts_with("--mono-") => {
                            if let Some(i) = tone::COLOURS.iter().position(|c| c.0.eq_ignore_ascii_case(other.trim_start_matches("--mono-"))) {
                                self.mono_mix[i] = value.clamp(-100.0, 100.0);
                            }
                        }
                        "--open-sections" => self.sections_reveal = value > 0.5,
                        "--with-settings" => self.with_settings = value > 0.5,
                        // `--wb-x 0.3 --wb-y 0.6`: what a click there with the eyedropper sets
                        // (texture coordinates), for testing.
                        "--wb-x" => self.spot_brush.sx = value,
                        "--wb-y" => {
                            let sample = self.spot_preview.average(self.spot_brush.sx, value, 2);
                            if let Some((kelvin, tint)) = color.neutral_for(sample) {
                                self.adjust.kelvin = kelvin;
                                self.tint = tint;
                            }
                        }
                        // Canvas HUD toggles, for testing.
                        "--clipping" => self.clipping = value > 0.5,
                        "--export-panel" => self.export_open = value > 0.5,
                        "--export-dng" => self.export_dng = value > 0.5,
                        "--ab" => {
                            self.ab_split = value > 0.0;
                            self.ab_pos = value.clamp(0.02, 0.98);
                        }
                        _ => {}
                    }
                }
                self.quarter_turns = image.quarter_turns;
                self.loaded = Some(Loaded {
                    name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    info: format!(
                        "{}\n{} × {}{}\nAs shot: {:.0} K · loaded in {:.1} s",
                        image.camera, image.full_width, image.full_height, reduced, color.as_shot_kelvin, seconds
                    ),
                    camera: image.camera.clone(),
                    ext: filmstrip::extension_of(&path),
                    full_size: (image.full_width, image.full_height),
                    bits: image.bits,
                    seconds,
                    exif: image.exif.clone(),
                    embedded: image.embedded.clone(),
                    width: shown.width as f32,
                    height: shown.height as f32,
                    color,
                    path,
                    photo: image.photo.clone(),
                });
                // Offer the camera's own artist / copyright text unless the user typed some.
                if self.export.artist.is_empty() {
                    self.export.artist = image.photo.artist.clone().unwrap_or_default();
                }
                if self.export.copyright.is_empty() {
                    self.export.copyright = image.photo.copyright.clone().unwrap_or_default();
                }
                self.export_status = None;
                // Bring back what this photo was last left at.
                self.denoise = false;
                self.defaults = Some(self.snapshot());
                // The camera's preview stands for the defaults; an edited photo's thumbnail
                // is drawn again once it is open.
                self.thumb_shows = self.defaults.clone();
                if self.remember || self.with_settings {
                    let saved = match self.remember {
                        true => self.loaded.as_ref().and_then(|l| self.store.get(&l.path)).cloned(),
                        false => self.loaded.as_ref().and_then(|l| settings::read_sidecar(&l.path)),
                    };
                    if let Some(saved) = saved {
                        self.apply_saved(&saved);
                        if saved.quarter_turns < 4 {
                            self.quarter_turns = saved.quarter_turns;
                        }
                        self.crop = saved.crop;
                        self.crop_wish = saved.crop;
                        self.angle = saved.angle;
                        self.render_key = None;
                        self.thumb_shows = None;
                    }
                }
                self.denoise = image.denoised;
                if self.exposure_shift != 0.0 {
                    self.adjust.exposure = (self.adjust.exposure + std::mem::take(&mut self.exposure_shift)).clamp(-5.0, 5.0);
                    self.render_key = None;
                }
                if let (Some(note), Some(loaded)) = (&image.denoise_note, &mut self.loaded) {
                    loaded.info.push('\n');
                    loaded.info.push_str(note);
                }
                if image.denoised {
                    self.thumb_shows = None;
                }
            }
            Err(message) => self.error = Some(message),
        }
    }

    /// Keep the open photo's filmstrip thumbnail showing its edit: once the mouse is let go
    /// after a change, render the picture small with the current settings, show it in the
    /// strip and save it for next time. Back at the defaults, the camera's preview returns.
    /// Render a thumbnail for one of the open photo's versions that has none yet (one a
    /// frame), and save it for the filmstrip. The version's settings are put on the photo for
    /// the render and the current edit is put straight back.
    fn update_version_thumbs(&mut self, ctx: &egui::Context) {
        let Some(path) = self.loaded.as_ref().map(|l| l.path.clone()) else { return };
        if self.render.is_none() || self.loading.is_some() || self.pending_version.is_some() || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        let versions = self.store.versions(&path).to_vec();
        let Some(version) = versions.iter().find(|v| {
            let key = settings::version_key(&path, v);
            !self.filmstrip.has_version_thumb(&key) && !settings::version_thumb_file(&key).is_some_and(|f| f.is_file())
        }) else {
            return;
        };
        let key = settings::version_key(&path, version);
        let current = self.snapshot();
        self.restore_snapshot(&version.saved);
        let rendered = self.loaded.as_ref().and_then(|loaded| {
            let (width, height) = self.output_size(loaded);
            let thumb_h = 200u32;
            let thumb_w = ((width as f32 * thumb_h as f32 / height.max(1) as f32).round() as u32).max(1);
            let uniforms = self.uniforms(loaded, [1.0, 1.0, 0.5, 0.5], true);
            let rs = &self.render_state;
            let renderer = rs.renderer.read();
            renderer
                .callback_resources
                .get::<ImageRenderer>()
                .and_then(|res| res.render_full(&rs.device, &rs.queue, &uniforms, thumb_w, thumb_h, self.crop_region()).ok())
                .map(|rgb16| (rgb16, thumb_w, thumb_h))
        });
        self.restore_snapshot(&current);
        let Some((rgb16, w, h)) = rendered else { return };
        let rgb8: Vec<u8> = rgb16.iter().map(|&v| ((v as u32 * 255 + 32767) / 65535) as u8).collect();
        if let Some(file) = settings::version_thumb_file(&key) {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = image::save_buffer_with_format(&file, &rgb8, w, h, image::ColorType::Rgb8, image::ImageFormat::Jpeg);
        }
        self.filmstrip.set_version_thumb(ctx, &key, egui::ColorImage::from_rgb([w as usize, h as usize], &rgb8));
        ctx.request_repaint();
    }

    /// Render the open photo small (long side `long`) with the current settings, as 8-bit RGB.
    fn render_small(&mut self, long: u32) -> Option<(Vec<u8>, usize, usize)> {
        self.sync_profile();
        let loaded = self.loaded.as_ref()?;
        let (width, height) = self.output_size(loaded);
        let scale = long as f32 / width.max(height).max(1) as f32;
        let (w, h) = (((width as f32 * scale).round() as u32).max(1), ((height as f32 * scale).round() as u32).max(1));
        let uniforms = self.uniforms(loaded, [1.0, 1.0, 0.5, 0.5], true);
        let rs = &self.render_state;
        let renderer = rs.renderer.read();
        let rgb16 = renderer.callback_resources.get::<ImageRenderer>()?.render_full(&rs.device, &rs.queue, &uniforms, w, h, self.crop_region()).ok()?;
        Some((rgb16.iter().map(|&v| ((v as u32 * 255 + 32767) / 65535) as u8).collect(), w as usize, h as usize))
    }

    /// Render the open photo at this temperature and tint, with exposure shifted by `shift`
    /// for the measurement only, and measure skin and neutral areas (for autowb.rs).
    fn wb_probe(&mut self, kelvin: f32, tint: f32, shift: f32) -> autowb::Probe {
        let exposure = self.adjust.exposure;
        self.adjust.kelvin = kelvin;
        self.tint = tint;
        self.adjust.exposure = (exposure + shift).clamp(-5.0, 5.0);
        let picture = self.render_small(800);
        self.adjust.exposure = exposure;
        let Some((rgb, w, h)) = picture else { return autowb::Probe::default() };
        let report = measure::measure(&rgb, w, h);
        let num = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0) as f32;
        autowb::Probe {
            skin: skin::read(&rgb, w, h).ok(),
            blue_minus_red: num(&report["neutral_areas"]["blue_minus_red_half"]),
            green_minus_magenta: num(&report["neutral_areas"]["green_minus_magenta"]),
            neutral_share: num(&report["neutral_areas"]["share_percent"]),
        }
    }

    /// Auto white balance for the open photo (see autowb.rs): sets temperature (and, without
    /// faces, tint); exposure is left as it is. One Undo step goes back.
    fn auto_white_balance(&mut self) -> Option<autowb::Outcome> {
        if self.loaded.is_none() || self.loading.is_some() {
            return None;
        }
        // Always from the camera's own white balance, so the answer does not depend on where
        // the sliders happen to be (a photo already pushed far off has nothing left to measure).
        let kelvin = self.loaded.as_ref().map_or(self.adjust.kelvin, |l| l.color.as_shot_kelvin);
        let tint = 0.0;
        let outcome = autowb::solve(kelvin, tint, |k, t, s| self.wb_probe(k, t, s));
        self.adjust.kelvin = outcome.kelvin;
        self.tint = outcome.tint;
        self.render_key = None;
        self.sync_profile();
        let how = match outcome.method {
            autowb::Method::Skin => format!("From skin: {:.0} K", outcome.kelvin),
            autowb::Method::Neutral => format!("From neutral areas: {:.0} K, tint {:.0}", outcome.kelvin, outcome.tint),
            autowb::Method::Unchanged => "Nothing to measure: unchanged".to_owned(),
        };
        self.auto_wb_note = Some(match outcome.check.first() {
            Some(c) if outcome.method != autowb::Method::Unchanged => format!("{how} ({c})"),
            _ => how,
        });
        Some(outcome)
    }

    fn update_thumbnail(&mut self, ctx: &egui::Context) {
        let (Some(loaded), Some(defaults)) = (&self.loaded, &self.defaults) else { return };
        if self.render.is_none() || self.loading.is_some() {
            return;
        }
        let current = self.snapshot();
        if self.thumb_shows.as_ref() == Some(&current) || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        let cache = settings::thumb_file(&loaded.path);
        if current == *defaults {
            if let Some(file) = &cache {
                let _ = std::fs::remove_file(file);
            }
            self.filmstrip.reload(&loaded.path);
        } else {
            let (width, height) = self.output_size(loaded);
            let thumb_h = 200u32;
            let thumb_w = ((width as f32 * thumb_h as f32 / height.max(1) as f32).round() as u32).max(1);
            let uniforms = self.uniforms(loaded, [1.0, 1.0, 0.5, 0.5], true);
            let rendered = {
                let rs = &self.render_state;
                let renderer = rs.renderer.read();
                renderer
                    .callback_resources
                    .get::<ImageRenderer>()
                    .and_then(|res| res.render_full(&rs.device, &rs.queue, &uniforms, thumb_w, thumb_h, self.crop_region()).ok())
            };
            if let Some(rgb16) = rendered {
                let rgb8: Vec<u8> = rgb16.iter().map(|&v| ((v as u32 * 255 + 32767) / 65535) as u8).collect();
                if let Some(file) = &cache {
                    if let Some(dir) = file.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = image::save_buffer_with_format(
                        file,
                        &rgb8,
                        thumb_w,
                        thumb_h,
                        image::ColorType::Rgb8,
                        image::ImageFormat::Jpeg,
                    );
                }
                let picture = egui::ColorImage::from_rgb([thumb_w as usize, thumb_h as usize], &rgb8);
                self.filmstrip.set_thumb(ctx, &loaded.path, picture);
            }
        }
        self.thumb_shows = Some(current);
    }

    /// Put stored settings on the open photo. A stored white balance of 0 means "as shot".
    /// Rotation is left alone (the caller decides).
    fn apply_saved(&mut self, saved: &settings::Saved) {
        let mut values = saved.values;
        if values[0] <= 0.0 {
            values[0] = self.loaded.as_ref().map_or(KELVIN_DEFAULT, |l| l.color.as_shot_kelvin);
        }
        self.adjust = Adjust::from_array(values);
        self.sharpen = saved.sharpen;
        self.presence = saved.presence;
        self.curve = saved.curve.clone();
        self.regions = saved.regions;
        self.hsl = saved.hsl;
        self.mono = saved.mono;
        self.mono_mix = saved.mono_mix;
        self.curve_drag = None;
        self.spots = saved.spots.clone();
        self.spot_selected = self.spot_selected.filter(|&i| i < self.spots.len());
        self.spot_drag = None;
        self.lens = saved.lens.clone();
        self.tint = saved.tint;
        self.profile = match saved.profile.as_str() {
            "Linear" => Profile::Linear,
            "Standard" => Profile::Standard,
            label => match self.dcps.iter().position(|(l, _, _)| l == label) {
                Some(i) => Profile::Dcp(i),
                None => self.profile,
            },
        };
        self.render_key = None;
    }

    /// `saved` in the form it is stored and copied: white balance 0 when it is the photo's
    /// own, rotation 255 when it is the camera's, so it can be pasted onto other photos.
    fn storable(&self, mut saved: settings::Saved) -> settings::Saved {
        if let Some(defaults) = &self.defaults {
            if saved.values[0] == defaults.values[0] {
                saved.values[0] = 0.0;
            }
            if saved.quarter_turns == defaults.quarter_turns {
                saved.quarter_turns = 255;
            }
        }
        saved
    }

    /// Carry out a click or menu choice from the filmstrip or the picture's right-click menu.
    fn apply_action(&mut self, action: filmstrip::Action, ctx: &egui::Context) {
        use filmstrip::Action;
        let open = self.loaded.as_ref().map(|l| l.path.clone());
        match action {
            Action::Open(path) => self.start_loading(path, ctx),
            Action::ToDng(targets, embed) => self.start_dng(targets, embed, ctx),
            Action::Export(targets) => {
                if targets.len() == 1 && open.as_ref() == Some(&targets[0]) {
                    self.export_dialog_for(targets, ctx);
                } else if !targets.is_empty() {
                    self.start_batch(targets);
                }
            }
            Action::Copy(path) => {
                self.clipboard = if open.as_ref() == Some(&path) {
                    Some(self.storable(self.snapshot()))
                } else {
                    self.store.get(&path).cloned()
                };
                // Also put a line of text on the system clipboard. On Windows, Ctrl+V only
                // reaches Develop when the clipboard holds text, so without this a paste after
                // copying a screenshot (or with an empty clipboard) did nothing.
                if self.clipboard.is_some() {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ctx.output_mut(|o| o.copied_text = format!("Develop settings copied from {name}"));
                }
            }
            Action::Paste(targets) => {
                let Some(copied) = self.clipboard.clone() else { return };
                for target in targets {
                    if open.as_ref() == Some(&target) {
                        self.apply_saved(&copied);
                        if copied.denoise != self.denoise {
                            self.denoise_override = Some(copied.denoise);
                            self.reload = true;
                        }
                    } else {
                        // Not open: store the settings, keep the photo's own rotation, and
                        // drop its saved thumbnail (it shows the old edit).
                        let mut saved = copied.clone();
                        let existing = self.store.get(&target);
                        saved.quarter_turns = existing.map_or(255, |s| s.quarter_turns);
                        saved.crop = existing.map_or(crop::FULL, |s| s.crop);
                        saved.angle = existing.map_or(0.0, |s| s.angle);
                        self.store.put(&target, saved);
                        if let Some(file) = settings::thumb_file(&target) {
                            let _ = std::fs::remove_file(file);
                        }
                        self.filmstrip.reload(&target);
                    }
                }
            }
            Action::OpenVersion(path, n) => {
                if open.as_ref() == Some(&path) {
                    if let Some(v) = self.store.versions(&path).get(n).cloned() {
                        self.restore_snapshot(&v.saved);
                    }
                } else {
                    self.pending_version = Some((path.clone(), n));
                    self.start_loading(path, ctx);
                }
            }
            Action::DeleteVersion(path, n) => {
                let mut versions = self.store.versions(&path).to_vec();
                if n < versions.len() {
                    if let Some(file) = settings::version_thumb_file(&settings::version_key(&path, &versions[n])) {
                        let _ = std::fs::remove_file(file);
                    }
                    versions.remove(n);
                    self.store.set_versions(&path, versions);
                }
            }
            Action::Rate(targets, stars) => {
                for target in targets {
                    self.store.set_rating(&target, stars);
                }
            }
        }
    }

    /// The open photo's settings in the form they are remembered.
    fn snapshot(&self) -> settings::Saved {
        settings::Saved {
            denoise: self.denoise,
            crop: self.crop,
            angle: self.angle,
            sharpen: self.sharpen,
            tint: self.tint,
            presence: self.presence,
            curve: self.curve.clone(),
            regions: self.regions,
            hsl: self.hsl,
            mono: self.mono,
            mono_mix: self.mono_mix,
            lens: self.lens.clone(),
            spots: self.spots.clone(),
            values: self.adjust.to_array(),
            quarter_turns: self.quarter_turns % 4,
            profile: match self.profile {
                Profile::Linear => "Linear".to_owned(),
                Profile::Standard => "Standard".to_owned(),
                Profile::Dcp(i) => self.dcps.get(i).map_or_else(|| "Standard".to_owned(), |(l, _, _)| l.clone()),
            },
        }
    }

    /// Rebuild the profile's matrices and lookup tables when the profile changes, or when the
    /// white-balance temperature moves under an Adobe profile (its tables depend on it).
    fn sync_profile(&mut self) {
        let Some(loaded) = &self.loaded else { return };
        let kelvin = match self.profile {
            Profile::Dcp(_) => self.adjust.kelvin.round() as i32,
            _ => 0,
        };
        let key = (self.profile, kelvin, (self.baseline_exposure + self.maker_baseline).to_bits());
        if self.render_key == Some(key) {
            return;
        }
        // A DNG states its own baseline exposure; other raws use the constant.
        let baseline = self.file_baseline.unwrap_or(self.baseline_exposure + self.maker_baseline);
        let mut render = match self.profile {
            Profile::Linear => profile::builtin(&loaded.color, false),
            Profile::Standard => profile::builtin(&loaded.color, true),
            Profile::Dcp(i) => match self.dcps.get(i) {
                Some((_, d, contrast)) => profile::from_dcp(d, &loaded.color, self.adjust.kelvin, baseline, *contrast),
                None => profile::builtin(&loaded.color, true),
            },
        };
        if self.input_mode == 2 {
            // A finished picture is already in sRGB with its tone curve: no camera matrix,
            // no base curve, whatever the profile choice says.
            render = profile::builtin(&loaded.color, false);
            render.cam_to_work = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            render.pre_saturation = 1.0;
        } else if !matches!(self.profile, Profile::Dcp(_)) {
            render.exposure_bias += self.file_baseline.unwrap_or(0.0);
        }
        {
            let rs = &self.render_state;
            let mut renderer = rs.renderer.write();
            if let Some(res) = renderer.callback_resources.get_mut::<ImageRenderer>() {
                res.set_profile(&rs.device, &rs.queue, &render);
            }
        }
        self.render = Some(render);
        self.render_key = Some(key);
    }

    /// Everything the shader needs for the current settings. `view` is the part of the
    /// picture to draw (size and centre in image-UV units); the viewport and export differ
    /// only in that and in who applies the sRGB encoding.
    fn uniforms(&self, loaded: &Loaded, view: [f32; 4], shader_encodes_srgb: bool) -> Uniforms {
        self.measure_scene(loaded);
        let mut u = self.uniforms_with(loaded, view, shader_encodes_srgb, &self.adjust, self.tint, self.sharpen, self.presence, &self.lens);
        // Only gains worked out for the spots as they are now (a spot changed since the last
        // `sync_heal` uses the shader's own estimate until the next frame).
        let grids = self.heal_grids.borrow();
        let current: Vec<Option<&spots::HealGrid>> = (0..self.spots.len().min(spots::MAX_SPOTS))
            .map(|i| grids.get(i).and_then(|g| g.as_ref()).filter(|g| !self.spots[i].clone && g.key == spots::heal_key(&self.spots, i, &self.spot_preview)))
            .collect();
        spots::fill_uniforms(&self.spots, &current, loaded.width / loaded.height.max(1.0), &mut u);
        // Tone curve and Color Mixer (the "before" view and the scene measurement leave them out).
        if !tone::curve_is_identity(&self.curve, &self.regions) {
            let lut = tone::lut(&self.curve, &self.regions);
            for (i, v) in lut.iter().enumerate() {
                u.curve_lut[i / 4][i % 4] = *v;
            }
            u.curve_info[0] = 1.0;
        }
        if self.hsl.iter().flatten().any(|v| v.abs() > 1e-3) {
            u.hsl = tone::hsl_uniform(&self.hsl);
            u.hsl_info[0] = 1.0;
        }
        if self.mono {
            u.mono_info[0] = 1.0;
            for (i, v) in self.mono_mix.iter().enumerate() {
                u.mono_mix[i / 4][i % 4] = v / 100.0;
            }
        }
        u
    }

    /// Measure the photo's display levels (`Uniforms::scene`) when the photo or the settings
    /// they depend on (exposure, white balance, profile, lens) have changed: a small render
    /// with the tone and presence sliders at zero, read back from the GPU.
    fn measure_scene(&self, loaded: &Loaded) {
        let blank = Adjust { contrast: 1.0, saturation: 0.0, vibrance: 0.0, whites: 0.0, highlights: 0.0, shadows: 0.0, blacks: 0.0, ..self.adjust };
        let key = format!(
            "{}|{:?}|{}|{}|{}|{:?}|{:?}",
            loaded.name, loaded.full_size, blank.exposure, blank.kelvin, self.tint, self.lens, self.profile
        );
        if *self.scene_for.borrow() == key {
            return;
        }
        let (w, h) = if loaded.width >= loaded.height { (192, ((192.0 * loaded.height / loaded.width.max(1.0)) as u32).max(16)) } else { (((192.0 * loaded.width / loaded.height.max(1.0)) as u32).max(16), 192) };
        let uniforms = self.uniforms_with(loaded, [1.0, 1.0, 0.5, 0.5], true, &blank, self.tint, [0.0, 1.0, 0.0, 0.0], [0.0; 3], &self.lens);
        let rendered = {
            let rs = &self.render_state;
            let renderer = rs.renderer.read();
            renderer.callback_resources.get::<ImageRenderer>().and_then(|res| res.render_full(&rs.device, &rs.queue, &uniforms, w, h, [0.0, 0.0, 1.0, 1.0]).ok())
        };
        let Some(rgb) = rendered else { return };
        let mut levels: Vec<f32> = rgb.chunks_exact(3).map(|p| (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) / 65535.0).collect();
        let mut darkest: Vec<f32> = rgb.chunks_exact(3).map(|p| p[0].min(p[1]).min(p[2]) as f32 / 65535.0).collect();
        darkest.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if let Some(&d) = darkest.get(darkest.len() * 98 / 100) {
            self.scene2.set([d, 0.0, 0.0, 0.0]);
        }
        if levels.is_empty() {
            return;
        }
        levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = levels.len();
        let bright = levels.iter().filter(|&&l| l > 176.0 / 255.0).count() as f32 / n as f32;
        self.scene.set([bright, levels[(n * 98 / 100).min(n - 1)], levels[n / 2], levels[(n * 997 / 1000).min(n - 1)]]);
        *self.scene_for.borrow_mut() = key;
    }

    /// The same picture with the photo's default sliders: the "before" of the A/B split.
    /// Profile, crop, rotation and denoise stay as they are.
    fn uniforms_before(&self, loaded: &Loaded, view: [f32; 4], shader_encodes_srgb: bool) -> Uniforms {
        // A version picked for comparison is the "before", with its colour treatment.
        if let Some(v) = &self.compare {
            let mut u = self.uniforms_with(loaded, view, shader_encodes_srgb, &Adjust::from_array(v.values), v.tint, v.sharpen, v.presence, &v.lens);
            if !tone::curve_is_identity(&v.curve, &v.regions) {
                for (i, x) in tone::lut(&v.curve, &v.regions).iter().enumerate() {
                    u.curve_lut[i / 4][i % 4] = *x;
                }
                u.curve_info[0] = 1.0;
            }
            if v.hsl.iter().flatten().any(|x| x.abs() > 1e-3) {
                u.hsl = tone::hsl_uniform(&v.hsl);
                u.hsl_info[0] = 1.0;
            }
            if v.mono {
                u.mono_info[0] = 1.0;
                for (i, x) in v.mono_mix.iter().enumerate() {
                    u.mono_mix[i / 4][i % 4] = x / 100.0;
                }
            }
            return u;
        }
        match &self.defaults {
            Some(d) => self.uniforms_with(loaded, view, shader_encodes_srgb, &Adjust::from_array(d.values), d.tint, d.sharpen, d.presence, &d.lens),
            None => self.uniforms(loaded, view, shader_encodes_srgb),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn uniforms_with(&self, loaded: &Loaded, view: [f32; 4], shader_encodes_srgb: bool, a: &Adjust, tint: f32, sharpen: [f32; 4], presence: [f32; 3], lens_settings: &lens::LensSettings) -> Uniforms {
        // Lenses whose distortion correction Lightroom always applies get it here too.
        let builtin;
        let lens_settings = if !lens_settings.enabled && self.builtin_distortion(loaded) {
            builtin = lens::LensSettings { enabled: true, source: lens::Source::Lensfun, profile: String::new(), profile_distortion: 100.0, profile_vignetting: 0.0, ..lens_settings.clone() };
            &builtin
        } else {
            lens_settings
        };
        let active = self.lens_active(lens_settings, loaded);
        let lens_gpu = lens::Gpu::new(lens_settings, &active, &self.lens_db, &loaded.exif, loaded.embedded.as_ref(), loaded.width / loaded.height.max(1.0));
        // Tint: off the Planckian locus, as Lightroom does it (see CameraColor::wb_gains_tinted).
        let gains = loaded.color.wb_gains_tinted(a.kelvin, tint);
        let min_gain = gains[0].min(gains[1]).min(gains[2]);
        let fallback;
        let render = match &self.render {
            Some(render) => render,
            None => {
                fallback = profile::builtin(&loaded.color, true);
                &fallback
            }
        };
        let rows = |m: &color::Mat3| {
            [[m[0][0], m[0][1], m[0][2], 0.0], [m[1][0], m[1][1], m[1][2], 0.0], [m[2][0], m[2][1], m[2][2], 0.0]]
        };
        let table_dims = |t: &Option<profile::GpuTable>| match t {
            Some(t) => [t.dims[0] as f32, t.dims[1] as f32, t.dims[2] as f32, if t.srgb_value_axis { 1.0 } else { 0.0 }],
            None => [1.0, 1.0, 1.0, 0.0],
        };
        let (rot, rot_off) = match self.quarter_turns % 4 {
            0 => ([1.0, 0.0, 0.0, 1.0], [0.0, 0.0]),
            1 => ([0.0, 1.0, -1.0, 0.0], [0.0, 1.0]),
            2 => ([-1.0, 0.0, 0.0, -1.0], [1.0, 1.0]),
            _ => ([0.0, -1.0, 1.0, 0.0], [1.0, 0.0]),
        };
        Uniforms {
            cam_to_work: rows(&render.cam_to_work),
            work_to_srgb: rows(&render.work_to_srgb),
            luma: [render.luma[0], render.luma[1], render.luma[2], a.vibrance * 0.01],
            wb: [gains[0], gains[1], gains[2], min_gain],
            basic: [
                (a.exposure + render.exposure_bias).exp2(),
                a.contrast,
                a.saturation * 0.01,
                if shader_encodes_srgb { 1.0 } else { 0.0 },
            ],
            tone: [a.whites * 0.01, a.highlights * 0.01, a.shadows * 0.01, a.blacks * 0.01],
            view,
            rot,
            rot_off: [rot_off[0], rot_off[1], render.post_contrast, 0.0],
            profile: [
                render.curve_mode,
                render.pre_saturation,
                if render.hue_sat.is_some() { 1.0 } else { 0.0 },
                if render.look.is_some() { 1.0 } else { 0.0 },
            ],
            hue_sat_dims: table_dims(&render.hue_sat),
            look_dims: table_dims(&render.look),
            straighten: {
                let (w, h) = self.display_size(loaded);
                let g = crop::Geometry::new(w as f32, h as f32, self.angle);
                [g.cos, g.sin, g.aspect, 0.0]
            },
            crop: self.crop_display(),
            input: [self.input_mode as f32, 0.0, 0.0, 0.0],
            presence: [presence[0] / 100.0, presence[1] / 100.0, presence[2] / 100.0, 0.0],
            lens_dist: lens_gpu.dist,
            lens_geo: lens_gpu.geo,
            lens_str: lens_gpu.strength,
            tca_r: lens_gpu.tca_r,
            tca_b: lens_gpu.tca_b,
            vig: lens_gpu.vig,
            vig2: lens_gpu.vig2,
            fringe: lens_gpu.fringe,
            knots: lens_gpu.knots,
            scene: self.scene.get(),
            scene2: self.scene2.get(),
            spot_info: [0.0; 4],
            spot_a: [[0.0; 4]; spots::MAX_SPOTS],
            spot_b: [[0.0; 4]; spots::MAX_SPOTS],
            spot_c: [[0.0; 4]; spots::MAX_SPOTS],
            spot_box: [[0.0; 4]; spots::MAX_SPOTS],
            spot_pts: [[0.0; 4]; spots::MAX_POINTS / 2],
            spot_g: [[0.0; 4]; spots::MAX_SPOTS],
            spot_t: [[0.0; 4]; spots::MAX_SPOTS],
            curve_lut: [[0.0; 4]; tone::LUT_SIZE / 4],
            curve_info: [0.0; 4],
            hsl: [[0.0; 4]; 6],
            hsl_info: [0.0; 4],
            mono_info: [0.0; 4],
            mono_mix: [[0.0; 4]; 2],
            srgb_to_work: {
                let inv = color::invert(&render.work_to_srgb).unwrap_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
                rows(&inv)
            },
            sharpen: {
                let [amount, radius, detail, masking] = sharpen;
                [
                    // Slightly less than proportional at the top, as measured.
                    amount * (1.0 - 0.0005 * amount) * self.sharpen_scale,
                    radius,
                    0.01 + 0.24 * (detail * 0.01).powf(1.5),
                    masking * 0.01 * 0.05,
                ]
            },
        }
    }

    /// The crop rectangle as it lies on the displayed (quarter-turned) picture.
    fn crop_display(&self) -> crop::Rect {
        crop::to_display(self.quarter_turns, self.crop)
    }

    fn set_crop_display(&mut self, rect: crop::Rect) {
        self.crop = crop::to_stored(self.quarter_turns, rect);
    }

    /// Size in pixels of what is shown and exported: the displayed picture after cropping.
    fn output_size(&self, loaded: &Loaded) -> (u32, u32) {
        let (w, h) = self.display_size(loaded);
        let c = self.crop_display();
        (((w as f32 * (c[2] - c[0])).round() as u32).max(1), ((h as f32 * (c[3] - c[1])).round() as u32).max(1))
    }

    /// The crop as `render_full` wants it: x, y, width, height.
    fn crop_region(&self) -> [f32; 4] {
        let c = self.crop_display();
        [c[0], c[1], c[2] - c[0], c[3] - c[1]]
    }

    fn geometry(&self) -> Option<crop::Geometry> {
        let loaded = self.loaded.as_ref()?;
        let (w, h) = self.display_size(loaded);
        Some(crop::Geometry::new(w as f32, h as f32, self.angle))
    }

    /// After the straighten angle changed: fit the wanted crop inside the turned picture.
    fn refit_crop(&mut self) {
        if let Some(geometry) = self.geometry() {
            let fitted = geometry.constrain(crop::to_display(self.quarter_turns, self.crop_wish));
            self.set_crop_display(fitted);
        }
    }

    fn reset_crop(&mut self) {
        self.crop = crop::FULL;
        self.crop_wish = crop::FULL;
        self.angle = 0.0;
    }

    /// Lock the crop to `ratio` (width, height), turned to match the crop's current
    /// orientation, and shrink the crop to that shape. `None` frees it.
    fn set_crop_ratio(&mut self, label: String, ratio: Option<(f32, f32)>) {
        self.crop_ratio_label = label;
        self.crop_ratio = ratio;
        let (Some((w, h)), Some(geometry)) = (ratio, self.geometry()) else { return };
        let c = self.crop_display();
        let portrait = (c[2] - c[0]) * geometry.aspect < (c[3] - c[1]);
        let (w, h) = if portrait == (w > h) { (h, w) } else { (w, h) };
        self.crop_ratio = Some((w, h));
        self.set_crop_display(crop::with_ratio(c, w / h / geometry.aspect));
        self.crop_wish = self.crop;
        self.crop_mode = true;
        self.zoom = None;
    }

    /// The "Crop" section of the inspector.
    fn crop_panel(&mut self, ui: &mut egui::Ui) {
        const STANDARD: [(&str, f32, f32); 8] = [
            ("1 : 1", 1.0, 1.0),
            ("5 : 4", 5.0, 4.0),
            ("4 : 3", 4.0, 3.0),
            ("3 : 2", 3.0, 2.0),
            ("7 : 5", 7.0, 5.0),
            ("16 : 10", 16.0, 10.0),
            ("16 : 9", 16.0, 9.0),
            ("2 : 1", 2.0, 1.0),
        ];
        let has = self.loaded.is_some();
        ui.add_enabled_ui(has, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let reset_width = 58.0;
                let (text, look) = if self.crop_mode { ("Done Cropping", ui::Look::teal()) } else { ("Crop & Straighten", ui::Look::raised()) };
                let width = ui.available_width() - reset_width - 6.0;
                let hint = "R. Drag the frame or its edges to crop; drag outside the frame to rotate. Enter or double-click to finish.";
                if ui::button(ui, text, egui::vec2(width, 28.0), look, theme::font(12.5), Some(ui::Icon::Crop)).on_hover_text(hint).clicked() {
                    self.crop_mode = !self.crop_mode;
                    self.crop_wish = self.crop;
                    self.zoom = None;
                }
                if ui::button(ui, "Reset", egui::vec2(reset_width, 28.0), ui::Look::secondary(), theme::font(12.0), None).on_hover_text("Clear the crop and the angle").clicked() {
                    self.reset_crop();
                }
            });
        });
        ui.add_space(6.0);

        let mut pick: Option<(String, Option<(f32, f32)>)> = None;
        let mut swap = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Ratio").color(theme::TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().interact_size.y = 24.0;
                if ui
                    .add_enabled_ui(has && self.crop_ratio.is_some(), |ui| ui::button(ui, "Swap", egui::vec2(0.0, 24.0), ui::Look::bordered(), theme::font(12.0), None))
                    .inner
                    .on_hover_text("Landscape / portrait")
                    .clicked()
                {
                    swap = true;
                }
                egui::ComboBox::from_id_source("crop_ratio").selected_text(self.crop_ratio_label.clone()).width(130.0).show_ui(ui, |ui| {
                    if ui.selectable_label(self.crop_ratio.is_none(), "Free").clicked() {
                        pick = Some(("Free".to_owned(), None));
                    }
                    if let Some(loaded) = &self.loaded {
                        let (w, h) = self.display_size(loaded);
                        if ui.selectable_label(self.crop_ratio_label == "Original", "Original").clicked() {
                            pick = Some(("Original".to_owned(), Some((w as f32, h as f32))));
                        }
                    }
                    for (label, w, h) in STANDARD {
                        if ui.selectable_label(self.crop_ratio_label == label, label).clicked() {
                            pick = Some((label.to_owned(), Some((w, h))));
                        }
                    }
                    if !self.crop_presets.list.is_empty() {
                        ui.separator();
                    }
                    for (name, w, h) in &self.crop_presets.list {
                        if ui.selectable_label(self.crop_ratio_label == *name, format!("{name}  ({w} : {h})")).clicked() {
                            pick = Some((name.clone(), Some((*w, *h))));
                        }
                    }
                });
            });
        });
        if has {
            if let Some((label, ratio)) = pick {
                self.set_crop_ratio(label, ratio);
            }
            if swap {
                if let (Some((w, h)), Some(geometry)) = (self.crop_ratio, self.geometry()) {
                    self.crop_ratio = Some((h, w));
                    let c = self.crop_display();
                    // Same centre, the other orientation, as large as fits where it was.
                    let (cx, cy) = ((c[0] + c[2]) * 0.5, (c[1] + c[3]) * 0.5);
                    let side = (c[2] - c[0]).max((c[3] - c[1]) / geometry.aspect);
                    let around = [cx - side * 0.5, cy - side * geometry.aspect * 0.5, cx + side * 0.5, cy + side * geometry.aspect * 0.5];
                    let wanted = crop::with_ratio(around, h / w / geometry.aspect);
                    self.set_crop_display(geometry.constrain(wanted));
                    self.crop_wish = self.crop;
                }
            }
        }
        ui.add_space(6.0);

        let mut angle = self.angle;
        ui::pro_slider(ui, "Angle", &mut angle, -45.0..=45.0, 0.0, "°", theme::ACCENT);
        if has && angle != self.angle {
            self.angle = angle;
            self.refit_crop();
        }

        // Save a ratio of your own.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(egui::RichText::new("New ratio").color(theme::TEXT));
            ui.add(egui::DragValue::new(&mut self.new_ratio.0).speed(0.1).clamp_range(0.1..=1000.0));
            ui.label(egui::RichText::new(":").color(theme::FADED));
            ui.add(egui::DragValue::new(&mut self.new_ratio.1).speed(0.1).clamp_range(0.1..=1000.0));
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add(egui::TextEdit::singleline(&mut self.new_ratio_name).desired_width(ui.available_width() - 116.0).hint_text("name (optional)"));
            if ui::button(ui, "Save", egui::vec2(50.0, 22.0), ui::Look::bordered(), theme::font(12.0), None).on_hover_text("Keep this ratio in the Ratio list").clicked() {
                let (w, h) = self.new_ratio;
                let typed = self.new_ratio_name.trim().to_owned();
                let name = if typed.is_empty() { format!("{w} : {h}") } else { typed };
                self.crop_presets.add(&name, w, h);
                if has {
                    self.set_crop_ratio(name, Some((w, h)));
                }
            }
            let selected = self.crop_ratio_label.clone();
            let is_preset = self.crop_presets.list.iter().any(|(n, _, _)| *n == selected);
            if ui
                .add_enabled_ui(is_preset, |ui| ui::button(ui, "Delete", egui::vec2(54.0, 22.0), ui::Look::secondary(), theme::font(12.0), None))
                .inner
                .on_hover_text("Remove the selected saved ratio")
                .clicked()
            {
                self.crop_presets.remove(&selected);
                self.crop_ratio = None;
                self.crop_ratio_label = "Free".to_owned();
            }
        });
    }

    /// The picture area while cropping: the whole (straightened) picture with the crop
    /// frame over it. Drag the frame to move it, its edges or corners to resize, and
    /// anywhere outside it to rotate.
    fn crop_viewport(&mut self, ui: &mut egui::Ui, rect: egui::Rect, response: &egui::Response) {
        let Some(full) = self.loaded.as_ref().map(|l| self.display_size(l)).map(|(w, h)| egui::vec2(w as f32, h as f32)) else { return };
        let ppp = ui.ctx().pixels_per_point();
        let view_px = rect.size() * ppp;
        // How many frame units the viewport spans, for a given angle.
        let span_for = |angle: f32| {
            let (bw, bh) = crop::Geometry::new(full.x, full.y, angle).bounds();
            let dims = egui::vec2(full.x * bw, full.y * bh);
            let fit = (view_px.x / dims.x).min(view_px.y / dims.y) * 0.9;
            view_px / (dims * fit) * egui::vec2(bw, bh)
        };
        let half = egui::vec2(0.5, 0.5);
        let frame_rect = |c: crop::Rect, span: egui::Vec2| {
            let at = |x: f32, y: f32| rect.min + rect.size() * (half + egui::vec2(x - 0.5, y - 0.5) / span);
            egui::Rect::from_min_max(at(c[0], c[1]), at(c[2], c[3]))
        };

        let span = span_for(self.angle);
        let shown = self.crop_display();
        let on_screen = frame_rect(shown, span);
        let tolerance = 14.0;
        let handle_at = |p: egui::Pos2| -> Option<crop::Handle> {
            if !on_screen.expand(tolerance).contains(p) {
                return None; // outside the frame: rotate
            }
            let left = (p.x - on_screen.left()).abs() < tolerance;
            let top = (p.y - on_screen.top()).abs() < tolerance;
            Some(crop::Handle {
                left,
                right: !left && (p.x - on_screen.right()).abs() < tolerance,
                top,
                bottom: !top && (p.y - on_screen.bottom()).abs() < tolerance,
            })
        };

        if let Some(p) = response.hover_pos() {
            let icon = match handle_at(p) {
                None => egui::CursorIcon::Grab,
                Some(h) if h.is_move() => egui::CursorIcon::Move,
                Some(h) if (h.left && h.top) || (h.right && h.bottom) => egui::CursorIcon::ResizeNwSe,
                Some(h) if (h.right && h.top) || (h.left && h.bottom) => egui::CursorIcon::ResizeNeSw,
                Some(h) if h.left || h.right => egui::CursorIcon::ResizeHorizontal,
                Some(_) => egui::CursorIcon::ResizeVertical,
            };
            ui.ctx().set_cursor_icon(icon);
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(origin) = response.interact_pointer_pos() {
                self.crop_drag = Some(CropDrag { handle: handle_at(origin), start: shown, origin, angle: self.angle });
            }
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            if let (Some(drag), Some(p)) = (self.crop_drag, response.interact_pointer_pos()) {
                let geometry = crop::Geometry::new(full.x, full.y, self.angle);
                match drag.handle {
                    Some(handle) => {
                        let delta = (p - drag.origin) / rect.size() * span;
                        let ratio = self.crop_ratio.map(|(w, h)| w / h / geometry.aspect);
                        let wanted = crop::dragged(drag.start, handle, (delta.x, delta.y), ratio);
                        self.set_crop_display(geometry.toward(shown, wanted));
                        self.crop_wish = self.crop;
                    }
                    None => {
                        // Turn the picture with the pointer, about the middle of the frame.
                        let centre = on_screen.center();
                        let mut turn = ((p - centre).angle() - (drag.origin - centre).angle()).to_degrees();
                        if turn > 180.0 {
                            turn -= 360.0;
                        } else if turn < -180.0 {
                            turn += 360.0;
                        }
                        self.angle = (drag.angle - turn).clamp(-45.0, 45.0);
                        self.refit_crop();
                    }
                }
            }
        }
        let rotating = self.crop_drag.is_some_and(|d| d.handle.is_none());
        if response.drag_stopped() {
            self.crop_drag = None;
        }
        let finish = response.double_clicked() || (!ui.ctx().wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Enter)));

        // Draw the whole straightened picture, then the frame over it.
        let span = span_for(self.angle);
        if let Some(loaded) = &self.loaded {
            let renderer = self.render_state.renderer.read();
            if let Some(res) = renderer.callback_resources.get::<ImageRenderer>() {
                let mut uniforms = self.uniforms(loaded, [span.x, span.y, 0.5, 0.5], res.shader_encodes_srgb);
                uniforms.crop = [-9.0, -9.0, 9.0, 9.0];
                res.write_uniforms(&self.render_state.queue, &uniforms);
            }
        }
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, ImageCallback { before: false }));

        let frame = frame_rect(self.crop_display(), span);
        // Whole pixels, so the four shaded panels meet without a visible seam.
        let frame = egui::Rect::from_min_max(frame.min.round(), frame.max.round());
        let painter = ui.painter().with_clip_rect(rect);
        let shade = egui::Color32::from_black_alpha(150);
        painter.rect_filled(egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), frame.top())), 0.0, shade);
        painter.rect_filled(egui::Rect::from_min_max(egui::pos2(rect.left(), frame.bottom()), rect.max), 0.0, shade);
        painter.rect_filled(egui::Rect::from_min_max(egui::pos2(rect.left(), frame.top()), frame.left_bottom()), 0.0, shade);
        painter.rect_filled(egui::Rect::from_min_max(frame.right_top(), egui::pos2(rect.right(), frame.bottom())), 0.0, shade);
        let lines = if rotating { 8 } else { 3 };
        let thin = egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(if rotating { 70 } else { 110 }));
        for i in 1..lines {
            let t = i as f32 / lines as f32;
            let x = frame.left() + frame.width() * t;
            let y = frame.top() + frame.height() * t;
            painter.line_segment([egui::pos2(x, frame.top()), egui::pos2(x, frame.bottom())], thin);
            painter.line_segment([egui::pos2(frame.left(), y), egui::pos2(frame.right(), y)], thin);
        }
        painter.rect_stroke(frame, 0.0, egui::Stroke::new(1.5_f32, egui::Color32::WHITE));
        for corner in [frame.left_top(), frame.right_top(), frame.left_bottom(), frame.right_bottom()] {
            painter.rect_filled(egui::Rect::from_center_size(corner, egui::vec2(9.0, 9.0)), 1.0, egui::Color32::WHITE);
        }
        let label = match &self.loaded {
            Some(loaded) => {
                let (w, h) = self.output_size(loaded);
                format!("{w} × {h} px   {:.1}°", self.angle)
            }
            None => String::new(),
        };
        let pos = rect.left_bottom() + egui::vec2(10.0, -8.0);
        let font = egui::FontId::proportional(13.0);
        painter.text(pos + egui::vec2(1.0, 1.0), egui::Align2::LEFT_BOTTOM, &label, font.clone(), egui::Color32::BLACK);
        painter.text(pos, egui::Align2::LEFT_BOTTOM, &label, font, egui::Color32::from_gray(220));

        if finish {
            self.crop_mode = false;
            self.crop_drag = None;
        }
    }

    /// Size of the picture as displayed (after rotation), in pixels.
    fn display_size(&self, loaded: &Loaded) -> (u32, u32) {
        let (w, h) = (loaded.width as u32, loaded.height as u32);
        if self.quarter_turns % 2 == 1 {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// Render the picture at full size on the GPU, then resize, encode and write it on a
    /// background thread. The GPU part takes a moment and runs here; the slow part (resize
    /// and compression) does not block the window.
    fn start_export(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.sync_heal();
        let Some(loaded) = &self.loaded else { return };
        if self.export_job.is_some() {
            return;
        }
        let (width, height) = self.output_size(loaded);
        let region = self.crop_region();
        let uniforms = self.uniforms(loaded, [1.0, 1.0, 0.5, 0.5], true);
        let rendered = {
            let rs = &self.render_state;
            let renderer = rs.renderer.read();
            match renderer.callback_resources.get::<ImageRenderer>() {
                Some(res) => res.render_full(&rs.device, &rs.queue, &uniforms, width, height, region),
                None => Err("The renderer is not ready.".to_owned()),
            }
        };
        let rgb = match rendered {
            Ok(rgb) => rgb,
            Err(message) => {
                self.export_status = Some(Err(message));
                return;
            }
        };

        let settings = self.export.clone();
        let meta = loaded.photo.clone();
        let path_for_reveal = path.clone();
        let focus_out = self.focus_out.take();
        if let Some(info_path) = self.info_out.take() {
            let info = serde_json::json!({
                "as_shot_temperature": loaded.color.as_shot_kelvin.round(),
                "temperature": self.adjust.kelvin.round(),
            });
            let _ = std::fs::write(info_path, info.to_string());
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            if let Some(focus_path) = focus_out {
                if let Err(e) = focus::write(&focus_path, &rgb, width, height) {
                    eprintln!("Focus measurement failed: {e}");
                }
            }
            let result = export::save(&path, rgb, width, height, &settings, &meta).map(|saved| {
                format!(
                    "Saved {} × {} px, {:.1} MB in {:.1} s\n{}",
                    saved.width,
                    saved.height,
                    saved.bytes as f64 / 1.0e6,
                    started.elapsed().as_secs_f32(),
                    path.display()
                )
            });
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.export_status = None;
        self.export_job = Some(rx);
        if self.batch.is_none() {
            self.export_last = Some(path_for_reveal);
        }
    }

    fn poll_export(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.export_job else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("The export stopped unexpectedly.".to_owned()),
        };
        self.export_job = None;
        self.export_status = Some(result);
        if self.batch.is_none() && !self.quit_after_export {
            self.export_popup = true;
        }
        if self.quit_after_export {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// The photos Export applies to: the filmstrip selection, or else the open photo.
    fn export_targets(&self) -> Vec<PathBuf> {
        match &self.loaded {
            Some(loaded) => self.filmstrip.targets(Some(&loaded.path)),
            None => Vec::new(),
        }
    }

    /// Ask for a folder, then export every one of `targets` into it.
    fn start_batch(&mut self, targets: Vec<PathBuf>) {
        let mut dialog = rfd::FileDialog::new().set_title(format!("Export {} photos to folder", targets.len()));
        let start = self.presets.last_dir.clone().or_else(|| self.loaded.as_ref().and_then(|l| l.path.parent().map(PathBuf::from)));
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
        if let Some(dir) = dialog.pick_folder() {
            self.presets.last_dir = Some(dir.clone());
            self.export_status = None;
            self.batch = Some(Batch {
                total: targets.len(),
                queue: targets.into(),
                dir,
                done: 0,
                failed: Vec::new(),
                current: None,
                exporting: false,
                return_to: self.loaded.as_ref().map(|l| l.path.clone()),
                quit: false,
            });
        }
    }

    /// One step of a batch export; called every frame while one is running.
    fn advance_batch(&mut self, ctx: &egui::Context) {
        if self.loading.is_some() || self.export_job.is_some() {
            return;
        }
        let Some(mut batch) = self.batch.take() else { return };
        ctx.request_repaint();
        let name_of = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let open = self.loaded.as_ref().map(|l| l.path.clone());

        match batch.current.clone() {
            // Pick the next photo and open it (unless it is the one already open).
            None => match batch.queue.pop_front() {
                Some(next) => {
                    if open.as_ref() != Some(&next) {
                        self.start_loading(next.clone(), ctx);
                    }
                    batch.current = Some(next);
                    batch.exporting = false;
                }
                None => {
                    let mut message = format!("Exported {} of {} photos to\n{}", batch.done, batch.total, batch.dir.display());
                    if !batch.failed.is_empty() {
                        message.push_str(&format!("\nFailed: {}", batch.failed.join(", ")));
                    }
                    self.export_status = Some(if batch.failed.is_empty() { Ok(message) } else { Err(message) });
                    if !batch.quit {
                        self.export_last = Some(batch.dir.clone());
                        self.export_popup = true;
                    }
                    if batch.quit {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    } else if let Some(back) = batch.return_to.filter(|p| open.as_ref() != Some(p)) {
                        self.start_loading(back, ctx);
                    }
                    return; // the batch is finished and stays removed
                }
            },
            // The photo has finished opening: export it.
            Some(photo) if !batch.exporting => {
                if open.as_ref() != Some(&photo) {
                    batch.failed.push(name_of(&photo)); // it could not be opened
                    batch.current = None;
                } else if self.render_key.is_some() {
                    let stem = photo.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "export".to_owned());
                    let _ = std::fs::create_dir_all(&batch.dir);
                    let target = free_name(&batch.dir, &stem, self.export.format.extension());
                    self.start_export(target, ctx);
                    if self.export_job.is_some() {
                        batch.exporting = true;
                    } else {
                        batch.failed.push(name_of(&photo));
                        batch.current = None;
                    }
                }
            }
            // Its export has finished.
            Some(photo) => {
                match &self.export_status {
                    Some(Ok(_)) => batch.done += 1,
                    _ => batch.failed.push(name_of(&photo)),
                }
                batch.current = None;
            }
        }
        self.batch = Some(batch);
    }

    /// Ask where to save, then export with the current settings.
    fn export_dialog(&mut self, ctx: &egui::Context) {
        let targets = self.export_targets();
        self.export_dialog_for(targets, ctx);
    }

    /// Export `targets`: one open photo through a save dialog, several into a folder.
    fn export_dialog_for(&mut self, targets: Vec<PathBuf>, ctx: &egui::Context) {
        if targets.len() > 1 {
            self.start_batch(targets);
            return;
        }
        let Some(loaded) = &self.loaded else { return };
        let format = self.export.format;
        let stem = loaded.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "export".to_owned());
        let mut dialog = rfd::FileDialog::new()
            .set_title("Export")
            .set_file_name(format!("{stem}.{}", format.extension()))
            .add_filter(format.label(), &[format.extension()]);
        if let Some(dir) = self.presets.last_dir.as_deref().or(loaded.path.parent()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(mut path) = dialog.save_file() {
            self.presets.last_dir = path.parent().map(PathBuf::from);
            // Make sure the name carries the right extension for the chosen format.
            let ok = path.extension().and_then(|e| e.to_str()).and_then(export::Format::from_extension) == Some(format);
            if !ok {
                let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
                name.push(format!(".{}", format.extension()));
                path.set_file_name(name);
            }
            if path == loaded.path {
                self.export_status = Some(Err("Refusing to overwrite the raw file.".to_owned()));
                return;
            }
            self.start_export(path, ctx);
        }
    }

    fn export_window(&mut self, ctx: &egui::Context) {
        if !self.export_open {
            return;
        }
        let Some(loaded) = &self.loaded else {
            self.export_open = false;
            return;
        };
        let (width, height) = self.output_size(loaded);
        let has_location = loaded.photo.gps.is_some();
        let busy = self.export_job.is_some() || self.batch.is_some() || self.dng_job.is_some();
        let count = self.export_targets().len();
        let progress = self.batch.as_ref().map(|b| ((b.done + b.failed.len() + 1).min(b.total), b.total));
        let mut cancel = false;
        let mut open = true;
        let mut export_now = false;
        egui::Window::new("Export").open(&mut open).collapsible(false).resizable(false).default_width(340.0).show(ctx, |ui| {
            // Presets: pick one to load it; type a name and Save to keep the current settings.
            ui.horizontal(|ui| {
                ui.label("Preset");
                let current = self.presets.list.iter().find(|(_, p)| *p == self.export).map(|(n, _)| n.clone());
                egui::ComboBox::from_id_source("export_preset")
                    .selected_text(current.clone().unwrap_or_else(|| "Custom".to_owned()))
                    .width(190.0)
                    .show_ui(ui, |ui| {
                        if self.presets.list.is_empty() {
                            ui.weak("No presets saved yet");
                        }
                        for (name, preset) in &self.presets.list {
                            if ui.selectable_label(current.as_deref() == Some(name.as_str()), name).clicked() {
                                self.export = preset.clone();
                                self.preset_name = name.clone();
                            }
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Save as");
                ui.add(egui::TextEdit::singleline(&mut self.preset_name).desired_width(130.0).hint_text("preset name"));
                let name = self.preset_name.trim().to_owned();
                if ui.add_enabled(!name.is_empty(), egui::Button::new("Save")).on_hover_text("Keep the settings below under this name").clicked() {
                    self.presets.add(&name, &self.export);
                }
                let exists = self.presets.list.iter().any(|(n, _)| *n == name);
                if ui.add_enabled(exists, egui::Button::new("Delete")).clicked() {
                    self.presets.remove(&name);
                }
            });
            ui.separator();
            let s = &mut self.export;
            egui::Grid::new("export_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Format");
                let dng = &mut self.export_dng;
                let shown = if *dng { "DNG (original raw)".to_owned() } else { s.format.label().to_owned() };
                egui::ComboBox::from_id_source("export_format").selected_text(shown).show_ui(ui, |ui| {
                    for f in export::Format::ALL {
                        if ui.selectable_label(!*dng && s.format == f, f.label()).clicked() {
                            s.format = f;
                            *dng = false;
                        }
                    }
                    if ui
                        .selectable_label(*dng, "DNG (original raw)")
                        .on_hover_text("The original raw data in Adobe's open raw format, losslessly compressed, for editing elsewhere")
                        .clicked()
                    {
                        *dng = true;
                    }
                });
                ui.end_row();

                if *dng {
                    ui.label("Contents");
                    ui.label("Original raw data, lossless.\nDevelop's edits are saved beside it (.dvlp);\nother apps open it unedited.");
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut self.export_dng_embed, "Embed the original raw file")
                        .on_hover_text("Stores the original raw file inside the DNG so it can be extracted again (about twice the size)");
                    ui.end_row();
                    ui.label("Artist");
                    ui.add(egui::TextEdit::singleline(&mut s.artist).desired_width(200.0));
                    ui.end_row();
                    return;
                }

                if s.format == export::Format::Jpeg {
                    ui.label("Quality");
                    ui.add(egui::Slider::new(&mut s.quality, 1..=100));
                    ui.end_row();
                } else {
                    ui.label("Bit depth");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut s.sixteen_bit, false, "8-bit");
                        ui.radio_value(&mut s.sixteen_bit, true, "16-bit");
                    });
                    ui.end_row();
                }

                ui.label("Colour space");
                ui.label("sRGB");
                ui.end_row();

                ui.label("Size");
                ui.horizontal(|ui| {
                    let before = s.resize;
                    egui::ComboBox::from_id_source("export_resize").selected_text(s.resize.label()).show_ui(ui, |ui| {
                        for r in export::Resize::ALL {
                            ui.selectable_value(&mut s.resize, r, r.label());
                        }
                    });
                    if s.resize != before {
                        s.resize_value = s.resize.default_value();
                    }
                    match s.resize {
                        export::Resize::Original => {}
                        export::Resize::Megapixels => {
                            ui.add(egui::DragValue::new(&mut s.resize_value).speed(0.1).clamp_range(0.1..=400.0).suffix(s.resize.unit()));
                        }
                        export::Resize::Percent => {
                            ui.add(egui::DragValue::new(&mut s.resize_value).speed(0.5).clamp_range(1.0..=400.0).suffix(s.resize.unit()));
                        }
                        _ => {
                            ui.add(egui::DragValue::new(&mut s.resize_value).speed(4.0).clamp_range(16.0..=30000.0).fixed_decimals(0).suffix(s.resize.unit()));
                        }
                    }
                });
                ui.end_row();

                ui.label("");
                ui.add_enabled(s.resize != export::Resize::Original, egui::Checkbox::new(&mut s.no_enlarge, "Don't enlarge"));
                ui.end_row();

                let (tw, th) = export::target_size(s, width, height);
                ui.label("Output");
                ui.label(format!("{tw} × {th} px  ({:.1} MP)", tw as f64 * th as f64 / 1.0e6));
                ui.end_row();

                ui.label("Resolution");
                ui.add(egui::DragValue::new(&mut s.ppi).clamp_range(1..=2400).suffix(" ppi"))
                    .on_hover_text("Print size hint written to the file. It does not change the pixels.");
                ui.end_row();

                ui.label("Metadata");
                egui::ComboBox::from_id_source("export_metadata").selected_text(s.metadata.label()).show_ui(ui, |ui| {
                    for m in export::Metadata::ALL {
                        let hover = match m {
                            export::Metadata::All => "Camera, lens, exposure, capture date, artist and copyright",
                            export::Metadata::CopyrightOnly => "Artist and copyright only",
                            export::Metadata::None => "No metadata at all",
                        };
                        ui.selectable_value(&mut s.metadata, m, m.label()).on_hover_text(hover);
                    }
                });
                ui.end_row();
                if s.format == export::Format::Tiff && s.metadata == export::Metadata::All {
                    ui.label("");
                    ui.weak("TIFF: camera, date, artist and copyright only");
                    ui.end_row();
                }

                let is_tiff = s.format == export::Format::Tiff;
                let writes_metadata = s.metadata != export::Metadata::None;
                ui.label("");
                ui.add_enabled(
                    writes_metadata && !is_tiff && s.metadata == export::Metadata::All && has_location,
                    egui::Checkbox::new(&mut s.remove_location, "Remove location"),
                )
                .on_disabled_hover_text(if has_location { "Location is only written with \"All\"" } else { "This file has no location data" });
                ui.end_row();

                ui.label("Artist");
                ui.add_enabled(writes_metadata, egui::TextEdit::singleline(&mut s.artist).desired_width(200.0));
                ui.end_row();

                ui.label("Copyright");
                ui.add_enabled(writes_metadata, egui::TextEdit::singleline(&mut s.copyright).desired_width(200.0));
                ui.end_row();
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let label = if count > 1 { format!("Export {count} photos…") } else { "Export…".to_owned() };
                let hint = if count > 1 { "Each selected photo is exported with its own settings, into a folder you choose" } else { "Choose where to save" };
                if ui.add_enabled(!busy, egui::Button::new(label)).on_hover_text(hint).clicked() {
                    export_now = true;
                }
                if busy {
                    ui.spinner();
                    match progress {
                        Some((at, total)) => {
                            ui.label(format!("Exporting {at} of {total}…"));
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        }
                        None => {
                            ui.label("Exporting…");
                        }
                    }
                }
            });
            match &self.export_status {
                Some(Ok(message)) => {
                    ui.add_space(4.0);
                    ui.label(message);
                }
                Some(Err(message)) => {
                    ui.add_space(4.0);
                    ui.colored_label(ui.visuals().error_fg_color, message);
                }
                None => {}
            }
        });
        if !open {
            self.export_open = false;
        }
        if cancel {
            if let Some(batch) = &mut self.batch {
                batch.total -= batch.queue.len();
                batch.queue.clear();
            }
        }
        if export_now {
            if self.export_dng {
                let targets = self.export_targets();
                if targets.len() > 1 {
                    self.start_dng(targets, self.export_dng_embed, ctx);
                } else {
                    self.export_dng_file(ctx);
                }
            } else {
                self.export_dialog(ctx);
            }
        }
    }

    /// The open photo's settings as the edit history records them (the AI denoise switch is
    /// left out: changing it reopens the photo).
    fn history_snapshot(&self) -> settings::Saved {
        settings::Saved { denoise: false, ..self.snapshot() }
    }

    /// Record a change in the open photo's history once it is finished: not while a slider
    /// is being dragged or a number typed, so one drag is one undo step.
    fn track_history(&mut self, ctx: &egui::Context) {
        if self.loading.is_some() {
            return;
        }
        let Some(path) = self.loaded.as_ref().map(|l| l.path.clone()) else { return };
        let now = self.history_snapshot();
        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.wants_keyboard_input();
        let history = self.history.entry(path).or_default();
        match &history.committed {
            None => history.committed = Some(now),
            Some(before) if *before != now && !busy => {
                history.undo.push(before.clone());
                if history.undo.len() > HISTORY_STEPS {
                    history.undo.remove(0);
                }
                history.redo.clear();
                history.committed = Some(now);
            }
            _ => {}
        }
    }

    /// Take in edits, ratings and versions that another program saved for photos of the
    /// folder (an AI assistant through the MCP server, another copy of Develop): the newer
    /// change wins. The open photo shows them at once, and Undo brings back what it had.
    fn take_outside_changes(&mut self, ctx: &egui::Context) {
        let Some(watch) = &mut self.folder_watch else { return };
        let settled = watch.settled(ctx);
        let dir = watch.dir().to_path_buf();
        self.outside.extend(settled.photos);
        if settled.everything {
            self.outside.extend(self.store.photos_in(&dir));
        }
        // Not while a photo is opening: its settings are put on it when it is ready.
        if self.outside.is_empty() || self.loading.is_some() {
            return;
        }
        let open = self.loaded.as_ref().map(|l| l.path.clone());
        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.wants_keyboard_input();
        for photo in std::mem::take(&mut self.outside) {
            if busy && open.as_ref() == Some(&photo) {
                self.outside.insert(photo);
                continue;
            }
            if !self.store.reload_sidecar(&photo) {
                continue;
            }
            ctx.request_repaint();
            if open.as_ref() != Some(&photo) {
                // Its saved thumbnail shows the old edit.
                if let Some(file) = settings::thumb_file(&photo) {
                    let _ = std::fs::remove_file(file);
                }
                self.filmstrip.reload(&photo);
                continue;
            }
            let Some(defaults) = self.defaults.clone() else { continue };
            let theirs = self.store.get(&photo).cloned().unwrap_or_else(|| self.storable(defaults));
            if theirs == self.storable(self.snapshot()) {
                continue; // only the rating or versions changed
            }
            if theirs.denoise != self.denoise {
                // AI denoise is part of opening the photo: open it again with the new switch.
                self.denoise = theirs.denoise;
                self.denoise_override = Some(theirs.denoise);
                self.reload = true;
            }
            self.restore_snapshot(&theirs);
            self.outside_note = Some((photo, Instant::now()));
            ctx.request_repaint_after(OUTSIDE_NOTE_TIME);
        }
    }

    /// Put a recorded state back on the open photo.
    fn restore_snapshot(&mut self, saved: &settings::Saved) {
        self.apply_saved(saved);
        // 255 (settings saved in their stored form, as by the MCP server) means the camera's
        // own orientation.
        self.quarter_turns = match saved.quarter_turns {
            q if q < 4 => q,
            _ => self.defaults.as_ref().map_or(self.quarter_turns, |d| d.quarter_turns % 4),
        };
        self.crop = saved.crop;
        self.crop_wish = saved.crop;
        self.angle = saved.angle;
        self.render_key = None;
    }

    fn can_undo(&self) -> bool {
        self.loaded.as_ref().and_then(|l| self.history.get(&l.path)).is_some_and(|h| !h.undo.is_empty())
    }

    fn can_redo(&self) -> bool {
        self.loaded.as_ref().and_then(|l| self.history.get(&l.path)).is_some_and(|h| !h.redo.is_empty())
    }

    /// Step the open photo back (`back`) or forward through its history.
    fn step_history(&mut self, back: bool) {
        let Some(path) = self.loaded.as_ref().map(|l| l.path.clone()) else { return };
        let current = self.history_snapshot();
        let Some(history) = self.history.get_mut(&path) else { return };
        let target = if back { history.undo.pop() } else { history.redo.pop() };
        let Some(target) = target else { return };
        let here = history.committed.clone().unwrap_or(current);
        if back {
            history.redo.push(here);
        } else {
            history.undo.push(here);
        }
        history.committed = Some(target.clone());
        self.restore_snapshot(&target);
    }

    /// Convert the open photo to a DNG file chosen with a save dialog, in the background.
    fn export_dng_file(&mut self, ctx: &egui::Context) {
        let Some(photo) = self.loaded.as_ref().map(|l| l.path.clone()) else { return };
        if self.dng_job.is_some() {
            return;
        }
        let stem = photo.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "photo".to_owned());
        let mut dialog = rfd::FileDialog::new().set_title("Export as DNG").set_file_name(format!("{stem}.dng")).add_filter("DNG", &["dng"]);
        if let Some(dir) = self.presets.last_dir.as_deref().or(photo.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut target) = dialog.save_file() else { return };
        if target.extension().map_or(true, |e| !e.eq_ignore_ascii_case("dng")) {
            target.set_extension("dng");
        }
        self.presets.last_dir = target.parent().map(PathBuf::from);
        self.store.flush(true);
        let (embed, artist) = (self.export_dng_embed, Some(self.export.artist.clone()).filter(|a| !a.is_empty()));
        let (tx, rx) = mpsc::channel();
        let (job_target, repaint, name) = (target.clone(), ctx.clone(), photo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        std::thread::spawn(move || {
            let _ = tx.send(todng::Event::Progress { done: 0, total: 1, name });
            let started = Instant::now();
            let result = todng::convert_to(&photo, &job_target, embed, artist).map(|out| {
                let size = std::fs::metadata(&out).map_or(0, |m| m.len());
                format!("Saved DNG, {:.1} MB in {:.1} s\n{}", size as f64 / 1.0e6, started.elapsed().as_secs_f32(), out.display())
            });
            let _ = tx.send(todng::Event::Finished(result));
            repaint.request_repaint();
        });
        self.dng_progress = (0, 1, String::new());
        self.dng_cancel = None;
        self.dng_job = Some(rx);
        self.export_last = Some(target);
    }

    /// Ask for a folder, then convert `targets` to DNG there in the background.
    fn start_dng(&mut self, targets: Vec<PathBuf>, embed: bool, ctx: &egui::Context) {
        if self.dng_job.is_some() || targets.is_empty() {
            return;
        }
        let title = if targets.len() > 1 { format!("Convert {} photos to DNG in folder", targets.len()) } else { "Convert to DNG in folder".to_owned() };
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(dir) = targets[0].parent() {
            dialog = dialog.set_directory(dir);
        }
        let Some(dir) = dialog.pick_folder() else { return };
        // The edits on show must be in their sidecars before those are copied.
        self.store.flush(true);
        let artist = Some(self.export.artist.clone()).filter(|a| !a.is_empty());
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (stop, repaint, folder) = (cancel.clone(), ctx.clone(), dir.clone());
        std::thread::spawn(move || {
            todng::convert_all(targets, folder, embed, artist, tx, stop);
            repaint.request_repaint();
        });
        self.dng_progress = (0, 0, String::new());
        self.dng_cancel = Some(cancel);
        self.dng_job = Some(rx);
        self.export_last = Some(dir);
    }

    /// Export progress (a bar with the percentage done) while exporting, then a window saying
    /// it is complete, with the way to the exported files.
    fn export_windows(&mut self, ctx: &egui::Context) {
        // Convert to DNG: take in its progress; when it is done, the completion window below
        // shows the result.
        if let Some(rx) = &self.dng_job {
            let mut finished = None;
            while let Ok(event) = rx.try_recv() {
                match event {
                    todng::Event::Progress { done, total, name } => self.dng_progress = (done, total, name),
                    todng::Event::Finished(result) => finished = Some(result),
                }
            }
            if let Some(result) = finished {
                self.dng_job = None;
                self.dng_cancel = None;
                self.export_status = Some(result);
                self.export_popup = true;
            }
        }
        if self.dng_job.is_some() {
            let (done, total, name) = self.dng_progress.clone();
            let mut cancel = false;
            egui::Window::new("Converting to DNG").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.label(egui::RichText::new(format!("Converting photo {} of {}", (done + 1).min(total.max(1)), total.max(1))).font(theme::bold(14.0)).color(theme::TEXT));
                ui.label(egui::RichText::new(name).font(theme::font(12.0)).color(theme::FADED));
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).show_percentage().desired_width(360.0));
                ui.add_space(6.0);
                if ui::button(ui, "Cancel", egui::vec2(0.0, 26.0), ui::Look::bordered(), theme::font(12.5), None).on_hover_text("Stop after the photo being converted").clicked() {
                    cancel = true;
                }
            });
            if cancel {
                if let Some(flag) = &self.dng_cancel {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        let window = |title: &str| egui::Window::new(title.to_owned()).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).default_width(380.0);
        let name_of = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(batch) = self.batch.as_ref().filter(|b| !b.quit) {
            let finished = batch.done + batch.failed.len();
            let fraction = finished as f32 / batch.total.max(1) as f32;
            let current = batch.current.as_deref().map(name_of).unwrap_or_default();
            let mut cancel = false;
            window("Exporting").show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.label(egui::RichText::new(format!("Exporting photo {} of {}", (finished + 1).min(batch.total), batch.total)).font(theme::bold(14.0)).color(theme::TEXT));
                ui.label(egui::RichText::new(current).font(theme::font(12.0)).color(theme::FADED));
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(fraction).show_percentage().desired_width(360.0));
                ui.add_space(6.0);
                ui.label(egui::RichText::new(format!("To {}", batch.dir.display())).font(theme::font(11.5)).color(theme::FADED));
                ui.add_space(4.0);
                if ui::button(ui, "Cancel", egui::vec2(0.0, 26.0), ui::Look::bordered(), theme::font(12.5), None).on_hover_text("Stop after the photo being exported").clicked() {
                    cancel = true;
                }
            });
            if cancel {
                if let Some(batch) = &mut self.batch {
                    batch.queue.clear();
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        if self.export_job.is_some() && !self.quit_after_export {
            let file = self.export_last.as_deref().map(name_of).unwrap_or_default();
            window("Exporting").show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.label(egui::RichText::new("Exporting").font(theme::bold(14.0)).color(theme::TEXT));
                ui.label(egui::RichText::new(file).font(theme::font(12.0)).color(theme::FADED));
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(0.0).animate(true).desired_width(360.0).text("Saving…"));
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        if !self.export_popup {
            return;
        }
        let Some(status) = self.export_status.clone() else {
            self.export_popup = false;
            return;
        };
        let (title, message, ok) = match status {
            Ok(message) => ("Export complete", message, true),
            Err(message) => ("Export finished with problems", message, false),
        };
        let mut close = false;
        window(title).show(ctx, |ui| {
            ui.set_min_width(360.0);
            ui.horizontal(|ui| {
                let (mark, color) = if ok { ("✔", theme::TEAL) } else { ("⚠", ui.visuals().error_fg_color) };
                ui.label(egui::RichText::new(mark).font(theme::bold(18.0)).color(color));
                ui.label(egui::RichText::new(title).font(theme::bold(14.0)).color(theme::TEXT));
            });
            ui.add_space(4.0);
            ui.label(egui::RichText::new(message).font(theme::font(12.0)).color(theme::TEXT));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui::button(ui, "OK", egui::vec2(72.0, 28.0), ui::Look::primary(), theme::bold(13.0), None).clicked() {
                    close = true;
                }
                if let Some(target) = self.export_last.clone() {
                    if ui::button(ui, "Show in folder", egui::vec2(0.0, 28.0), ui::Look::bordered(), theme::font(12.5), None).clicked() {
                        reveal_in_folder(&target);
                        close = true;
                    }
                }
            });
        });
        if close {
            self.export_popup = false;
        }
    }

    /// Work out the healing gains of Heal spots that are new or have changed (reusing any
    /// already worked out for the same spot, if it only moved in the list) and upload them
    /// to the spot's atlas slot. Called once a frame, before anything holds the renderer.
    fn sync_heal(&self) {
        if self.loaded.is_none() || self.spot_preview.width == 0 {
            return;
        }
        let n = self.spots.len().min(spots::MAX_SPOTS);
        let mut grids = self.heal_grids.borrow_mut();
        let mut changed = Vec::new();
        let mut fresh: Vec<Option<spots::HealGrid>> = Vec::with_capacity(n);
        for i in 0..n {
            if self.spots[i].clone {
                fresh.push(None);
                continue;
            }
            let key = spots::heal_key(&self.spots, i, &self.spot_preview);
            if grids.get(i).and_then(|g| g.as_ref()).is_some_and(|g| g.key == key) {
                fresh.push(grids[i].clone());
                continue;
            }
            let started = std::time::Instant::now();
            let grid = grids.iter().flatten().find(|g| g.key == key).cloned().unwrap_or_else(|| self.spot_preview.heal_grid(&self.spots, i));
            if std::env::var_os("DEVELOP_DEBUG_HEAL").is_some() {
                eprintln!("heal grid {i}: {} x {} in {:.1} ms", grid.nx, grid.ny, started.elapsed().as_secs_f64() * 1000.0);
            }
            changed.push(i);
            fresh.push(Some(grid));
        }
        *grids = fresh;
        if changed.is_empty() {
            return;
        }
        let rs = &self.render_state;
        let renderer = rs.renderer.read();
        if let Some(res) = renderer.callback_resources.get::<ImageRenderer>() {
            for i in changed {
                if let Some(grid) = &grids[i] {
                    res.write_heal_grid(&rs.queue, i, grid);
                }
            }
        }
    }

    /// Switch the white balance eyedropper on or off (the spot tool and crop mode go off).
    fn set_wb_pick(&mut self, on: bool) {
        self.wb_pick = on;
        if on {
            self.set_spot_tool(false);
            self.crop_mode = false;
        }
    }

    fn set_spot_tool(&mut self, on: bool) {
        if on {
            self.wb_pick = false;
        }
        self.spot_tool = on;
        self.spot_drag = None;
        self.spot_paint = None;
        if on {
            self.crop_mode = false;
            self.spot_hide = false;
            self.spot_reveal = true;
        } else {
            self.spot_selected = None;
        }
    }

    /// Where the picture lies on screen in the viewport, for the spot tool.
    fn spot_map(&self, loaded: &Loaded, rect: egui::Rect, scale: egui::Vec2, cropped: [f32; 4]) -> spots::ScreenMap {
        let view = [scale.x * cropped[2], scale.y * cropped[3], cropped[0] + self.center.x * cropped[2], cropped[1] + self.center.y * cropped[3]];
        let u = self.uniforms_with(loaded, view, true, &self.adjust, self.tint, self.sharpen, self.presence, &self.lens);
        spots::ScreenMap::new(&u, rect, loaded.width / loaded.height.max(1.0))
    }

    /// The spot the panel acts on: the selected one, else the brush for new spots.
    fn spot_target(&mut self) -> &mut spots::Spot {
        match self.spot_selected.filter(|&i| i < self.spots.len()) {
            Some(i) => &mut self.spots[i],
            None => &mut self.spot_brush,
        }
    }

    /// [ / ] (smaller / bigger) and Shift+[ / Shift+] (harder / softer edge): the brush, and
    /// the selected spot if there is one.
    fn spot_keys(&mut self, up: bool, shift: bool) {
        let size = |r: f32| (if up { r * 1.15 } else { r / 1.15 }).clamp(SPOT_MIN, SPOT_MAX);
        let soft = |f: f32| ((f + if up { 0.1 } else { -0.1 }) * 10.0).round().clamp(0.0, 10.0) / 10.0;
        let selected = self.spot_selected.filter(|&i| i < self.spots.len());
        for spot in std::iter::once(&mut self.spot_brush).chain(selected.map(|i| &mut self.spots[i])) {
            if shift {
                spot.feather = soft(spot.feather);
            } else {
                spot.radius = size(spot.radius);
            }
        }
    }

    fn spot_hit(&self, map: &spots::ScreenMap, t: [f32; 2]) -> Option<(usize, bool)> {
        spot_hit(&self.spots, self.spot_selected, map, t)
    }

    /// The spots' outlines, the brush under the pointer (or the stroke being painted), and a
    /// hint, over the picture.
    fn spot_overlay(&self, ui: &egui::Ui, response: &egui::Response, map: &spots::ScreenMap, rect: egui::Rect) {
        let painter = ui.painter().with_clip_rect(rect);
        let shadow = egui::Stroke::new(3.0_f32, egui::Color32::from_black_alpha(110));
        let ring = |edges: Vec<[egui::Pos2; 2]>, stroke: egui::Stroke| {
            for e in &edges {
                painter.line_segment(*e, shadow);
            }
            for e in edges {
                painter.line_segment(e, stroke);
            }
        };
        let hovered = response.hover_pos().map(|p| map.to_texture(p));
        let hit = hovered.and_then(|t| self.spot_hit(map, t));
        if !self.spot_hide {
            for (i, s) in self.spots.iter().enumerate() {
                let points: Vec<[f32; 2]> = s.points().collect();
                if self.spot_selected == Some(i) {
                    let (a, b) = (map.to_screen([s.x, s.y]), map.to_screen([s.sx, s.sy]));
                    let dir = (a - b).normalized();
                    let r_a = s.radius * map.points_per_height([s.x, s.y]);
                    let r_b = s.radius * map.points_per_height([s.sx, s.sy]);
                    // A line from the source's edge to the target's (at their first points),
                    // then both outlines.
                    if (a - b).length() > r_a + r_b {
                        painter.line_segment([b + dir * r_b, a - dir * r_a], egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(170)));
                    }
                    ring(self.spot_edges(map, &points, s.offset(), s.radius), egui::Stroke::new(1.25_f32, egui::Color32::from_white_alpha(170)));
                    ring(self.spot_edges(map, &points, [0.0, 0.0], s.radius), egui::Stroke::new(2.0_f32, theme::ACCENT));
                    // The feather's inner edge, on circles (on a stroke it only adds clutter).
                    if s.feather > 0.0 && s.feather < 1.0 && s.path.is_empty() {
                        for e in self.spot_edges(map, &points, [0.0, 0.0], s.radius * (1.0 - s.feather)) {
                            painter.line_segment(e, egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(90)));
                        }
                    }
                } else {
                    let color = if hit.map(|h| h.0) == Some(i) { egui::Color32::WHITE } else { egui::Color32::from_white_alpha(150) };
                    ring(self.spot_edges(map, &points, [0.0, 0.0], s.radius), egui::Stroke::new(1.25_f32, color));
                }
            }
        }
        // The stroke being painted, or the brush where a click or stroke would start.
        if let Some(path) = &self.spot_paint {
            let fill = self.spot_edges(map, path, [0.0, 0.0], self.spot_brush.radius);
            ring(fill, egui::Stroke::new(1.5_f32, egui::Color32::WHITE));
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else {
            let space = ui.input(|i| i.key_down(egui::Key::Space));
            match (response.hover_pos(), hovered) {
                _ if space => ui.ctx().set_cursor_icon(if response.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab }),
                (Some(p), Some(t)) if hit.is_none() && self.spot_drag.is_none() => {
                    let r = self.spot_brush.radius * map.points_per_height(t);
                    painter.circle_stroke(p, r, shadow);
                    painter.circle_stroke(p, r, egui::Stroke::new(1.25_f32, egui::Color32::WHITE));
                    if self.spot_brush.feather > 0.0 && self.spot_brush.feather < 1.0 {
                        painter.circle_stroke(p, r * (1.0 - self.spot_brush.feather), egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(110)));
                    }
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                }
                _ if self.spot_drag.is_some() => ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing),
                _ if hit.is_some() => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
                _ => {}
            }
        }
        let hint = format!(
            "{}  ·  Click or paint over a spot  ·  Drag an outline to move it  ·  Space+drag pans  ·  [ ] size  ·  Shift+[ ] feather  ·  H hide  ·  Esc done",
            if self.spot_target_ref().clone { "Clone" } else { "Heal" }
        );
        let galley = painter.layout_no_wrap(hint, theme::font(11.5), theme::TEXT);
        let back = egui::Rect::from_center_size(rect.center_top() + egui::vec2(0.0, 10.0 + galley.size().y * 0.5 + 4.0), galley.size() + egui::vec2(16.0, 8.0));
        painter.rect_filled(back, 5.0, egui::Color32::from_rgba_unmultiplied(0x18, 0x1A, 0x1E, 0xD8));
        painter.galley(back.min + egui::vec2(8.0, 4.0), galley, theme::TEXT);
    }

    /// A spot's outline on screen as line pieces: a circle, or the edge of a painted stroke
    /// as one merged shape (traced once per stroke and size, then kept). `offset` shifts it
    /// to the source.
    fn spot_edges(&self, map: &spots::ScreenMap, points: &[[f32; 2]], offset: [f32; 2], radius: f32) -> Vec<[egui::Pos2; 2]> {
        let shift = |p: [f32; 2]| map.to_screen([p[0] + offset[0], p[1] + offset[1]]);
        if points.len() < 2 {
            let p = points.first().copied().unwrap_or([0.0, 0.0]);
            let ring = map.outline(p[0] + offset[0], p[1] + offset[1], radius);
            return (0..ring.len()).map(|i| [ring[i], ring[(i + 1) % ring.len()]]).collect();
        }
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for p in points {
            (p[0].to_bits(), p[1].to_bits()).hash(&mut hasher);
        }
        (radius.to_bits(), map.aspect().to_bits()).hash(&mut hasher);
        let key = hasher.finish();
        let mut cache = self.spot_outlines.borrow_mut();
        if cache.len() > 256 {
            cache.clear();
        }
        let edges = cache.entry(key).or_insert_with(|| std::rc::Rc::new(spots::outline_segments(points, radius, map.aspect()))).clone();
        drop(cache);
        edges.iter().map(|e| [shift(e[0]), shift(e[1])]).collect()
    }

    fn spot_target_ref(&self) -> &spots::Spot {
        self.spot_selected.and_then(|i| self.spots.get(i)).unwrap_or(&self.spot_brush)
    }

    /// The "Spot Removal" section: the tool, Heal or Clone, and the size, feather and opacity
    /// of the selected spot (or of new spots when none is selected).
    fn spot_panel(&mut self, ui: &mut egui::Ui) {
        let amber = theme::ACCENT;
        ui.add_enabled_ui(self.loaded.is_some(), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let clear_width = 58.0;
                let (text, look) = if self.spot_tool { ("Done", ui::Look::teal()) } else { ("Remove Spots", ui::Look::raised()) };
                let width = ui.available_width() - clear_width - 6.0;
                let hint = "Q. Click a spot, or paint over a longer one, to remove it; Develop picks a nearby area to copy from. Drag either outline to move it. Later spots see earlier Clone spots, so you can clone an edge first and heal over it.";
                if ui::button(ui, text, egui::vec2(width, 28.0), look, theme::font(12.5), None).on_hover_text(hint).clicked() {
                    self.set_spot_tool(!self.spot_tool);
                }
                if ui
                    .add_enabled_ui(!self.spots.is_empty(), |ui| ui::button(ui, "Clear", egui::vec2(clear_width, 28.0), ui::Look::secondary(), theme::font(12.0), None))
                    .inner
                    .on_hover_text("Remove all spots")
                    .clicked()
                {
                    self.spots.clear();
                    self.spot_selected = None;
                }
            });
            ui.add_space(6.0);
            let selected = self.spot_selected.filter(|&i| i < self.spots.len());
            let mut clone = self.spot_target().clone;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Mode").color(theme::TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.selectable_value(&mut clone, true, "Clone").on_hover_text("Copy the source exactly");
                    ui.selectable_value(&mut clone, false, "Heal").on_hover_text("Copy the source's texture and blend it into the surroundings");
                });
            });
            self.spot_target().clone = clone;
            let target = self.spot_target().clone();
            let (mut size, mut feather, mut opacity) = (target.radius * 1000.0, target.feather * 100.0, target.opacity * 100.0);
            if ui::pro_slider_with(ui, "Size", &mut size, SPOT_MIN * 1000.0..=SPOT_MAX * 1000.0, 12.0, "", amber, 0, 1.0, true) {
                self.spot_target().radius = (size / 1000.0).clamp(SPOT_MIN, SPOT_MAX);
            }
            if ui::pro_slider_with(ui, "Feather", &mut feather, 0.0..=100.0, 50.0, "", amber, 0, 1.0, true) {
                self.spot_target().feather = (feather / 100.0).clamp(0.0, 1.0);
            }
            if ui::pro_slider_with(ui, "Opacity", &mut opacity, 0.0..=100.0, 100.0, "", amber, 0, 1.0, true) {
                self.spot_target().opacity = (opacity / 100.0).clamp(0.0, 1.0);
            }
            let status = match (self.spots.len(), selected) {
                (0, _) => "No spots".to_owned(),
                (n, Some(i)) => format!("Spot {} of {n} selected · Delete removes it", i + 1),
                (n, None) => format!("{n} spot{}  ·  the sliders set new spots", if n == 1 { "" } else { "s" }),
            };
            ui.add_space(4.0);
            ui.label(ui::dim(&status, 11.5));
            ui.label(ui::dim("Click or paint  ·  Space+drag pans  ·  [ ] size  ·  Shift+[ ] feather  ·  H hide", 11.5));
        });
    }

    /// The "Tone Curve" section: the point curve graph and the four region sliders.
    fn curve_panel(&mut self, ui: &mut egui::Ui) {
        let amber = theme::ACCENT;
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, width), egui::Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let to_screen = |p: [f32; 2]| egui::pos2(rect.left() + p[0] * rect.width(), rect.bottom() - p[1] * rect.height());
        let from_screen = |p: egui::Pos2| [((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0), ((rect.bottom() - p.y) / rect.height()).clamp(0.0, 1.0)];
        // Background, quarter grid, the straight line, and the region boundaries.
        painter.rect_filled(rect, 3.0, egui::Color32::from_rgb(0x15, 0x17, 0x1A));
        let grid = egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(18));
        for k in 1..4 {
            let t = k as f32 / 4.0;
            painter.line_segment([to_screen([t, 0.0]), to_screen([t, 1.0])], grid);
            painter.line_segment([to_screen([0.0, t]), to_screen([1.0, t])], grid);
        }
        painter.line_segment([to_screen([0.0, 0.0]), to_screen([1.0, 1.0])], egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(30)));
        // The whole curve (regions and points), then the points.
        let lut = tone::lut(&self.curve, &self.regions);
        let line: Vec<egui::Pos2> = lut.iter().enumerate().step_by(2).chain(std::iter::once((tone::LUT_SIZE - 1, &lut[tone::LUT_SIZE - 1]))).map(|(i, &y)| to_screen([i as f32 / (tone::LUT_SIZE - 1) as f32, y])).collect();
        painter.add(egui::Shape::line(line, egui::Stroke::new(1.75_f32, egui::Color32::from_gray(230))));
        let pointer = response.hover_pos().or(response.interact_pointer_pos());
        let near = |p: egui::Pos2, points: &[[f32; 2]]| points.iter().position(|q| (to_screen(*q) - p).length() < 9.0);
        let hovered = pointer.and_then(|p| near(p, &self.curve));
        for (i, q) in self.curve.iter().enumerate() {
            let active = self.curve_drag == Some(i) || hovered == Some(i);
            painter.circle(to_screen(*q), if active { 5.0 } else { 4.0 }, egui::Color32::from_rgb(0x15, 0x17, 0x1A), egui::Stroke::new(1.5_f32, if active { amber } else { egui::Color32::from_gray(230) }));
        }
        // Click on the curve to add a point, drag a point to move it, double-click to remove.
        let spline = tone::Spline::new(&self.curve);
        if response.drag_started() {
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                self.curve_drag = near(origin, &self.curve).or_else(|| {
                    let x = from_screen(origin)[0];
                    (self.curve.len() < 16).then(|| {
                        let at = self.curve.partition_point(|q| q[0] < x);
                        self.curve.insert(at, [x, spline.eval(x)]);
                        at
                    })
                });
            }
        }
        if let (Some(i), Some(p)) = (self.curve_drag, response.interact_pointer_pos()) {
            if response.dragged() && i < self.curve.len() {
                let [x, y] = from_screen(p);
                let lo = if i == 0 { 0.0 } else { self.curve[i - 1][0] + 0.01 };
                let hi = if i + 1 == self.curve.len() { 1.0 } else { self.curve[i + 1][0] - 0.01 };
                self.curve[i] = [x.clamp(lo, hi.max(lo)), y];
            }
        }
        if response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            self.curve_drag = None;
        }
        if response.double_clicked() {
            if let Some(i) = pointer.and_then(|p| near(p, &self.curve)) {
                if i > 0 && i + 1 < self.curve.len() {
                    self.curve.remove(i);
                }
            }
        } else if response.clicked() && hovered.is_none() && self.curve.len() < 16 {
            if let Some(p) = response.interact_pointer_pos() {
                let x = from_screen(p)[0];
                let at = self.curve.partition_point(|q| q[0] < x);
                self.curve.insert(at, [x, spline.eval(x)]);
            }
        }
        // What the pointer is over, on Lightroom's 0-255 scale.
        let readout = match (self.curve_drag.or(hovered), pointer) {
            (Some(i), _) if i < self.curve.len() => {
                let q = self.curve[i];
                format!("Input {:.0}  ·  Output {:.0}", q[0] * 255.0, q[1] * 255.0)
            }
            (_, Some(p)) if rect.contains(p) => {
                let x = from_screen(p)[0];
                format!("Input {:.0}  ·  Output {:.0}", x * 255.0, lut[((x * 255.0).round() as usize).min(255)] * 255.0)
            }
            _ => String::new(),
        };
        ui.horizontal(|ui| {
            ui.label(ui::dim(&readout, 11.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let changed = !tone::curve_is_identity(&self.curve, &self.regions);
                if ui.add_enabled_ui(changed, |ui| ui::button(ui, "Reset", egui::vec2(0.0, 22.0), ui::Look::bordered(), theme::font(11.5), None)).inner.on_hover_text("Straight curve, regions at 0").clicked() {
                    self.curve = tone::identity_curve();
                    self.regions = [0.0; 4];
                }
            });
        });
        ui.add_space(4.0);
        for (i, name) in ["Highlights", "Lights", "Darks", "Shadows"].iter().enumerate() {
            ui::pro_slider_with(ui, name, &mut self.regions[i], -100.0..=100.0, 0.0, "", amber, 0, 1.0, false);
        }
    }

    /// The "Color Mixer" section: Hue, Saturation or Luminance for the eight colours.
    fn mixer_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (i, name) in ["Hue", "Saturation", "Luminance"].iter().enumerate() {
                ui.selectable_value(&mut self.hsl_tab, i, *name);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let row = &mut self.hsl[self.hsl_tab];
                let changed = row.iter().any(|v| v.abs() > 1e-3);
                if ui.add_enabled_ui(changed, |ui| ui::button(ui, "Reset", egui::vec2(0.0, 22.0), ui::Look::bordered(), theme::font(11.5), None)).inner.on_hover_text("This tab's sliders back to 0").clicked() {
                    *row = [0.0; 8];
                }
            });
        });
        ui.add_space(4.0);
        let tab = self.hsl_tab;
        for (i, (name, _, rgb)) in tone::COLOURS.iter().enumerate() {
            let colour = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            ui::pro_slider_with(ui, name, &mut self.hsl[tab][i], -100.0..=100.0, 0.0, "", colour, 0, 1.0, false);
        }
    }

    /// The "B&W Mix" section (black & white): how bright each colour becomes.
    fn mono_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(ui::dim("Brighten or darken each colour's grey", 11.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let changed = self.mono_mix.iter().any(|v| v.abs() > 1e-3);
                if ui.add_enabled_ui(changed, |ui| ui::button(ui, "Reset", egui::vec2(0.0, 22.0), ui::Look::bordered(), theme::font(11.5), None)).inner.clicked() {
                    self.mono_mix = [0.0; 8];
                }
            });
        });
        ui.add_space(4.0);
        for (i, (name, _, rgb)) in tone::COLOURS.iter().enumerate() {
            let colour = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            ui::pro_slider_with(ui, name, &mut self.mono_mix[i], -100.0..=100.0, 0.0, "", colour, 0, 1.0, false);
        }
    }

    /// A photo's settings with nothing changed, for a photo that has never been edited (the
    /// camera's white balance and rotation, Develop's defaults otherwise).
    fn blank_saved(&self) -> settings::Saved {
        settings::Saved {
            values: Adjust { kelvin: 0.0, ..Adjust::default() }.to_array(),
            quarter_turns: 255,
            profile: String::new(),
            denoise: false,
            crop: crop::FULL,
            angle: 0.0,
            sharpen: settings::SHARPEN_DEFAULT,
            tint: 0.0,
            presence: [0.0; 3],
            lens: lens::LensSettings::default(),
            spots: Vec::new(),
            curve: tone::identity_curve(),
            regions: [0.0; 4],
            hsl: [[0.0; 8]; 3],
            mono: false,
            mono_mix: [0.0; 8],
        }
    }

    /// Apply a develop preset to `targets` (the open photo and/or others).
    fn apply_preset(&mut self, name: &str, targets: &[PathBuf]) {
        let Some(preset) = self.develop_presets.get(name).cloned() else { return };
        let open = self.loaded.as_ref().map(|l| l.path.clone());
        for target in targets {
            if open.as_ref() == Some(target) {
                let current = self.storable(self.snapshot());
                self.apply_saved(&preset.apply_to(&current));
            } else {
                let base = self.store.get(target).cloned().unwrap_or_else(|| self.blank_saved());
                self.store.put(target, preset.apply_to(&base));
                if let Some(file) = settings::thumb_file(target) {
                    let _ = std::fs::remove_file(file);
                }
                self.filmstrip.reload(target);
            }
        }
    }

    /// The "Presets" section.
    fn presets_panel(&mut self, ui: &mut egui::Ui) {
        let open = self.loaded.as_ref().map(|l| l.path.clone());
        let targets = self.filmstrip.targets(open.as_deref());
        let mut apply = None;
        let mut remove = None;
        if self.develop_presets.list.is_empty() {
            ui.label(ui::dim("No presets yet. Save the current look below.", 11.5));
        }
        let hint = if targets.len() > 1 { format!("Apply to the {} selected photos", targets.len()) } else { "Apply to this photo".to_owned() };
        for preset in &self.develop_presets.list {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Delete this preset").clicked() {
                        remove = Some(preset.name.clone());
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let label = if preset.white_balance { format!("{}  ·  WB", preset.name) } else { preset.name.clone() };
                        let row = ui.add_enabled(!targets.is_empty(), egui::Label::new(egui::RichText::new(label).color(theme::TEXT)).sense(egui::Sense::click()).truncate(true));
                        if row.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if row.on_hover_text(&hint).clicked() {
                            apply = Some(preset.name.clone());
                        }
                    });
                });
            });
        }
        if let Some(name) = apply {
            self.apply_preset(&name, &targets);
        }
        if let Some(name) = remove {
            self.develop_presets.remove(&name);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let name = settings::clean_name(&self.develop_preset_name);
                if ui.add_enabled_ui(!name.is_empty() && self.loaded.is_some(), |ui| ui::button(ui, "Save", egui::vec2(52.0, 22.0), ui::Look::bordered(), theme::font(11.5), None)).inner.on_hover_text("Save the current tone, presence, colour, curve, mixer, detail and profile settings (a preset with the same name is replaced)").clicked() {
                    let saved = self.storable(self.snapshot());
                    self.develop_presets.add(settings::Preset { name, white_balance: self.preset_wb, saved });
                    self.develop_preset_name.clear();
                }
                ui.add(egui::TextEdit::singleline(&mut self.develop_preset_name).hint_text("New preset name").desired_width(ui.available_width()));
            });
        });
        ui.checkbox(&mut self.preset_wb, "Include white balance");
    }

    /// The "Versions" section.
    fn versions_panel(&mut self, ui: &mut egui::Ui) {
        let Some(path) = self.loaded.as_ref().map(|l| l.path.clone()) else {
            ui.label(ui::dim("Open a photo to save versions of its edit.", 11.5));
            return;
        };
        let mut versions = self.store.versions(&path).to_vec();
        let mut changed = false;
        let mut restore = None;
        if versions.is_empty() {
            ui.label(ui::dim("No versions yet. Save one to come back to this edit later.", 11.5));
        }
        let mut remove = None;
        for (i, v) in versions.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Delete this version").clicked() {
                        remove = Some(i);
                    }
                    if ui.small_button("Update").on_hover_text("Replace this version with the current edit").clicked() {
                        v.saved = self.snapshot();
                        v.time = settings::now_seconds();
                        changed = true;
                    }
                    let comparing = self.compare.as_ref() == Some(&v.saved) && self.ab_split;
                    if ui.selectable_label(comparing, "A/B").on_hover_text("Show this version as before in the A/B split").clicked() {
                        if comparing {
                            self.compare = None;
                            self.ab_split = false;
                        } else {
                            self.compare = Some(v.saved.clone());
                            self.ab_split = true;
                        }
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let text = egui::RichText::new(format!("{}   ", v.name)).color(theme::TEXT);
                        let row = ui.add(egui::Label::new(text).sense(egui::Sense::click()).truncate(true));
                        ui.label(ui::dim(&when(v.time), 11.0));
                        if row.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if row.on_hover_text("Switch to this version (Undo goes back)").clicked() {
                            restore = Some(v.saved.clone());
                        }
                    });
                });
            });
        }
        if let Some(i) = remove {
            versions.remove(i);
            changed = true;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let default_name = format!("Version {}", versions.len() + 1);
                if ui::button(ui, "Save", egui::vec2(52.0, 22.0), ui::Look::bordered(), theme::font(11.5), None).on_hover_text("Save the current edit as a version of this photo").clicked() {
                    let name = settings::clean_name(&self.version_name);
                    versions.push(settings::Version { name: if name.is_empty() { default_name.clone() } else { name }, time: settings::now_seconds(), saved: self.snapshot() });
                    self.version_name.clear();
                    changed = true;
                }
                ui.add(egui::TextEdit::singleline(&mut self.version_name).hint_text(default_name).desired_width(ui.available_width()));
            });
        });
        if changed {
            self.store.set_versions(&path, versions);
        }
        if let Some(saved) = restore {
            self.restore_snapshot(&saved);
        }
    }

    fn reset_all(&mut self) {
        // White balance goes back to what the camera recorded for this file (5500 K with no file).
        let kelvin = self.loaded.as_ref().map_or(KELVIN_DEFAULT, |l| l.color.as_shot_kelvin);
        self.adjust = Adjust { kelvin, ..Adjust::default() };
        self.sharpen = settings::SHARPEN_DEFAULT;
        self.tint = 0.0;
        self.presence = [0.0; 3];
        self.curve = tone::identity_curve();
        self.regions = [0.0; 4];
        self.hsl = [[0.0; 8]; 3];
        self.mono = false;
        self.mono_mix = [0.0; 8];
        self.spots.clear();
        self.spot_selected = None;
        self.lens = lens::LensSettings::default();
        self.reset_crop();
    }

    /// The bar across the top of the window: panel toggle, app name, what is open, and
    /// which graphics backend is in use.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if ui::icon_button(ui, ui::Icon::Sidebar, 22.0, ui::Look::ghost()).on_hover_text("Show or hide the folder panel  [").clicked() {
                self.show_left = !self.show_left;
            }
            if let Some(icon) = &self.icon {
                ui.add(egui::Image::new(icon).fit_to_exact_size(egui::vec2(18.0, 18.0)));
            }
            ui.label(egui::RichText::new("Develop").font(theme::bold(13.5)).color(theme::TEXT_BRIGHT));

            // Telemetry pill: the open file in one line.
            if let Some(loaded) = &self.loaded {
                let megapixels = loaded.full_size.0 as f32 * loaded.full_size.1 as f32 / 1.0e6;
                let kind = match self.input_mode {
                    0 => format!("{}-bit RAW", loaded.bits),
                    1 => format!("{}-bit Linear RAW", loaded.bits),
                    _ => format!("{}-bit {}", loaded.bits, loaded.ext.to_uppercase()),
                };
                let summary = format!(
                    "{} • {} • {megapixels:.1} MP ({}×{}) • {kind} • Decode {:.2}s",
                    loaded.name, loaded.camera, loaded.full_size.0, loaded.full_size.1, loaded.seconds
                );
                ui::pill(ui, theme::WELL, egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(18)), 999.0, egui::Margin::symmetric(10.0, 3.0), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 7.0;
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(6.0, 14.0), egui::Sense::hover());
                        ui::dot(ui.painter(), dot.center(), theme::TEAL);
                        ui.label(egui::RichText::new(summary).font(theme::mono(11.5)).color(theme::TEXT));
                    });
                });
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui::icon_button(ui, ui::Icon::Sidebar, 22.0, ui::Look::ghost()).on_hover_text("Show or hide the inspector  ]").clicked() {
                    self.show_right = !self.show_right;
                }
                // Which graphics API the picture is being drawn with.
                let backend = match self.render_state.adapter.get_info().backend {
                    wgpu::Backend::Metal => "Metal",
                    wgpu::Backend::Dx12 => "DX12",
                    wgpu::Backend::Vulkan => "Vulkan",
                    wgpu::Backend::Gl => "OpenGL",
                    wgpu::Backend::BrowserWebGpu => "WebGPU",
                    _ => "GPU",
                };
                ui::pill(ui, theme::tint(theme::TEAL, 0.10), egui::Stroke::new(1.0_f32, theme::tint(theme::TEAL, 0.45)), 5.0, egui::Margin::symmetric(8.0, 2.0), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(6.0, 14.0), egui::Sense::hover());
                        ui::dot(ui.painter(), dot.center(), theme::TEAL);
                        ui.label(egui::RichText::new(format!("{backend} Active")).font(theme::mono(11.5)).color(theme::TEAL));
                    });
                });
                if ui.available_width() > 330.0 {
                    ui.label(ui::dim("Double-click label to reset • Drag canvas to pan", 11.5));
                }
            });
        });
    }

    /// Right pane, pinned at the top: what is open, Reset All, then View and the scopes.
    fn pane_top(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            ui.set_height(24.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui::button(ui, "Reset All", egui::vec2(0.0, 22.0), ui::Look::bordered(), theme::font(12.0), None)
                    .on_hover_text("All sliders, crop and sharpening back to their defaults")
                    .clicked()
                {
                    self.reset_all();
                }
                let (redo_key, undo_key) = if cfg!(target_os = "macos") { ("Shift+Cmd+Z", "Cmd+Z") } else { ("Ctrl+Shift+Z or Ctrl+Y", "Ctrl+Z") };
                let can_redo = self.can_redo();
                if ui.add_enabled_ui(can_redo, |ui| ui::button(ui, "Redo", egui::vec2(0.0, 22.0), ui::Look::bordered(), theme::font(12.0), None)).inner
                    .on_hover_text(format!("Redo the last change undone on this photo ({redo_key})"))
                    .clicked()
                {
                    self.step_history(false);
                }
                let can_undo = self.can_undo();
                if ui.add_enabled_ui(can_undo, |ui| ui::button(ui, "Undo", egui::vec2(0.0, 22.0), ui::Look::bordered(), theme::font(12.0), None)).inner
                    .on_hover_text(format!("Undo the last change on this photo ({undo_key})"))
                    .clicked()
                {
                    self.step_history(true);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if let Some((name, _)) = &self.loading {
                        ui.spinner();
                        let text = match denoise::progress() {
                            Some((done, total)) => format!("Denoising {name}… {done} of {total}"),
                            None => format!("Opening {name}…"),
                        };
                        ui.add(egui::Label::new(egui::RichText::new(text).color(theme::TEXT)).truncate(true));
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                    } else if let Some(loaded) = &self.loaded {
                        let room = (ui.available_width() - 44.0).max(40.0);
                        ui.allocate_ui_with_layout(egui::vec2(room, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(egui::RichText::new(&loaded.name).font(theme::bold(14.0)).color(theme::TEXT_BRIGHT)).truncate(true));
                        });
                        ui::badge(ui, &loaded.ext, theme::ACCENT);
                    } else {
                        ui.label(egui::RichText::new("No file open").color(theme::FADED));
                    }
                });
            });
        });
        if self.loading.is_none() {
            if let Some(loaded) = &self.loaded {
                let line = format!("{} • {}×{} • {:.0} K", loaded.camera, loaded.full_size.0, loaded.full_size.1, loaded.color.as_shot_kelvin);
                ui.add(egui::Label::new(egui::RichText::new(line).font(theme::font(11.5)).color(theme::TEXT_HEADER)).truncate(true))
                    .on_hover_text(&loaded.info);
            }
        }
        if let (Some((photo, at)), Some(loaded)) = (&self.outside_note, &self.loaded) {
            if *photo == loaded.path && at.elapsed() < OUTSIDE_NOTE_TIME {
                ui.add(egui::Label::new(egui::RichText::new("Edited by another program. Undo reverts.").font(theme::font(11.5)).color(theme::TEAL)).truncate(true))
                    .on_hover_text("Another program saved new settings for this photo: an AI assistant using Develop's tools, or another copy of Develop.");
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.add_space(5.0);
        ui::hairline(ui);

        ui::section(ui, "View", true, "Scroll to zoom, drag to pan, double-click for 100% / fit", |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                for (label, mode) in [("Fit", None), ("100%", Some(1.0_f32)), ("200%", Some(2.0_f32))] {
                    let active = match (self.zoom, mode) {
                        (None, None) => true,
                        (Some(zoom), Some(mode)) => (zoom - mode).abs() < 0.005,
                        _ => false,
                    };
                    let look = if active { ui::Look::teal() } else { ui::Look::bordered() };
                    if ui::button(ui, label, egui::vec2(50.0, 26.0), look, theme::font(12.0), None).clicked() {
                        self.zoom = mode;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui::icon_button(ui, ui::Icon::RotateCw, 26.0, ui::Look::bordered()).on_hover_text("Rotate right").clicked() {
                        self.quarter_turns = (self.quarter_turns + 1) % 4;
                        self.zoom = None;
                    }
                    if ui::icon_button(ui, ui::Icon::RotateCcw, 26.0, ui::Look::bordered()).on_hover_text("Rotate left").clicked() {
                        self.quarter_turns = (self.quarter_turns + 3) % 4;
                        self.zoom = None;
                    }
                });
            });
        });
        ui::section(ui, "Histogram", true, "Red, green and blue; white where all three overlap, the mixed colour where two do", |ui| {
            self.scope(ui, scopes::Kind::Histogram, 88.0);
        });
        ui::section(ui, "Vectorscope", true, "Colour by hue (angle) and strength (distance from the centre); the amber line is skin tone", |ui| {
            self.scope(ui, scopes::Kind::Vectorscope, 116.0);
        });
    }

    /// One of the scopes: a dark well drawn into by the GPU from the counted bins.
    fn scope(&mut self, ui: &mut egui::Ui, kind: scopes::Kind, height: f32) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
        ui.painter().rect_filled(rect, 4.0, theme::WELL);
        if self.loaded.is_none() {
            return;
        }
        let inner = rect.shrink(1.0);
        let renderer = self.render_state.renderer.read();
        if let Some(scopes) = renderer.callback_resources.get::<scopes::Scopes>() {
            scopes.set_shape(&self.render_state.queue, kind, inner.width() / inner.height().max(1.0));
        }
        drop(renderer);
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(inner, scopes::ScopeCallback(kind)));
        if kind == scopes::Kind::Vectorscope {
            // Names of the colour targets, beside the ticks the shader draws.
            let half = inner.height() * 0.5;
            for (name, cb, cr) in scopes::TARGETS {
                let p = egui::vec2(cb, -cr) * (scopes::RING / scopes::RING_CHROMA);
                let at = inner.center() + p * half * 1.22;
                ui.painter().text(at, egui::Align2::CENTER_CENTER, name, theme::font(8.5), theme::FADED);
            }
        }
    }

    /// Give the scopes the open picture as it currently looks (whole crop, display-encoded)
    /// and ask for a recount. Called every frame the picture is drawn.
    fn feed_scopes(&self) {
        let Some(loaded) = &self.loaded else { return };
        let c = self.crop_region();
        let uniforms = self.uniforms(loaded, [c[2], c[3], c[0] + 0.5 * c[2], c[1] + 0.5 * c[3]], true);
        let renderer = self.render_state.renderer.read();
        if let (Some(image), Some(scopes)) = (renderer.callback_resources.get::<ImageRenderer>(), renderer.callback_resources.get::<scopes::Scopes>()) {
            image.write_scope_uniforms(&self.render_state.queue, &uniforms);
            scopes.request();
        }
    }

    /// Right pane, scrolling middle: the editing sections.
    fn pane_middle(&mut self, ui: &mut egui::Ui) {
        let amber = theme::ACCENT;
        // In the order most photos are worked on: profile, crop, white balance, tone, presence,
        // detail, lens. Retouching comes last, after the darkroom adjustments.
        if std::mem::take(&mut self.sections_reveal) {
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::new(("section", "Presets")), true);
                d.insert_temp(egui::Id::new(("section", "Versions")), true);
            });
        }
        ui::section(ui, "Presets", false, "Save the current look as a preset, and apply presets to the open or selected photos", |ui| self.presets_panel(ui));
        ui::section(ui, "Versions", false, "Save versions of this photo's edit, switch between them, compare one with the current edit", |ui| self.versions_panel(ui));
        ui::section(ui, "Profile", true, "How the camera's colours and base tone are rendered", |ui| {
            // Colour or black & white treatment.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Treatment").color(theme::TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.selectable_value(&mut self.mono, true, "Black & White").on_hover_text("Black and white, with a B&W mix that sets how bright each colour becomes");
                    ui.selectable_value(&mut self.mono, false, "Color");
                });
            });
            ui.add_space(2.0);
            let is_raw = self.input_mode != 2;
            ui.add_enabled_ui(is_raw, |ui| {
                ui.horizontal(|ui| {
                    let selected = match self.profile {
                        _ if !is_raw => "Picture (sRGB)".to_owned(),
                        Profile::Linear => "Linear".to_owned(),
                        Profile::Standard => "Standard (built-in)".to_owned(),
                        Profile::Dcp(i) => self.dcps.get(i).map_or_else(|| "Standard (built-in)".to_owned(), |(l, _, _)| l.clone()),
                    };
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().interact_size.y = 24.0;
                        egui::ComboBox::from_id_source("profile").selected_text(selected).width(ui.available_width()).show_ui(ui, |ui| {
                            for (i, (label, _, _)) in self.dcps.iter().enumerate() {
                                ui.selectable_value(&mut self.profile, Profile::Dcp(i), label)
                                    .on_hover_text("Adobe camera profile installed on this computer");
                            }
                            if !self.dcps.is_empty() {
                                ui.separator();
                            }
                            ui.selectable_value(&mut self.profile, Profile::Standard, "Standard (built-in)")
                                .on_hover_text("Built-in base tone curve, no camera colour tables");
                            ui.selectable_value(&mut self.profile, Profile::Linear, "Linear")
                                .on_hover_text("No base curve: flat, dark, exactly what the sensor recorded");
                        });
                    });
                });
            });
        });
        ui::section(ui, "Crop", false, "Crop, straighten and aspect ratio  (R)", |ui| self.crop_panel(ui));

        ui::section(ui, "White Balance", true, "", |ui| {
            let as_shot = self.loaded.as_ref().map(|l| l.color.as_shot_kelvin);
            let a = &mut self.adjust;
            ui::pro_slider_with(ui, "Temp", &mut a.kelvin, KELVIN_MIN..=KELVIN_MAX, as_shot.unwrap_or(KELVIN_DEFAULT), " K", amber, 0, 10.0, false);
            let mut pick = None;
            ui.columns(3, |columns| {
                let width = columns[0].available_width();
                let look = if self.wb_pick { ui::Look::teal() } else { ui::Look::bordered() };
                if columns[0]
                    .add_enabled_ui(self.loaded.is_some(), |ui| ui::button(ui, "Pick", egui::vec2(width, 24.0), look, theme::font(12.0), None))
                    .inner
                    .on_hover_text("W. Click something on the picture that should be neutral grey or white")
                    .clicked()
                {
                    pick = Some(!self.wb_pick);
                }
                let width = columns[1].available_width();
                if columns[1].add_enabled_ui(as_shot.is_some(), |ui| ui::button(ui, "As Shot", egui::vec2(width, 24.0), ui::Look::bordered(), theme::font(12.0), None)).inner.clicked() {
                    a.kelvin = as_shot.unwrap_or(KELVIN_DEFAULT);
                    self.tint = 0.0;
                }
                let width = columns[2].available_width();
                if ui::button(&mut columns[2], "5500 K", egui::vec2(width, 24.0), ui::Look::bordered(), theme::font(12.0), None).clicked() {
                    a.kelvin = KELVIN_DEFAULT;
                }
            });
            if let Some(on) = pick {
                self.set_wb_pick(on);
            }
            ui.add_space(4.0);
            let width = ui.available_width();
            if ui
                .add_enabled_ui(self.loaded.is_some() && self.loading.is_none(), |ui| ui::button(ui, "Auto", egui::vec2(width, 24.0), ui::Look::bordered(), theme::font(12.0), None))
                .inner
                .on_hover_text("Set the white balance from the grey and white parts of the picture, or, when there is almost nothing grey (firelight, tungsten), from people's skin. Exposure is not changed; Undo goes back.")
                .clicked()
            {
                self.auto_white_balance();
            }
            if let Some(note) = &self.auto_wb_note {
                ui.add(egui::Label::new(egui::RichText::new(note).font(theme::font(11.0)).color(theme::TEXT_HEADER)).wrap(true));
            }
            ui.add_space(8.0);
            ui::pro_slider(ui, "Tint", &mut self.tint, -150.0..=150.0, 0.0, "", amber);
        });

        ui::section(ui, "Tone", true, "", |ui| {
            let a = &mut self.adjust;
            ui::pro_slider(ui, "Exposure", &mut a.exposure, -5.0..=5.0, 0.0, " EV", amber);
            // Shown on Lightroom's -100..100 scale; stored as a factor (Lightroom's +-100 measured
            // as 2^(+-0.5) on reference exports).
            let mut contrast = (CONTRAST_UNITS * a.contrast.max(0.01).log2()).clamp(-100.0, 100.0);
            if ui::pro_slider(ui, "Contrast", &mut contrast, -100.0..=100.0, 0.0, "", amber) {
                a.contrast = (contrast / CONTRAST_UNITS).exp2();
            }
            ui::pro_slider(ui, "Highlights", &mut a.highlights, -100.0..=100.0, 0.0, "", amber);
            ui::pro_slider(ui, "Shadows", &mut a.shadows, -100.0..=100.0, 0.0, "", amber);
            ui::pro_slider(ui, "Whites", &mut a.whites, -100.0..=100.0, 0.0, "", amber);
            ui::pro_slider(ui, "Blacks", &mut a.blacks, -100.0..=100.0, 0.0, "", amber);
        });

        ui::section(ui, "Presence", true, "", |ui| {
            let [texture, clarity, dehaze] = &mut self.presence;
            ui::pro_slider_with(ui, "Texture", texture, -100.0..=100.0, 0.0, "", amber, 0, 1.0, false);
            ui::pro_slider_with(ui, "Clarity", clarity, -100.0..=100.0, 0.0, "", amber, 0, 1.0, false);
            ui::pro_slider_with(ui, "Dehaze", dehaze, -100.0..=100.0, 0.0, "", amber, 0, 1.0, false);
            ui.add_space(8.0);
            let a = &mut self.adjust;
            // No colour to adjust in black & white.
            ui.add_enabled_ui(!self.mono, |ui| {
                ui::pro_slider(ui, "Vibrance", &mut a.vibrance, -100.0..=100.0, 0.0, "", amber);
                ui::pro_slider(ui, "Saturation", &mut a.saturation, -100.0..=100.0, 0.0, "", amber);
            });
        });

        if std::mem::take(&mut self.curve_reveal) {
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
        ui::section(ui, "Tone Curve", true, "Click the curve to add a point, drag points, double-click a point to remove it", |ui| self.curve_panel(ui));
        if self.mono {
            ui::section(ui, "B&W Mix", true, "How bright each colour becomes in black and white", |ui| self.mono_panel(ui));
        } else {
            ui::section(ui, "Color Mixer", true, "Hue, saturation and luminance of eight colours", |ui| self.mixer_panel(ui));
        }

        ui::section(ui, "Detail", true, "", |ui| {
            let has_model = denoise::model_path(denoise::Model::Bayer).is_some() || denoise::model_path(denoise::Model::Linear).is_some();
            let mut on = self.denoise;
            let can_switch = self.loaded.is_some() && self.loading.is_none() && self.batch.is_none() && self.can_denoise;
            let hint = "Cleans noise from the sensor data with the RawNIND model. Takes a moment each time the photo is opened; the sliders stay live afterwards.";
            let flipped = ui
                .add_enabled_ui(can_switch, |ui| ui::toggle(ui, &mut on, "AI Denoise"))
                .inner
                .on_hover_text(hint)
                .on_disabled_hover_text("Works on raw photos when the model files are installed")
                .changed();
            if flipped {
                self.denoise_override = Some(on);
                self.reload = true;
            }
            if !has_model {
                ui.label(ui::dim("Model files not found (models/rawnind_bayer.onnx, rawnind_linear.onnx).", 11.5));
            }
            ui.add_space(8.0);
            let [amount, radius, detail, masking] = &mut self.sharpen;
            let defaults = settings::SHARPEN_DEFAULT;
            ui::pro_slider(ui, "Sharpening", amount, 0.0..=150.0, defaults[0], "", amber);
            ui::pro_slider_with(ui, "Radius", radius, 0.5..=3.0, defaults[1], " px", amber, 1, 0.1, true);
            ui::pro_slider(ui, "Detail", detail, 0.0..=100.0, defaults[2], "", amber);
            ui::pro_slider(ui, "Masking", masking, 0.0..=100.0, defaults[3], "", amber);
        });

        ui::section(ui, "Lens Correction", false, "Lens profiles (Lensfun XML or the camera's own data) and manual distortion, fringe and vignette", |ui| self.lens_panel(ui));

        // Switching the spot tool on opens this section and scrolls it into view.
        let reveal = std::mem::take(&mut self.spot_reveal);
        if reveal {
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("section", "Retouch")), true));
        }
        ui::section(ui, "Retouch", false, "Remove dust spots and blemishes by healing or cloning  (Q)", |ui| self.spot_panel(ui));
        if reveal {
            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
        }
    }

    /// True when Lightroom would correct this lens's distortion even with profile corrections
    /// off: Nikon Z, Panasonic, Olympus / OM System and Fujifilm cameras carry the correction
    /// in the raw file and Adobe always applies it. The Lensfun profile stands in for that data
    /// (distortion only, no vignetting).
    fn builtin_distortion(&self, loaded: &Loaded) -> bool {
        self.lens_auto.is_some_and(|i| lens::builtin_distortion(&loaded.exif.make, &self.lens_db.lenses[i]))
    }

    /// Which lens correction applies to the open photo with these settings.
    fn lens_active(&self, settings: &lens::LensSettings, loaded: &Loaded) -> lens::Active {
        if !settings.enabled {
            return lens::Active::None;
        }
        let lensfun = if settings.profile.is_empty() { self.lens_auto } else { self.lens_db.find_key(&settings.profile) };
        let embedded = loaded.embedded.is_some();
        match settings.source {
            lens::Source::Auto => match lensfun {
                Some(i) => lens::Active::Lensfun(i),
                None if embedded => lens::Active::Embedded,
                None => lens::Active::None,
            },
            lens::Source::Lensfun => lensfun.map_or(lens::Active::None, lens::Active::Lensfun),
            lens::Source::Embedded if embedded => lens::Active::Embedded,
            lens::Source::Embedded => lens::Active::None,
        }
    }

    /// Read the Lensfun folders again and match the open photo.
    fn reload_lenses(&mut self) {
        self.lens_db = lens::Database::load();
        self.lens_auto = self.loaded.as_ref().and_then(|l| self.lens_db.auto_match(&l.exif));
    }

    /// A small uppercase label inside a section.
    fn sub_heading(ui: &mut egui::Ui, text: &str) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(text.to_uppercase()).font(theme::bold(10.5)).color(theme::FADED));
        ui.add_space(2.0);
    }

    /// The "Lens Correction" section.
    fn lens_panel(&mut self, ui: &mut egui::Ui) {
        let amber = theme::ACCENT;
        let loaded_exif = self.loaded.as_ref().map(|l| l.exif.clone());
        let has_embedded = self.loaded.as_ref().is_some_and(|l| l.embedded.is_some());

        let mut enabled = self.lens.enabled;
        if ui::toggle(ui, &mut enabled, "Enable Profile Corrections").changed() {
            self.lens.enabled = enabled;
        }
        ui.add_space(4.0);
        ui.add_enabled_ui(self.lens.enabled, |ui| {
            let row = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label).color(theme::TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().interact_size.y = 24.0;
                        add(ui);
                    });
                });
            };
            row(ui, "Source", &mut |ui| {
                egui::ComboBox::from_id_source("lens_source").selected_text(self.lens.source.label()).width(176.0).show_ui(ui, |ui| {
                    for source in lens::Source::ALL {
                        ui.selectable_value(&mut self.lens.source, source, source.label());
                    }
                });
            });

            // Make, Model, Profile: the Lensfun entry in use (automatic unless one is chosen).
            let current = if self.lens.profile.is_empty() { self.lens_auto } else { self.lens_db.find_key(&self.lens.profile) };
            let current_lens = current.map(|i| self.lens_db.lenses[i].clone());
            let make = self.lens_make_pick.clone().or_else(|| current_lens.as_ref().map(|l| l.maker.clone()));
            let auto = self.lens.profile.is_empty();
            let mut choose: Option<String> = None; // Some("") = automatic
            let makers = self.lens_db.makers();
            row(ui, "Make", &mut |ui| {
                let shown = match (&make, auto) {
                    (Some(m), true) => format!("{m} (auto)"),
                    (Some(m), false) => m.clone(),
                    (None, _) => "Auto".to_owned(),
                };
                egui::ComboBox::from_id_source("lens_make").selected_text(shown).width(176.0).show_ui(ui, |ui| {
                    if ui.selectable_label(auto, "Auto").clicked() {
                        choose = Some(String::new());
                    }
                    for m in &makers {
                        if ui.selectable_label(make.as_deref() == Some(m.as_str()), m).clicked() {
                            self.lens_make_pick = Some(m.clone());
                        }
                    }
                });
            });
            let models: Vec<(String, String)> = match &make {
                Some(m) => {
                    let mut list: Vec<(String, String)> = self.lens_db.lenses.iter().filter(|l| &l.maker == m).map(|l| (l.model.clone(), l.key())).collect();
                    list.dedup_by(|a, b| a.0 == b.0);
                    list
                }
                None => Vec::new(),
            };
            row(ui, "Model", &mut |ui| {
                let shown = current_lens.as_ref().filter(|l| Some(&l.maker) == make.as_ref()).map_or_else(|| "—".to_owned(), |l| l.model.clone());
                egui::ComboBox::from_id_source("lens_model").selected_text(shown).width(176.0).show_ui(ui, |ui| {
                    if models.is_empty() {
                        ui.label(ui::dim("No lenses", 12.0));
                    }
                    for (model, key) in &models {
                        let selected = current_lens.as_ref().is_some_and(|l| &l.model == model);
                        if ui.selectable_label(selected, model).clicked() {
                            choose = Some(key.clone());
                        }
                    }
                });
            });
            let variants: Vec<lens::Lens> = match &current_lens {
                Some(c) => self.lens_db.lenses.iter().filter(|l| l.maker == c.maker && l.model == c.model).cloned().collect(),
                None => Vec::new(),
            };
            row(ui, "Profile", &mut |ui| {
                let label = |l: &lens::Lens| format!("{} — {}", l.file.trim_end_matches(".xml"), l.mounts.first().cloned().unwrap_or_default());
                let shown = current_lens.as_ref().map_or_else(|| "None".to_owned(), label);
                egui::ComboBox::from_id_source("lens_profile").selected_text(shown).width(176.0).show_ui(ui, |ui| {
                    for v in &variants {
                        let selected = current_lens.as_ref().is_some_and(|c| c.key() == v.key());
                        if ui.selectable_label(selected, label(v)).on_hover_text(v.contents()).clicked() {
                            choose = Some(v.key());
                        }
                    }
                });
            });
            if let Some(key) = choose {
                if key.is_empty() {
                    self.lens_make_pick = None;
                }
                self.lens.profile = key;
            }

            // What is being applied, and where the profiles come from.
            let status = match self.loaded.as_ref().map(|l| self.lens_active(&self.lens, l)) {
                Some(lens::Active::Lensfun(i)) => {
                    let l = &self.lens_db.lenses[i];
                    let at = loaded_exif.as_ref().map_or(String::new(), |e| if e.focal > 0.0 { format!(" at {:.0} mm", e.focal) } else { String::new() });
                    (format!("Lensfun: {} {}{at}", l.maker, l.model), theme::TEAL, l.contents())
                }
                Some(lens::Active::Embedded) => ("Camera embedded corrections".to_owned(), theme::TEAL, "distortion, chromatic aberration, vignetting".to_owned()),
                _ => {
                    let lens_name = loaded_exif.as_ref().map(|e| e.lens_model.clone()).filter(|m| !m.is_empty()).unwrap_or_else(|| "this lens".to_owned());
                    (format!("No profile for {lens_name}"), theme::FADED, if has_embedded { "The camera's embedded corrections are available".to_owned() } else { String::new() })
                }
            };
            ui.add(egui::Label::new(egui::RichText::new(status.0).font(theme::font(11.5)).color(status.1)).truncate(true)).on_hover_text(status.2);
        });

        ui.horizontal(|ui| {
            let files = self.lens_db.files;
            let text = format!("{files} Lensfun file{} • {} lenses", if files == 1 { "" } else { "s" }, self.lens_db.lenses.len());
            ui.label(ui::dim(text, 11.5)).on_hover_text(self.lens_db.folders.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join("\n"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui::icon_button(ui, ui::Icon::Folder, 22.0, ui::Look::ghost()).on_hover_text("Open the Lensfun folder: drop Lensfun .xml files there, then reload").clicked() {
                    if let Some(dir) = self.lens_db.folder_to_open() {
                        let _ = std::fs::create_dir_all(&dir);
                        let opener = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
                        let _ = std::process::Command::new(opener).arg(&dir).spawn();
                    }
                }
                if ui::icon_button(ui, ui::Icon::Refresh, 22.0, ui::Look::ghost()).on_hover_text("Reload the Lensfun files").clicked() {
                    self.reload_lenses();
                }
            });
        });
        if !self.lens_db.problems.is_empty() {
            ui.label(egui::RichText::new(format!("{} file(s) could not be read", self.lens_db.problems.len())).font(theme::font(11.5)).color(ui.visuals().error_fg_color))
                .on_hover_text(self.lens_db.problems.join("\n"));
        }

        if !self.lens.enabled && self.loaded.as_ref().is_some_and(|l| self.builtin_distortion(l)) {
            ui.label(egui::RichText::new("Built-in distortion correction is applied, as Lightroom does for this camera").font(theme::font(11.5)).color(theme::FADED));
        }
        let active = self.loaded.as_ref().map(|l| self.lens_active(&self.lens, l));
        if !matches!(active, None | Some(lens::Active::None)) {
            ui.add_space(6.0);
            ui::pro_slider(ui, "Profile Distortion", &mut self.lens.profile_distortion, 0.0..=200.0, 100.0, "%", amber);
            ui::pro_slider(ui, "Profile Vignetting", &mut self.lens.profile_vignetting, 0.0..=200.0, 100.0, "%", amber);
        }

        Self::sub_heading(ui, "Distortion");
        ui::pro_slider(ui, "Distortion", &mut self.lens.distortion, -100.0..=100.0, 0.0, "", amber);
        let mut constrain = self.lens.constrain;
        if ui::toggle(ui, &mut constrain, "Constrain Crop").on_hover_text("Enlarge just enough that corrections never show empty borders").changed() {
            self.lens.constrain = constrain;
        }
        ui.add_space(6.0);

        Self::sub_heading(ui, "Defringe");
        ui::pro_slider_with(ui, "Red / Cyan", &mut self.lens.red_cyan, -100.0..=100.0, 0.0, "", amber, 0, 1.0, false);
        ui::pro_slider_with(ui, "Blue / Yellow", &mut self.lens.blue_yellow, -100.0..=100.0, 0.0, "", amber, 0, 1.0, false);
        ui::pro_slider_with(ui, "Purple Amount", &mut self.lens.purple_amount, 0.0..=100.0, 0.0, "", amber, 0, 1.0, true);
        ui::pro_slider_with(ui, "Purple Hue", &mut self.lens.purple_hue, 0.0..=100.0, 50.0, "", amber, 0, 1.0, true);
        ui::pro_slider_with(ui, "Green Amount", &mut self.lens.green_amount, 0.0..=100.0, 0.0, "", amber, 0, 1.0, true);
        ui::pro_slider_with(ui, "Green Hue", &mut self.lens.green_hue, 0.0..=100.0, 50.0, "", amber, 0, 1.0, true);

        Self::sub_heading(ui, "Vignette");
        ui::pro_slider_with(ui, "Amount", &mut self.lens.vignette, -100.0..=100.0, 0.0, "", amber, 0, 1.0, false);
        ui::pro_slider_with(ui, "Midpoint", &mut self.lens.midpoint, 0.0..=100.0, 50.0, "", amber, 0, 1.0, true);
    }



    /// Right pane, pinned at the bottom: Export.
    fn pane_bottom(&mut self, ui: &mut egui::Ui) {
        let count = self.export_targets().len();
        let label = if count > 1 { format!("Export {count} photos…") } else { "Export…".to_owned() };
        let width = ui.available_width();
        let clicked = ui
            .add_enabled_ui(self.loaded.is_some(), |ui| ui::button(ui, &label, egui::vec2(width, 32.0), ui::Look::primary(), theme::bold(13.5), None))
            .inner
            .on_hover_text("Ctrl+E")
            .clicked();
        if clicked {
            self.export_open = true;
        }
    }

    /// The floating strip at the bottom-left of the canvas: zoom, before/after, clipping.
    fn canvas_hud(&mut self, ui: &mut egui::Ui, outer: egui::Rect, zoom: f32) {
        let at = egui::Rect::from_min_size(outer.left_bottom() + egui::vec2(16.0, -46.0), egui::vec2(360.0, 30.0));
        let fill = egui::Color32::from_rgba_unmultiplied(0x18, 0x1A, 0x1E, 0xE6);
        let border = egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(20));
        ui.allocate_ui_at_rect(at, |ui| {
            ui::pill(ui, fill, border, 6.0, egui::Margin::symmetric(4.0, 3.0), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let label = if self.zoom.is_none() { format!("Fit ({:.0}%)", zoom * 100.0) } else { format!("{:.0}%", zoom * 100.0) };
                    if ui::hud_item(ui, &label, theme::mono(11.5), theme::TEAL, false, None).on_hover_text("Switch between fit and 100%").clicked() {
                        self.zoom = if self.zoom.is_some() { None } else { Some(1.0) };
                    }
                    ui::vdivider(ui);
                    if ui::hud_item(ui, "A/B Split", theme::font(12.0), theme::TEXT, self.ab_split, None)
                        .on_hover_text("Before on the left, after on the right; drag the divider")
                        .clicked()
                    {
                        self.ab_split = !self.ab_split;
                    }
                    ui::vdivider(ui);
                    if ui::hud_item(ui, "Clipping", theme::font(12.0), theme::TEXT, self.clipping, Some(theme::ACCENT))
                        .on_hover_text("Mark blown highlights in red (all channels) or amber (some), and crushed blacks in blue")
                        .clicked()
                    {
                        self.clipping = !self.clipping;
                    }
                });
            });
        });
    }

    fn viewport(&mut self, ui: &mut egui::Ui) {
        let (outer, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        // The photo keeps a margin from the panel edges; nothing is drawn around it.
        let rect = outer.shrink(16.0);
        // Right-click: copy / paste settings and rating, as in the filmstrip.
        if let Some(path) = self.loaded.as_ref().map(|l| l.path.clone()) {
            let targets = self.filmstrip.targets_for(&path);
            let can_paste = self.clipboard.is_some();
            response.context_menu(|ui| {
                if let Some(action) = filmstrip::edit_menu(ui, &path, &targets, true, can_paste) {
                    self.menu_action = Some(action);
                }
            });
        }
        let Some(loaded) = &self.loaded else {
            let text = if self.loading.is_some() { "Opening…" } else { "Choose a folder on the left, or drop a photo here" };
            ui.painter().text(outer.center(), egui::Align2::CENTER_CENTER, text, theme::font(15.0), theme::FADED);
            return;
        };
        if rect.width() < 2.0 || rect.height() < 2.0 {
            return;
        }
        if self.crop_mode {
            self.spot_tool = false;
            self.spot_paint = None;
            self.crop_viewport(ui, rect, &response);
            return;
        }

        let ppp = ui.ctx().pixels_per_point();
        let view_px = rect.size() * ppp;
        // What is shown is the cropped picture; zoom and pan work within it.
        let cropped = self.crop_region();
        let dims = if self.quarter_turns % 2 == 1 {
            egui::vec2(loaded.height * cropped[2], loaded.width * cropped[3])
        } else {
            egui::vec2(loaded.width * cropped[2], loaded.height * cropped[3])
        };
        let fit = (view_px.x / dims.x).min(view_px.y / dims.y);
        let max_zoom = fit.max(16.0);
        let mut zoom = self.zoom.unwrap_or(fit).clamp(fit, max_zoom);
        let half = egui::vec2(0.5, 0.5);
        // White balance eyedropper: a click sets the white balance that makes that spot neutral.
        if self.wb_pick {
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    let t = self.spot_map(loaded, rect, view_px / (dims * zoom), cropped).to_texture(p);
                    if (0.0..=1.0).contains(&t[0]) && (0.0..=1.0).contains(&t[1]) {
                        let sample = self.spot_preview.average(t[0], t[1], 2);
                        match loaded.color.neutral_for(sample) {
                            Some((kelvin, tint)) => {
                                self.adjust.kelvin = kelvin;
                                self.tint = tint;
                                self.wb_pick = false;
                            }
                            None => self.error = Some("That spot is too dark or too bright to set the white balance from; pick a mid grey or a light area that isn't blown out.".to_owned()),
                        }
                    }
                }
            }
        }
        // Spot tool: clicks add and select spots, drags move them (before panning sees them).
        if self.spot_tool {
            let map = self.spot_map(loaded, rect, view_px / (dims * zoom), cropped);
            // Space held (or the middle button) pans instead of painting.
            let pan = ui.input(|i| i.key_down(egui::Key::Space));
            let mut io = SpotInput { spots: &mut self.spots, selected: &mut self.spot_selected, drag: &mut self.spot_drag, paint: &mut self.spot_paint };
            io.handle(&self.spot_brush, &self.spot_preview, ui, &response, &map, pan);
        }
        // Offset of the pointer from the viewport centre, in viewport fractions.
        let pointer = response.hover_pos().map_or(egui::Vec2::ZERO, |p| (p - rect.min) / rect.size() - half);

        // Zoom about the pointer.
        let mut factor = 1.0;
        if response.hovered() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            factor = (scroll * 0.0025).exp() * pinch;
        }
        let mut target = None;
        if factor != 1.0 {
            target = Some(zoom * factor);
        }
        if response.double_clicked() && !self.spot_tool && !self.wb_pick {
            target = Some(if self.zoom.is_some() { fit } else { 1.0_f32.max(fit * 2.0) });
        }
        if let Some(target) = target {
            let new_zoom = target.clamp(fit, max_zoom);
            let old_scale = view_px / (dims * zoom);
            let new_scale = view_px / (dims * new_zoom);
            self.center += pointer * old_scale - pointer * new_scale;
            zoom = new_zoom;
        }
        let space = ui.input(|i| i.key_down(egui::Key::Space));
        let spot_busy = self.spot_tool && (!space || self.spot_drag.is_some() || self.spot_paint.is_some());
        if (response.dragged_by(egui::PointerButton::Primary) && !spot_busy) || response.dragged_by(egui::PointerButton::Middle) {
            self.center -= response.drag_delta() * ppp / (dims * zoom);
        }
        self.zoom = if zoom <= fit * 1.0001 { None } else { Some(zoom) };

        // Keep the picture on screen: centre it on any axis where it is smaller than the view.
        let scale = view_px / (dims * zoom);
        let clamp_axis = |c: f32, s: f32| if s >= 1.0 { 0.5 } else { c.clamp(0.5 * s, 1.0 - 0.5 * s) };
        self.center = egui::vec2(clamp_axis(self.center.x, scale.x), clamp_axis(self.center.y, scale.y));

        // Stage 2 input: a handful of floats. This is all a slider movement costs.
        let renderer = self.render_state.renderer.read();
        if let Some(res) = renderer.callback_resources.get::<ImageRenderer>() {
            let view = [
                scale.x * cropped[2],
                scale.y * cropped[3],
                cropped[0] + self.center.x * cropped[2],
                cropped[1] + self.center.y * cropped[3],
            ];
            let mut uniforms = self.uniforms(loaded, view, res.shader_encodes_srgb);
            if self.clipping {
                uniforms.input[1] = 1.0;
            }
            res.write_uniforms(&self.render_state.queue, &uniforms);
            if self.ab_split {
                res.write_before_uniforms(&self.render_state.queue, &self.uniforms_before(loaded, view, res.shader_encodes_srgb));
            }
        }
        drop(renderer);

        if self.ab_split {
            // Before (the photo's defaults) left of the divider, after (the edit) right of it.
            let x = rect.left() + rect.width() * self.ab_pos;
            let (left, right) = (egui::Rect::from_min_max(rect.min, egui::pos2(x, rect.bottom())), egui::Rect::from_min_max(egui::pos2(x, rect.top()), rect.max));
            ui.painter().with_clip_rect(left).add(egui_wgpu::Callback::new_paint_callback(rect, ImageCallback { before: true }));
            ui.painter().with_clip_rect(right).add(egui_wgpu::Callback::new_paint_callback(rect, ImageCallback { before: false }));
            let handle = ui.interact(egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(16.0, rect.height())), ui.id().with("ab_divider"), egui::Sense::drag());
            if handle.hovered() || handle.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if handle.dragged() {
                if let Some(p) = handle.interact_pointer_pos() {
                    self.ab_pos = ((p.x - rect.left()) / rect.width()).clamp(0.02, 0.98);
                }
            }
            let painter = ui.painter();
            painter.vline(x, rect.y_range(), egui::Stroke::new(1.5_f32, egui::Color32::from_white_alpha(220)));
            painter.circle(egui::pos2(x, rect.center().y), 7.0, theme::PANEL, egui::Stroke::new(1.5_f32, egui::Color32::from_white_alpha(220)));
            let tag = |text: &str, at: egui::Pos2, align: egui::Align2| {
                let galley = painter.layout_no_wrap(text.to_owned(), theme::bold(10.5), theme::TEXT);
                let size = galley.size() + egui::vec2(12.0, 6.0);
                let min = if align == egui::Align2::RIGHT_TOP { at - egui::vec2(size.x, 0.0) } else { at };
                let back = egui::Rect::from_min_size(min, size);
                painter.rect_filled(back, 4.0, egui::Color32::from_rgba_unmultiplied(0x18, 0x1A, 0x1E, 0xD8));
                painter.galley(back.min + egui::vec2(6.0, 3.0), galley, theme::TEXT);
            };
            tag("BEFORE", egui::pos2(x - 8.0, rect.top() + 8.0), egui::Align2::RIGHT_TOP);
            tag("AFTER", egui::pos2(x + 8.0, rect.top() + 8.0), egui::Align2::LEFT_TOP);
        } else {
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, ImageCallback { before: false }));
        }
        if self.wb_pick {
            let painter = ui.painter().with_clip_rect(rect);
            let galley = painter.layout_no_wrap("White balance  ·  Click something that should be neutral grey or white  ·  Esc cancels".to_owned(), theme::font(11.5), theme::TEXT);
            let back = egui::Rect::from_center_size(rect.center_top() + egui::vec2(0.0, 10.0 + galley.size().y * 0.5 + 4.0), galley.size() + egui::vec2(16.0, 8.0));
            painter.rect_filled(back, 5.0, egui::Color32::from_rgba_unmultiplied(0x18, 0x1A, 0x1E, 0xD8));
            painter.galley(back.min + egui::vec2(8.0, 4.0), galley, theme::TEXT);
        }
        if self.spot_tool {
            // Circles drawn where the picture is now (after this frame's zoom and pan).
            let map = self.spot_map(loaded, rect, scale, cropped);
            self.spot_overlay(ui, &response, &map, rect);
        }
        self.canvas_hud(ui, outer, zoom);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame(ctx);
    }
}

impl App {
    /// One frame of the app: in a window, or run without one for exports (headless.rs).
    fn frame(&mut self, ctx: &egui::Context) {
        self.poll_loader();
        self.sync_heal();
        self.update_dialog(ctx);
        self.export_windows(ctx);
        // Record finished edits in the open photo's undo history.
        self.track_history(ctx);
        // A version clicked in the filmstrip, once its photo is open and its history has
        // started (so Undo goes back to the edit it had).
        if let Some((path, n)) = self.pending_version.clone() {
            if self.loading.is_none() && self.loaded.as_ref().map(|l| &l.path) == Some(&path) {
                self.pending_version = None;
                if let Some(v) = self.store.versions(&path).get(n).cloned() {
                    self.restore_snapshot(&v.saved);
                }
            } else if self.loading.is_none() && self.loading_path.is_none() && self.loaded.as_ref().map(|l| &l.path) != Some(&path) {
                // Opening it failed or another photo was opened meanwhile.
                self.pending_version = None;
            }
        }

        let dropped = ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone()));
        if let Some(path) = dropped {
            self.start_loading(path, ctx);
        }
        let open = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
        if ctx.input_mut(|i| i.consume_shortcut(&open)) && self.loading.is_none() {
            self.open_dialog(ctx);
        }
        // Undo / redo on the open photo (text fields keep their own while typing).
        if !ctx.wants_keyboard_input() && self.loading.is_none() {
            let redo_shift = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z);
            let redo_y = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y);
            let undo = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
            if ctx.input_mut(|i| i.consume_shortcut(&redo_shift) || i.consume_shortcut(&redo_y)) {
                self.step_history(false);
            } else if ctx.input_mut(|i| i.consume_shortcut(&undo)) {
                self.step_history(true);
            }
        }

        // Filmstrip: the open file's folder. Click a thumbnail, or use the left / right arrow
        // keys (when no slider or text box has the keyboard), to open another photo.
        // Open the photo again when its AI denoise switch changed. The store is brought up
        // to date first so the sliders come back exactly as they are.
        if self.reload && self.loading.is_none() {
            self.reload = false;
            if let Some(path) = self.loaded.as_ref().map(|l| l.path.clone()) {
                if self.remember {
                    if let Some(defaults) = &self.defaults {
                        let current = self.storable(self.snapshot());
                        let defaults = self.storable(defaults.clone());
                        self.store.update(&path, &current, &defaults);
                    }
                }
                let (zoom, center) = (self.zoom, self.center);
                self.start_loading(path, ctx);
                self.keep_view = Some((zoom, center));
            }
        }
        // Read the settings saved beside the photos of the folder on show (once per folder),
        // then watch it for changes made by another program.
        if self.remember {
            if let Some(dir) = self.filmstrip.dir().map(Path::to_path_buf) {
                self.store.load_folder(&dir);
                if self.folder_watch.as_ref().map(|w| w.dir()) != Some(dir.as_path()) {
                    self.folder_watch = watch::FolderWatch::new(&dir, ctx);
                    self.outside.clear();
                }
            }
            self.take_outside_changes(ctx);
        }
        self.filmstrip.poll(ctx);
        let current = self.loading_path.clone().or_else(|| self.loaded.as_ref().map(|l| l.path.clone()));
        let mut pick = self.menu_action.take();
        let can_paste = self.clipboard.is_some();
        // Command-line test runs (`app file.ARW --highlights -100 ...`) show the picture and
        // sliders only, so their captures keep the same layout.
        let browsing = self.remember;
        // [ and ] show or hide the side panels; with the spot tool on they size the brush
        // (and the selected spot), and with Shift change its feather.
        if !ctx.wants_keyboard_input() {
            let (left, right, shift) = ctx.input(|i| (i.key_pressed(egui::Key::OpenBracket), i.key_pressed(egui::Key::CloseBracket), i.modifiers.shift));
            if self.spot_tool && self.loaded.is_some() {
                if left || right {
                    self.spot_keys(right, shift);
                }
            } else {
                self.show_left ^= left;
                self.show_right ^= right;
            }
        }
        egui::TopBottomPanel::top("top_bar")
            .exact_height(34.0)
            .frame(egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(10.0, 0.0)))
            .show(ctx, |ui| self.top_bar(ui));
        egui::TopBottomPanel::bottom("filmstrip")
            .exact_height(filmstrip::PANEL_HEIGHT)
            .resizable(false)
            .frame(egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(10.0, 0.0)))
            .show_animated(ctx, browsing, |ui| {
            if let Some(action) = self.filmstrip.show(ui, current.as_deref(), &self.store, can_paste) {
                pick = Some(action);
            }
        });
        if pick.is_none() && self.loading.is_none() && !self.export_open && ctx.memory(|m| m.focused().is_none()) {
            let step = ctx.input(|i| i.key_pressed(egui::Key::ArrowRight) as isize - i.key_pressed(egui::Key::ArrowLeft) as isize);
            if step != 0 {
                pick = self.filmstrip.neighbour(current.as_deref(), step, &self.store).map(filmstrip::Action::Open);
            }
        }
        // Keys, when no text box is being typed in: 1-5 rate the selected photos (or the open
        // one), 0 removes the rating; Ctrl+C / Ctrl+V copy and paste settings.
        if pick.is_none() && !ctx.wants_keyboard_input() {
            let open = self.loaded.as_ref().map(|l| l.path.clone());
            let targets = self.filmstrip.targets(open.as_deref());
            pick = ctx.input(|i| {
                let digits = [egui::Key::Num0, egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5];
                if !i.modifiers.any() {
                    if let Some(stars) = digits.iter().position(|&key| i.key_pressed(key)) {
                        return Some(filmstrip::Action::Rate(targets.clone(), stars as u8));
                    }
                }
                let copy = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
                let paste = i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)));
                match (&open, copy, paste) {
                    (Some(path), true, _) => Some(filmstrip::Action::Copy(path.clone())),
                    (Some(_), _, true) => Some(filmstrip::Action::Paste(targets.clone())),
                    _ => None,
                }
            });
        }
        if self.batch.is_some() {
            pick = None; // a batch export is switching photos by itself
        }
        if let Some(action) = pick {
            self.apply_action(action, ctx);
        }

        // Folder tree: clicking a folder shows its photos in the filmstrip.
        if browsing {
            let mut folder = None;
            egui::SidePanel::left("explorer").exact_width(240.0).resizable(false).frame(egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(8.0, 6.0))).show_animated(ctx, self.show_left, |ui| {
                folder = self.explorer.show(ui, self.filmstrip.dir());
            });
            if let Some(folder) = folder {
                self.filmstrip.set_folder(Some(&folder));
            }
            self.session.remember(self.filmstrip.dir(), self.loaded.as_ref().map(|l| l.path.as_path()));
        }

        // Right pane: View and the scopes pinned at the top, Export pinned at the bottom,
        // the editing sections scrolling between them.
        egui::SidePanel::right("controls").exact_width(300.0).resizable(false).frame(egui::Frame::none().fill(theme::PANEL)).show_animated(ctx, self.show_right, |ui| {
            let side = egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin { left: 12.0, right: 12.0, top: 8.0, bottom: 0.0 });
            let footer = egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::same(12.0));
            egui::TopBottomPanel::top("pane_top").frame(side).show_separator_line(false).show_inside(ui, |ui| self.pane_top(ui));
            egui::TopBottomPanel::bottom("pane_bottom").frame(footer).show_inside(ui, |ui| self.pane_bottom(ui));
            egui::CentralPanel::default().frame(side.inner_margin(egui::Margin { left: 12.0, right: 12.0, top: 0.0, bottom: 0.0 })).show_inside(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.pane_middle(ui));
            });
        });
        self.sync_profile();
        self.feed_scopes();
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::CANVAS))
            .show(ctx, |ui| self.viewport(ui));

        // R switches crop mode on and off.
        if self.loaded.is_some() && !ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::R) && !i.modifiers.any()) {
            self.crop_mode = !self.crop_mode;
            self.crop_wish = self.crop;
            self.zoom = None;
        }
        // Spot removal: Q switches the tool on and off, Esc deselects then leaves it, Delete
        // removes the selected spot, H hides the circles.
        if self.loaded.is_some() && !ctx.wants_keyboard_input() {
            let (q, esc, delete, h) = ctx.input(|i| {
                let plain = !i.modifiers.any();
                (
                    plain && i.key_pressed(egui::Key::Q),
                    i.key_pressed(egui::Key::Escape),
                    i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                    plain && i.key_pressed(egui::Key::H),
                )
            });
            if q {
                self.set_spot_tool(!self.spot_tool);
            }
            // W: white balance eyedropper; Esc cancels it.
            let w = ctx.input(|i| !i.modifiers.any() && i.key_pressed(egui::Key::W));
            if w {
                self.set_wb_pick(!self.wb_pick);
            } else if esc && self.wb_pick {
                self.wb_pick = false;
            }
            if self.spot_tool {
                if esc {
                    if self.spot_selected.is_some() {
                        self.spot_selected = None;
                    } else {
                        self.set_spot_tool(false);
                    }
                }
                if delete {
                    if let Some(i) = self.spot_selected.take().filter(|&i| i < self.spots.len()) {
                        self.spots.remove(i);
                    }
                }
                if h {
                    self.spot_hide = !self.spot_hide;
                }
            }
        }
        let export_key = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::E);
        if ctx.input_mut(|i| i.consume_shortcut(&export_key)) && self.loaded.is_some() {
            self.export_open = true;
        }
        self.export_window(ctx);
        self.poll_export(ctx);
        self.advance_batch(ctx);
        if self.remember && !ctx.input(|i| i.pointer.any_down()) {
            self.presets.save_if_changed(&self.export);
        }

        // Remember this photo's settings; the file is written shortly after the last change.
        if self.remember {
            if let (Some(loaded), Some(defaults)) = (&self.loaded, &self.defaults) {
                let current = self.storable(self.snapshot());
                let defaults = self.storable(defaults.clone());
                self.store.update(&loaded.path, &current, &defaults);
            }
            self.update_thumbnail(ctx);
            self.update_version_thumbs(ctx);
            self.store.flush(false);
            if self.store.pending() {
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
        }
        // `--auto-wb <file>`: once the picture is ready, find the white balance, write it, quit.
        if self.loaded.is_some() && self.render.is_some() && self.loading.is_none() {
            if let Some(path) = self.pending_auto_wb.take() {
                let result = self.auto_white_balance().map_or_else(|| serde_json::json!({"error": "the photo could not be measured"}), |o| o.to_json());
                let _ = std::fs::write(&path, result.to_string());
                if self.pending_export.is_none() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        // `--export <file>`: once the picture and its profile are ready, export and quit.
        if self.loaded.is_some() && self.render.is_some() && self.export_job.is_none() {
            if let Some(path) = self.pending_export.take() {
                if let Some(format) = path.extension().and_then(|e| e.to_str()).and_then(export::Format::from_extension) {
                    self.export.format = format;
                }
                self.quit_after_export = true;
                self.start_export(path, ctx);
                if self.export_job.is_none() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
}

/// Which spot is under `t` (texture coordinates): the spot, and whether it is the source.
/// The selected spot is checked first, then the newest.
fn spot_hit(spots: &[spots::Spot], selected: Option<usize>, map: &spots::ScreenMap, t: [f32; 2]) -> Option<(usize, bool)> {
    let order = selected.into_iter().chain((0..spots.len()).rev());
    for i in order {
        let Some(s) = spots.get(i) else { continue };
        if s.distance(t, map.aspect()) <= s.radius {
            return Some((i, false));
        }
        if s.source_distance(t, map.aspect()) <= s.radius {
            return Some((i, true));
        }
    }
    None
}

/// What the spot tool changes as the pointer clicks, drags and paints.
struct SpotInput<'a> {
    spots: &'a mut Vec<spots::Spot>,
    selected: &'a mut Option<usize>,
    /// The outline being dragged: spot, source rather than target, grab offset.
    drag: &'a mut Option<(usize, bool, [f32; 2])>,
    paint: &'a mut Option<Vec<[f32; 2]>>,
}

impl SpotInput<'_> {
    /// Clicks add a circle (or select the spot clicked); drags on an outline move it; drags
    /// elsewhere paint a stroke, which becomes a spot when the button is let go. Nothing
    /// starts while `pan` (Space) is held.
    fn handle(&mut self, brush: &spots::Spot, preview: &spots::Preview, ui: &egui::Ui, response: &egui::Response, map: &spots::ScreenMap, pan: bool) {
        let aspect = map.aspect();
        let inside = |t: [f32; 2]| (0.0..=1.0).contains(&t[0]) && (0.0..=1.0).contains(&t[1]);
        if response.drag_started_by(egui::PointerButton::Primary) && !pan {
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                let t = map.to_texture(origin);
                match spot_hit(self.spots, *self.selected, map, t) {
                    Some((i, source)) => {
                        let s = &self.spots[i];
                        let anchor = if source { [s.sx, s.sy] } else { [s.x, s.y] };
                        *self.drag = Some((i, source, [anchor[0] - t[0], anchor[1] - t[1]]));
                        *self.selected = Some(i);
                    }
                    None if inside(t) && self.spots.len() < spots::MAX_SPOTS => *self.paint = Some(vec![t]),
                    None => {}
                }
            }
        }
        let released = !ui.input(|i| i.pointer.primary_down());
        if let Some((i, source, offset)) = *self.drag {
            if let (Some(p), Some(s)) = (response.interact_pointer_pos(), self.spots.get_mut(i)) {
                let t = map.to_texture(p);
                let at = [t[0] + offset[0], t[1] + offset[1]];
                if source {
                    (s.sx, s.sy) = (at[0], at[1]);
                } else {
                    s.move_target(at[0], at[1]);
                }
            }
            if released {
                *self.drag = None;
            }
        }
        if let Some(path) = self.paint.as_mut() {
            if let Some(p) = response.interact_pointer_pos() {
                let t = map.to_texture(p);
                let t = [t[0].clamp(0.0, 1.0), t[1].clamp(0.0, 1.0)];
                let last = *path.last().unwrap_or(&t);
                if ((t[0] - last[0]) * aspect).hypot(t[1] - last[1]) >= 0.1 * brush.radius {
                    path.push(t);
                }
            }
            if released {
                let path = self.paint.take().unwrap_or_default();
                self.add(path, brush, preview, aspect);
            }
        }
        if response.clicked() && self.paint.is_none() {
            let Some(p) = response.interact_pointer_pos() else { return };
            let t = map.to_texture(p);
            if !inside(t) {
                return;
            }
            match spot_hit(self.spots, *self.selected, map, t) {
                Some((i, _)) => *self.selected = Some(i),
                None => self.add(vec![t], brush, preview, aspect),
            }
        }
    }

    /// Add a spot along `path` (one point = a circle) with the brush's settings and an
    /// automatically chosen source, thinning the stroke to fit what the shader can hold.
    fn add(&mut self, path: Vec<[f32; 2]>, brush: &spots::Spot, preview: &spots::Preview, aspect: f32) {
        if path.is_empty() || self.spots.len() >= spots::MAX_SPOTS {
            return;
        }
        let room = spots::MAX_POINTS.saturating_sub(self.spots.iter().map(|s| 1 + s.path.len()).sum::<usize>());
        let mut step = 0.3 * brush.radius;
        let mut points = spots::simplify(&path, step, aspect);
        while points.len() > room && points.len() > 1 {
            step *= 1.6;
            points = spots::simplify(&path, step, aspect);
        }
        if points.len() > room {
            return;
        }
        let [dx, dy] = preview.find_offset(&points, brush.radius);
        let (x, y) = (points[0][0], points[0][1]);
        self.spots.push(spots::Spot { x, y, sx: x + dx, sy: y + dy, path: points[1..].to_vec(), ..brush.clone() });
        *self.selected = Some(self.spots.len() - 1);
    }
}

/// When a version was saved, briefly: "14:32" today, "12 Nov" this year, else "12 Nov 2025".
fn when(seconds: u64) -> String {
    let now = settings::now_seconds();
    // Local time is not needed to the minute here; UTC offset from the system is enough.
    let days = |s: u64| s / 86_400;
    let (y, m, d) = civil(days(seconds) as i64);
    let (ny, _, _) = civil(days(now) as i64);
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    if days(seconds) == days(now) {
        let t = seconds % 86_400;
        format!("today {:02}:{:02} UTC", t / 3600, t / 60 % 60)
    } else if y == ny {
        format!("{d} {}", MONTHS[(m - 1) as usize])
    } else {
        format!("{d} {} {y}", MONTHS[(m - 1) as usize])
    }
}

/// Year, month, day from days since 1970 (Howard Hinnant's civil-from-days).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}
