//! The preview panel: text with line numbers, tables, images and GeoTIFFs.

use std::sync::Arc;

use eframe::egui::{
    self, Align, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, TextFormat, Ui, Vec2, text::LayoutJob,
};
use egui_extras::{Column, TableBuilder};

use super::{App, PreviewContent, widgets};
use crate::format::format_size;
use crate::geotiff::{Colormap, Layout as TiffLayout, Raster, Stretch, Style, TiffPreview};
use crate::preview::highlight::{self, Language, Span};
use crate::preview::{Preview, Table};
use crate::settings::PreviewSide;
use crate::theme::Theme;
use crate::tr;

/// Zoom and pan of an image preview. Zoom 1 fits the image into the panel.
#[derive(Clone, Copy)]
pub struct ImageView {
    zoom: f32,
    offset: Vec2,
}

impl Default for ImageView {
    fn default() -> Self {
        ImageView { zoom: 1.0, offset: Vec2::ZERO }
    }
}

impl App {
    pub(super) fn preview_ui(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !self.preview.badge.is_empty() {
                    widgets::badge(ui, &theme, &self.preview.badge);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&self.preview.title)
                                .size(theme.shape.heading_size)
                                .strong()
                                .color(theme.colors.section.0),
                        )
                        .truncate(),
                    );
                });
            });
        });
        if !self.preview.meta.is_empty() {
            ui.label(widgets::muted(&theme, &self.preview.meta));
        }
        if !self.preview.notice.is_empty() {
            ui.add(egui::Label::new(RichText::new(&self.preview.notice).color(theme.colors.notice.0)).wrap());
        }
        if let PreviewContent::Tiff { tiff, .. } = &self.preview.content
            && tiff.info.sampled.is_some()
            && ui
                .button(format!("⬇ {}", tr!("tiff_load_all", size = format_size(Some(tiff.info.level_bytes)))))
                .on_hover_text(tr!("tiff_load_all_tip"))
                .clicked()
        {
            self.load_full_tiff();
        }
        ui.add_space(4.0);
        egui::Panel::bottom("preview-buttons")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin { top: 6, ..Default::default() }))
            .show_separator_line(false)
            .show(ui, |ui| self.preview_buttons(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.preview_content_ui(ui));
    }

    fn preview_content_ui(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        let preview = &mut self.preview;
        match &mut preview.content {
            PreviewContent::Message(message) => {
                widgets::centered_message(ui, RichText::new(message.as_str()).color(theme.colors.muted.0));
            }
            PreviewContent::Text(text) => {
                if preview.editing {
                    editor_ui(ui, &theme, &mut preview.edit_text, text.kind.language());
                } else if let Some(table) = &text.table {
                    table_ui(ui, &theme, table);
                } else {
                    code_ui(ui, &theme, text);
                }
            }
            PreviewContent::Image { texture, view, .. } => {
                let hover_id = egui::Id::unique("image-hover");
                let hovered: Option<Pos2> = ui.data(|d| d.get_temp(hover_id)).flatten();
                egui::Panel::bottom("image-info")
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin { top: 4, ..Default::default() }))
                    .show_separator_line(false)
                    .show(ui, |ui| {
                        let line = match hovered {
                            Some(point) => format!("{} {}, {}", tr!("geo_pixel"), point.x as u64, point.y as u64),
                            None => tr!("geo_zoom_hint"),
                        };
                        ui.label(widgets::muted(&theme, line));
                    });
                let hovered = egui::CentralPanel::no_frame()
                    .show(ui, |ui| image_ui(ui, &theme, texture, view, ui.available_height()))
                    .inner;
                ui.data_mut(|d| d.insert_temp(hover_id, hovered));
            }
            PreviewContent::Tiff { tiff, texture, style, view } => {
                // The readout below the image is filled from the previous frame's hover.
                let hover_id = egui::Id::unique("tiff-hover");
                let hovered: Option<Pos2> = ui.data(|d| d.get_temp(hover_id)).flatten();
                let mut restyle = false;
                egui::Panel::bottom("tiff-controls")
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin { top: 4, ..Default::default() }))
                    .show_separator_line(false)
                    .show(ui, |ui| {
                        let (values, place) = hover_lines(tiff, hovered);
                        ui.add(egui::Label::new(widgets::muted(&theme, values)).truncate());
                        ui.add(egui::Label::new(widgets::muted(&theme, place)).truncate());
                        restyle = style_controls(ui, tiff, style);
                        details_ui(ui, &theme, tiff);
                    });
                if restyle {
                    texture.set(to_color_image(&tiff.raster, style), texture_options());
                }
                let hovered = egui::CentralPanel::no_frame()
                    .show(ui, |ui| image_ui(ui, &theme, texture, view, ui.available_height()))
                    .inner;
                ui.data_mut(|d| d.insert_temp(hover_id, hovered));
            }
        }
    }

    fn preview_buttons(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            let has_text = self.preview.text().is_some();
            if widgets::icon_button(ui, &theme, "📋", &tr!("copy"), has_text).clicked() {
                self.copy_preview(&ctx);
            }
            let editable = matches!(&self.preview.content, PreviewContent::Text(text) if text.editable());
            if editable {
                if self.preview.editing {
                    let can_save = self.preview.dirty() && self.connected();
                    if widgets::icon_button(ui, &theme, "💾", &tr!("save_edits_tip"), can_save).clicked() {
                        self.save_edits();
                    }
                    if widgets::icon_button(ui, &theme, "✖", &tr!("stop_editing"), true).clicked() {
                        if self.preview.dirty() {
                            self.dialog = Some(super::Dialog::Confirm {
                                title: tr!("discard_title"),
                                body: tr!("discard_body"),
                                action: super::Confirmed::StopEditing,
                            });
                        } else {
                            self.preview.editing = false;
                        }
                    }
                } else if widgets::icon_button(ui, &theme, "✏", &tr!("edit_tip"), self.connected()).clicked() {
                    self.start_editing();
                }
            }
            let file = self.preview.entry.as_ref().is_some_and(|entry| !entry.is_folder);
            if widgets::icon_button(ui, &theme, "⬇", &tr!("download_file_tip"), file && self.transfer.is_none())
                .clicked()
            {
                self.download_current();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let detached = self.settings.preview_detached;
                let (icon, tip) = if detached { ("↙", tr!("pop_in")) } else { ("↗", tr!("pop_out")) };
                if widgets::icon_button(ui, &theme, icon, &tip, true).clicked() {
                    self.settings.preview_detached = !detached;
                    let _ = self.settings.save(&self.root);
                }
                if !detached {
                    let (icon, tip, side) = match self.settings.preview_side {
                        PreviewSide::Right => ("⇤", tr!("dock_left"), PreviewSide::Left),
                        PreviewSide::Left => ("⇥", tr!("dock_right"), PreviewSide::Right),
                    };
                    if widgets::icon_button(ui, &theme, icon, &tip, true).clicked() {
                        self.settings.preview_side = side;
                        let _ = self.settings.save(&self.root);
                    }
                }
            });
        });
    }
}

