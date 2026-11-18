//! The app's own widgets, painted by hand so nothing looks like stock egui: the slider,
//! section headers, buttons, toggles, badges, pills and the small vector icons.

use std::ops::RangeInclusive;

use eframe::egui::{self, Color32, Pos2, Rect, Response, Rounding, Sense, Shape, Stroke, Ui, Vec2};

use crate::theme::{self, ACCENT, ELEVATED, FADED, TEAL, TEXT, TEXT_BRIGHT, TEXT_HEADER, TEXT_VALUE, TRACK};

// ---------------------------------------------------------------------------------------
// Slider
// ---------------------------------------------------------------------------------------

/// The app's slider. Row 1: name on the left (double-click resets), value on the right
/// (borderless; click to type, drag to nudge). Row 2: a thin dark track with the fill in
/// `accent_color`, from the centre tick outward when the default sits inside the range
/// (Exposure, Contrast, Tint...) and from the left edge otherwise (Sharpening...).
/// Returns true when the value changed. Precision follows the size of the range.
pub fn pro_slider(ui: &mut Ui, label: &str, value: &mut f32, range: RangeInclusive<f32>, default_val: f32, suffix: &str, accent_color: Color32) -> bool {
    let span = range.end() - range.start();
    let (decimals, step) = if span <= 20.0 {
        (2, 0.01)
    } else if span <= 100.0 {
        (1, 0.1)
    } else {
        (0, 1.0)
    };
    // Ranges that start at zero (Sharpening, Detail, Masking) fill from the left edge.
    let from_left = *range.start() == 0.0;
    pro_slider_with(ui, label, value, range, default_val, suffix, accent_color, decimals, step, from_left)
}

/// `pro_slider` with the number of decimals and the step chosen by the caller.
#[allow(clippy::too_many_arguments)]
pub fn pro_slider_with(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
    default_val: f32,
    suffix: &str,
    accent_color: Color32,
    decimals: usize,
    step: f32,
    from_left: bool,
) -> bool {
    let before = *value;
    let (min, max) = (*range.start(), *range.end());
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;

        // Row 1: name and value.
        ui.horizontal(|ui| {
            ui.set_height(18.0);
            let name = ui
                .add(egui::Label::new(egui::RichText::new(label).font(theme::font(12.5)).color(TEXT)).sense(Sense::click()))
                .on_hover_text("Double-click to reset");
            if name.double_clicked() {
                *value = default_val;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let widgets = &mut ui.visuals_mut().widgets;
                widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                widgets.inactive.bg_fill = Color32::TRANSPARENT;
                for state in [&mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
                    state.weak_bg_fill = ELEVATED;
                    state.bg_fill = ELEVATED;
                    state.rounding = Rounding::same(4.0);
                }
                ui.visuals_mut().extreme_bg_color = ELEVATED;
                ui.visuals_mut().override_text_color = Some(TEXT_VALUE);
                ui.style_mut().override_font_id = Some(theme::font(12.0));
                ui.spacing_mut().button_padding = egui::vec2(5.0, 1.0);
                ui.add(
                    egui::DragValue::new(value)
                        .speed(step as f64)
                        .clamp_range(range.clone())
                        .min_decimals(decimals)
                        .max_decimals(decimals)
                        // Always the same number of decimals, with a plus sign on the positive
                        // side of sliders that go both ways.
                        .custom_formatter(move |v, _| if min < 0.0 && v > 0.0 { format!("+{v:.decimals$}") } else { format!("{v:.decimals$}") })
                        .suffix(suffix),
                );
            });
        });
        ui.add_space(4.0);

        // Row 2: the track.
        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 12.0), Sense::click_and_drag());
        let (left, right) = (rect.left() + 5.0, rect.right() - 5.0);
        if response.double_clicked() {
            *value = default_val;
        } else if response.dragged() || response.clicked() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let t = ((pointer.x - left) / (right - left)).clamp(0.0, 1.0);
                let raw = min + t * (max - min);
                *value = ((raw / step).round() * step).clamp(min, max);
            }
        }
        let x_of = |v: f32| left + (right - left) * ((v - min) / (max - min)).clamp(0.0, 1.0);
        let y = rect.center().y;
        let painter = ui.painter();
        let track = Rect::from_min_max(egui::pos2(left, y - 1.75), egui::pos2(right, y + 1.75));
        painter.rect_filled(track, 2.0, TRACK);
        let thumb_x = x_of(*value);
        let epsilon = (max - min) * 1.0e-4;
        let bipolar = !from_left && default_val > min + epsilon && default_val < max - epsilon;
        let from = if bipolar { x_of(default_val) } else { left };
        if (thumb_x - from).abs() > 0.5 {
            let fill = Rect::from_min_max(egui::pos2(from.min(thumb_x), y - 1.75), egui::pos2(from.max(thumb_x), y + 1.75));
            painter.rect_filled(fill, 2.0, accent_color);
        }
        if bipolar {
            painter.line_segment([egui::pos2(from, y - 4.0), egui::pos2(from, y + 4.0)], Stroke::new(1.0_f32, Color32::from_white_alpha(70)));
        }
        let radius = if response.hovered() || response.dragged() { 5.75 } else { 5.0 };
        let centre = egui::pos2(thumb_x, y);
        painter.circle_filled(centre + egui::vec2(0.0, 1.0), radius + 1.0, Color32::from_black_alpha(90));
        painter.circle_filled(centre, radius, TEXT_BRIGHT);
        ui.add_space(10.0);
    });
    *value != before
}

