//! Modal dialogs. Each variant only describes its fields and the action it submits; the frame,
//! buttons and keyboard handling are shared.

use std::path::PathBuf;

use egui::{RichText, TextEdit, Ui};
use furkst_core::{Branch, FlowKind, MergeStyle, Oid, Operation, RefsSnapshot, RemoteBranch, ResetMode};

use super::Action;
use crate::settings::Settings;

pub enum Dialog {
    Confirm { title: String, message: String, op: Operation },
    CreateBranch { at: Oid, at_label: String, name: String, checkout: bool },
    RenameBranch { from: String, to: String },
    CheckoutRemote { remote_branch: String, local_name: String },
    CreateTag { at: Oid, at_label: String, name: String, message: String },
    Stash { message: String, include_untracked: bool, keep_index: bool },
    Pull { rebase: bool },
    Push { remote: String, branch: String, set_upstream: bool, force: bool, tags: bool },
    Merge { target: String, style: MergeStyle },
    Reset { target: Oid, mode: ResetMode },
    AddRemote { name: String, url: String },
    Rebase(super::rebase::Editor),
    FlowInit { main: String, develop: String },
    FlowStart { kind: FlowKind, name: String },
    FlowFinish { kind: FlowKind, name: String, style: MergeStyle },
    AddWorktree { path: String, branch: String, create: bool, start: String },
    RemoveWorktree { path: PathBuf, force: bool },
    /// A local branch and the remote one that goes with it: each deleted when ticked. `remote` is (remote, branch).
    DeleteBranch { local: Option<String>, remote: Option<(String, String)>, delete_local: bool, delete_remote: bool },
}

pub enum Outcome {
    Open,
    Cancel,
    Submit(Action),
}

impl Dialog {
    /// New branch at `HEAD`, named after the current branch; `None` before the first commit.
    pub fn create_branch_at_head(refs: &RefsSnapshot) -> Option<Self> {
        let head = refs.head.target()?;
        Some(Self::create_branch(head, refs.current_branch().map_or("HEAD", |b| b.name.as_str())))
    }

    pub fn stash() -> Self {
        Self::Stash { message: String::new(), include_untracked: true, keep_index: false }
    }

    pub fn pull(settings: &Settings) -> Self {
        Self::Pull { rebase: settings.pull_rebase }
    }

    /// Push dialog for the checked-out branch; `None` when detached.
    pub fn push_current(refs: &RefsSnapshot) -> Option<Self> {
        refs.current_branch().map(|branch| Self::push(refs, branch))
    }

    /// Deleting local branch `name`, offering its remote branch too, unticked.
    pub fn delete_local(refs: &RefsSnapshot, name: &str) -> Self {
        let remote = refs.remote_counterpart(name).map(|r| (r.remote.clone(), r.name.clone()));
        Self::DeleteBranch { local: Some(name.to_owned()), remote, delete_local: true, delete_remote: false }
    }

    /// Deleting `branch` from its remote, offering the local branch that goes with it too, unticked (unless it is
    /// checked out, which git can't delete).
    pub fn delete_remote(refs: &RefsSnapshot, branch: &RemoteBranch) -> Self {
        let local = refs.local_counterpart(&branch.full_name()).filter(|b| !b.is_head).map(|b| b.name.clone());
        let remote = Some((branch.remote.clone(), branch.name.clone()));
        Self::DeleteBranch { local, remote, delete_local: false, delete_remote: true }
    }

    pub fn create_branch(at: Oid, at_label: &str) -> Self {
        Self::CreateBranch { at, at_label: at_label.to_owned(), name: String::new(), checkout: true }
    }

    pub fn create_tag(at: Oid, at_label: &str) -> Self {
        Self::CreateTag { at, at_label: at_label.to_owned(), name: String::new(), message: String::new() }
    }

    /// Add-worktree dialog; `branch` preselects an existing branch, otherwise a new one starts at `start`.
    pub fn add_worktree(branch: Option<&str>, start: &str) -> Self {
        Self::AddWorktree {
            path: String::new(),
            branch: branch.unwrap_or_default().to_owned(),
            create: branch.is_none(),
            start: start.to_owned(),
        }
    }

    /// Push dialog for `branch`, targeting its upstream remote or the first remote.
    pub fn push(refs: &RefsSnapshot, branch: &Branch) -> Self {
        let upstream = branch.upstream.as_deref().and_then(|u| u.split_once('/'));
        let remote = upstream.map(|(r, _)| r.to_owned()).or_else(|| refs.remotes.first().map(|r| r.name.clone()));
        Self::Push {
            remote: remote.unwrap_or_default(),
            branch: branch.name.clone(),
            set_upstream: upstream.is_none(),
            force: false,
            tags: false,
        }
    }