fn code_frame(theme: &Theme) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.colors.code_bg.0)
        .stroke(egui::Stroke::new(1.0, theme.colors.line.0))
        .corner_radius(widgets::radius(10.0))
        .inner_margin(egui::Margin::same(6))
}

fn line_job(text: &str, start: usize, end: usize, spans: &[Span], theme: &Theme, font: &FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let plain = TextFormat::simple(font.clone(), theme.colors.code_text.0);
    let mut position = start;
    for span in highlight::spans_in(spans, start, end) {
        if span.start > position {
            job.append(&text[position..span.start], 0.0, plain.clone());
        }
        job.append(&text[span.start..span.end], 0.0, TextFormat::simple(font.clone(), theme.token(span.token)));
        position = span.end;
    }
    if position < end {
        job.append(&text[position..end], 0.0, plain);
    }
    job
}

/// Read-only text, laid out only for the visible lines.
fn code_ui(ui: &mut Ui, theme: &Theme, preview: &Preview) {
    let font = FontId::monospace(theme.shape.mono_size);
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let digits = preview.lines.len().max(1).to_string().len();
    let char_width = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '9'));
    let gutter = 14.0 + char_width * digits as f32;
    code_frame(theme).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        // Rows must be exactly `row_height` apart, or show_rows misjudges the visible range.
        ui.spacing_mut().item_spacing = Vec2::new(10.0, 0.0);
        ui.spacing_mut().interact_size.y = row_height;
        egui::ScrollArea::both().id_salt("code").auto_shrink([false, false]).show_rows(
            ui,
            row_height,
            preview.lines.len(),
            |ui, range| {
                for index in range {
                    let (start, end) = preview.lines[index];
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(gutter, row_height), Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, theme.colors.gutter_bg.0);
                        ui.painter().text(
                            rect.right_center() - Vec2::new(6.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            (index + 1).to_string(),
                            font.clone(),
                            theme.colors.gutter_text.0,
                        );
                        let job = line_job(&preview.text, start, end, &preview.spans, theme, &font);
                        ui.add(egui::Label::new(job).extend());
                    });
                }
            },
        );
    });
}

