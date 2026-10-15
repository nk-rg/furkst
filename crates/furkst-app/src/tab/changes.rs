//! Local changes: unstaged and staged files, the commit form, and the diff with staging controls.

use egui::{RichText, TextEdit, Ui};
use furkst_core::{ChangeKind, DiffTarget, FileChange, Operation, WorkingStatus};

use super::selection::Selection;
use super::{Action, RepoTab, files, menus};
use crate::settings::Settings;
use crate::widgets;

#[derive(Default)]
pub struct CommitForm {
    subject: String,
    body: String,
    amend: bool,
    /// The repository's `commit.template`, which every new message starts from.
    template: Option<String>,
}

impl CommitForm {
    pub fn with_template(template: Option<String>) -> Self {
        let mut form = Self { template, ..Self::default() };
        form.clear();
        form
    }

    pub fn prefill(&mut self, message: &str) {
        let (subject, body) = message.split_once('\n').unwrap_or((message, ""));
        // Keep trailing spaces: a template subject like "feat: " expects typing right after it.
        self.subject = subject.trim_start().trim_end_matches('\r').to_owned();
        self.body = body.trim().to_owned();
    }

    /// Empties the form back to the template.
    pub fn clear(&mut self) {
        *self = Self { template: self.template.take(), ..Self::default() };
        if let Some(template) = self.template.clone() {
            self.prefill(&template);
        }
    }

    fn message(&self) -> String {
        let body = self.body.trim();
        if body.is_empty() { self.subject.trim().to_owned() } else { format!("{}\n\n{body}", self.subject.trim()) }
    }
}

pub fn show(ui: &mut Ui, tab: &mut RepoTab, settings: &mut Settings, actions: &mut Vec<Action>) {
    if let Some(operation) = tab.status.operation {
        in_progress_banner(ui, operation, tab.status.has_conflicts(), actions);
    }
    widgets::left_panel(ui, "changes_lists", 380.0).show(ui, |ui| {
        egui::Panel::bottom("commit_form").show(ui, |ui| commit_form(ui, tab, actions));
        file_lists(ui, tab, actions);
    });
    egui::CentralPanel::no_frame().show(ui, |ui| diff(ui, tab, settings, false, actions));
}

/// The unstaged and staged files, each in half the room `ui` has.
pub fn file_lists(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let half = (ui.available_height() / 2.0 - 30.0).max(80.0);
    file_section(ui, &tab.status, &tab.diff, &mut tab.selections, DiffTarget::Unstaged, half, actions);
    ui.separator();
    file_section(ui, &tab.status, &tab.diff, &mut tab.selections, DiffTarget::Staged, half, actions);
}

/// The selected file's diff, with staging controls, or how many files are selected; `in_window` when it is drawn in
/// the Changes window.
pub fn diff(ui: &mut Ui, tab: &mut RepoTab, settings: &mut Settings, in_window: bool, actions: &mut Vec<Action>) {
    if tab.selections.is_multiple() {
        return widgets::placeholder(ui, &format!("{} files selected", tab.selections.count()));
    }
    tab.diff.show(ui, settings, Action::SelectFile, Some(in_window), actions);
}

/// The selections of the unstaged and staged lists; only one of them is ever non-empty.
#[derive(Default)]
pub struct Selections {
    unstaged: Selection,
    staged: Selection,
    /// Which list keyboard shortcuts go to.
    staged_focused: bool,
}

impl Selections {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Drops files that are no longer in `status`.
    pub fn retain(&mut self, status: &WorkingStatus) {
        let paths = |files: &[FileChange]| files.iter().map(|c| c.path.clone()).collect::<Vec<_>>();
        let (unstaged, staged) = (paths(&status.unstaged), paths(&status.staged));
        self.unstaged.retain(&unstaged.iter().map(String::as_str).collect::<Vec<_>>());
        self.staged.retain(&staged.iter().map(String::as_str).collect::<Vec<_>>());
    }

    pub fn count(&self) -> usize {
        self.unstaged.len() + self.staged.len()
    }