    fn title(&self) -> &str {
        match self {
            Self::Confirm { title, .. } => title,
            Self::CreateBranch { .. } => "Create branch",
            Self::RenameBranch { .. } => "Rename branch",
            Self::CheckoutRemote { .. } => "Checkout remote branch",
            Self::CreateTag { .. } => "Create tag",
            Self::Stash { .. } => "Stash changes",
            Self::Pull { .. } => "Pull",
            Self::Push { .. } => "Push",
            Self::Merge { .. } => "Merge",
            Self::Reset { .. } => "Reset",
            Self::AddRemote { .. } => "Add remote",
            Self::Rebase(_) => "Interactive rebase",
            Self::FlowInit { .. } => "Initialize git-flow",
            Self::FlowStart { kind: FlowKind::Feature, .. } => "Start feature",
            Self::FlowStart { kind: FlowKind::Release, .. } => "Start release",
            Self::FlowStart { kind: FlowKind::Hotfix, .. } => "Start hotfix",
            Self::FlowFinish { kind: FlowKind::Feature, .. } => "Finish feature",
            Self::FlowFinish { kind: FlowKind::Release, .. } => "Finish release",
            Self::FlowFinish { kind: FlowKind::Hotfix, .. } => "Finish hotfix",
            Self::AddWorktree { .. } => "Add worktree",
            Self::RemoveWorktree { .. } => "Remove worktree",
            Self::DeleteBranch { .. } => "Delete branch",
        }
    }

    /// Dialogs with multi-line text fields need Enter for new lines, so only their button submits.
    fn has_multiline_text(&self) -> bool {
        matches!(self, Self::CreateTag { .. } | Self::Rebase(_))
    }

    fn width(&self) -> f32 {
        if matches!(self, Self::Rebase(_)) { 560.0 } else { 380.0 }
    }

    fn submit_label(&self) -> &str {
        match self {
            Self::Confirm { .. } => "Continue",
            Self::CreateBranch { .. } | Self::CreateTag { .. } => "Create",
            Self::RenameBranch { .. } => "Rename",
            Self::CheckoutRemote { .. } => "Checkout",
            Self::Stash { .. } => "Stash",
            Self::Pull { .. } => "Pull",
            Self::Push { .. } => "Push",
            Self::Merge { .. } => "Merge",
            Self::Reset { .. } => "Reset",
            Self::Rebase(_) => "Start rebase",
            Self::FlowInit { .. } => "Initialize",
            Self::FlowStart { .. } => "Start",
            Self::FlowFinish { .. } => "Finish",
            Self::AddRemote { .. } | Self::AddWorktree { .. } => "Add",
            Self::RemoveWorktree { .. } => "Remove",
            Self::DeleteBranch { .. } => "Delete",
        }
    }

