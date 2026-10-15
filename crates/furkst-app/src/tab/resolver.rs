//! Merge conflicts: the prompt shown when an operation stops on them, and the Conflicts window, a window of its own
//! listing the conflicted files and resolving one at a time, conflict by conflict, in the words VS Code uses ("current"
//! for the checked-out side, "incoming" for the side being brought in).

use egui::{Color32, FontId, RichText, Ui};
use furkst_core::{
    ConflictRegion, ConflictSides, FileChange, InProgress, Operation, Repo, Resolution, Segment, WorkingStatus, assemble,
    conflict_count, lines,
};

use super::Action;
use super::highlight::{self, Spans};
use crate::theme::DiffColors;
use crate::widgets;

/// Taking one side's version of a whole file: label, tooltip and whether it is the current side.
pub const WHOLE_FILE: [(&str, &str, bool); 2] = [
    (
        "Use the current version",
        "The whole file as the checked-out side has it (deleted, if that side deleted it), without the incoming changes",
        true,
    ),
    ("Use the incoming version", "The whole file as the side being brought in has it (deleted, if that side deleted it)", false),
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
    /// Whether uncommitted changes were set aside before it, to come back once it is finished or aborted.
    pub kept_aside: bool,
}

impl Stopped {
    /// What an operation that failed with `error` left behind, if it stopped on conflicts.
    pub fn after_failure(repo: &Repo, error: &str) -> Option<Self> {
        let status = repo.status().ok()?;
        let files: Vec<FileChange> = status.conflicted().cloned().collect();
        (!files.is_empty()).then_some(Self { operation: status.operation, files, kept_aside: furkst_core::kept_aside(error) })
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
        if stopped.kept_aside {
            ui.label(RichText::new("Your uncommitted changes are set aside in a stash; they come back once this is finished or aborted.").weak());
        }
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
    font: &FontId,
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
                if let Some(action) = current.show(ui, font) {
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

/// One file's conflicts and what each turns into.
struct FileResolver {
    change: FileChange,
    /// Its conflicts, or why it has none to choose between.
    content: Result<Conflicts, Unparsed>,
    /// The merge tool git is set up with (`merge.tool`), if any.
    merge_tool: Option<String>,
    /// The merge tool the file was opened in, while it may still be open.
    opened_in: Option<String>,
}

/// A conflicted file without conflict markers: deleted on one side, or binary.
struct Unparsed {
    message: String,
    sides: Option<ConflictSides>,
}

struct Conflicts {
    segments: Vec<Segment>,
    conflicts: Vec<Conflict>,
    /// Syntax colors, for the dark theme or not: three blocks per conflict, its current, base and incoming lines.
    highlights: Option<(bool, Option<Vec<Vec<Spans>>>)>,
    /// The conflict the arrows went to last, and whether to scroll to it.
    at: usize,
    scroll: bool,
}

struct Conflict {
    region: ConflictRegion,
    /// Which lines of the current and the incoming side are kept.
    ours: Vec<bool>,
    theirs: Vec<bool>,
    theirs_first: bool,
    /// What the conflict turns into; `None` until something is chosen or typed.
    result: Option<String>,
    /// The line number the current, base and incoming text start at, in their side's version of the file.
    starts: [usize; 3],
}

impl Conflict {
    fn choose(&mut self, resolution: Resolution) {
        (self.ours, self.theirs) = resolution.picks(&self.region);
        self.theirs_first = resolution == Resolution::TheirsThenOurs;
        self.result = Some(self.region.resolved(resolution));
    }

    /// Keeps or drops one line of the current side (`ours`) or the incoming one.
    fn toggle(&mut self, ours: bool, line: usize) {
        let picks = if ours { &mut self.ours } else { &mut self.theirs };
        picks[line] = !picks[line];
        self.result = Some(self.region.pick(&self.ours, &self.theirs, self.theirs_first));
    }

    fn reset(&mut self) {
        self.ours.fill(false);
        self.theirs.fill(false);
        self.result = None;
    }

    /// The choice the result matches, if it matches one.
    fn chosen(&self) -> Option<Resolution> {
        let result = self.result.as_ref()?;
        Resolution::ALL.into_iter().find(|resolution| self.region.resolved(*resolution) == *result)
    }
}

impl Conflicts {
    fn new(segments: Vec<Segment>) -> Self {
        let mut conflicts = Vec::new();
        let mut next = [1, 1, 1];
        for segment in &segments {
            match segment {
                Segment::Common(text) => next = next.map(|n| n + lines(text).count()),
                Segment::Conflict(region) => {
                    let base = region.base.as_deref().unwrap_or_default();
                    let sizes = [region.ours.as_str(), base, region.theirs.as_str()].map(|text| lines(text).count());
                    conflicts.push(Conflict {
                        region: region.clone(),
                        ours: vec![false; sizes[0]],
                        theirs: vec![false; sizes[2]],
                        theirs_first: false,
                        result: None,
                        starts: next,
                    });
                    next = [0, 1, 2].map(|side| next[side] + sizes[side]);
                }
            }
        }
        Self { segments, conflicts, highlights: None, at: 0, scroll: false }
    }

    fn results(&self) -> Vec<Option<String>> {
        self.conflicts.iter().map(|c| c.result.clone()).collect()
    }

    /// Syntax colors of every conflict's sides, worked out again when the theme changes.
    fn highlights(&mut self, path: &str, dark: bool) -> Option<&Vec<Vec<Spans>>> {
        if self.highlights.as_ref().is_none_or(|(was_dark, _)| *was_dark != dark) {
            let shown: Vec<Vec<String>> = self
                .conflicts
                .iter()
                .flat_map(|c| [c.region.ours.as_str(), c.region.base.as_deref().unwrap_or_default(), c.region.theirs.as_str()])
                .map(|text| lines(text).map(shown_line).collect())
                .collect();
            let blocks: Vec<Vec<&str>> = shown.iter().map(|lines| lines.iter().map(String::as_str).collect()).collect();
            self.highlights = Some((dark, highlight::highlight_blocks(path, &blocks, dark)));
        }
        self.highlights.as_ref().and_then(|(_, spans)| spans.as_ref())
    }
}

/// A line as drawn: without its line break, tabs as spaces.
fn shown_line(line: &str) -> String {
    highlight::display_text(line.trim_end_matches(['\n', '\r'])).into_owned()
}

/// How code looks: the diff font, colors, and the file it is from (for syntax colors).
struct Code<'a> {
    font: FontId,
    path: &'a str,
    text: Color32,
    weak: Color32,
    dark: bool,
}

impl FileResolver {
    fn open(repo: &Repo, change: FileChange) -> Self {
        let content = match repo.conflict_segments(&change.path) {
            Ok(segments) if conflict_count(&segments) > 0 => Ok(Conflicts::new(segments)),
            parsed => {
                let sides = repo.conflict_sides(&change.path).ok();
                let message = match (sides, parsed) {
                    (Some(ConflictSides { ours: false, .. }), _) => {
                        "The current side deleted this file, and the incoming side changed it.".to_owned()
                    }
                    (Some(ConflictSides { theirs: false, .. }), _) => {
                        "The current side changed this file, and the incoming side deleted it.".to_owned()
                    }
                    (_, Err(err)) => format!("This file can't be resolved conflict by conflict: {err}."),
                    (_, Ok(_)) => "This file has no conflict markers to choose between.".to_owned(),
                };
                Err(Unparsed { message, sides })
            }
        };
        Self { change, content, merge_tool: repo.configured_merge_tool(), opened_in: None }
    }

    /// Returns the action that resolves the file, once one of its buttons says so.
    fn show(&mut self, ui: &mut Ui, font: &FontId) -> Option<Action> {
        let mut outcome = None;
        let code = Code {
            font: font.clone(),
            path: &self.change.path,
            text: ui.visuals().strong_text_color(),
            weak: ui.visuals().weak_text_color(),
            dark: ui.visuals().dark_mode,
        };
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            widgets::file_label(ui, &self.change, code.text, true);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(action) = merge_tool_menu(ui, &self.change.path, self.merge_tool.as_deref(), &mut self.opened_in) {
                    outcome = Some(action);
                }
                match &mut self.content {
                    Ok(conflicts) => {
                        for (label, resolution) in [("Accept all incoming", Resolution::Theirs), ("Accept all current", Resolution::Ours)] {
                            if ui.button(label).on_hover_text("In every conflict of this file").clicked() {
                                conflicts.conflicts.iter_mut().for_each(|c| c.choose(resolution));
                            }
                        }
                    }
                    Err(unparsed) => {
                        for (label, ours) in side_choices(unparsed.sides).into_iter().rev() {
                            if ui.button(label).clicked() {
                                outcome = Some(take_side(&self.change, ours));
                            }
                        }
                    }
                }
            });
        });
        if let Some(tool) = &self.opened_in {
            ui.label(RichText::new(format!("Opened in {tool}: save the result there and close it, and Furkst takes it from there.")).weak());
        }
        ui.separator();
        match &mut self.content {
            Err(unparsed) => widgets::placeholder(ui, &format!("{}\nChoose which version to keep with the buttons above.", unparsed.message)),
            Ok(conflicts) => {
                if body(ui, conflicts, &code) {
                    outcome = Some(Action::Run(Operation::ResolveMerged {
                        change: self.change.clone(),
                        content: assemble(&conflicts.segments, &conflicts.results()),
                    }));
                }
            }
        }
        outcome
    }
}

