//! The window: repository tabs and the start page.

use std::path::{Path, PathBuf};

use egui::{Key, RichText, Ui};
use furkst_core::Repo;

use crate::palette::{self, Item, Palette};
use crate::preferences::Preferences;
use crate::settings::Settings;
use crate::tab::RepoTab;
use crate::repositories;
use crate::tasks::{Progress, Tasks};
use crate::{theme, widgets};

pub struct FurkstApp {
    settings: Settings,
    tabs: Vec<RepoTab>,
    /// `None` shows the start page.
    active: Option<usize>,
    start: StartPage,
    focused: bool,
    /// Quick Launch on the start page, where no repository tab provides its own.
    palette: Option<Palette<StartEntry>>,
    preferences: Preferences,
}

enum StartEntry {
    Open,
    New,
    Clone,
    Recent(PathBuf),
}

const CLONE_URL_ID: &str = "clone-url";

struct StartPage {
    clone_url: String,
    clone_parent: String,
    error: Option<String>,
    cloning: Tasks<Result<Repo, String>>,
    clone_progress: Progress,
    repositories: repositories::RepositoryList,
}

impl FurkstApp {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: Settings, paths: &[PathBuf]) -> Self {
        theme::apply(&cc.egui_ctx, settings.theme);
        crate::fonts::apply_in_background(&cc.egui_ctx, settings.interface_font.clone(), settings.code_font.clone());
        furkst_core::cli::set_git_executable(settings.git_path.clone());
        let clone_parent = settings
            .clone_directory
            .clone()
            .or_else(|| directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()))
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let mut app = Self {
            start: StartPage {
                clone_url: String::new(),
                clone_parent,
                error: None,
                cloning: Tasks::new(&cc.egui_ctx),
                clone_progress: Progress::new(&cc.egui_ctx),
                repositories: Default::default(),
            },
            tabs: Vec::new(),
            active: None,
            focused: true,
            palette: None,
            preferences: Preferences::new(&cc.egui_ctx),
            settings,
        };
        for path in app.settings.open_tabs.clone() {
            if let Ok(repo) = Repo::open(&path) {
                app.tabs.push(RepoTab::new(&cc.egui_ctx, repo));
            }
        }
        if !app.tabs.is_empty() {
            app.active = Some(app.settings.active_tab.min(app.tabs.len() - 1));
        }
        for path in paths {
            app.open(&cc.egui_ctx, path, false);
        }
        app
    }

    fn add_tab(&mut self, ctx: &egui::Context, repo: Repo) {
        let path = repo.workdir().to_path_buf();
        if let Some(existing) = self.tabs.iter().position(|t| t.repo.workdir() == path) {
            self.active = Some(existing);
        } else {
            self.tabs.push(RepoTab::new(ctx, repo));
            self.active = Some(self.tabs.len() - 1);
        }
        self.settings.remember(path);
        self.start.error = None;
        self.persist();
    }

    fn open(&mut self, ctx: &egui::Context, path: &Path, create: bool) {
        let result = if create { Repo::init(path) } else { Repo::open(path) };
        match result {
            Ok(repo) => self.add_tab(ctx, repo),
            Err(err) => self.start.error = Some(format!("{}: {err}", path.display())),
        }
    }

    /// Asks for a folder and opens (or, with `create`, initializes) a repository there.
    fn pick_repository(&mut self, ctx: &egui::Context, create: bool) {
        let title = if create { "Create repository in" } else { "Open repository" };
        if let Some(path) = rfd::FileDialog::new().set_title(title).pick_folder() {
            self.open(ctx, &path, create);
        }
    }

    fn close_tab(&mut self, index: usize) {
        self.tabs.remove(index);
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(active) if active >= index => Some(active.saturating_sub(1)),
            other => other,
        };
        self.persist();
    }

    fn persist(&mut self) {
        self.settings.open_tabs = self.tabs.iter().map(|t| t.repo.workdir().to_path_buf()).collect();
        self.settings.active_tab = self.active.unwrap_or(0);
        self.settings.save();
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let (open, new_tab, close, refresh, next) = ctx.input_mut(|i| {
            let cmd = egui::Modifiers::COMMAND;
            (
                i.consume_key(cmd, Key::O),
                i.consume_key(cmd, Key::T),
                i.consume_key(cmd, Key::W),
                i.consume_key(egui::Modifiers::NONE, Key::F5),
                i.consume_key(egui::Modifiers::CTRL, Key::Tab),
            )
        });
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::Comma)) {
            self.preferences.toggle();
        }
        if open {
            self.pick_repository(ctx, false);
        }
        if new_tab {
            self.active = None;
        }
        if close && let Some(active) = self.active {
            self.close_tab(active);
        }
        if refresh && let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) {
            tab.refresh();
        }
        if next && !self.tabs.is_empty() {
            self.active = Some(self.active.map_or(0, |i| (i + 1) % self.tabs.len()));
        }
    }

    fn tab_bar(&mut self, ui: &mut Ui) {
        let mut close = None;
        ui.horizontal(|ui| {
            for (index, tab) in self.tabs.iter().enumerate() {
                let selected = self.active == Some(index);
                let mut title = RichText::new(tab.title());
                if selected {
                    title = title.strong();
                }
                let response = ui.add(egui::Button::selectable(selected, title)).on_hover_text(tab.repo.workdir().display().to_string());
                if response.clicked() {
                    self.active = Some(index);
                }
                if response.middle_clicked() || (selected && ui.small_button("×").on_hover_text("Close tab").clicked()) {
                    close = Some(index);
                }
                ui.separator();
            }
            if ui.add(egui::Button::selectable(self.active.is_none(), "+")).on_hover_text("Open another repository").clicked() {
                self.active = None;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙").on_hover_text("Preferences (Ctrl+,)").clicked() {
                    self.preferences.toggle();
                }
            });
        });
        if let Some(index) = close {
            self.close_tab(index);
        }
    }

    fn start_page(&mut self, ui: &mut Ui) {
        for result in self.start.cloning.poll() {
            self.start.clone_progress.clear();
            match result {
                Ok(repo) => self.add_tab(ui.ctx(), repo),
                Err(err) => self.start.error = Some(err),
            }
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.set_max_width(560.0);
                ui.add_space(40.0);
                ui.heading(RichText::new("Furkst").size(30.0).strong());
                ui.label(RichText::new("A git client powered by gitoxide").weak());
                ui.add_space(24.0);

                ui.horizontal(|ui| {
                    if ui.button("📂  Open repository…").clicked() {
                        self.pick_repository(ui.ctx(), false);
                    }
                    if ui.button("✨  New repository…").clicked() {
                        self.pick_repository(ui.ctx(), true);
                    }
                });
                ui.add_space(16.0);
                self.clone_form(ui);

                if let Some(error) = &self.start.error {
                    ui.add_space(8.0);
                    ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
                }

                ui.add_space(24.0);
                if let Some(path) = repositories::show(ui, &mut self.start.repositories, &mut self.settings) {
                    self.open(ui.ctx(), &path, false);
                }
            });
        });
    }

    /// Ctrl/Cmd+P on the start page: recent repositories plus Open/New/Clone.
    fn start_palette(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::P)) {
            self.palette = match self.palette {
                Some(_) => None,
                None => {
                    let mut items = vec![
                        Item::new("Open repository…", StartEntry::Open).detail("command"),
                        Item::new("New repository…", StartEntry::New).detail("command"),
                        Item::new("Clone repository…", StartEntry::Clone).detail("command"),
                    ];
                    items.extend(
                        self.settings.known_repositories().into_iter().map(|path| palette::open_repo_item(path, StartEntry::Recent(path.clone()))),
                    );
                    Some(Palette::new("start-quick-launch", "Type a command or repository…", items))
                }
            };
        }
        let Some(palette) = &mut self.palette else { return };
        match palette.show(ctx) {
            None => {}
            Some(palette::Event::Close) => self.palette = None,
            Some(palette::Event::Run(entry)) => {
                self.palette = None;
                match entry {
                    StartEntry::Open => self.pick_repository(ctx, false),
                    StartEntry::New => self.pick_repository(ctx, true),
                    StartEntry::Clone => ctx.memory_mut(|m| m.request_focus(egui::Id::new(CLONE_URL_ID))),
                    StartEntry::Recent(path) => self.open(ctx, &path, false),
                }
            }
        }
    }

    fn clone_form(&mut self, ui: &mut Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            widgets::section_label(ui, "Clone");
            ui.add(
                egui::TextEdit::singleline(&mut self.start.clone_url)
                    .id(egui::Id::new(CLONE_URL_ID))
                    .hint_text("Repository URL")
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.start.clone_parent).hint_text("Parent folder").desired_width(400.0));
                if ui.button("Browse…").clicked()
                    && let Some(dir) = rfd::FileDialog::new().pick_folder()
                {
                    self.start.clone_parent = dir.display().to_string();
                }
            });
            let url = self.start.clone_url.trim().to_owned();
            let name = clone_name(&url);
            let destination = PathBuf::from(&self.start.clone_parent).join(&name);
            if !name.is_empty() {
                ui.label(RichText::new(format!("Into {}", destination.display())).weak().small());
            }
            ui.horizontal(|ui| {
                let busy = self.start.cloning.is_busy();
                if ui.add_enabled(!busy && !name.is_empty(), egui::Button::new("Clone")).clicked() {
                    self.settings.clone_directory = Some(PathBuf::from(&self.start.clone_parent));
                    self.start.error = None;
                    let progress = self.start.clone_progress.clone();
                    self.start.cloning.spawn(move || {
                        Repo::clone_from(&url, &destination, &|line| progress.report(line)).map_err(|e| e.to_string())
                    });
                }
                if busy {
                    ui.spinner();
                    ui.label(self.start.clone_progress.current().unwrap_or_else(|| "Cloning…".to_owned()));
                }
            });
        });
    }
}

