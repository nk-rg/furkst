//! The list of changed files, shared by commit details and local changes.

use egui::{Key, Modifiers, Ui};
use furkst_core::{DiffTarget, FileChange};

use super::Action;
use super::diff_view::DiffPanel;
use super::selection::Selection;
use crate::widgets;

/// Multi-selection state for a list, owned by the caller so it outlives frames.
pub struct Multi<'a> {
    pub selection: &'a mut Selection,
    /// Whether this list receives keyboard shortcuts (Ctrl+A, arrows, Enter/Space).
    pub keyboard: bool,
}

#[derive(Default)]
pub struct Outcome {
    /// Files to act on after a double-click or Enter/Space.
    pub activated: Vec<FileChange>,
    /// The user clicked or typed in this list, so it is the focused one.
    pub interacted: bool,
    /// A file was clicked (not right-clicked) to see it.
    pub opened: bool,
}

/// The files of `files` that are in `selection`, in list order.
pub fn selected(files: &[FileChange], selection: &Selection) -> Vec<FileChange> {
    files.iter().filter(|c| selection.contains(&c.path)).cloned().collect()
}

/// Renders `files`, in a tree of their folders (folders first, like the File Tree tab) with `tree`, or as a flat list. Without
/// `multi`, clicking selects a file's diff; with it, rows follow the usual click/ctrl/shift/keyboard selection rules and
/// a single selected file shows its diff. `menu` fills each row's context menu with the files it applies to (the whole
/// selection when the row is part of it).
#[allow(clippy::too_many_arguments)]
pub fn list(
    ui: &mut Ui,
    files: &[FileChange],
    target: &DiffTarget,
    diff: &DiffPanel,
    mut multi: Option<Multi>,
    tree: bool,
    actions: &mut Vec<Action>,
    mut menu: impl FnMut(&mut Ui, &[FileChange], &mut Vec<Action>),
) -> Outcome {
    let mut outcome = Outcome::default();
    let mut changed = false;
    let mut scroll_to = None;
    let mut shown: Vec<&FileChange> = files.iter().collect();
    if tree {
        shown.sort_by(|a, b| super::file_tree::tree_order(&a.path, &b.path));
    }
    // Keyboard steps follow the rows as shown.
    let order: Vec<&str> = shown.iter().map(|c| c.path.as_str()).collect();

    if let Some(Multi { selection, keyboard: true }) = &mut multi {
        let (step, all, activate) = ui.input_mut(|i| {
            let step = [(Key::ArrowUp, -1), (Key::ArrowDown, 1)].into_iter().find_map(|(key, delta)| {
                if i.consume_key(Modifiers::SHIFT, key) {
                    Some((delta, true))
                } else {
                    i.consume_key(Modifiers::NONE, key).then_some((delta, false))
                }
            });
            let activate = i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Space);
            (step, i.consume_key(Modifiers::COMMAND, Key::A), activate)
        });
        if let Some((delta, extend)) = step {
            scroll_to = selection.step(&order, delta, extend);
            changed = true;
        } else if all {
            selection.select_all(&order);
            changed = true;
        }
        if activate {
            outcome.activated = selected(files, selection);
        }
        outcome.interacted = step.is_some() || all || activate;
    }

    ui.spacing_mut().item_spacing.y = 0.0;
    let mut row = |ui: &mut Ui, change: &FileChange| {
        let is_selected = match &multi {
            Some(m) => m.selection.contains(&change.path),
            None => diff.is_showing(target, &change.path),
        };
        let response = widgets::file_row(ui, change, is_selected, !tree, None);
        if scroll_to == Some(change.path.as_str()) {
            response.scroll_to_me(None);
        }
        let modifiers = ui.input(|i| i.modifiers);
        if response.double_clicked() {
            outcome.interacted = true;
            outcome.activated = match &multi {
                Some(m) => selected(files, m.selection),
                None => vec![change.clone()],
            };
        } else if response.clicked() || (response.secondary_clicked() && !is_selected) {
            outcome.interacted = true;
            outcome.opened |= response.clicked();
            match &mut multi {
                Some(m) if response.clicked() && modifiers.command => m.selection.toggle(&change.path),
                Some(m) if response.clicked() && modifiers.shift => m.selection.extend_to(&order, &change.path),
                Some(m) => m.selection.select_only(&change.path),
                None => actions.push(Action::SelectFile(target.clone(), change.clone())),
            }
            changed = true;
        }
        response.context_menu(|ui| {
            let targets = match &multi {
                Some(m) if m.selection.contains(&change.path) => selected(files, m.selection),
                _ => vec![change.clone()],
            };
            menu(ui, &targets, actions);
        });
    };
    if tree {
        // The diff target names the list, so each list (and each commit) keeps its own folders open or closed.
        widgets::folder_tree(ui, &format!("{target:?}"), &shown, 0, &|_| true, &|c| c.path.as_str(), &mut |ui, _, c| row(ui, c));
    } else {
        shown.iter().for_each(|change| row(ui, change));
    }

    if changed
        && let Some(m) = &multi
        && let Some(path) = m.selection.single()
        && let Some(change) = files.iter().find(|c| c.path == path)
    {
        actions.push(Action::SelectFile(target.clone(), change.clone()));
    }
    outcome
}