/// The versions to choose between for a file without conflict markers, as (label, whether it is the current one).
fn side_choices(sides: Option<ConflictSides>) -> [(&'static str, bool); 2] {
    match sides {
        Some(ConflictSides { ours: false, .. }) => [("Keep it deleted", true), ("Keep the incoming file", false)],
        Some(ConflictSides { theirs: false, .. }) => [("Keep the current file", true), ("Delete it", false)],
        _ => WHOLE_FILE.map(|(label, _, ours)| (label, ours)),
    }
}

/// "Open in merge tool": the one git is set up with, and the installed editors with a merge view.
fn merge_tool_menu(ui: &mut Ui, path: &str, configured: Option<&str>, opened_in: &mut Option<String>) -> Option<Action> {
    let mut action = None;
    ui.menu_button("Open in merge tool", |ui| {
        let mut open = |ui: &mut Ui, name: String, command: Option<String>| {
            *opened_in = Some(name);
            action = Some(Action::Run(Operation::MergeTool { path: path.to_owned(), command }));
            ui.close();
        };
        if let Some(tool) = configured
            && ui.button(format!("{tool} (git mergetool)")).clicked()
        {
            open(ui, tool.to_owned(), None);
        }
        let editors: Vec<_> = crate::editors::installed().iter().filter_map(|e| Some((e, e.merge_command()?))).collect();
        for (editor, command) in &editors {
            if ui.button(&editor.name).clicked() {
                open(ui, editor.name.clone(), Some(command.clone()));
            }
        }
        if configured.is_none() && editors.is_empty() {
            ui.label(RichText::new("No merge tool found: set one up with git config merge.tool, or install VS Code.").weak());
        }
    });
    action
}

/// Every conflict, the text around them folded away, with the arrows between them. Returns whether "Save and mark
/// resolved" was clicked.
fn body(ui: &mut Ui, conflicts: &mut Conflicts, code: &Code) -> bool {
    let colors = DiffColors::of(ui.visuals());
    let count = conflicts.conflicts.len();
    let chosen = conflicts.conflicts.iter().filter(|c| c.result.is_some()).count();
    ui.horizontal(|ui| {
        let previous = ui.add_enabled(conflicts.at > 0, egui::Button::new("⬆")).on_hover_text("Previous conflict");
        let next = ui.add_enabled(conflicts.at + 1 < count, egui::Button::new("⬇")).on_hover_text("Next conflict");
        if previous.clicked() {
            conflicts.at -= 1;
            conflicts.scroll = true;
        }
        if next.clicked() {
            conflicts.at += 1;
            conflicts.scroll = true;
        }
        ui.label(format!("Conflict {} of {count}", conflicts.at + 1));
        ui.label(RichText::new(format!("{chosen} of {count} chosen")).weak());
    });
    let spans = conflicts.highlights(code.path, code.dark).cloned();
    let height = (ui.available_height() - 90.0).max(120.0);
    egui::ScrollArea::vertical().max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        let mut index = 0;
        for segment in &conflicts.segments {
            match segment {
                Segment::Common(text) => {
                    let lines = lines(text).count();
                    egui::CollapsingHeader::new(RichText::new(format!("{lines} unchanged line{}", if lines == 1 { "" } else { "s" })).weak())
                        .id_salt(("common", index, lines))
                        .show(ui, |ui| ui.label(RichText::new(text).font(code.font.clone())));
                }
                Segment::Conflict(_) => {
                    let blocks = spans.as_ref().map(|spans| &spans[index * 3..index * 3 + 3]);
                    let rect = conflict_ui(ui, &mut conflicts.conflicts[index], index, blocks, &colors, code);
                    if conflicts.scroll && conflicts.at == index {
                        ui.scroll_to_rect(rect, Some(egui::Align::TOP));
                        conflicts.scroll = false;
                    }
                    index += 1;
                }
            }
        }
    });
    ui.separator();
    egui::CollapsingHeader::new("Preview of the whole file").show(ui, |ui| {
        egui::ScrollArea::both().id_salt("preview").max_height(160.0).show(ui, |ui| {
            ui.label(RichText::new(assemble(&conflicts.segments, &conflicts.results())).font(code.font.clone()));
        });
    });
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add_enabled(chosen == count, egui::Button::new(RichText::new("Save and mark resolved").strong()))
            .on_disabled_hover_text("Choose what to keep in every conflict first")
            .clicked()
    })
    .inner
}

