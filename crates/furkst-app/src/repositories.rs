//! The start page's repository list: named groups and recent repositories.

use std::path::{Path, PathBuf};

use egui::{CollapsingHeader, RichText, Ui};

use crate::settings::{RepoGroup, Settings};
use crate::widgets::{self, menu_item as item};

/// A group name being typed: for a new group (optionally created around a repository) or a rename.
struct Naming {
    group: Option<usize>,
    with_repo: Option<PathBuf>,
    name: String,
    /// Whether the field still has to grab keyboard focus (only on its first frame).
    focus: bool,
}

#[derive(Default)]
pub struct RepositoryList {
    naming: Option<Naming>,
}

/// A change requested while drawing, applied afterwards so the lists aren't modified while iterated.
enum Edit {
    Open(PathBuf),
    AddToGroup(PathBuf, usize),
    RemoveFromGroup(PathBuf, usize),
    Forget(PathBuf),
    StartNaming(Option<usize>, Option<PathBuf>),
    DeleteGroup(usize),
}

/// Draws the groups and the ungrouped recent repositories; returns a repository to open.
pub fn show(ui: &mut Ui, list: &mut RepositoryList, settings: &mut Settings) -> Option<PathBuf> {
    let mut edits = Vec::new();
    let names: Vec<String> = settings.groups.iter().map(|g| g.name.clone()).collect();

    for (g, group) in settings.groups.iter().enumerate() {
        let title = RichText::new(format!("{}  {}", group.name.to_uppercase(), group.repos.len())).small().strong();
        let header = CollapsingHeader::new(title).id_salt(("repo-group", g)).default_open(true).show(ui, |ui| {
            if group.repos.is_empty() {
                ui.label(RichText::new("Empty: right-click a repository and choose Add to group").weak().small());
            }
            for path in &group.repos {
                repo_row(ui, path, Some(g), &names, &mut edits);
            }
        });
        header.header_response.context_menu(|ui| {
            item(ui, "Rename…", &mut edits, || Edit::StartNaming(Some(g), None));
            item(ui, "Delete group", &mut edits, || Edit::DeleteGroup(g));
        });
    }

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        widgets::section_label(ui, "Recent repositories");
        if ui.small_button("+ Group").on_hover_text("Create a group of repositories").clicked() {
            edits.push(Edit::StartNaming(None, None));
        }
    });
    let grouped: Vec<&PathBuf> = settings.groups.iter().flat_map(|g| &g.repos).collect();
    for path in settings.recent.iter().filter(|p| !grouped.contains(p)) {
        repo_row(ui, path, None, &names, &mut edits);
    }
    naming_row(ui, list, settings);

    let mut open = None;
    let mut changed = false;
    for edit in edits {
        changed |= apply(edit, list, settings, &mut open);
    }
    if changed {
        settings.save();
    }
    open
}

/// Applies `edit`; returns whether the settings changed and need saving.
fn apply(edit: Edit, list: &mut RepositoryList, settings: &mut Settings, open: &mut Option<PathBuf>) -> bool {
    match edit {
        Edit::Open(path) => {
            *open = Some(path);
            return false;
        }
        Edit::StartNaming(group, with_repo) => {
            let name = group.map(|g| settings.groups[g].name.clone()).unwrap_or_default();
            list.naming = Some(Naming { group, with_repo, name, focus: true });
            return false;
        }
        Edit::AddToGroup(path, g) => {
            settings.groups.iter_mut().for_each(|group| group.repos.retain(|p| p != &path));
            settings.groups[g].repos.push(path);
        }
        Edit::RemoveFromGroup(path, g) => {
            settings.groups[g].repos.retain(|p| p != &path);
            keep_in_recent(settings, path);
        }
        Edit::Forget(path) => {
            settings.recent.retain(|p| p != &path);
            settings.groups.iter_mut().for_each(|group| group.repos.retain(|p| p != &path));
        }
        Edit::DeleteGroup(g) => {
            for path in settings.groups.remove(g).repos {
                keep_in_recent(settings, path);
            }
        }
    }
    true
}

/// Keeps a repository that leaves a group listed under the recent ones.
fn keep_in_recent(settings: &mut Settings, path: PathBuf) {
    if !settings.recent.contains(&path) {
        settings.recent.push(path);
    }
}

