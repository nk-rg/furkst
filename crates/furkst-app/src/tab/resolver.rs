//! Merge conflicts: the prompt shown when an operation stops on them, and the Conflicts window, a window of its own
//! listing the conflicted files and resolving one at a time, conflict by conflict, in the words VS Code uses ("current"
//! for the checked-out side, "incoming" for the side being brought in).

use egui::{RichText, Ui};
use furkst_core::{
    ConflictRegion, FileChange, InProgress, Operation, Repo, Resolution, Segment, WorkingStatus, assemble, conflict_count,
};

use super::Action;
use crate::theme::DiffColors;
use crate::widgets;

/// Taking one side for a whole file: label, tooltip and whether it is the current side.
pub const WHOLE_FILE: [(&str, &str, bool); 2] = [
    ("Accept all current", "Keep the checked-out version of the whole file", true),
    ("Accept all incoming", "Take the version being brought in, for the whole file", false),
];

/// Resolves `change` by taking one side of it entirely.
pub fn take_side(change: &FileChange, ours: bool) -> Action {
    Action::Run(Operation::ResolveConflict { change: change.clone(), ours })
}

/// An operation that stopped because of conflicts, as the prompt describes it.
pub struct Stopped {
    /// What is in progress; `None` when nothing is (applying a stash, a squash merge), so there is nothing to abort.
    pub operation: Option<InProgress>,
    pub files: Vec<FileChange>,
}

impl Stopped {
    /// What a failed operation left behind, if it stopped on conflicts.
    pub fn after_failure(repo: &Repo) -> Option<Self> {
        let status = repo.status().ok()?;
        let files: Vec<FileChange> = status.conflicted().cloned().collect();
        (!files.is_empty()).then_some(Self { operation: status.operation, files })
    }
}

/// Asks what to do about the conflicts an operation stopped on: resolve them, abort, or decide later. Returns whether
/// the prompt is done.
pub fn prompt(ctx: &egui::Context, stopped: &Stopped, actions: &mut Vec<Action>) -> bool {
    let mut done = false;
    let what = stopped.operation.map_or("The changes", |op| match op {
        InProgress::Merge => "The merge",
        InProgress::Rebase => "The rebase",
        InProgress::CherryPick => "The cherry-pick",
        InProgress::Revert => "The revert",
        InProgress::Bisect | InProgress::ApplyMailbox => "The operation",
    });
    let response = egui::Modal::new(egui::Id::new("conflicts-prompt")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.heading("Conflicts");
        ui.add_space(8.0);
        let count = match stopped.files.len() {
            1 => "1 file".to_owned(),
            n => format!("{n} files"),
        };
        ui.label(format!("{what} stopped: {count} changed on both sides in ways git can't combine on its own."));
        ui.add_space(6.0);
        const LISTED: usize = 8;
        for change in stopped.files.iter().take(LISTED) {
            ui.horizontal(|ui| widgets::file_label(ui, change, ui.visuals().strong_text_color(), true));
        }
        if stopped.files.len() > LISTED {
            ui.label(RichText::new(format!("and {} more", stopped.files.len() - LISTED)).weak());
        }
        ui.add_space(6.0);
        let next = match stopped.operation {
            Some(op) => format!("Choose what to keep in each, then continue the {0}; or abort the {0} to go back to how everything was before it.", op.name()),
            None => "Choose what to keep in each, then commit.".to_owned(),
        };
        ui.label(RichText::new(next).weak());
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(RichText::new("Resolve conflicts").strong()).clicked() {
                actions.push(Action::Resolve(stopped.files.first().cloned()));
                done = true;
            }
            if let Some(op) = stopped.operation
                && ui.button(format!("Abort {}", op.name())).clicked()
            {
                actions.push(Action::Run(Operation::Abort(op)));
                done = true;
            }
            if ui.button("Later").on_hover_text("The banner at the top keeps the way back to them").clicked() {
                done = true;
            }
        });
    });
    done || response.should_close()
}

