//! The look of the app: a neutral darkroom built from a ladder of near-black surfaces, with
//! warm amber for primary actions and selection and crisp teal for active tools and
//! technical readouts. Applied once at start-up.

use eframe::egui::{self, Color32, FontFamily, FontId, Rounding, Stroke, TextStyle};

// Surfaces, darkest first. Nothing is pure black.
/// The well the photo sits in.
pub const CANVAS: Color32 = Color32::from_rgb(0x0E, 0x0F, 0x12);
/// Telemetry pills, scope wells, thumbnail cards.
pub const WELL: Color32 = Color32::from_rgb(0x11, 0x13, 0x17);
/// Top bar, side panels, filmstrip dock.
pub const PANEL: Color32 = Color32::from_rgb(0x17, 0x19, 0x1D);
/// Raised controls, dropdowns, hover states.
pub const ELEVATED: Color32 = Color32::from_rgb(0x22, 0x25, 0x2C);
/// Slider tracks.
pub const TRACK: Color32 = Color32::from_rgb(0x28, 0x2B, 0x33);

// Accents.
/// Primary buttons, format badges, slider fills, selection.
pub const ACCENT: Color32 = Color32::from_rgb(0xF5, 0x9E, 0x0B);
/// Active view and tool toggles, status, technical readouts.
pub const TEAL: Color32 = Color32::from_rgb(0x2D, 0xD4, 0xBF);

// Text.
pub const TEXT_BRIGHT: Color32 = Color32::from_rgb(0xF3, 0xF4, 0xF6);
pub const TEXT_VALUE: Color32 = Color32::from_rgb(0xE5, 0xE7, 0xEB);
pub const TEXT: Color32 = Color32::from_rgb(0xD1, 0xD5, 0xDB);
pub const TEXT_HEADER: Color32 = Color32::from_rgb(0x9C, 0xA3, 0xAF);
pub const FADED: Color32 = Color32::from_rgb(0x6B, 0x72, 0x80);

/// One-pixel dividers.
/// Thin dividing lines (between sections, under headers, menu separators): the main pane's
/// dark tone, cut into the panel's grey.
pub fn divider() -> Stroke {
    Stroke::new(1.0_f32, CANVAS)
}

pub fn hairline() -> Stroke {
    Stroke::new(1.0_f32, Color32::from_white_alpha(16))
}

/// `color` at `alpha` (0..1), for tinted fills and borders.
pub fn tint(color: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (alpha * 255.0).round() as u8)
}

/// The heavier weight used for section headers and titles.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, semibold())
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}


/// Use the system's interface fonts where they exist (Segoe UI and Consolas on Windows, the
/// San Francisco text fonts on macOS), keeping egui's own as the fallback for missing glyphs.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut load = |name: &str, candidates: &[&str]| -> bool {
        for path in candidates {
            if let Ok(bytes) = std::fs::read(path) {
                fonts.font_data.insert(name.to_owned(), egui::FontData::from_owned(bytes));
                return true;
            }
        }
        false
    };
    let regular = load("ui-regular", &["C:\\Windows\\Fonts\\segoeui.ttf", "/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc"]);
    let heavy = load("ui-semibold", &["C:\\Windows\\Fonts\\seguisb.ttf", "C:\\Windows\\Fonts\\segoeuib.ttf", "/System/Library/Fonts/SFNSRounded.ttf"]);
    let mono = load("ui-mono", &["C:\\Windows\\Fonts\\consola.ttf", "/System/Library/Fonts/SFNSMono.ttf", "/System/Library/Fonts/Menlo.ttc"]);

    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if regular {
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "ui-regular".to_owned());
    }
    let mut heavy_family = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or(fallback);
    if heavy {
        heavy_family.insert(0, "ui-semibold".to_owned());
    }
    fonts.families.insert(semibold(), heavy_family);
    if mono {
        fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "ui-mono".to_owned());
    }
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    let mut style = (*ctx.style()).clone();

    style.text_styles = [
        (TextStyle::Heading, bold(14.0)),
        (TextStyle::Body, font(12.5)),
        (TextStyle::Button, font(12.5)),
        (TextStyle::Small, font(11.5)),
        (TextStyle::Monospace, mono(11.5)),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(9.0, 3.0);
    style.spacing.interact_size.y = 22.0;
    style.spacing.indent = 12.0;
    style.spacing.window_margin = egui::Margin::same(12.0);
    style.spacing.menu_margin = egui::Margin::same(6.0);
    // Thin overlay scroll bars that widen a little under the pointer.
    style.spacing.scroll = egui::style::ScrollStyle::floating();
    style.spacing.scroll.bar_width = 4.0;
    style.spacing.scroll.floating_width = 4.0;
    style.spacing.scroll.floating_allocated_width = 0.0;

    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = Color32::from_rgb(0x1B, 0x1D, 0x22);
    v.window_stroke = Stroke::new(1.0_f32, Color32::from_white_alpha(24));
    v.window_rounding = Rounding::same(8.0);
    v.menu_rounding = Rounding::same(6.0);
    v.extreme_bg_color = WELL; // text boxes
    v.faint_bg_color = Color32::from_rgb(0x1B, 0x1D, 0x22);
    v.selection.bg_fill = tint(ACCENT, 0.30);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.hyperlink_color = TEAL;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.indent_has_left_vline = false;
    v.collapsing_header_frame = false;
    v.override_text_color = None;

    let rounding = Rounding::same(4.0);
    // Dividers and panel edges: hairlines only.
    v.widgets.noninteractive.bg_stroke = divider();
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.noninteractive.rounding = rounding;
    // Controls: flat fills, no outlines.
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x1D, 0x20, 0x26);
    v.widgets.inactive.bg_fill = TRACK;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.rounding = rounding;
    v.widgets.hovered.weak_bg_fill = ELEVATED;
    v.widgets.hovered.bg_fill = ELEVATED;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT_BRIGHT);
    v.widgets.hovered.rounding = rounding;
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x2A, 0x2E, 0x36);
    v.widgets.active.bg_fill = ELEVATED;
    v.widgets.active.bg_stroke = Stroke::NONE;
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT_BRIGHT);
    v.widgets.active.rounding = rounding;
    v.widgets.active.expansion = 0.0;
    v.widgets.open.weak_bg_fill = ELEVATED;
    v.widgets.open.bg_fill = ELEVATED;
    v.widgets.open.bg_stroke = Stroke::NONE;
    v.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT_BRIGHT);
    v.widgets.open.rounding = rounding;

    ctx.set_style(style);
}