// ---------------------------------------------------------------------------------------
// Sections and dividers
// ---------------------------------------------------------------------------------------

/// A full-width one-pixel divider.
pub fn hairline(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, theme::divider());
}

/// Uppercase, letter-spaced text for section headers.
fn tracked(text: &str, size: f32, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(&text.to_uppercase(), 0.0, egui::TextFormat { font_id: theme::bold(size), color, extra_letter_spacing: 0.9, ..Default::default() });
    job
}

/// A collapsible section: an uppercase header with a chevron at the right, its contents
/// when open, and a hairline underneath. `tooltip` explains the section on hover.
pub fn section(ui: &mut Ui, title: &str, default_open: bool, tooltip: &str, add_contents: impl FnOnce(&mut Ui)) {
    let id = egui::Id::new(("section", title));
    let mut open = ui.ctx().data_mut(|d| *d.get_temp_mut_or(id, default_open));
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), Sense::click());
    if !tooltip.is_empty() {
        response = response.on_hover_text(tooltip);
    }
    if response.clicked() {
        open = !open;
        ui.ctx().data_mut(|d| d.insert_temp(id, open));
    }
    let color = if response.hovered() { TEXT } else { TEXT_HEADER };
    let galley = ui.fonts(|f| f.layout_job(tracked(title, 11.5, color)));
    ui.painter().galley(egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5), galley, color);
    let chevron = Rect::from_center_size(egui::pos2(rect.right() - 6.0, rect.center().y), Vec2::splat(8.0));
    draw_icon(ui.painter(), if open { Icon::ChevronDown } else { Icon::ChevronRight }, chevron, color);
    if open {
        ui.add_space(2.0);
        add_contents(ui);
        ui.add_space(6.0);
    }
    hairline(ui);
}

// ---------------------------------------------------------------------------------------
// Buttons, toggles, badges, pills
// ---------------------------------------------------------------------------------------

/// How a painted button looks.
#[derive(Clone, Copy)]
pub struct Look {
    pub fill: Color32,
    pub hover_fill: Color32,
    pub border: Stroke,
    pub text: Color32,
    pub rounding: f32,
}

impl Look {
    /// Compact bordered button on a panel (Reset All).
    pub fn bordered() -> Self {
        Self { fill: Color32::from_rgb(0x1D, 0x20, 0x26), hover_fill: ELEVATED, border: theme::hairline(), text: TEXT, rounding: 4.0 }
    }