/// The Conflicts window: the conflicted files on the left, the selected one's conflicts on the right.
#[derive(Default)]
pub struct ConflictsWindow {
    /// Every file that had conflicts while the window was open, in the order they turned up.
    files: Vec<FileChange>,
    /// The file being resolved.
    current: Option<FileResolver>,
    /// Files saved as resolved that the status still lists as conflicted, until it catches up; they aren't picked again
    /// meanwhile.
    saved: Vec<String>,
    /// Asking whether to really abort, since resolved files would be lost.
    confirm_abort: bool,
}

impl ConflictsWindow {
    /// Shows `change`'s conflicts; `None` leaves the selection as it is.
    pub fn select(&mut self, repo: &Repo, change: Option<FileChange>) {
        let Some(change) = change else { return };
        self.remember(&change);
        self.saved.retain(|path| *path != change.path);
        self.current = Some(FileResolver::open(repo, change));
    }

    fn remember(&mut self, change: &FileChange) {
        if !self.files.iter().any(|known| known.path == change.path) {
            self.files.push(change.clone());
        }
    }

    /// The file is done with: no longer shown, and not picked again before the status catches up.
    fn finish_current(&mut self) {
        if let Some(done) = self.current.take() {
            self.saved.push(done.change.path);
        }
    }
}

/// Shows the Conflicts window as a window of its own. Returns whether it should close.
pub fn show(
    ctx: &egui::Context,
    window: &mut ConflictsWindow,
    status: &WorkingStatus,
    repo_name: &str,
    id: egui::ViewportId,
    actions: &mut Vec<Action>,
) -> bool {
    let conflicted: Vec<&FileChange> = status.conflicted().collect();
    for change in &conflicted {
        window.remember(change);
    }
    // Once the status no longer lists a saved file as conflicted it has caught up; should it conflict again later (the
    // next commit of a rebase), it is picked like any other.
    window.saved.retain(|path| conflicted.iter().any(|c| c.path == *path));
    let pending: Vec<&FileChange> = conflicted.iter().copied().filter(|c| !window.saved.contains(&c.path)).collect();
    if let Some(current) = &window.current
        && !pending.iter().any(|c| c.path == current.change.path)
    {
        // Resolved elsewhere (the file list's menu, an editor).
        window.current = None;
    }
    if window.current.is_none()
        && let Some(next) = pending.first()
    {
        actions.push(Action::Resolve(Some((*next).clone())));
    }

    let builder = egui::ViewportBuilder::default()
        .with_title(format!("Conflicts — {repo_name}"))
        .with_inner_size([1200.0, 800.0])
        .with_min_inner_size([700.0, 400.0])
        .with_icon(crate::icon::window_icon());
    let mut close = false;
    ctx.show_viewport_immediate(id, builder, |ui, _| {
        let typing = ui.ctx().egui_wants_keyboard_input();
        close = ui.input(|i| i.viewport().close_requested() || (!typing && i.key_pressed(egui::Key::Escape)));
        egui::Panel::top("conflicts_progress").show(ui, |ui| progress_bar(ui, window, status, &pending, actions));
        widgets::left_panel(ui, "conflicts_files", 300.0).show(ui, |ui| file_list(ui, window, &pending, actions));
        egui::CentralPanel::default().show(ui, |ui| match &mut window.current {
            Some(current) => {
                if let Some(action) = current.show(ui) {
                    actions.push(action);
                    window.finish_current();
                }
            }
            None if pending.is_empty() => all_resolved(ui, status.operation, actions),
            None => widgets::placeholder(ui, "Loading…"),
        });
    });
    close
}