    fn fields(&mut self, ui: &mut Ui, refs: Option<&RefsSnapshot>) {
        let current = refs.and_then(|r| r.head.branch_name()).unwrap_or("HEAD").to_owned();
        match self {
            Self::Confirm { message, .. } => {
                ui.label(message.as_str());
            }
            Self::CreateBranch { at_label, name, checkout, .. } => {
                ui.label(format!("Starting at {at_label}"));
                text(ui, name, "Branch name");
                ui.checkbox(checkout, "Check out after creation");
            }
            Self::RenameBranch { from, to } => {
                ui.label(format!("Rename {from} to:"));
                text(ui, to, "New name");
            }
            Self::CheckoutRemote { remote_branch, local_name } => {
                ui.label(format!("Create a local branch tracking {remote_branch}:"));
                text(ui, local_name, "Local branch name");
            }
            Self::CreateTag { at_label, name, message, .. } => {
                ui.label(format!("Tagging {at_label}"));
                text(ui, name, "Tag name");
                ui.add(TextEdit::multiline(message).hint_text("Message (optional, creates an annotated tag)").desired_rows(3));
            }
            Self::Stash { message, include_untracked, keep_index } => {
                text(ui, message, "Message (optional)");
                ui.checkbox(include_untracked, "Include untracked files");
                ui.checkbox(keep_index, "Keep staged changes");
            }
            Self::Pull { rebase } => {
                ui.label(format!("Pull into {current} from its upstream."));
                ui.checkbox(rebase, "Rebase instead of merge");
            }
            Self::Push { remote, branch, set_upstream, force, tags } => {
                egui::ComboBox::from_label("Remote").selected_text(remote.as_str()).show_ui(ui, |ui| {
                    for r in refs.into_iter().flat_map(|r| &r.remotes) {
                        ui.selectable_value(remote, r.name.clone(), &r.name);
                    }
                });
                ui.label(format!("Branch: {branch}"));
                ui.checkbox(set_upstream, "Track this remote branch");
                ui.checkbox(force, "Force push (with lease)");
                ui.checkbox(tags, "Push all tags");
            }
            Self::Merge { target, style } => {
                ui.label(format!("Merge {target} into {current}."));
                merge_style_choice(ui, style, &MergeStyle::ALL);
            }
            Self::Reset { target, mode } => {
                ui.label(format!("Reset {current} to {}:", furkst_core::short_id(target)));
                ui.radio_value(mode, ResetMode::Soft, "Soft — keep all changes staged");
                ui.radio_value(mode, ResetMode::Mixed, "Mixed — keep changes unstaged");
                ui.radio_value(mode, ResetMode::Hard, "Hard — discard all changes");
            }
            Self::Rebase(editor) => editor.fields(ui),
            Self::FlowInit { main, develop } => {
                ui.label("Releases are merged and tagged on the production branch; features branch from development.");
                text(ui, main, "Production branch");
                text(ui, develop, "Development branch (created if missing)");
            }
            Self::FlowFinish { kind, name, style } => {
                let flow = refs.and_then(|r| r.flow.as_ref());
                let (main, develop) = flow.map_or(("main", "develop"), |f| (f.main.as_str(), f.develop.as_str()));
                let branch = format!("{}{name}", flow.map_or("", |f| f.prefix(*kind)));
                if *kind == FlowKind::Feature {
                    ui.label(format!("Merge {branch} into {develop} and delete it."));
                    merge_style_choice(ui, style, &[MergeStyle::MergeCommit, MergeStyle::Squash, MergeStyle::Rebase]);
                } else {
                    let tag = format!("{}{name}", flow.map_or("", |f| f.version_tag_prefix.as_str()));
                    ui.label(format!("Merge {branch} into {main} with a merge commit, tag it {tag}, merge it into {develop}, and delete it."));
                }
            }
            Self::FlowStart { kind, name } => {
                let flow = refs.and_then(|r| r.flow.as_ref());
                let base = flow.map_or("", |f| f.base(*kind));
                let prefix = flow.map_or("", |f| f.prefix(*kind));
                ui.label(format!("Creates {prefix}{name} from {base}."));
                text(ui, name, if *kind == FlowKind::Feature { "Feature name" } else { "Version, e.g. 1.2.0" });
            }
            Self::AddRemote { name, url } => {
                text(ui, name, "Name");
                text(ui, url, "URL");
            }
            Self::AddWorktree { path, branch, create, start } => {
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(path).hint_text("Folder for the new worktree").desired_width(270.0));
                    if ui.button("Browse…").clicked()
                        && let Some(dir) = rfd::FileDialog::new().set_title("Parent folder of the worktree").pick_folder()
                    {
                        *path = dir.join(branch.replace('/', "-")).display().to_string();
                    }
                });
                ui.radio_value(create, true, "New branch");
                ui.radio_value(create, false, "Existing branch");
                if *create {
                    text(ui, branch, "New branch name");
                    text(ui, start, "Start point (default: HEAD)");
                } else {
                    egui::ComboBox::from_label("Branch").selected_text(branch.as_str()).show_ui(ui, |ui| {
                        for b in refs.into_iter().flat_map(|r| &r.branches) {
                            ui.selectable_value(branch, b.name.clone(), &b.name);
                        }
                    });
                }
            }
            Self::RemoveWorktree { path, force } => {
                ui.label(format!("Remove the worktree at {}?", path.display()));
                ui.checkbox(force, "Force (discard uncommitted changes, remove even if dirty or locked)");
            }
            Self::DeleteBranch { local, remote, delete_local, delete_remote } => {
                if let Some(local) = local {
                    ui.checkbox(delete_local, format!("Delete local branch {local}"));
                }
                if let Some((remote, branch)) = remote {
                    let label = if local.is_some() { "Also delete" } else { "Delete" };
                    ui.checkbox(delete_remote, format!("{label} {remote}/{branch} on the remote"));
                }
            }
        }
    }

    /// The action this dialog would submit, or `None` while the input is invalid.
    pub(super) fn action(&self) -> Option<Action> {
        let valid = |s: &str| is_valid_ref_name(s.trim()).then(|| s.trim().to_owned());
        Some(Action::Run(match self {
            Self::Confirm { op, .. } => op.clone(),
            Self::CreateBranch { at, name, checkout, .. } => {
                Operation::CreateBranch { name: valid(name)?, at: *at, checkout: *checkout }
            }
            Self::RenameBranch { from, to } => {
                Operation::RenameBranch { from: from.clone(), to: valid(to).filter(|to| to != from)? }
            }
            Self::CheckoutRemote { remote_branch, local_name } => {
                Operation::CheckoutRemoteBranch { remote_branch: remote_branch.clone(), local_name: valid(local_name)? }
            }
            Self::CreateTag { at, name, message, .. } => {
                Operation::CreateTag { name: valid(name)?, at: *at, message: Some(message.clone()) }
            }
            Self::Stash { message, include_untracked, keep_index } => Operation::StashSave {
                message: message.clone(),
                include_untracked: *include_untracked,
                keep_index: *keep_index,
            },
            Self::Pull { rebase } => Operation::Pull { rebase: *rebase },
            Self::Push { remote, branch, set_upstream, force, tags } => {
                if remote.is_empty() {
                    return None;
                }
                Operation::Push {
                    remote: remote.clone(),
                    branch: branch.clone(),
                    set_upstream: *set_upstream,
                    force: *force,
                    tags: *tags,
                }
            }
            Self::Merge { target, style } => Operation::Merge { target: target.clone(), style: *style },
            Self::FlowFinish { kind, name, style } => Operation::FlowFinish { kind: *kind, name: name.clone(), style: *style },
            Self::Reset { target, mode } => Operation::Reset { target: *target, mode: *mode },
            Self::Rebase(editor) => editor.operation().ok()?,
            Self::FlowInit { main, develop } => Operation::FlowInit { main: valid(main)?, develop: valid(develop)? },
            Self::FlowStart { kind, name } => Operation::FlowStart { kind: *kind, name: valid(name)? },
            Self::AddRemote { name, url } => {
                if url.trim().is_empty() {
                    return None;
                }
                Operation::AddRemote { name: valid(name)?, url: url.trim().to_owned() }
            }
            Self::AddWorktree { path, branch, create, start } => {
                if path.trim().is_empty() {
                    return None;
                }
                Operation::AddWorktree {
                    path: path.trim().into(),
                    branch: if *create { valid(branch)? } else { Some(branch.clone()).filter(|b| !b.is_empty())? },
                    create: *create,
                    start: Some(start.trim().to_owned()).filter(|s| !s.is_empty()),
                }
            }
            Self::RemoveWorktree { path, force } => Operation::RemoveWorktree { path: path.clone(), force: *force },
            Self::DeleteBranch { local, remote, delete_local, delete_remote } => {
                let local = local.clone().filter(|_| *delete_local);
                let remote = remote.clone().filter(|_| *delete_remote);
                if local.is_none() && remote.is_none() {
                    return None;
                }
                Operation::DeleteBranches { local, remote }
            }
        }))
    }
}

