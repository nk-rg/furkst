//! Floating windows for file history, blame and the reflog.

use egui::{RichText, Sense, Ui, Vec2};
use furkst_core::{BlameResult, CommitInfo, Oid, ReflogEntry, RefsSnapshot, Signature};

use super::diff_view::DiffPanel;
use super::{Action, Loaded};
use crate::settings::Settings;
use crate::widgets;

/// The reflog window's key: it always shows the reflog of `HEAD`.
const REFLOG: &str = "HEAD";

/// A window whose content loads in the background.
struct Window<T> {
    path: String,
    open: bool,
    content: Option<Loaded<T>>,
}

impl<T> Window<T> {
    fn new(path: &str) -> Self {
        Self { path: path.to_owned(), open: true, content: None }
    }

    fn loaded(window: &mut Option<Self>, path: &str, result: Loaded<T>) {
        if let Some(window) = window.as_mut().filter(|w| w.path == path) {
            window.content = Some(result);
        }
    }

    fn show(slot: &mut Option<Self>, ctx: &egui::Context, title: &str, size: [f32; 2], body: impl FnOnce(&mut Ui, &T)) {
        let Some(window) = slot else { return };
        egui::Window::new(format!("{title} — {}", window.path))
            .id(egui::Id::new(title))
            .open(&mut window.open)
            .default_size(size)
            .show(ctx, |ui| match &window.content {
                None => widgets::placeholder(ui, "Loading…"),
                Some(Err(err)) => widgets::placeholder(ui, err),
                Some(Ok(content)) => body(ui, content),
            });
        if !window.open {
            *slot = None;
        }
    }
}

#[derive(Default)]
pub struct Tools {
    file_history: Option<Window<Vec<CommitInfo>>>,
    /// The file history's selected commit, whose change to the file `version_diff` shows.
    version: Option<Oid>,
    version_diff: DiffPanel,
    blame: Option<Window<BlameResult>>,
    reflog: Option<Window<Vec<ReflogEntry>>>,
}

impl Tools {
    pub fn open_file_history(&mut self, path: &str) {
        self.file_history = Some(Window::new(path));
        self.version = None;
        self.version_diff.clear();
    }

    /// The file the history window shows, while it is open.
    pub fn file_history_path(&self) -> Option<String> {
        self.file_history.as_ref().map(|w| w.path.clone())
    }

    /// The newest commit in the loaded file history, to show first.
    pub fn newest_version(&self) -> Option<Oid> {
        match &self.file_history.as_ref()?.content {
            Some(Ok(commits)) => commits.first().map(|c| c.id),
            _ => None,
        }
    }

    pub fn select_version(&mut self, id: Oid) {
        self.version = Some(id);
    }

    pub fn version_diff(&mut self) -> &mut DiffPanel {
        &mut self.version_diff
    }

    pub fn open_blame(&mut self, path: &str) {
        self.blame = Some(Window::new(path));
    }

    pub fn file_history_loaded(&mut self, path: &str, result: Loaded<Vec<CommitInfo>>) {
        Window::loaded(&mut self.file_history, path, result);
    }

    pub fn blame_loaded(&mut self, path: &str, result: Loaded<BlameResult>) {
        Window::loaded(&mut self.blame, path, result);
    }

    pub fn open_reflog(&mut self) {
        self.reflog = Some(Window::new(REFLOG));
    }

    pub fn reflog_loaded(&mut self, result: Loaded<Vec<ReflogEntry>>) {
        Window::loaded(&mut self.reflog, REFLOG, result);
    }
}

pub fn show(ctx: &egui::Context, tools: &mut Tools, refs: Option<&RefsSnapshot>, settings: &mut Settings, actions: &mut Vec<Action>) {
    let (version, version_diff) = (tools.version, &mut tools.version_diff);
    Window::show(&mut tools.file_history, ctx, "File history", [960.0, 560.0], |ui, commits| {
        if commits.is_empty() {
            return widgets::placeholder(ui, "No commits touch this file");
        }
        // The commits that changed the file, and how the selected one changed it.
        widgets::left_panel(ui, "file-history-commits", 340.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for commit in commits {
                    let response = commit_row(ui, commit, None, &commit.author, version == Some(commit.id), refs, actions);
                    if response.double_clicked() {
                        actions.push(Action::SelectCommit(commit.id));
                    } else if response.clicked() {
                        actions.push(Action::SelectVersion(commit.id));
                    }
                    response.on_hover_text("Double-click to show it in the history");
                }
            });
        });
        egui::CentralPanel::no_frame().show(ui, |ui| version_diff.show(ui, settings, Action::ShowVersionDiff, None, actions));
    });

    Window::show(&mut tools.reflog, ctx, "Reflog", [640.0, 480.0], |ui, entries| {
        if entries.is_empty() {
            return widgets::placeholder(ui, "The reflog is empty");
        }
        ui.label(RichText::new("Everywhere HEAD has been. Right-click an entry to reset or branch from it.").weak());
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for entry in entries {
                if commit_row(ui, &entry.commit, Some(&entry.action), &entry.when, false, refs, actions).clicked() {
                    actions.push(Action::SelectCommit(entry.commit.id));
                }
            }
        });
    });

    Window::show(&mut tools.blame, ctx, "Blame", [640.0, 480.0], |ui, blame| {
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::both().auto_shrink(false).show_rows(ui, row_height, blame.lines.len(), |ui, range| {
            for index in range {
                let line = &blame.lines[index];
                let starts_block = index == 0 || blame.lines[index - 1].commit != line.commit;
                ui.horizontal(|ui| {
                    ui.set_min_height(row_height);
                    let (rect, response) = ui.allocate_exact_size(Vec2::new(240.0, row_height), Sense::click());
                    if starts_block && let Some(commit) = blame.commits.get(&line.commit) {
                        let text = format!("{} {} {}", commit.short_id(), commit.author.format_time("%Y-%m-%d"), commit.author.name);
                        ui.painter().text(
                            rect.left_center(),
                            egui::Align2::LEFT_CENTER,
                            text,
                            egui::FontId::monospace(11.0),
                            crate::theme::hashed(&commit.id.to_string()),
                        );
                        if response.on_hover_text(&commit.summary).clicked() {
                            actions.push(Action::SelectCommit(commit.id));
                        }
                    }
                    ui.label(RichText::new(format!("{:>5}  {}", line.line_no, line.text)).monospace());
                });
            }
        });
    });
}

/// A clickable commit line: avatar, short SHA, an optional note (e.g. the reflog action), summary and date, highlighted
/// when `selected`. Right-clicking opens the usual commit menu; the caller decides what a click does.
fn commit_row(
    ui: &mut Ui,
    commit: &CommitInfo,
    note: Option<&str>,
    when: &Signature,
    selected: bool,
    refs: Option<&RefsSnapshot>,
    actions: &mut Vec<Action>,
) -> egui::Response {
    let frame = egui::Frame::new().fill(if selected { ui.visuals().selection.bg_fill } else { egui::Color32::TRANSPARENT });
    let response = frame
        .show(ui, |ui| {
            ui.horizontal(|ui| {
            widgets::avatar(ui, &commit.author, &commit.committer, 20.0);
            ui.label(RichText::new(commit.short_id()).monospace().weak());
            if let Some(note) = note {
                ui.label(RichText::new(note).strong());
            }
            ui.add(egui::Label::new(&commit.summary).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(widgets::format_date(when)).weak());
                });
            });
        })
        .response
        .interact(Sense::click());
    if let Some(refs) = refs {
        response.context_menu(|ui| super::menus::commit(ui, commit, refs, actions));
    }
    response
}