    pub fn is_multiple(&self) -> bool {
        self.count() > 1
    }

    /// Selects just `path` in the list for `target`, emptying the other list.
    pub fn select_only(&mut self, target: &DiffTarget, path: &str) {
        self.focus(*target == DiffTarget::Staged);
        self.list(*target == DiffTarget::Staged).select_only(path);
    }

    fn list(&mut self, staged: bool) -> &mut Selection {
        if staged { &mut self.staged } else { &mut self.unstaged }
    }

    /// Makes one list the active one, clearing the other.
    fn focus(&mut self, staged: bool) {
        self.staged_focused = staged;
        self.list(!staged).clear();
    }
}

/// One of the two file lists, with its bulk action buttons.
fn file_section(
    ui: &mut Ui,
    status: &WorkingStatus,
    diff: &super::diff_view::DiffPanel,
    selections: &mut Selections,
    target: DiffTarget,
    height: f32,
    actions: &mut Vec<Action>,
) {
    let staged = target == DiffTarget::Staged;
    let files = if staged { &status.staged } else { &status.unstaged };

    let keyboard = selections.staged_focused == staged && !ui.ctx().egui_wants_keyboard_input();
    let selection = selections.list(staged);
    ui.horizontal(|ui| {
        let title = if staged { "Staged files" } else { "Unstaged files" };
        widgets::section_label(ui, &format!("{title} ({})", files.len()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = if staged { "Unstage all" } else { "Stage all" };
            if ui.add_enabled(!files.is_empty(), egui::Button::new(label).small()).clicked() {
                actions.push(Action::Run(toggle(staged, files.clone())));
            }
            if selection.len() > 1 {
                let label = if staged { "Unstage selected" } else { "Stage selected" };
                if ui.add(egui::Button::new(label).small()).clicked() {
                    actions.push(Action::Run(toggle(staged, files::selected(files, selection))));
                }
            }
        });
    });
    let mut interacted = false;
    egui::ScrollArea::vertical().id_salt(staged).max_height(height).min_scrolled_height(height).auto_shrink([false, false]).show(
        ui,
        |ui| {
            if files.is_empty() {
                ui.label(RichText::new(if staged { "Nothing staged" } else { "No changes" }).weak());
            }
            let multi = files::Multi { selection, keyboard };
            let outcome = files::list(ui, files, &target, diff, Some(multi), false, actions, |ui, targets, actions| {
                file_menu(ui, targets, staged, actions);
            });
            interacted = outcome.interacted;
            if let [change] = outcome.activated.as_slice()
                && !staged
                && change.kind == ChangeKind::Conflicted
            {
                actions.push(Action::Resolve(change.clone()));
            } else if !outcome.activated.is_empty() {
                actions.push(Action::Run(toggle(staged, outcome.activated)));
            }
        },
    );
    if interacted {
        selections.focus(staged);
    }
}

fn discard(files: Vec<FileChange>) -> Action {
    let what = match files.as_slice() {
        [change] => change.path.clone(),
        _ => format!("{} files", files.len()),
    };
    Action::Confirm {
        title: "Discard changes".into(),
        message: format!("Discard all changes to {what}? This cannot be undone."),
        op: Operation::Discard(files),
    }
}

/// Stages unstaged files or unstages staged ones.
fn toggle(staged: bool, files: Vec<FileChange>) -> Operation {
    if staged { Operation::Unstage(files) } else { Operation::Stage(files) }
}