fn editor_ui(ui: &mut Ui, theme: &Theme, text: &mut String, language: Language) {
    let font = FontId::monospace(theme.shape.mono_size);
    // Highlighting the whole buffer on every frame is wasteful, so the spans are kept
    // until the text changes. Galleys are not kept: they go stale when fonts are rebuilt.
    let cache_id = egui::Id::unique("editor-spans");
    let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, _wrap: f32| -> Arc<egui::Galley> {
        let text = buffer.as_str();
        let hash = egui::util::hash(text);
        let spans = match ui.data(|data| data.get_temp::<(u64, Arc<Vec<Span>>)>(cache_id)) {
            Some((cached, spans)) if cached == hash => spans,
            _ => {
                let spans = Arc::new(highlight::highlight(language, text));
                ui.data_mut(|data| data.insert_temp(cache_id, (hash, spans.clone())));
                spans
            }
        };
        let job = line_job(text, 0, text.len(), &spans, theme, &font);
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let lines = text.lines().count().max(1) + usize::from(text.ends_with('\n'));
    let numbers: String = (1..=lines).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
    code_frame(theme).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        egui::ScrollArea::both().id_salt("editor").auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(numbers).font(font.clone()).color(theme.colors.gutter_text.0));
                ui.add(
                    egui::TextEdit::multiline(text)
                        .code_editor()
                        .font(font.clone())
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .desired_rows(lines.max(10))
                        .layouter(&mut layouter),
                );
            });
        });
    });
}

fn table_ui(ui: &mut Ui, theme: &Theme, table: &Table) {
    let row_height = theme.shape.row_height - 4.0;
    let last_number = table.row_numbers.last().copied().unwrap_or(table.rows.len());
    let number_width = 14.0 + 8.0 * last_number.to_string().len() as f32;
    widgets::card(theme).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        egui::ScrollArea::horizontal().id_salt("csv-h").auto_shrink([false, false]).show(ui, |ui| {
            let mut builder = TableBuilder::new(ui)
                .id_salt("csv")
                .striped(true)
                .resizable(true)
                .auto_shrink([false, false])
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::exact(number_width));
            for index in 0..table.headers.len() {
                let last = index + 1 == table.headers.len();
                builder = builder.column(
                    if last {
                        Column::remainder().at_least(60.0)
                    } else {
                        Column::auto().at_least(40.0).at_most(320.0)
                    }
                    .clip(true),
                );
            }
            builder
                .header(row_height, |mut header| {
                    header.col(|_| {});
                    for name in &table.headers {
                        header.col(|ui| {
                            ui.add(
                                egui::Label::new(RichText::new(name).strong().color(theme.colors.header_text.0))
                                    .truncate(),
                            );
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_height, table.rows.len(), |mut row| {
                        let index = row.index();
                        let number = table.row_numbers.get(index).copied().unwrap_or(index + 1);
                        row.col(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(number.to_string()).color(theme.colors.muted.0));
                            });
                        });
                        for cell in &table.rows[index] {
                            row.col(|ui| {
                                ui.add(egui::Label::new(cell.as_str()).truncate());
                            });
                        }
                    });
                });
        });
    });
}

