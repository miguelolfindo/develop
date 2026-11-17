//! Exports without a window: `develop <photo> --export <file> ...`, as the MCP server runs it
//! for every check and render.
//!
//! The app is driven exactly as in a window (the same settings, the same GPU pipeline and the
//! same export code), but on a GPU connection made without a window, and its frames are run
//! without being drawn. So nothing appears on screen or in the taskbar while an assistant works.
//! If the GPU cannot be reached this way, the export runs in a window as before.

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::{egui, egui_wgpu, wgpu};

/// Give up on an export that has not finished in this long.
const LIMIT: Duration = Duration::from_secs(300);

/// The GPU, set up as eframe would for a window (same limits), but without one.
fn gpu() -> Option<egui_wgpu::RenderState> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::PRIMARY, ..Default::default() });
    let adapter = pollster_block(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))?;
    let (device, queue) = pollster_block(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("develop device (no window)"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits { max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d, ..wgpu::Limits::default() },
        },
        None,
    ))
    .ok()?;
    // The format a window would use; only the on-screen drawing (not used here) depends on it.
    let target_format = wgpu::TextureFormat::Bgra8Unorm;
    let renderer = egui_wgpu::Renderer::new(&device, target_format, None, 1);
    let adapter = Arc::new(adapter);
    Some(egui_wgpu::RenderState {
        available_adapters: Arc::new([]),
        adapter,
        device: Arc::new(device),
        queue: Arc::new(queue),
        target_format,
        renderer: Arc::new(egui::mutex::RwLock::new(renderer)),
    })
}

/// Wait for a future without an async runtime (wgpu's requests complete on their own).
fn pollster_block<F: std::future::Future>(future: F) -> F::Output {
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop(_: *const ()) {}
    fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
            return out;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Run the export without a window. Returns false when no GPU could be set up this way (the
/// caller then opens a window instead).
pub fn run(initial: Option<std::path::PathBuf>) -> bool {
    let Some(render_state) = gpu() else { return false };
    let ctx = egui::Context::default();
    let mut app = crate::App::with_gpu(&ctx, render_state, initial);
    let started = Instant::now();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
    loop {
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let output = ctx.run(input, |ctx| app.frame(ctx));
        let close = output.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::Close)));
        if close || started.elapsed() > LIMIT {
            break;
        }
        // Loading and exporting happen on other threads; look again shortly.
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}