    /// A quieter secondary button.
    pub fn secondary() -> Self {
        Self { fill: Color32::from_rgb(0x1A, 0x1C, 0x21), hover_fill: ELEVATED, border: Stroke::NONE, text: TEXT_HEADER, rounding: 4.0 }
    }

    /// A raised tool button (Crop & Straighten).
    pub fn raised() -> Self {
        Self { fill: Color32::from_rgb(0x23, 0x26, 0x2D), hover_fill: Color32::from_rgb(0x2B, 0x2F, 0x37), border: Stroke::new(1.0_f32, Color32::from_white_alpha(28)), text: TEXT, rounding: 5.0 }
    }

    /// An active view or tool toggle.
    pub fn teal() -> Self {
        Self { fill: theme::tint(TEAL, 0.12), hover_fill: theme::tint(TEAL, 0.18), border: Stroke::new(1.0_f32, theme::tint(TEAL, 0.55)), text: TEAL, rounding: 4.0 }
    }

    /// The primary action (Export).
    pub fn primary() -> Self {
        Self { fill: ACCENT, hover_fill: Color32::from_rgb(0xFB, 0xB0, 0x2A), border: Stroke::NONE, text: Color32::from_rgb(0x11, 0x12, 0x14), rounding: 6.0 }
    }

    /// No fill until hovered (icon buttons in headers).
    pub fn ghost() -> Self {
        Self { fill: Color32::TRANSPARENT, hover_fill: ELEVATED, border: Stroke::NONE, text: TEXT_HEADER, rounding: 4.0 }
    }
}

/// A painted text button. `size.x == 0` means "as wide as the text needs".
pub fn button(ui: &mut Ui, text: &str, size: Vec2, look: Look, font: egui::FontId, icon: Option<Icon>) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, look.text);
    let icon_room = if icon.is_some() { 20.0 } else { 0.0 };
    let width = if size.x > 0.0 { size.x } else { galley.size().x + icon_room + 20.0 };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, size.y), Sense::click());
    let enabled = ui.is_enabled();
    let fade = |c: Color32| if enabled { c } else { c.linear_multiply(0.45) };
    let fill = if response.hovered() && enabled { look.hover_fill } else { look.fill };
    ui.painter().rect(rect, look.rounding, fade(fill), Stroke::new(look.border.width, fade(look.border.color)));
    let content = galley.size().x + icon_room;
    let mut x = rect.center().x - content * 0.5;
    if let Some(icon) = icon {
        draw_icon(ui.painter(), icon, Rect::from_center_size(egui::pos2(x + 7.0, rect.center().y), Vec2::splat(13.0)), fade(look.text));
        x += icon_room;
    }
    ui.painter().galley(egui::pos2(x, rect.center().y - galley.size().y * 0.5), galley, fade(look.text));
    response
}

/// A square button holding one vector icon.
pub fn icon_button(ui: &mut Ui, icon: Icon, side: f32, look: Look) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    let fill = if response.hovered() { look.hover_fill } else { look.fill };
    ui.painter().rect(rect, look.rounding, fill, look.border);
    let color = if response.hovered() { TEXT_BRIGHT } else { look.text };
    draw_icon(ui.painter(), icon, rect.shrink(side * 0.24), color);
    response
}

/// A small outlined label such as a file-format badge. Returns the rectangle it used.
pub fn badge_at(painter: &egui::Painter, top_left: Pos2, text: &str, color: Color32) -> Rect {
    let galley = painter.layout_no_wrap(text.to_uppercase(), theme::bold(10.5), color);
    let rect = Rect::from_min_size(top_left, galley.size() + egui::vec2(8.0, 2.0));
    painter.rect(rect, 3.0, theme::tint(color, 0.12), Stroke::new(1.0_f32, theme::tint(color, 0.45)));
    painter.galley(rect.min + egui::vec2(4.0, 1.0), galley, color);
    rect
}