/// A single-line field. The first one in an open dialog takes keyboard focus, so typing works right away.
fn text(ui: &mut Ui, value: &mut String, hint: &str) {
    let response = ui.add(TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY));
    if ui.memory(|m| m.focused().is_none()) {
        response.request_focus();
    }
}

/// GitHub-style merge options as radio buttons, with an explanation of the chosen one.
fn merge_style_choice(ui: &mut Ui, style: &mut MergeStyle, options: &[MergeStyle]) {
    for &option in options {
        ui.radio_value(style, option, option.label());
    }
    ui.label(RichText::new(style.description()).weak().small());
}

/// A subset of `git check-ref-format` rules, enough to catch typos before git complains.
fn is_valid_ref_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['-', '/', '.'])
        && !name.ends_with(['/', '.'])
        && !name.ends_with(".lock")
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && !name.chars().any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
}

pub fn show(ctx: &egui::Context, dialog: &mut Dialog, refs: Option<&RefsSnapshot>) -> Outcome {
    let mut outcome = Outcome::Open;
    let response = egui::Modal::new(egui::Id::new("dialog")).show(ctx, |ui| {
        ui.set_width(dialog.width());
        ui.heading(dialog.title());
        ui.add_space(8.0);
        dialog.fields(ui, refs);
        ui.add_space(12.0);
        let action = dialog.action();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let can_submit = action.is_some();
            let submit = ui.add_enabled(can_submit, egui::Button::new(RichText::new(dialog.submit_label()).strong()));
            let enter = can_submit && !dialog.has_multiline_text() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
            if submit.clicked() || enter {
                outcome = action.map_or(Outcome::Open, Outcome::Submit);
            }
            if ui.button("Cancel").clicked() {
                outcome = Outcome::Cancel;
            }
        });
    });
    if response.should_close() && matches!(outcome, Outcome::Open) {
        outcome = Outcome::Cancel;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::is_valid_ref_name;

    #[test]
    fn ref_names() {
        assert!(is_valid_ref_name("feature/login-v2"));
        for bad in ["", "a b", "a..b", "-x", "x/", "x.lock", "a~1", "a:b", "@{x"] {
            assert!(!is_valid_ref_name(bad), "{bad}");
        }
    }
}