/// One repository: its name (click to open), its path, and a menu to file it into groups.
fn repo_row(ui: &mut Ui, path: &Path, group: Option<usize>, names: &[String], edits: &mut Vec<Edit>) {
    ui.horizontal(|ui| {
        let response = ui.link(RichText::new(widgets::path_name(path)).strong());
        if response.clicked() {
            edits.push(Edit::Open(path.to_path_buf()));
        }
        ui.label(RichText::new(path.display().to_string()).weak().small());
        let (remove, hint) = match group {
            Some(g) => (Edit::RemoveFromGroup(path.to_path_buf(), g), "Remove from this group"),
            None => (Edit::Forget(path.to_path_buf()), "Remove from list"),
        };
        if ui.small_button("×").on_hover_text(hint).clicked() {
            edits.push(remove);
        }
        response.context_menu(|ui| {
            ui.menu_button("Add to group", |ui| {
                for (g, name) in names.iter().enumerate().filter(|(g, _)| Some(*g) != group) {
                    item(ui, name, edits, || Edit::AddToGroup(path.to_path_buf(), g));
                }
                item(ui, "New group…", edits, || Edit::StartNaming(None, Some(path.to_path_buf())));
            });
            if let Some(g) = group {
                item(ui, "Remove from group", edits, || Edit::RemoveFromGroup(path.to_path_buf(), g));
            }
            item(ui, "Remove from list", edits, || Edit::Forget(path.to_path_buf()));
        });
    });
}

/// The inline name field for a new or renamed group.
fn naming_row(ui: &mut Ui, list: &mut RepositoryList, settings: &mut Settings) {
    let Some(naming) = &mut list.naming else { return };
    let mut done = None;
    ui.horizontal(|ui| {
        let field = ui.add(egui::TextEdit::singleline(&mut naming.name).hint_text("Group name").desired_width(220.0));
        if std::mem::take(&mut naming.focus) {
            field.request_focus();
        }
        let name = naming.name.trim();
        let enter = (field.has_focus() || field.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let submit = !name.is_empty() && (ui.add_enabled(!name.is_empty(), egui::Button::new("Save")).clicked() || enter);
        if submit {
            done = Some(true);
        } else if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = Some(false);
        }
    });
    match done {
        Some(true) => {
            let Naming { group, with_repo, name, .. } = list.naming.take().expect("naming is active");
            let name = name.trim().to_owned();
            match group {
                Some(g) => settings.groups[g].name = name,
                None => settings.groups.push(RepoGroup { name, repos: with_repo.into_iter().collect() }),
            }
            settings.save();
        }
        Some(false) => list.naming = None,
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        let p = PathBuf::from;
        Settings {
            recent: vec![p("/a"), p("/b"), p("/c")],
            groups: vec![RepoGroup { name: "Work".into(), repos: vec![p("/a")] }, RepoGroup { name: "Fun".into(), repos: vec![] }],
            ..Settings::default()
        }
    }

    fn run(settings: &mut Settings, edit: Edit) {
        apply(edit, &mut RepositoryList::default(), settings, &mut None);
    }

    #[test]
    fn moving_a_repository_between_groups() {
        let mut s = settings();
        run(&mut s, Edit::AddToGroup("/a".into(), 1));
        assert!(s.groups[0].repos.is_empty());
        assert_eq!(s.groups[1].repos, [PathBuf::from("/a")]);
    }

    #[test]
    fn leaving_a_group_keeps_the_repository_listed() {
        let mut s = settings();
        s.recent.retain(|p| p != Path::new("/a"));
        run(&mut s, Edit::RemoveFromGroup("/a".into(), 0));
        assert!(s.groups[0].repos.is_empty() && s.recent.contains(&"/a".into()));
        run(&mut s, Edit::AddToGroup("/b".into(), 1));
        run(&mut s, Edit::DeleteGroup(1));
        assert_eq!(s.groups.len(), 1);
        assert!(s.recent.contains(&"/b".into()));
    }

    #[test]
    fn forgetting_removes_it_everywhere() {
        let mut s = settings();
        run(&mut s, Edit::Forget("/a".into()));
        assert!(!s.recent.contains(&"/a".into()) && s.groups[0].repos.is_empty());
        assert_eq!(s.known_repositories(), [&PathBuf::from("/b"), &PathBuf::from("/c")]);
    }
}
