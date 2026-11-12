//! The connection card at the top: saved connections, credentials, options.

use eframe::egui::{self, Align, Key, Layout, RichText, Ui};

use super::{App, REGION_CHOICES, widgets};
use crate::i18n::{self, Language};
use crate::s3::SignatureVersion;
use crate::tr;

const LABEL_WIDTH: f32 = 150.0;

impl App {
    pub(super) fn connection_ui(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        let collapsed = self.settings.connection_collapsed;
        widgets::card(&theme).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (arrow, tip) = if collapsed { ("▼", tr!("fold_expand")) } else { ("▲", tr!("fold_collapse")) };
                if ui.button(RichText::new(arrow).strong()).on_hover_text(tip).clicked() {
                    self.settings.connection_collapsed = !collapsed;
                    self.save_settings();
                }
                widgets::section(ui, &theme, tr!("connection"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if collapsed {
                        ui.label(widgets::muted(&theme, self.connection_summary()));
                    }
                    self.theme_menu(ui);
                    let language = if i18n::language() == Language::En { "EN" } else { "DE" };
                    if ui.button(RichText::new(language).strong()).on_hover_text(tr!("language")).clicked() {
                        self.toggle_language();
                    }
                    if collapsed {
                        let connected = self.connected();
                        if widgets::icon_button(ui, &theme, "⏏", &tr!("disconnect"), connected && !self.connecting)
                            .clicked()
                        {
                            self.disconnect();
                        }
                        let tip = if self.connecting { tr!("connecting") } else { tr!("connect") };
                        if widgets::icon_button(ui, &theme, "🔌", &tip, !self.connecting).clicked() {
                            self.connect();
                        }
                    }
                    self.profile_combo(ui);
                });
            });
            if collapsed {
                return;
            }
            ui.horizontal(|ui| {
                if widgets::text_button(ui, &theme, "💾", &tr!("save_profile"), &tr!("save_profile_tip"), true)
                    .clicked()
                {
                    self.save_named_profile();
                }
                let has_profile = !self.profile.is_empty();
                if widgets::text_button(
                    ui,
                    &theme,
                    "🗑",
                    &tr!("delete_profile"),
                    &tr!("delete_profile_tip"),
                    has_profile,
                )
                .clicked()
                {
                    self.delete_named_profile();
                }
                let label = if self.connecting { tr!("connecting") } else { tr!("connect") };
                if widgets::text_button(ui, &theme, "🔌", &label, &label, !self.connecting).clicked() {
                    self.connect();
                }
                let connected = self.connected();
                if widgets::text_button(
                    ui,
                    &theme,
                    "⏏",
                    &tr!("disconnect"),
                    &tr!("disconnect"),
                    connected && !self.connecting,
                )
                .clicked()
                {
                    self.disconnect();
                }
            });
            ui.add_space(4.0);
            self.connection_fields(ui);
            ui.add_space(2.0);
            ui.add(egui::Label::new(widgets::muted(&theme, tr!("hint"))).wrap());
        });
    }

    fn connection_summary(&self) -> String {
        let mut text = self.form.endpoint.trim().to_owned();
        if text.contains("://") {
            let host = crate::s3::host_of(&text);
            if !host.is_empty() {
                text = host;
            }
        }
        match (self.profile.is_empty(), text.is_empty()) {
            (_, true) => tr!("summary_empty"),
            (false, false) => format!("{} · {text}", self.profile),
            (true, false) => text,
        }
    }

    fn profile_combo(&mut self, ui: &mut Ui) {
        let selected = if self.profile.is_empty() { tr!("saved_connection") } else { self.profile.clone() };
        let mut chosen: Option<String> = None;
        egui::ComboBox::from_id_salt("profile")
            .selected_text(selected)
            .width(ui.available_width().clamp(220.0, 420.0))
            .show_ui(ui, |ui| {
                if ui.selectable_label(self.profile.is_empty(), tr!("saved_connection")).clicked() {
                    chosen = Some(String::new());
                }
                for name in &self.profiles {
                    if ui.selectable_label(*name == self.profile, name).clicked() {
                        chosen = Some(name.clone());
                    }
                }
            })
            .response
            .on_hover_text(tr!("profile_tip"));
        if let Some(name) = chosen {
            self.select_profile(name);
        }
    }

    fn theme_menu(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let current = self
            .themes
            .entries()
            .iter()
            .find(|entry| entry.id == self.settings.theme)
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| self.settings.theme.clone());
        let mut chosen: Option<String> = None;
        let mut export = false;
        let mut open_folder = false;
        egui::ComboBox::from_id_salt("theme")
            .selected_text(format!("🎨 {current}"))
            .width(170.0)
            .show_ui(ui, |ui| {
                for entry in self.themes.entries() {
                    let label = if entry.user { format!("{} ✎", entry.name) } else { entry.name.clone() };
                    if ui.selectable_label(entry.id == self.settings.theme, label).clicked() {
                        chosen = Some(entry.id.clone());
                    }
                }
                ui.separator();
                export = ui.button(format!("💾 {}", tr!("theme_export"))).clicked();
                open_folder = ui.button(format!("📂 {}", tr!("theme_folder"))).clicked();
            })
            .response
            .on_hover_text(tr!("theme_tip", path = self.themes.dir().display()));
        if let Some(id) = chosen {
            self.set_theme(&ctx, &id);
            self.save_settings();
        }
        if export {
            self.export_theme(&ctx);
        }
        if open_folder {
            self.open_theme_folder(&ctx);
        }
    }

    fn connection_fields(&mut self, ui: &mut Ui) {
        let mut submit = false;
        let enter = |ui: &Ui, response: &egui::Response| {
            response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter))
        };
        let spacing = ui.spacing().item_spacing.x;

        ui.horizontal(|ui| {
            widgets::field_label(ui, tr!("endpoint"), LABEL_WIDTH);
            let region_part = 70.0 + 150.0 + 30.0 + spacing * 4.0;
            let width = (ui.available_width() - region_part).max(160.0);
            let response = ui
                .add(
                    egui::TextEdit::singleline(&mut self.form.endpoint)
                        .hint_text(tr!("endpoint_ph"))
                        .desired_width(width),
                )
                .on_hover_text(tr!("endpoint_tip"));
            submit |= enter(ui, &response);
            widgets::field_label(ui, tr!("region"), 70.0);
            let response = ui
                .add(egui::TextEdit::singleline(&mut self.form.region).desired_width(150.0))
                .on_hover_text(tr!("region_tip"));
            submit |= enter(ui, &response);
            ui.menu_button("⏷", |ui| {
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for region in REGION_CHOICES {
                        if ui.selectable_label(self.form.region == *region, *region).clicked() {
                            self.form.region = (*region).to_owned();
                            ui.close();
                        }
                    }
                });
            })
            .response
            .on_hover_text(tr!("region_tip"));
        });

        ui.horizontal(|ui| {
            widgets::field_label(ui, tr!("access_key"), LABEL_WIDTH);
            let half = ((ui.available_width() - LABEL_WIDTH - 40.0 - spacing * 4.0) / 2.0).max(140.0);
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.form.access_key).hint_text(tr!("access_ph")).desired_width(half),
            );
            submit |= enter(ui, &response);
            widgets::field_label(ui, tr!("secret_key"), LABEL_WIDTH);
            let response = ui
                .add(
                    egui::TextEdit::singleline(&mut self.form.secret_key)
                        .hint_text(tr!("secret_ph"))
                        .password(!self.form.show_secret)
                        .desired_width(half),
                )
                .on_hover_text(tr!("secret_tip"));
            submit |= enter(ui, &response);
            let (icon, tip) =
                if self.form.show_secret { ("🙈", tr!("hide_secret")) } else { ("👁", tr!("show_secret")) };
            if ui.selectable_label(self.form.show_secret, icon).on_hover_text(tip).clicked() {
                self.form.show_secret = !self.form.show_secret;
            }
        });

        ui.horizontal(|ui| {
            widgets::field_label(ui, tr!("bucket"), LABEL_WIDTH);
            let response = ui
                .add(egui::TextEdit::singleline(&mut self.form.bucket).hint_text(tr!("bucket_ph")).desired_width(180.0))
                .on_hover_text(tr!("bucket_tip"));
            submit |= enter(ui, &response);
            ui.add_space(12.0);
            ui.label(tr!("signature"));
            egui::ComboBox::from_id_salt("signature")
                .selected_text(self.form.signature.label())
                .show_ui(ui, |ui| {
                    for version in [SignatureVersion::V4, SignatureVersion::V2] {
                        ui.selectable_value(&mut self.form.signature, version, version.label());
                    }
                })
                .response
                .on_hover_text(tr!("signature_tip"));
            ui.add_space(12.0);
            ui.checkbox(&mut self.form.path_style, tr!("path_style")).on_hover_text(tr!("path_tip"));
            ui.checkbox(&mut self.form.verify_ssl, tr!("verify_tls")).on_hover_text(tr!("verify_tip"));
        });

        if submit && !self.connecting {
            self.connect();
        }
    }
}
