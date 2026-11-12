//! Small widgets drawn in the colours of the current style.

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Margin, RichText, Stroke, Ui, Vec2};

use crate::theme::Theme;

pub fn radius(value: f32) -> CornerRadius {
    CornerRadius::same(value.round().clamp(0.0, 255.0) as u8)
}

pub fn card(theme: &Theme) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.colors.card.0)
        .stroke(Stroke::new(1.0, theme.colors.line.0))
        .corner_radius(radius(theme.shape.card_radius))
        .inner_margin(Margin::symmetric(10, 6))
}

pub fn section(ui: &mut Ui, theme: &Theme, text: impl Into<String>) -> egui::Response {
    ui.label(RichText::new(text).size(theme.shape.heading_size).strong().color(theme.colors.section.0))
}

pub fn muted(theme: &Theme, text: impl Into<String>) -> RichText {
    RichText::new(text).color(theme.colors.muted.0)
}

/// A square button with an icon glyph and a tooltip.
pub fn icon_button(ui: &mut Ui, theme: &Theme, icon: &str, tip: &str, enabled: bool) -> egui::Response {
    let size = theme.shape.font_size + 20.0;
    let button = egui::Button::new(RichText::new(icon).size(theme.shape.font_size + 2.0)).min_size(Vec2::splat(size));
    ui.add_enabled(enabled, button).on_hover_text(tip).on_disabled_hover_text(tip)
}

/// A button with icon and text, used in the connection card.
pub fn text_button(ui: &mut Ui, theme: &Theme, icon: &str, text: &str, tip: &str, enabled: bool) -> egui::Response {
    let size = Vec2::new(0.0, theme.shape.font_size + 20.0);
    let button = egui::Button::new(format!("{icon}  {text}")).min_size(size);
    ui.add_enabled(enabled, button).on_hover_text(tip).on_disabled_hover_text(tip)
}

/// Accent-coloured button for the main action of a dialog.
pub fn primary_button(ui: &mut Ui, theme: &Theme, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(theme.colors.accent_text.0))
            .fill(theme.colors.accent.0)
            .min_size(Vec2::new(80.0, theme.shape.font_size + 16.0)),
    )
}

pub fn badge(ui: &mut Ui, theme: &Theme, text: &str) {
    egui::Frame::new()
        .fill(theme.colors.badge_bg.0)
        .corner_radius(radius(8.0))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| ui.label(RichText::new(text).strong().color(theme.colors.badge_text.0)));
}

/// The red error banner. Returns true when the close button was clicked.
pub fn banner(ui: &mut Ui, theme: &Theme, text: &str) -> bool {
    let mut close = false;
    egui::Frame::new()
        .fill(theme.colors.banner_bg.0)
        .stroke(Stroke::new(1.0, theme.colors.banner_line.0))
        .corner_radius(radius(8.0))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                close = ui
                    .add(egui::Button::new(RichText::new("✖").color(theme.colors.banner_text.0)).frame(false))
                    .clicked();
                ui.add(egui::Label::new(RichText::new(text).color(theme.colors.banner_text.0)).wrap());
            });
        });
    close
}

/// A text field with a clear button inside the row.
pub fn filter_field(ui: &mut Ui, text: &mut String, hint: &str, tip: &str, width: f32) -> egui::Response {
    ui.horizontal(|ui| {
        let response = ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(width)).on_hover_text(tip);
        if !text.is_empty() && ui.add(egui::Button::new("✖").frame(false)).on_hover_text("Clear").clicked() {
            text.clear();
        }
        response
    })
    .inner
}

pub fn spinner_color(theme: &Theme) -> Color32 {
    theme.colors.accent.0
}

/// A message in the middle of the remaining space, wrapped and centred line by line.
pub fn centered_message(ui: &mut Ui, text: RichText) {
    let height = ui.available_height();
    ui.vertical_centered(|ui| {
        ui.add_space((height * 0.42 - 20.0).max(0.0));
        ui.add(egui::Label::new(text).halign(Align::Center).wrap());
    });
}

/// A left-aligned label in a column of fixed width, for form rows.
pub fn field_label(ui: &mut Ui, text: impl Into<egui::WidgetText>, width: f32) {
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(Vec2::new(width, height), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_width(width);
        ui.add(egui::Label::new(text).truncate());
    });
}