pub(super) fn texture_options() -> egui::TextureOptions {
    egui::TextureOptions {
        magnification: egui::TextureFilter::Nearest,
        minification: egui::TextureFilter::Linear,
        ..Default::default()
    }
}

fn to_color_image(raster: &Raster, style: &Style) -> egui::ColorImage {
    let image = raster.render(style);
    egui::ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.pixels)
}

/// Draws a zoomable, draggable image. Returns the hovered texel, if any.
fn image_ui(
    ui: &mut Ui,
    theme: &Theme,
    texture: &egui::TextureHandle,
    view: &mut ImageView,
    height: f32,
) -> Option<Pos2> {
    let size = Vec2::new(ui.available_width(), height.max(80.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, widgets::radius(10.0), theme.colors.image_bg.0);
    let texture_size = texture.size_vec2();
    if texture_size.x < 1.0 || texture_size.y < 1.0 {
        return None;
    }
    let fit = (rect.width() / texture_size.x).min(rect.height() / texture_size.y);
    let mut scale = fit * view.zoom;
    if response.double_clicked() {
        *view = ImageView::default();
        scale = fit;
    }
    if response.dragged() {
        view.offset += response.drag_delta();
    }
    let mut image_rect = Rect::from_center_size(rect.center() + view.offset, texture_size * scale);
    if response.hovered() {
        let (scroll, pinch) = ui.input(|input| (input.smooth_scroll_delta.y, input.zoom_delta()));
        let factor = (scroll * 0.002).exp() * pinch;
        if (factor - 1.0).abs() > 1e-4
            && let Some(pointer) = response.hover_pos()
        {
            let new_zoom = (view.zoom * factor).clamp(0.25, 64.0);
            let new_scale = fit * new_zoom;
            let anchor = (pointer - image_rect.min) / scale;
            let new_min = pointer - anchor * new_scale;
            let new_center = new_min + texture_size * new_scale / 2.0;
            view.zoom = new_zoom;
            view.offset = new_center - rect.center();
            scale = new_scale;
            image_rect = Rect::from_center_size(rect.center() + view.offset, texture_size * scale);
        }
    }
    painter.image(texture.id(), image_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    let pointer = response.hover_pos()?;
    if !image_rect.contains(pointer) {
        return None;
    }
    let texel = (pointer - image_rect.min) / scale;
    Some(Pos2::new(texel.x.min(texture_size.x - 0.001), texel.y.min(texture_size.y - 0.001)))
}

fn format_coordinate(value: f64, geographic: bool) -> String {
    if geographic { format!("{value:.6}") } else { format!("{value:.2}") }
}

fn format_value(value: f32) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    if value.fract() == 0.0 && value.abs() < 1e9 { format!("{value:.0}") } else { format!("{value:.4}") }
}

fn lat_lon(lon: f64, lat: f64) -> String {
    let ns = if lat >= 0.0 { "N" } else { "S" };
    let ew = if lon >= 0.0 { "E" } else { "W" };
    format!("{:.5}° {ns}, {:.5}° {ew}", lat.abs(), lon.abs())
}

/// Two lines for the hovered pixel: values and position, then map coordinates.
fn hover_lines(tiff: &TiffPreview, texel: Option<Pos2>) -> (String, String) {
    let Some(texel) = texel else { return (tr!("geo_zoom_hint"), String::new()) };
    let raster = &tiff.raster;
    let column = texel.x as f64 * raster.full_width as f64 / raster.width as f64;
    let row = texel.y as f64 * raster.full_height as f64 / raster.height as f64;
    let index = (texel.y as usize).min(raster.height - 1) * raster.width + (texel.x as usize).min(raster.width - 1);
    let values: Vec<String> =
        raster.bands.iter().take(raster.data_bands()).map(|band| format_value(band[index])).collect();
    let first = format!(
        "{}: {}  ·  {} {}, {}",
        tr!("geo_value"),
        values.join(" / "),
        tr!("geo_pixel"),
        column as u64,
        row as u64
    );
    let mut place = Vec::new();
    if let Some(geo) = &tiff.info.geo
        && let Some((x, y)) = geo.pixel_to_map(column, row)
    {
        if let Some((lon, lat)) = geo.to_wgs84(x, y) {
            place.push(lat_lon(lon, lat));
        }
        if !geo.is_geographic() {
            place.push(format!(
                "{} / {}{}",
                format_coordinate(x, false),
                format_coordinate(y, false),
                geo.unit_label()
            ));
        }
    }
    (first, place.join("  ·  "))
}

fn band_label(tiff: &TiffPreview, index: usize) -> String {
    match tiff.info.band_names.get(index).filter(|name| !name.is_empty()) {
        Some(name) => format!("{} · {name}", index + 1),
        None => format!("{}", index + 1),
    }
}

/// Band, stretch and colour map choices. Returns true when the style changed.
fn style_controls(ui: &mut Ui, tiff: &TiffPreview, style: &mut Style) -> bool {
    let raster = &tiff.raster;
    if raster.palette.is_some() {
        return false;
    }
    let before = *style;
    let bands = raster.data_bands();
    ui.horizontal_wrapped(|ui| {
        if bands >= 2 {
            ui.selectable_value(&mut style.rgb, true, "RGB");
            ui.selectable_value(&mut style.rgb, false, tr!("geo_mode_gray"));
            ui.separator();
        }
        let band_combo = |ui: &mut Ui, id: &str, label: &str, value: &mut usize| {
            ui.label(label);
            egui::ComboBox::from_id_salt(id).selected_text(band_label(tiff, *value)).width(48.0).show_ui(ui, |ui| {
                for band in 0..bands {
                    ui.selectable_value(value, band, band_label(tiff, band));
                }
            });
        };
        if style.rgb {
            band_combo(ui, "band-r", "R", &mut style.bands[0]);
            band_combo(ui, "band-g", "G", &mut style.bands[1]);
            band_combo(ui, "band-b", "B", &mut style.bands[2]);
        } else {
            if bands >= 2 {
                band_combo(ui, "band-gray", &tr!("geo_band"), &mut style.gray);
            }
            ui.label(tr!("geo_colormap"));
            egui::ComboBox::from_id_salt("colormap").selected_text(style.colormap.label()).width(80.0).show_ui(
                ui,
                |ui| {
                    for colormap in Colormap::ALL {
                        ui.selectable_value(&mut style.colormap, colormap, colormap.label());
                    }
                },
            );
        }
        ui.label(tr!("geo_stretch"));
        let stretch_label = |stretch: Stretch| match stretch {
            Stretch::None => tr!("geo_stretch_none"),
            Stretch::Percent => tr!("geo_stretch_percent"),
            Stretch::MinMax => tr!("geo_stretch_minmax"),
        };
        egui::ComboBox::from_id_salt("stretch").selected_text(stretch_label(style.stretch)).width(80.0).show_ui(
            ui,
            |ui| {
                if raster.is_u8 {
                    ui.selectable_value(&mut style.stretch, Stretch::None, stretch_label(Stretch::None));
                }
                ui.selectable_value(&mut style.stretch, Stretch::Percent, stretch_label(Stretch::Percent));
                ui.selectable_value(&mut style.stretch, Stretch::MinMax, stretch_label(Stretch::MinMax));
            },
        );
    });
    *style != before
}

fn details_ui(ui: &mut Ui, theme: &Theme, tiff: &TiffPreview) {
    let info = &tiff.info;
    egui::CollapsingHeader::new(RichText::new(tr!("geo_info")).strong()).id_salt("geo-details").show(ui, |ui| {
        egui::ScrollArea::vertical().min_scrolled_height(220.0).max_height(260.0).show(ui, |ui| {
            egui::Grid::new("geo-grid").num_columns(2).spacing([14.0, 4.0]).striped(true).show(ui, |ui| {
                let mut row = |name: String, value: String| {
                    ui.label(widgets::muted(theme, name));
                    ui.add(egui::Label::new(value).wrap());
                    ui.end_row();
                };
                row(tr!("geo_size"), format!("{} × {} px", info.width, info.height));
                let mut bands = info.samples.to_string();
                if !info.band_names.is_empty() {
                    bands += &format!(" ({})", info.band_names.join(", "));
                }
                row(tr!("geo_bands"), bands);
                row(tr!("geo_dtype"), info.dtype());
                row(tr!("geo_compression"), info.compression.clone());
                let mut layout = match info.layout {
                    TiffLayout::Tiled { width, height } => tr!("geo_tiled", width = width, height = height),
                    TiffLayout::Stripped { rows } => tr!("geo_stripped", rows = rows),
                };
                if info.planar_separate {
                    layout += ", planar";
                }
                row(tr!("geo_layout"), layout);
                let overviews = if info.overviews.is_empty() {
                    tr!("geo_overviews_none")
                } else {
                    info.overviews.iter().map(|(w, h)| format!("{w}×{h}")).collect::<Vec<_>>().join(", ")
                };
                row(tr!("geo_overviews"), overviews);
                row(tr!("geo_level"), format!("{}×{}", info.level_size.0, info.level_size.1));
                if let Some(nodata) = info.nodata {
                    row(tr!("geo_nodata"), nodata.to_string());
                }
                row(
                    tr!("geo_fetched"),
                    format!(
                        "{} / {} ({} × GET)",
                        format_size(Some(info.fetched)),
                        format_size(Some(info.file_size)),
                        info.requests
                    ),
                );
                if let Some(geo) = &info.geo {
                    row(tr!("geo_crs"), geo.crs_label());
                    if let Some(t) = geo.transform {
                        let geographic = geo.is_geographic();
                        let unit = geo.unit_label();
                        row(
                            tr!("geo_pixel_size"),
                            format!(
                                "{} × {}{unit}",
                                format_coordinate(t[1].abs(), geographic),
                                format_coordinate(t[5].abs(), geographic)
                            ),
                        );
                        row(
                            tr!("geo_origin"),
                            format!(
                                "{} / {}",
                                format_coordinate(t[0], geographic),
                                format_coordinate(t[3], geographic)
                            ),
                        );
                        let corners = [(0.0, 0.0), (info.width as f64, info.height as f64)];
                        let points: Vec<(f64, f64)> =
                            corners.iter().filter_map(|(c, r)| geo.pixel_to_map(*c, *r)).collect();
                        if let [a, b] = points[..] {
                            row(
                                tr!("geo_bounds"),
                                format!(
                                    "x {} … {}\ny {} … {}",
                                    format_coordinate(a.0.min(b.0), geographic),
                                    format_coordinate(a.0.max(b.0), geographic),
                                    format_coordinate(a.1.min(b.1), geographic),
                                    format_coordinate(a.1.max(b.1), geographic)
                                ),
                            );
                        }
                    }
                }
            });
            if let Some((lon, lat)) = center_wgs84(tiff) {
                ui.horizontal(|ui| {
                    ui.label(widgets::muted(theme, tr!("geo_center")));
                    ui.label(lat_lon(lon, lat));
                    if ui.button(format!("🗺 {}", tr!("geo_open_map"))).on_hover_text(tr!("geo_open_map_tip")).clicked()
                    {
                        let url = format!(
                            "https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map=12/{lat:.6}/{lon:.6}"
                        );
                        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                    }
                });
            }
        });
    });
}

fn center_wgs84(tiff: &TiffPreview) -> Option<(f64, f64)> {
    let geo = tiff.info.geo.as_ref()?;
    let (x, y) = geo.pixel_to_map(tiff.info.width as f64 / 2.0, tiff.info.height as f64 / 2.0)?;
    geo.to_wgs84(x, y)
}