/// A file-format badge laid out inline.
pub fn badge(ui: &mut Ui, text: &str, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_uppercase(), theme::bold(10.5), color);
    let (rect, _) = ui.allocate_exact_size(galley.size() + egui::vec2(8.0, 2.0), Sense::hover());
    badge_at(ui.painter(), rect.min, text, color);
}

/// A filled dot, for status indicators.
pub fn dot(painter: &egui::Painter, centre: Pos2, color: Color32) {
    painter.circle_filled(centre, 3.0, color);
}

// ---------------------------------------------------------------------------------------
// Vector icons
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    RotateCcw,
    RotateCw,
    Refresh,
    Crop,
    Folder,
    ChevronDown,
    ChevronRight,
    Ascending,
    Descending,
    Grid,
}

/// Points along an arc; angles in radians, clockwise from the positive x axis (screen y down).
fn arc(centre: Pos2, radius: f32, from: f32, to: f32) -> Vec<Pos2> {
    (0..=24).map(|i| {
        let a = from + (to - from) * i as f32 / 24.0;
        centre + egui::vec2(a.cos(), a.sin()) * radius
    })
    .collect()
}

/// Draw `icon` inside `rect` in one colour.
pub fn draw_icon(painter: &egui::Painter, icon: Icon, rect: Rect, color: Color32) {
    let stroke = Stroke::new(1.4_f32, color);
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5;
    let tau = std::f32::consts::TAU;
    match icon {
        Icon::RotateCcw | Icon::RotateCw | Icon::Refresh => {
            // Three quarters of a circle with an arrow head at the open end.
            let clockwise = icon != Icon::RotateCcw;
            let _ = tau;
            let (from, to) = if clockwise { (-2.2, 2.4) } else { (-0.94, -5.54) };
            let points = arc(c, r * 0.82, from, to);
            let (tip, before_tip) = (points[points.len() - 1], points[points.len() - 3]);
            painter.add(Shape::line(points, stroke));
            // The head sits at the end of the arc, pointing the way it turns.
            let along = (tip - before_tip).normalized();
            let across = egui::vec2(-along.y, along.x);
            let head = r * 0.55;
            painter.add(Shape::convex_polygon(
                vec![tip + along * head * 0.9, tip - along * head * 0.2 + across * head * 0.6, tip - along * head * 0.2 - across * head * 0.6],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Grid => {
            // Four small squares.
            let g = rect.width() * 0.14;
            let s = (rect.width() - g) * 0.5;
            for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                let min = rect.min + egui::vec2(dx * (s + g), dy * (s + g));
                painter.rect_stroke(Rect::from_min_size(min, Vec2::splat(s)), 1.0, stroke);
            }
        }
        Icon::Crop => {
            let (l, t, rt, b) = (rect.left(), rect.top(), rect.right(), rect.bottom());
            let inset = rect.width() * 0.26;
            painter.add(Shape::line(vec![egui::pos2(l + inset, t), egui::pos2(l + inset, b - inset), egui::pos2(rt, b - inset)], stroke));
            painter.add(Shape::line(vec![egui::pos2(l, t + inset), egui::pos2(rt - inset, t + inset), egui::pos2(rt - inset, b)], stroke));
        }
        Icon::Folder => {
            let (l, t, rt, b) = (rect.left(), rect.top() + rect.height() * 0.12, rect.right(), rect.bottom() - rect.height() * 0.08);
            let tab = rect.width() * 0.42;
            let lip = rect.height() * 0.2;
            painter.add(Shape::closed_line(
                vec![egui::pos2(l, t), egui::pos2(l + tab * 0.8, t), egui::pos2(l + tab, t + lip), egui::pos2(rt, t + lip), egui::pos2(rt, b), egui::pos2(l, b)],
                Stroke::new(1.2_f32, color),
            ));
        }
        Icon::ChevronDown => {
            painter.add(Shape::line(vec![c + egui::vec2(-r * 0.7, -r * 0.35), c + egui::vec2(0.0, r * 0.35), c + egui::vec2(r * 0.7, -r * 0.35)], stroke));
        }
        Icon::ChevronRight => {
            painter.add(Shape::line(vec![c + egui::vec2(-r * 0.35, -r * 0.7), c + egui::vec2(r * 0.35, 0.0), c + egui::vec2(-r * 0.35, r * 0.7)], stroke));
        }
        Icon::Ascending | Icon::Descending => {
            let up = icon == Icon::Ascending;
            let (tail, tip) = if up { (egui::pos2(c.x, rect.bottom()), egui::pos2(c.x, rect.top())) } else { (egui::pos2(c.x, rect.top()), egui::pos2(c.x, rect.bottom())) };
            let dir = if up { -1.0 } else { 1.0 };
            painter.line_segment([tail, tip], stroke);
            painter.add(Shape::line(vec![tip + egui::vec2(-r * 0.6, -dir * r * 0.6), tip, tip + egui::vec2(r * 0.6, -dir * r * 0.6)], stroke));
        }
    }
}

/// A rounded "pill" background for inline groups (telemetry, HUD). Returns the inner response.
pub fn pill<R>(ui: &mut Ui, fill: Color32, border: Stroke, rounding: f32, margin: egui::Margin, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::none().fill(fill).stroke(border).rounding(rounding).inner_margin(margin).show(ui, add_contents).inner
}

/// Dim text for secondary information.
pub fn dim(text: impl Into<String>, size: f32) -> egui::RichText {
    egui::RichText::new(text).font(theme::font(size)).color(FADED)
}

/// One item of the canvas HUD: text (optionally after a coloured dot) that highlights in
/// teal when `active`.
pub fn hud_item(ui: &mut Ui, text: &str, font: egui::FontId, color: Color32, active: bool, dot_color: Option<Color32>) -> Response {
    let color = if active { TEAL } else { color };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let dot_room = if dot_color.is_some() { 11.0 } else { 0.0 };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(galley.size().x + dot_room + 16.0, 22.0), Sense::click());
    if active {
        ui.painter().rect(rect, 4.0, theme::tint(TEAL, 0.12), Stroke::new(1.0_f32, theme::tint(TEAL, 0.45)));
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, ELEVATED);
    }
    let mut x = rect.left() + 8.0;
    if let Some(dot_color) = dot_color {
        dot(ui.painter(), egui::pos2(x + 3.0, rect.center().y), dot_color);
        x += dot_room;
    }
    ui.painter().galley(egui::pos2(x, rect.center().y - galley.size().y * 0.5), galley, color);
    response
}

/// A thin vertical divider between inline items.
pub fn vdivider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 16.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0_f32, Color32::from_white_alpha(26)));
}

/// An on/off switch with its label. Returns true when it was flipped.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), theme::font(12.5), TEXT);
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(34.0 + galley.size().x, 20.0), Sense::click());
    let enabled = ui.is_enabled();
    if response.clicked() && enabled {
        *on = !*on;
        response.mark_changed();
    }
    let fade = |c: Color32| if enabled { c } else { c.linear_multiply(0.4) };
    let track = Rect::from_min_size(egui::pos2(rect.left(), rect.center().y - 7.0), egui::vec2(26.0, 14.0));
    ui.painter().rect_filled(track, 7.0, fade(if *on { theme::tint(TEAL, 0.85) } else { TRACK }));
    let knob_x = if *on { track.right() - 7.0 } else { track.left() + 7.0 };
    ui.painter().circle_filled(egui::pos2(knob_x, track.center().y), 5.0, fade(TEXT_BRIGHT));
    ui.painter().galley(egui::pos2(rect.left() + 34.0, rect.center().y - galley.size().y * 0.5), galley, fade(TEXT));
    response
}