/// How far along the resolving is, and the way out: continue once everything is resolved, or abort.
fn progress_bar(ui: &mut Ui, window: &mut ConflictsWindow, status: &WorkingStatus, pending: &[&FileChange], actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let total = window.files.len();
        let done = total - pending.len().min(total);
        let what = status.operation.map_or_else(String::new, |op| format!("{}: ", op.label()));
        ui.label(RichText::new(format!("{what}{done} of {total} files resolved")).strong());
        let Some(op) = status.operation else { return };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if window.confirm_abort {
                if ui.button("Keep resolving").clicked() {
                    window.confirm_abort = false;
                }
                if ui.button(RichText::new(format!("Abort {}", op.name())).color(ui.visuals().error_fg_color)).clicked() {
                    actions.push(Action::Run(Operation::Abort(op)));
                    window.confirm_abort = false;
                }
                ui.label(format!("Abort the {}? Everything resolved so far is lost.", op.name()));
            } else {
                if op.command().is_some()
                    && ui.add_enabled(pending.is_empty(), egui::Button::new(format!("Continue {}", op.name()))).clicked()
                {
                    actions.push(Action::Run(Operation::Continue(op)));
                }
                if ui.button(format!("Abort {}…", op.name())).clicked() {
                    window.confirm_abort = true;
                }
            }
        });
    });
    ui.add_space(4.0);
}

/// The files still conflicted, then those resolved.
fn file_list(ui: &mut Ui, window: &ConflictsWindow, pending: &[&FileChange], actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add_space(4.0);
        widgets::section_label(ui, &format!("Conflicted ({})", pending.len()));
        if pending.is_empty() {
            ui.label(RichText::new("None left").weak());
        }
        for change in pending {
            let selected = window.current.as_ref().is_some_and(|c| c.change.path == change.path);
            if widgets::file_row(ui, change, selected, true, None).clicked() && !selected {
                actions.push(Action::Resolve(Some((*change).clone())));
            }
        }
        let resolved: Vec<_> = window.files.iter().filter(|f| !pending.iter().any(|p| p.path == f.path)).collect();
        if !resolved.is_empty() {
            ui.add_space(10.0);
            widgets::section_label(ui, &format!("Resolved ({})", resolved.len()));
            for change in resolved {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("✔").color(ui.visuals().weak_text_color()));
                    ui.add(egui::Label::new(RichText::new(&change.path).weak()).truncate());
                });
            }
        }
    });
}

fn all_resolved(ui: &mut Ui, operation: Option<InProgress>, actions: &mut Vec<Action>) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);
        ui.heading("All conflicts are resolved");
        ui.add_space(8.0);
        match operation {
            Some(op) if op.command().is_some() => {
                ui.label(RichText::new(format!("Continue the {} to finish it.", op.name())).weak());
                ui.add_space(8.0);
                if ui.button(RichText::new(format!("Continue {}", op.name())).strong()).clicked() {
                    actions.push(Action::Run(Operation::Continue(op)));
                }
            }
            _ => {
                ui.label(RichText::new("Commit the result from Local Changes when you're ready.").weak());
            }
        }
    });
}

/// One file's conflicts and the choice made for each.
struct FileResolver {
    change: FileChange,
    /// The parsed file, or why it can't be resolved conflict by conflict.
    content: Result<Vec<Segment>, String>,
    /// One entry per conflict, in file order.
    choices: Vec<Option<Resolution>>,
}

impl FileResolver {
    fn open(repo: &Repo, change: FileChange) -> Self {
        let content = match repo.conflict_segments(&change.path) {
            Ok(segments) if conflict_count(&segments) == 0 => Err("This file has no conflict markers to choose between, \
                 for example because it was deleted on one side, or it is binary."
                .to_owned()),
            Ok(segments) => Ok(segments),
            Err(err) => Err(format!("This file can't be resolved conflict by conflict: {err}.")),
        };
        let choices = vec![None; content.as_ref().map_or(0, |s| conflict_count(s))];
        Self { change, content, choices }
    }