/// Context menu of a row; `targets` is the whole selection when the row is part of a multi-selection.
fn file_menu(ui: &mut Ui, targets: &[FileChange], staged: bool, actions: &mut Vec<Action>) {
    let count = if targets.len() > 1 { format!(" {} files", targets.len()) } else { String::new() };
    match targets {
        [change] if !staged && change.kind == ChangeKind::Conflicted => {
            menus::item(ui, "Resolve conflicts…", actions, || Action::Resolve(change.clone()));
            for (label, ours) in [("Resolve using mine", true), ("Resolve using theirs", false)] {
                menus::item(ui, label, actions, || Action::Run(Operation::ResolveConflict { change: change.clone(), ours }));
            }
            menus::item(ui, "Mark as resolved", actions, || Action::Run(toggle(false, targets.to_vec())));
        }
        _ => {
            let verb = if staged { "Unstage" } else { "Stage" };
            menus::item(ui, format!("{verb}{count}"), actions, || Action::Run(toggle(staged, targets.to_vec())));
        }
    }
    if !staged {
        menus::item(ui, "Discard changes…", actions, || discard(targets.to_vec()));
        if let [change] = targets
            && let Some(extension) = std::path::Path::new(&change.path).extension().and_then(|e| e.to_str())
        {
            let pattern = format!("*.{extension}");
            menus::item(ui, format!("Track {pattern} with Git LFS"), actions, || Action::Run(Operation::LfsTrack { pattern }));
        }
    }
    if let [change] = targets {
        ui.separator();
        menus::file(ui, &change.path, None, actions);
    }
}

fn commit_form(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    ui.add_space(6.0);
    let form = &mut tab.commit_form;
    ui.horizontal(|ui| {
        const COUNTER_WIDTH: f32 = 28.0;
        let width = ui.available_width() - COUNTER_WIDTH - ui.spacing().item_spacing.x;
        ui.add(TextEdit::singleline(&mut form.subject).hint_text("Commit subject").desired_width(width));
        subject_length(ui, &form.subject);
    });
    ui.add(TextEdit::multiline(&mut form.body).hint_text("Description").desired_rows(4).desired_width(f32::INFINITY));
    ui.horizontal(|ui| {
        let was_amending = form.amend;
        ui.checkbox(&mut form.amend, "Amend last commit");
        if form.amend && !was_amending && form.subject.is_empty() {
            let head = tab.refs.as_ref().and_then(|r| r.head.target());
            if let Some(message) = head.and_then(|id| tab.repo.commit_info(id).ok()).map(|c| c.message) {
                form.prefill(&message);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let staged = tab.status.staged.len();
            let ready = !form.subject.trim().is_empty() && (staged > 0 || form.amend || tab.status.operation.is_some());
            let label = match staged {
                0 => "Commit".to_owned(),
                1 => "Commit 1 file".to_owned(),
                n => format!("Commit {n} files"),
            };
            let shortcut = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
            let clicked = ui.add_enabled(ready, egui::Button::new(RichText::new(label).strong())).on_hover_text("Ctrl+Enter").clicked();
            if ready && (clicked || shortcut) {
                actions.push(Action::Run(Operation::Commit { message: form.message(), amend: form.amend }));
            }
        });
    });
    ui.add_space(4.0);
}

fn in_progress_banner(ui: &mut Ui, operation: furkst_core::InProgress, conflicts: bool, actions: &mut Vec<Action>) {
    egui::Frame::new().fill(ui.visuals().warn_fg_color.gamma_multiply(0.15)).inner_margin(8.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            let hint = if conflicts { "Resolve the conflicts, then continue." } else { "Ready to continue." };
            ui.label(RichText::new(format!("{} in progress. {hint}", operation.label())).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Abort").clicked() {
                    actions.push(Action::Confirm {
                        title: format!("Abort {}", operation.label().to_lowercase()),
                        message: "All progress of the current operation will be lost.".into(),
                        op: Operation::Abort(operation),
                    });
                }
                if operation.command().is_some() && ui.add_enabled(!conflicts, egui::Button::new("Continue")).clicked() {
                    actions.push(Action::Run(Operation::Continue(operation)));
                }
            });
        });
    });
}

/// The subject's length, turning to a warning past 50 characters and an error past 72 (git's conventions).
fn subject_length(ui: &mut Ui, subject: &str) {
    let length = subject.trim().chars().count();
    let color = match length {
        0..=50 => ui.visuals().weak_text_color(),
        51..=72 => ui.visuals().warn_fg_color,
        _ => ui.visuals().error_fg_color,
    };
    ui.label(RichText::new(length.to_string()).small().color(color))
        .on_hover_text("Keep the subject within 50 characters; many git tools cut it off after 72.");
}
