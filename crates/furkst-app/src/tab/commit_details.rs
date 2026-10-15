//! Details of the selected commit: metadata, changed files and the diff.

use std::collections::BTreeMap;

use egui::{RichText, Ui};
use furkst_core::{CommitDetails, DiffTarget, FileChange, Oid, RefsSnapshot};

use super::diff_view::DiffPanel;
use super::file_tree::DetailsTab;
use super::{Action, RepoTab, files, menus};
use crate::settings::Settings;
use crate::widgets;

pub fn show(ui: &mut Ui, tab: &mut RepoTab, settings: &mut Settings, actions: &mut Vec<Action>) {
    let (Some(details), Some(refs)) = (&tab.details, &tab.refs) else {
        let text = if tab.history.selected.is_some() { "Loading…" } else { "Select a commit" };
        return widgets::placeholder(ui, text);
    };
    // The commit itself, its changes with their diff, or all of its files.
    tab_row(ui, tab, settings, actions);
    ui.separator();
    match tab.tree.tab {
        DetailsTab::Commit => {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                header(ui, details, refs, actions);
                ui.separator();
                expandable_files(ui, details, &mut tab.inline_diffs, settings, actions);
            });
        }
        DetailsTab::Changes => {
            widgets::left_panel(ui, "commit_files", 380.0).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| changed_files(ui, details, &tab.diff, actions));
            });
            egui::CentralPanel::no_frame().show(ui, |ui| tab.diff.show(ui, settings, Action::SelectFile, Some(false), actions));
        }
        DetailsTab::FileTree => {
            widgets::left_panel(ui, "commit_tree", 380.0).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| tab.tree.list(ui, details.info.id, actions));
            });
            egui::CentralPanel::no_frame().show(ui, |ui| tab.tree.show_content(ui));
        }
    }
}

/// The files the commit changed, in a tree of their folders; returns whether one was clicked to see it.
pub fn changed_files(ui: &mut Ui, details: &CommitDetails, diff: &DiffPanel, actions: &mut Vec<Action>) -> bool {
    let mut opened = false;
    for (label, commit, changes) in sections(details) {
        widgets::section_label(ui, &format!("{} {label}", changes.len()));
        let target = DiffTarget::Commit(commit);
        let outcome = files::list(ui, changes, &target, diff, None, true, actions, |ui, targets, actions| {
            menus::file(ui, &targets[0].path, Some(commit), actions);
        });
        opened |= outcome.opened;
    }
    opened
}

fn header(ui: &mut Ui, details: &CommitDetails, refs: &RefsSnapshot, actions: &mut Vec<Action>) {
    let info = &details.info;
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        widgets::avatar(ui, &info.author, &info.committer, 36.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(&info.author.name).strong());
            ui.label(RichText::new(format!("{} · {}", info.author.email, widgets::format_date(&info.author))).weak());
        });
    });
    if info.committer.email != info.author.email || info.committer.name != info.author.name {
        ui.label(RichText::new(format!("Committed by {} · {}", info.committer.name, widgets::format_date(&info.committer))).weak());
    }
    ui.add_space(6.0);
    egui::Grid::new("commit_meta").num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
        ui.label(RichText::new("SHA").weak());
        widgets::copyable(ui, RichText::new(info.id.to_string()).monospace(), &info.id.to_string());
        ui.end_row();
        if !info.parents.is_empty() {
            ui.label(RichText::new("Parents").weak());
            ui.horizontal(|ui| {
                for parent in &info.parents {
                    if ui.link(RichText::new(furkst_core::short_id(parent)).monospace()).clicked() {
                        actions.push(Action::SelectCommit(*parent));
                    }
                }
            });
            ui.end_row();
        }
        if !refs.labels_for(&info.id).is_empty() {
            ui.label(RichText::new("Refs").weak());
            ui.horizontal_wrapped(|ui| menus::ref_badges(ui, info.id, refs, actions));
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new(&info.summary).strong().size(14.0));
    let body = info.body();
    if !body.is_empty() {
        ui.add_space(2.0);
        ui.label(body);
    }
    ui.add_space(4.0);
}

/// The Commit, Changes and File Tree tabs, with a button at the end that collapses the details to just this row, or
/// expands them again (picking a tab expands them too).
pub fn tab_row(ui: &mut Ui, tab: &RepoTab, settings: &mut Settings, actions: &mut Vec<Action>) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let before = actions.len();
        tab.tree.tabs(ui, actions);
        if actions.len() > before {
            settings.details_collapsed = false;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (icon, tip) = if settings.details_collapsed { ("⬆", "Show the details") } else { ("⬇", "Hide the details") };
            if ui.small_button(icon).on_hover_text(tip).clicked() {
                settings.details_collapsed = !settings.details_collapsed;
            }
        });
    });
}

/// The commit's changed files, and for a stash the untracked files it saved: those diff against the commit of their
/// own that a stash keeps them in.
fn sections(details: &CommitDetails) -> impl Iterator<Item = (&'static str, Oid, &Vec<FileChange>)> {
    let untracked = details.stash_untracked.as_ref().map(|(id, files)| ("untracked files", *id, files));
    [("changed files", details.info.id, &details.changes)].into_iter().chain(untracked)
}

/// The Commit tab's files, one under another, each opening (by its arrow or its name) to show its diff in place.
fn expandable_files(
    ui: &mut Ui,
    details: &CommitDetails,
    opened: &mut BTreeMap<String, DiffPanel>,
    settings: &Settings,
    actions: &mut Vec<Action>,
) {
    for (label, commit, changes) in sections(details) {
        widgets::section_label(ui, &format!("{} {label}", changes.len()));
        let target = DiffTarget::Commit(commit);
        for change in changes {
            let diff = opened.get_mut(&change.path);
            let response = widgets::file_row(ui, change, false, true, Some(diff.is_some()));
            if response.clicked() {
                actions.push(Action::ToggleInlineDiff(target.clone(), change.clone()));
            }
            response.context_menu(|ui| menus::file(ui, &change.path, Some(commit), actions));
            if let Some(diff) = diff {
                ui.indent(("inline-diff", &change.path), |ui| diff.show_inline(ui, settings, actions));
                ui.add_space(6.0);
            }
        }
    }
}