/// The directory name git would pick for a clone of `url`.
fn clone_name(url: &str) -> String {
    let last = url.trim_end_matches('/').rsplit(['/', ':', '\\']).next().unwrap_or_default();
    last.strip_suffix(".git").unwrap_or(last).to_owned()
}

impl eframe::App for FurkstApp {
    /// Frameless panels (like the commit list) show this color, so it must follow the theme; eframe's default
    /// is dark gray, which made them black in the light theme.
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);
        // A cheap flag, so the Preferences checkbox takes effect on the next frame.
        crate::avatars::set_enabled(self.settings.show_avatars);

        // Refresh when the window comes back into focus: files may have changed meanwhile.
        let focused = ctx.input(|i| i.focused);
        if focused && !self.focused
            && let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i))
        {
            tab.refresh();
        }
        self.focused = focused;

        egui::Panel::top("tabs").show(ui, |ui| self.tab_bar(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| match self.active.and_then(|i| self.tabs.get_mut(i)) {
            Some(tab) => tab.ui(ui, &mut self.settings),
            None => self.start_page(ui),
        });
        match self.active {
            Some(index) => {
                if let Some(path) = self.tabs.get_mut(index).and_then(RepoTab::take_open_request) {
                    self.open(&ctx, &path, false);
                }
            }
            None => self.start_palette(&ctx),
        }

        self.preferences.show(&ctx, &mut self.settings);

        if ctx.input(|i| i.viewport().close_requested()) {
            self.persist();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::clone_name;

    #[test]
    fn clone_names() {
        assert_eq!(clone_name("https://github.com/GitoxideLabs/gitoxide.git"), "gitoxide");
        assert_eq!(clone_name("git@github.com:user/repo.git"), "repo");
        assert_eq!(clone_name("https://example.com/a/b/"), "b");
        assert_eq!(clone_name(""), "");
    }
}