/// One conflict: the choices, its sides line by line (each line of the current and incoming side can be kept or
/// dropped), and the result, which can also be typed. Returns where it was drawn.
fn conflict_ui(ui: &mut Ui, conflict: &mut Conflict, index: usize, spans: Option<&[Vec<Spans>]>, colors: &DiffColors, code: &Code) -> egui::Rect {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Conflict {}", index + 1)).strong());
            ui.add_space(8.0);
            let chosen = conflict.chosen();
            for resolution in Resolution::ALL {
                if ui.selectable_label(chosen == Some(resolution), resolution.label()).clicked() {
                    conflict.choose(resolution);
                }
            }
            if conflict.result.is_some() && ui.button("Reset").on_hover_text("Start this conflict over").clicked() {
                conflict.reset();
            }
        });
        // Green for the checked-out side and blue for the incoming one, as in VS Code.
        let region = &conflict.region;
        let mut sides = vec![(0, "Current", &region.ours_label, region.ours.as_str(), colors.added, "The checked-out version (HEAD)")];
        if let Some(base) = &region.base {
            sides.push((1, "Base", &region.base_label, base.as_str(), ui.visuals().faint_bg_color, "What both sides started from"));
        }
        sides.push((2, "Incoming", &region.theirs_label, region.theirs.as_str(), colors.hunk, "The version being brought in"));
        let mut toggled = None;
        ui.columns(sides.len(), |columns| {
            for (ui, (side, title, label, text, fill, meaning)) in columns.iter_mut().zip(sides) {
                let heading = if label.is_empty() { title.to_owned() } else { format!("{title} — {label}") };
                ui.label(widgets::section_title(&heading)).on_hover_text(meaning);
                let picks = match side {
                    0 => Some(&conflict.ours),
                    2 => Some(&conflict.theirs),
                    _ => None,
                };
                egui::Frame::new().fill(fill).inner_margin(4.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let spans = spans.map(|blocks| &blocks[side]);
                    if let Some(line) = side_lines(ui, (index, side), text, conflict.starts[side], picks, spans, code) {
                        toggled = Some((side == 0, line));
                    }
                });
            }
        });
        if let Some((ours, line)) = toggled {
            conflict.toggle(ours, line);
        }
        ui.add_space(4.0);
        ui.label(widgets::section_title("Result")).on_hover_text("What goes into the file; type to change it");
        let mut text = conflict.result.clone().unwrap_or_default();
        let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut job = highlight::code_job(code.path, buffer.as_str(), &code.font, code.text, code.dark);
            job.wrap.max_width = wrap_width;
            ui.fonts_mut(|fonts| fonts.layout_job(job))
        };
        let edit = egui::TextEdit::multiline(&mut text)
            .font(code.font.clone())
            .hint_text("Choose above, tick the lines to keep, or type the result here")
            .desired_width(f32::INFINITY)
            .desired_rows(1)
            .layouter(&mut layouter);
        if ui.add(edit).changed() {
            conflict.result = Some(text);
        }
    })
    .response
    .rect
}

