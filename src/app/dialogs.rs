//! Questions and the name prompt for saving a connection.

use eframe::egui::{self, Key, RichText};

use super::{App, widgets};
use crate::s3::{ConnectionSettings, ObjectEntry};
use crate::tr;
use crate::transfer::UploadPlan;

pub enum Confirmed {
    LoadPreview(ObjectEntry),
    StopEditing,
    Upload(UploadPlan),
    ReplaceProfile(String, ConnectionSettings),
    RemoveProfile(String),
}

pub enum Dialog {
    Confirm { title: String, body: String, action: Confirmed },
    SaveProfile { name: String, settings: ConnectionSettings, error: Option<String> },
}

enum Answer {
    Open,
    Yes,
    No,
}

impl App {
    pub(super) fn dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else { return };
        let theme = self.theme.clone();
        match dialog {
            Dialog::Confirm { title, body, action } => {
                let mut answer = Answer::Open;
                let response = egui::Modal::new(egui::Id::unique("confirm")).show(ctx, |ui| {
                    ui.set_width(440.0);
                    widgets::section(ui, &theme, &title);
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(&body).wrap());
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if widgets::primary_button(ui, &theme, &tr!("yes")).clicked()
                            || ui.input(|i| i.key_pressed(Key::Enter))
                        {
                            answer = Answer::Yes;
                        }
                        if ui.button(tr!("no")).clicked() {
                            answer = Answer::No;
                        }
                    });
                });
                if response.should_close() && matches!(answer, Answer::Open) {
                    answer = Answer::No;
                }
                match answer {
                    Answer::Open => self.dialog = Some(Dialog::Confirm { title, body, action }),
                    Answer::Yes => self.confirmed(action),
                    Answer::No => {
                        if matches!(action, Confirmed::LoadPreview(_)) {
                            self.reselect_preview_entry();
                        }
                    }
                }
            }
            Dialog::SaveProfile { mut name, settings, mut error } => {
                let mut answer = Answer::Open;
                let response = egui::Modal::new(egui::Id::unique("save-profile")).show(ctx, |ui| {
                    ui.set_width(420.0);
                    widgets::section(ui, &theme, tr!("save_connection_title"));
                    ui.add_space(6.0);
                    ui.label(tr!("save_connection_label"));
                    let field = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY));
                    if !field.has_focus() && !field.lost_focus() {
                        field.request_focus();
                    }
                    if let Some(error) = &error {
                        ui.label(RichText::new(error).color(theme.colors.banner_text.0));
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let enter = ui.input(|i| i.key_pressed(Key::Enter));
                        if widgets::primary_button(ui, &theme, &tr!("save_profile")).clicked() || enter {
                            answer = Answer::Yes;
                        }
                        if ui.button(tr!("cancel")).clicked() {
                            answer = Answer::No;
                        }
                    });
                });
                if response.should_close() && matches!(answer, Answer::Open) {
                    answer = Answer::No;
                }
                match answer {
                    Answer::Open => self.dialog = Some(Dialog::SaveProfile { name, settings, error }),
                    Answer::No => {}
                    Answer::Yes => {
                        if let Err(message) = self.store_profile(&name, &settings, false) {
                            error = Some(message);
                            self.dialog = Some(Dialog::SaveProfile { name, settings, error });
                        }
                    }
                }
            }
        }
    }

    fn confirmed(&mut self, action: Confirmed) {
        match action {
            Confirmed::LoadPreview(entry) => self.load_preview(entry, true),
            Confirmed::StopEditing => self.preview.editing = false,
            Confirmed::Upload(plan) => self.start_upload(plan),
            Confirmed::ReplaceProfile(name, settings) => {
                if let Err(message) = self.store_profile(&name, &settings, true) {
                    self.set_banner(Some(message));
                }
            }
            Confirmed::RemoveProfile(name) => self.remove_profile(&name),
        }
    }
}
