//! Bucket list, folder table, path and the status line.

use eframe::egui::{self, Align, Layout, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};

use super::{App, SortColumn, TransferKind, widgets};
use crate::format::{format_dt, format_size};
use crate::tr;

impl App {
    pub(super) fn buckets_ui(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        let query = self.bucket_filter.trim().to_lowercase();
        let visible: Vec<usize> = self
            .buckets
            .iter()
            .enumerate()
            .filter(|(_, bucket)| query.is_empty() || bucket.name.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect();
        ui.horizontal(|ui| {
            widgets::section(ui, &theme, tr!("buckets"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let total = self.buckets.len();
                let count = match (total, query.is_empty()) {
                    (0, _) => String::new(),
                    (_, true) => total.to_string(),
                    (_, false) => format!("{} / {total}", visible.len()),
                };
                ui.label(widgets::muted(&theme, count));
            });
        });
        let width = ui.available_width() - 30.0;
        widgets::filter_field(ui, &mut self.bucket_filter, &tr!("filter_buckets"), &tr!("filter_buckets_tip"), width);
        ui.add_space(4.0);
        let mut open: Option<String> = None;
        widgets::card(&theme).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(ui.available_width());
                for index in visible {
                    let bucket = &self.buckets[index];
                    let selected = bucket.name == self.bucket;
                    let text = RichText::new(format!("🗄  {}", bucket.name));
                    let text = if selected { text.color(theme.colors.selection_text.0) } else { text };
                    let button = egui::Button::selectable(selected, text)
                        .min_size(egui::vec2(ui.available_width(), theme.shape.row_height))
                        .truncate();
                    let mut response = ui.add(button);
                    if let Some(created) = bucket.created {
                        response = response.on_hover_text(tr!("created", when = format_dt(Some(created))));
                    }
                    if response.clicked() && !(selected && self.prefix.is_empty()) {
                        open = Some(bucket.name.clone());
                    }
                }
            });
        });
        if let Some(name) = open {
            self.open_bucket(name);
        }
    }

    pub(super) fn files_ui(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        let connected = self.connected();
        let title = if self.bucket.is_empty() { tr!("files") } else { self.bucket.clone() };
        widgets::section(ui, &theme, title);

        ui.horizontal(|ui| {
            if widgets::icon_button(ui, &theme, "⬆", &tr!("up"), connected && !self.prefix.is_empty()).clicked() {
                self.go_up();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let has_selection = connected && !self.selected_entries().is_empty();
                let can_upload = connected && !self.bucket.is_empty() && self.transfer.is_none();
                if widgets::icon_button(ui, &theme, "🔄", &tr!("refresh"), connected).clicked() {
                    self.refresh();
                }
                if widgets::icon_button(ui, &theme, "📁+", &tr!("new_folder_tip"), can_upload).clicked() {
                    self.ask_new_folder();
                }
                if widgets::icon_button(ui, &theme, "📂", &tr!("upload_folder_tip"), can_upload).clicked() {
                    self.upload_folder();
                }
                if widgets::icon_button(ui, &theme, "⬆🗋", &tr!("upload_tip"), can_upload).clicked() {
                    self.upload_files();
                }
                if widgets::icon_button(ui, &theme, "🗑", &tr!("delete_tip"), has_selection && self.transfer.is_none())
                    .clicked()
                {
                    self.delete_selected();
                }
                if widgets::icon_button(
                    ui,
                    &theme,
                    "💾",
                    &tr!("download_tip"),
                    has_selection && self.transfer.is_none(),
                )
                .clicked()
                {
                    self.download_selected();
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    egui::ScrollArea::horizontal().id_salt("crumbs").auto_shrink([false, true]).show(ui, |ui| {
                        ui.horizontal(|ui| self.crumbs_ui(ui));
                    });
                });
            });
        });

        ui.horizontal(|ui| {
            let regex = self.settings.file_filter_regex;
            let (hint, tip) = if regex {
                (tr!("filter_files_regex"), tr!("filter_regex_tip"))
            } else {
                (tr!("filter_files"), tr!("filter_files_tip"))
            };
            let response = widgets::filter_field(ui, &mut self.file_filter, &hint, &tip, ui.available_width() - 230.0);
            if std::mem::take(&mut self.focus_file_filter) {
                response.request_focus();
            }
            let toggle = ui.selectable_label(regex, RichText::new(".*").monospace().strong());
            if toggle.on_hover_text(tr!("filter_regex_toggle")).clicked() {
                self.settings.file_filter_regex = !regex;
                self.save_settings();
            }
            self.refresh_name_filter();
            if let Some(error) = self.name_filter.error() {
                ui.label(RichText::new(tr!("filter_regex_invalid")).color(theme.colors.banner_text.0))
                    .on_hover_text(error);
            } else if self.name_filter.is_active() {
                let listed = self.rows.iter().filter(|row| row.entry.is_some()).count();
                let matched = self.visible_rows().iter().filter(|&&index| self.rows[index].entry.is_some()).count();
                ui.label(widgets::muted(&theme, format!("{matched} / {listed}")));
            }
        });
        let visible = self.visible_rows();
        ui.add_space(4.0);

        if self.bucket.is_empty() {
            widgets::centered_message(
                ui,
                RichText::new(tr!("files_placeholder")).size(theme.shape.font_size + 1.0).color(theme.colors.muted.0),
            );
            return;
        }
        widgets::card(&theme).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            self.table_ui(ui, &visible);
        });
    }

    fn crumbs_ui(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        ui.spacing_mut().item_spacing.x = 2.0;
        let parts: Vec<String> = self.prefix.split('/').filter(|part| !part.is_empty()).map(str::to_owned).collect();
        let mut crumbs = vec![(self.bucket.clone(), String::new())];
        let mut walked = String::new();
        for part in &parts {
            walked.push_str(part);
            walked.push('/');
            crumbs.push((part.clone(), walked.clone()));
        }
        let last = crumbs.len() - 1;
        let mut open: Option<String> = None;
        for (index, (label, prefix)) in crumbs.into_iter().enumerate() {
            if index > 0 {
                ui.label(widgets::muted(&theme, "/"));
            }
            let color = if index == last { theme.colors.crumb_current.0 } else { theme.colors.crumb.0 };
            if ui.add(egui::Button::new(RichText::new(label).strong().color(color)).frame(false)).clicked() {
                open = Some(prefix);
            }
        }
        if let Some(prefix) = open {
            self.open_crumb(prefix);
        }
    }

    fn table_ui(&mut self, ui: &mut Ui, visible: &[usize]) {
        let theme = self.theme.clone();
        let row_height = theme.shape.row_height;
        let mut clicked: Option<(usize, bool)> = None;
        let mut sort_by: Option<SortColumn> = None;
        let scroll_target = if std::mem::take(&mut self.scroll_to_current) {
            self.current_index().and_then(|index| visible.iter().position(|&v| v == index))
        } else {
            None
        };
        let mut table = TableBuilder::new(ui)
            .id_salt(("files", &self.bucket, &self.prefix))
            .striped(true)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::remainder().clip(true).at_least(160.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(130.0))
            .auto_shrink([false, false]);
        if let Some(position) = scroll_target {
            table = table.scroll_to_row(position, None);
        }
        let header_label = |column: SortColumn, text: String, sort: (SortColumn, bool)| {
            let arrow = if sort.0 == column { if sort.1 { " ▲" } else { " ▼" } } else { "" };
            RichText::new(format!("{text}{arrow}")).strong().color(theme.colors.header_text.0)
        };
        let sort = self.sort;
        table
            .header(row_height, |mut header| {
                for (column, text) in [
                    (SortColumn::Name, tr!("col_name")),
                    (SortColumn::Size, tr!("col_size")),
                    (SortColumn::Modified, tr!("col_modified")),
                ] {
                    header.col(|ui| {
                        if ui.add(egui::Button::new(header_label(column, text, sort)).frame(false)).clicked() {
                            sort_by = Some(column);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(row_height, visible.len(), |mut row| {
                    let index = visible[row.index()];
                    let entry = self.rows[index].entry.as_ref();
                    let key = entry.map(|entry| entry.key.as_str()).unwrap_or("");
                    let selected = !key.is_empty() && self.selected.contains(key);
                    row.set_selected(selected);
                    let text_color = if selected { theme.colors.selection_text.0 } else { theme.colors.text.0 };
                    match entry {
                        None => {
                            row.col(|ui| {
                                ui.label(RichText::new("⬆  ..").color(text_color));
                            });
                            row.col(|_| {});
                            row.col(|_| {});
                        }
                        Some(entry) => {
                            let icon = if entry.is_folder { "📁" } else { "🗋" };
                            row.col(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!("{icon}  {}", entry.name)).color(text_color),
                                    )
                                    .truncate()
                                    .selectable(false),
                                );
                            });
                            row.col(|ui| {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if !entry.is_folder {
                                        ui.label(RichText::new(format_size(entry.size)).color(text_color));
                                    }
                                });
                            });
                            row.col(|ui| {
                                if !entry.is_folder {
                                    ui.label(RichText::new(format_dt(entry.last_modified)).color(text_color));
                                }
                            });
                        }
                    }
                    let tip = entry.map(|entry| entry.key.clone()).unwrap_or_else(|| tr!("parent_folder"));
                    let response = row.response().on_hover_text(tip);
                    if response.double_clicked() {
                        clicked = Some((index, true));
                    } else if response.clicked() {
                        clicked = Some((index, false));
                    }
                });
            });
        if let Some(column) = sort_by {
            self.sort = if self.sort.0 == column { (column, !self.sort.1) } else { (column, true) };
            self.sort_rows();
        }
        if let Some((index, double)) = clicked {
            self.click_row(ui.ctx(), index, visible, double);
        }
    }

    /// A click selects and previews; a double-click opens folders (and goes up on "..").
    fn click_row(&mut self, ctx: &egui::Context, index: usize, visible: &[usize], double: bool) {
        if double {
            self.activate(index, true);
            return;
        }
        let modifiers = ctx.input(|input| input.modifiers);
        let Some(key) = self.row_key(index) else { return };
        if modifiers.shift
            && let Some(anchor) = self.anchor.clone()
        {
            let position = |key: &str| visible.iter().position(|&i| self.row_key(i).as_deref() == Some(key));
            if let (Some(a), Some(b)) = (position(&anchor), position(&key)) {
                let (from, to) = (a.min(b), a.max(b));
                self.selected =
                    visible[from..=to].iter().filter_map(|&i| self.row_key(i)).filter(|k| !k.is_empty()).collect();
            }
        } else if modifiers.command {
            if !self.selected.remove(&key) && !key.is_empty() {
                self.selected.insert(key.clone());
            }
            self.anchor = Some(key.clone());
        } else {
            self.selected = if key.is_empty() { Default::default() } else { [key.clone()].into() };
            self.anchor = Some(key.clone());
        }
        self.current = Some(key);
        self.activate(index, false);
    }

    pub(super) fn status_ui(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        let mut cancel = false;
        if let Some(transfer) = &self.transfer {
            widgets::card(&theme).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let label = match transfer.kind {
                        TransferKind::Download => tr!("transfer_download"),
                        TransferKind::Upload => tr!("transfer_upload"),
                        TransferKind::Delete => tr!("transfer_delete"),
                    };
                    ui.label(RichText::new(label).strong());
                    let (done, total) = (transfer.progress.done(), transfer.progress.total());
                    let amount = if total > 0 && transfer.kind == TransferKind::Delete {
                        format!("{} / {total}", done.min(total))
                    } else if total > 0 {
                        format!("{} / {}", format_size(Some(done.min(total))), format_size(Some(total)))
                    } else {
                        String::new()
                    };
                    let button_width = 110.0;
                    let bar_width = (ui.available_width() - button_width - 150.0).max(80.0);
                    let bar = if total > 0 {
                        egui::ProgressBar::new(done.min(total) as f32 / total as f32).show_percentage()
                    } else {
                        egui::ProgressBar::new(0.0).animate(true)
                    };
                    ui.add(bar.desired_width(bar_width).fill(theme.colors.accent.0)).on_hover_text(&amount);
                    ui.add_sized([140.0, 20.0], egui::Label::new(widgets::muted(&theme, &amount)));
                    let enabled = !transfer.cancel.is_cancelled();
                    cancel = ui
                        .add_enabled(enabled, egui::Button::new(RichText::new(tr!("cancel_download")).strong()))
                        .clicked();
                });
            });
            ui.add_space(4.0);
        }
        if cancel {
            self.cancel_transfer();
        }
        ui.horizontal(|ui| {
            ui.label(&self.status);
            if self.busy > 0 && self.transfer.is_none() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add(egui::Spinner::new().color(widgets::spinner_color(&theme)));
                });
            }
        });
    }
}