    /// Returns the action that resolves the file, once one of its buttons says so.
    fn show(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut outcome = None;
        ui.horizontal(|ui| {
            widgets::file_label(ui, &self.change, ui.visuals().strong_text_color(), true);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (label, tooltip, ours) in WHOLE_FILE.into_iter().rev() {
                    if ui.button(label).on_hover_text(tooltip).clicked() {
                        outcome = Some(take_side(&self.change, ours));
                    }
                }
            });
        });
        ui.separator();
        match &self.content {
            Err(message) => {
                widgets::placeholder(ui, &format!("{message}\nKeep one side with the buttons above."));
            }
            Ok(segments) => {
                if body(ui, segments, &mut self.choices) {
                    outcome = Some(Action::Run(Operation::ResolveMerged {
                        change: self.change.clone(),
                        content: assemble(segments, &self.choices),
                    }));
                }
            }
        }
        outcome
    }
}

/// Every conflict with its choices, the text around them folded away. Returns whether "Save and mark resolved" was
/// clicked.
fn body(ui: &mut Ui, segments: &[Segment], choices: &mut [Option<Resolution>]) -> bool {
    let colors = DiffColors::of(ui.visuals());
    let height = (ui.available_height() - 90.0).max(120.0);
    egui::ScrollArea::vertical().max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        let mut index = 0;
        for segment in segments {
            match segment {
                Segment::Common(text) => {
                    let lines = text.lines().count();
                    egui::CollapsingHeader::new(RichText::new(format!("{lines} unchanged lines")).weak())
                        .id_salt(("common", index, lines))
                        .show(ui, |ui| ui.label(RichText::new(text).monospace()));
                }
                Segment::Conflict(region) => {
                    region_ui(ui, region, &mut choices[index], index, &colors);
                    index += 1;
                }
            }
        }
    });
    ui.separator();
    let resolved = choices.iter().flatten().count();
    egui::CollapsingHeader::new("Preview of the result").show(ui, |ui| {
        egui::ScrollArea::both().id_salt("preview").max_height(160.0).show(ui, |ui| {
            ui.label(RichText::new(assemble(segments, choices)).monospace());
        });
    });
    ui.horizontal(|ui| {
        ui.label(format!("{resolved} of {} conflicts chosen", choices.len()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled(resolved == choices.len(), egui::Button::new(RichText::new("Save and mark resolved").strong()))
                .on_disabled_hover_text("Choose what to keep in every conflict first")
                .clicked()
        })
        .inner
    })
    .inner
}

fn region_ui(ui: &mut Ui, region: &ConflictRegion, choice: &mut Option<Resolution>, index: usize, colors: &DiffColors) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Conflict {}", index + 1)).strong());
            ui.add_space(8.0);
            for resolution in Resolution::ALL {
                if ui.selectable_label(*choice == Some(resolution), resolution.label()).clicked() {
                    *choice = Some(resolution);
                }
            }
        });
        // Green for the checked-out side and blue for the incoming one, as in VS Code.
        let sides = [
            Some(("Current", &region.ours_label, &region.ours, colors.added, "The checked-out version (HEAD)")),
            region.base.as_ref().map(|base| ("Base", &region.base_label, base, ui.visuals().faint_bg_color, "What both sides started from")),
            Some(("Incoming", &region.theirs_label, &region.theirs, colors.hunk, "The version being brought in")),
        ];
        let sides: Vec<_> = sides.into_iter().flatten().collect();
        ui.columns(sides.len(), |columns| {
            for (ui, (title, label, text, fill, meaning)) in columns.iter_mut().zip(sides.iter()) {
                let heading = if label.is_empty() { (*title).to_owned() } else { format!("{title} — {label}") };
                ui.label(widgets::section_title(&heading)).on_hover_text(*meaning);
                egui::Frame::new().fill(*fill).inner_margin(4.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let text = if text.is_empty() { RichText::new("(nothing)").italics().weak() } else { RichText::new(text.as_str()).monospace() };
                    ui.label(text);
                });
            }
        });
    });
}
