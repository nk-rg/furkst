//! The repository tab's contents for the Quick Launch palette (Ctrl/Cmd+P).

use std::path::{Path, PathBuf};

use furkst_core::{Operation, RefsSnapshot};

use super::{Action, Dialog, Scope, View};
use crate::palette::{self, Item};
use crate::settings::Settings;

fn command(label: &str, action: Action) -> Item<Action> {
    Item::new(label, action).detail("command")
}

/// Commands, refs and recent repositories; tracked files follow via [`file_items`] once loaded.
pub fn items(refs: Option<&RefsSnapshot>, settings: &Settings, workdir: &Path) -> Vec<Item<Action>> {
    let mut items = vec![command("Fetch", Action::fetch_all(settings)), command("Pull…", Action::Open(Dialog::pull(settings)))];
    if let Some(push) = refs.and_then(Dialog::push_current) {
        items.push(command("Push…", Action::Open(push)));
    }
    items.push(command("Stash…", Action::Open(Dialog::stash())));
    if let Some(branch) = refs.and_then(Dialog::create_branch_at_head) {
        items.push(command("New branch…", Action::Open(branch)));
    }
    items.extend([
        command("Refresh", Action::Refresh),
        command("Local Changes", Action::ShowView(View::LocalChanges)),
        command("All Commits", Action::SetScope(Scope::All)),
        command("Reflog", Action::ShowReflog),
        command("Open in terminal", Action::OpenTerminal),
        command("Open in file manager", Action::OpenPath(PathBuf::new())),
    ]);

    let Some(refs) = refs else { return items };
    if refs.uses_lfs {
        items.extend([command("Git LFS pull", Action::Run(Operation::LfsPull)), command("Git LFS fetch", Action::Run(Operation::LfsFetch))]);
    }
    for branch in refs.branches.iter().filter(|b| !b.is_head) {
        let target = branch.name.clone();
        items.push(Item::new(format!("Checkout {target}"), Action::Run(Operation::Checkout { target })).detail("branch"));
    }
    for branch in refs.remotes.iter().flat_map(|r| &r.branches) {
        let dialog = Dialog::CheckoutRemote { remote_branch: branch.full_name(), local_name: branch.name.clone() };
        items.push(Item::new(format!("Checkout {}…", branch.full_name()), Action::Open(dialog)).detail("remote branch"));
        items.push(Item::new(format!("Show {}", branch.full_name()), Action::SelectCommit(branch.target)).detail("remote branch"));
    }
    for branch in &refs.branches {
        items.push(Item::new(format!("Show {}", branch.name), Action::SelectCommit(branch.target)).detail("branch"));
    }
    for tag in &refs.tags {
        items.push(Item::new(format!("Show {}", tag.name), Action::SelectCommit(tag.target)).detail("tag"));
    }
    let others = settings.known_repositories().into_iter().filter(|p| p.as_path() != workdir);
    items.extend(others.map(|path| palette::open_repo_item(path, Action::OpenRepository(path.clone()))));
    items
}

pub fn file_items(paths: Vec<String>) -> impl Iterator<Item = Item<Action>> {
    paths.into_iter().map(|path| Item::new(format!("File history: {path}"), Action::FileHistory(path)))
}
