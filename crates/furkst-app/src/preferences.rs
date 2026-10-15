//! The Preferences window: app-wide settings that save as soon as they change.

use std::path::PathBuf;

use egui::{Context, Grid, RichText, Ui};

use crate::settings::{Settings, ThemeChoice};
use crate::tasks::Tasks;
use crate::{theme, widgets};

pub struct Preferences {
    open: bool,
    /// The detected `git --version`, looked up each time the window opens.
    git_version: Option<String>,
    version_lookup: Tasks<String>,
}

impl Preferences {
    pub fn new(ctx: &Context) -> Self {
        Self { open: false, git_version: None, version_lookup: Tasks::new(ctx) }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.lookup_version();
        }
    }

    fn lookup_version(&mut self) {
        self.git_version = None;
        self.version_lookup.spawn(|| furkst_core::cli::git_version().unwrap_or_else(|err| err.to_string()));
    }

    /// Shows the window; every change is applied and saved immediately.
    pub fn show(&mut self, ctx: &Context, settings: &mut Settings) {
        if let Some(version) = self.version_lookup.poll().pop() {
            self.git_version = Some(version);
        }
        let mut open = self.open;
        let before = settings.clone();
        let mut git_path_changed = false;
        egui::Window::new("Preferences").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            widgets::section_label(ui, "General");
            Grid::new("prefs-general").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Theme");
                egui::ComboBox::from_id_salt("prefs-theme").selected_text(format!("{:?}", settings.theme)).show_ui(ui, |ui| {
                    for choice in [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark] {
                        ui.selectable_value(&mut settings.theme, choice, format!("{choice:?}"));
                    }
                });
                ui.end_row();

                ui.label("Default clone folder");
                path_row(ui, &mut settings.clone_directory, "Home folder", true);
                ui.end_row();

                ui.label("Git executable");
                git_path_changed = path_row(ui, &mut settings.git_path, "Automatic", false);
                ui.end_row();

                ui.label("");
                ui.label(RichText::new(self.git_version.as_deref().unwrap_or("Detecting git…")).weak().small());
                ui.end_row();
            });
            ui.checkbox(&mut settings.show_avatars, "Show avatars from GitHub").on_hover_text(
                "Shows the GitHub picture of authors known to be GitHub users: those with a GitHub no-reply address, \n                 and authors of commits made on GitHub, whose addresses are sent to GitHub to find them. Local \n                 commits keep their initials. Pictures are kept on this computer, so each is asked for once.",
            );

            ui.add_space(8.0);
            widgets::section_label(ui, "Git");
            ui.checkbox(&mut settings.fetch_prune, "Prune deleted remote branches when fetching");
            ui.checkbox(&mut settings.pull_rebase, "Rebase instead of merge when pulling");
            ui.horizontal(|ui| {
                ui.label("Auto-fetch every");
                ui.add(egui::DragValue::new(&mut settings.auto_fetch_minutes).range(0..=1440).suffix(" min"));
                ui.label(RichText::new("(0 = off)").weak());
            });

            ui.add_space(8.0);
            widgets::section_label(ui, "Fonts");
            Grid::new("prefs-fonts").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Interface");
                font_choice(ui, "prefs-interface-font", &mut settings.interface_font, |families| &families.all);
                ui.end_row();
                ui.label("Code");
                font_choice(ui, "prefs-code-font", &mut settings.code_font, |families| &families.monospaced);
                ui.end_row();
            });

            ui.add_space(8.0);
            widgets::section_label(ui, "Diff");
            ui.horizontal(|ui| {
                ui.label("Context lines");
                ui.add(egui::DragValue::new(&mut settings.diff_context).range(0..=50));
                ui.label("Font size");
                ui.add(egui::DragValue::new(&mut settings.diff_font_size).range(8.0..=32.0).speed(0.1).fixed_decimals(1));
            });
            ui.checkbox(&mut settings.ignore_whitespace, "Ignore whitespace changes");
            ui.checkbox(&mut settings.side_by_side, "Side by side");
        });
        self.open = open && !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

        if settings.theme != before.theme {
            theme::apply(ctx, settings.theme);
        }
        if (&settings.interface_font, &settings.code_font) != (&before.interface_font, &before.code_font) {
            crate::fonts::apply(ctx, settings.interface_font.as_deref(), settings.code_font.as_deref());
        }
        if git_path_changed {
            furkst_core::cli::set_git_executable(settings.git_path.clone());
            self.lookup_version();
        }
        if before != *settings {
            settings.save();
        }
    }
}

/// A path text box with Browse… and Clear; empty means "unset". Returns whether the value changed.
fn path_row(ui: &mut Ui, value: &mut Option<PathBuf>, hint: &str, folder: bool) -> bool {
    let mut text = value.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui.add(egui::TextEdit::singleline(&mut text).hint_text(hint).desired_width(260.0)).changed();
        if ui.button("Browse…").clicked() {
            let dialog = rfd::FileDialog::new();
            let picked = if folder { dialog.pick_folder() } else { dialog.pick_file() };
            if let Some(path) = picked {
                text = path.display().to_string();
                changed = true;
            }
        }
    });
    if changed {
        *value = (!text.trim().is_empty()).then(|| PathBuf::from(text.trim()));
    }
    changed
}

/// A font picker: "Built-in", or one of the installed font families `names` picks out (read when it is first opened).
fn font_choice(ui: &mut Ui, id: &str, choice: &mut Option<String>, names: fn(&crate::fonts::Families) -> &Vec<String>) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(choice.as_deref().unwrap_or("Built-in"))
        .width(240.0)
        .height(400.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(choice, None, "Built-in");
            for name in names(crate::fonts::families()) {
                ui.selectable_value(choice, Some(name.clone()), name);
            }
        });
}
