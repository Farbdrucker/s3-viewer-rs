//! Questions and name prompts (saving a connection, creating a folder).

use eframe::egui::{self, Key, RichText};

use super::{App, widgets};
use crate::s3::{ConnectionSettings, ObjectEntry};
use crate::theme::Theme;
use crate::tr;
use crate::transfer::UploadPlan;

pub enum Confirmed {
    LoadPreview(ObjectEntry),
    StopEditing,
    Upload(UploadPlan),
    ReplaceProfile(String, ConnectionSettings),
    RemoveProfile(String),
    Delete(Vec<ObjectEntry>),
}

pub enum Dialog {
    Confirm { title: String, body: String, action: Confirmed },
    SaveProfile { name: String, settings: ConnectionSettings, error: Option<String> },
    NewFolder { name: String, error: Option<String> },
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
            Dialog::SaveProfile { mut name, settings, error } => {
                let title = tr!("save_connection_title");
                let label = tr!("save_connection_label");
                match name_prompt(ctx, &theme, "save-profile", &title, &label, &tr!("save_profile"), &mut name, &error)
                {
                    Answer::Open => self.dialog = Some(Dialog::SaveProfile { name, settings, error }),
                    Answer::No => {}
                    Answer::Yes => {
                        if let Err(message) = self.store_profile(&name, &settings, false) {
                            self.dialog = Some(Dialog::SaveProfile { name, settings, error: Some(message) });
                        }
                    }
                }
            }
            Dialog::NewFolder { mut name, error } => {
                let title = tr!("new_folder");
                let label = tr!("new_folder_label");
                match name_prompt(ctx, &theme, "new-folder", &title, &label, &tr!("ok"), &mut name, &error) {
                    Answer::Open => self.dialog = Some(Dialog::NewFolder { name, error }),
                    Answer::No => {}
                    Answer::Yes => {
                        if let Err(message) = self.create_folder(&name) {
                            self.dialog = Some(Dialog::NewFolder { name, error: Some(message) });
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
            Confirmed::Delete(entries) => self.start_delete(entries),
        }
    }
}

/// A modal asking for a name. Problems from the last attempt show below the field.
#[allow(clippy::too_many_arguments)]
fn name_prompt(
    ctx: &egui::Context,
    theme: &Theme,
    id: &str,
    title: &str,
    label: &str,
    accept: &str,
    name: &mut String,
    error: &Option<String>,
) -> Answer {
    let mut answer = Answer::Open;
    let response = egui::Modal::new(egui::Id::unique(id)).show(ctx, |ui| {
        ui.set_width(420.0);
        widgets::section(ui, theme, title);
        ui.add_space(6.0);
        ui.label(label);
        let field = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
        if !field.has_focus() && !field.lost_focus() {
            field.request_focus();
        }
        if let Some(error) = error {
            ui.label(RichText::new(error).color(theme.colors.banner_text.0));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            if widgets::primary_button(ui, theme, accept).clicked() || enter {
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
    answer
}