/// The lines of one side, numbered, in syntax colors; with `picks`, each with a box to keep it. Returns the line whose
/// box (or text) was clicked.
fn side_lines(
    ui: &mut Ui,
    id: (usize, usize),
    text: &str,
    start: usize,
    picks: Option<&Vec<bool>>,
    spans: Option<&Vec<Spans>>,
    code: &Code,
) -> Option<usize> {
    if text.is_empty() {
        ui.label(RichText::new("(nothing)").italics().weak());
        return None;
    }
    let mut clicked = None;
    egui::ScrollArea::horizontal().id_salt(id).show(ui, |ui| {
        for (index, line) in lines(text).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if let Some(picks) = picks {
                    let mut keep = picks[index];
                    if ui.checkbox(&mut keep, "").on_hover_text("Keep this line").changed() {
                        clicked = Some(index);
                    }
                }
                ui.label(RichText::new(format!("{:>4}", start + index)).font(code.font.clone()).color(code.weak));
                let mut job = egui::text::LayoutJob::default();
                highlight::append(&mut job, &shown_line(line), spans.and_then(|s| s.get(index)), &code.font, code.text);
                let sense = if picks.is_some() { egui::Sense::click() } else { egui::Sense::hover() };
                let label = ui.add(egui::Label::new(job).extend().selectable(false).sense(sense));
                if picks.is_some() && label.clicked() {
                    clicked = Some(index);
                }
            });
        }
    });
    clicked
}
