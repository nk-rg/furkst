//! Context menus, shared by every place a commit, branch, tag or stash appears.

use egui::Ui;
use furkst_core::{
    Branch, CommitInfo, Operation, RefKind, RefsSnapshot, Remote, RemoteBranch, ResetMode, Stash, Submodule, Tag, Worktree,
};

use super::{Action, Dialog, Scope};

pub(super) use crate::widgets::menu_item as item;

fn copy(ui: &mut Ui, label: &str, value: String) {
    if ui.button(label).clicked() {
        ui.ctx().copy_text(value);
        ui.close();
    }
}

fn current_branch(refs: &RefsSnapshot) -> String {
    refs.head.branch_name().unwrap_or("HEAD").to_owned()
}

/// Entries for anything that can be merged into or rebased onto from the current branch.
/// `label` names `target` in menu text, e.g. a short SHA for a full one.
pub(super) fn integrate(ui: &mut Ui, label: &str, target: &str, refs: &RefsSnapshot, actions: &mut Vec<Action>) {
    let current = current_branch(refs);
    item(ui, format!("Merge {label} into {current}…"), actions, || {
        Action::Open(Dialog::Merge { target: target.to_owned(), style: furkst_core::MergeStyle::default() })
    });
    item(ui, format!("Rebase {current} onto {label}"), actions, || Action::Confirm {
        title: "Rebase".into(),
        message: format!("Rebase {current} onto {label}?"),
        op: Operation::Rebase { onto: target.to_owned() },
    });
}

pub fn commit(ui: &mut Ui, commit: &CommitInfo, refs: &RefsSnapshot, actions: &mut Vec<Action>) {
    let (id, short) = (commit.id, commit.short_id());
    item(ui, format!("Checkout {short}…"), actions, || detached_checkout("commit", &short, id.to_string()));
    ui.separator();
    item(ui, "Create branch here…", actions, || Action::Open(Dialog::create_branch(id, &short)));
    item(ui, "Create tag here…", actions, || Action::Open(Dialog::create_tag(id, &short)));
    ui.separator();
    item(ui, "Cherry-pick commit", actions, || Action::Run(Operation::CherryPick { commit: id }));
    item(ui, "Revert commit", actions, || Action::Run(Operation::Revert { commit: id }));
    integrate(ui, &short, &id.to_string(), refs, actions);
    item(ui, "Interactive rebase from here…", actions, || Action::InteractiveRebase(Some(id)));
    if commit.parents.is_empty() {
        item(ui, "Interactive rebase from the root…", actions, || Action::InteractiveRebase(None));
    }
    item(ui, format!("Reset {} to here…", current_branch(refs)), actions, || {
        Action::Open(Dialog::Reset { target: id, mode: ResetMode::Mixed })
    });
    ui.separator();
    copy(ui, "Copy SHA", id.to_string());
    copy(ui, "Copy message", commit.message.clone());
}

pub fn branch(ui: &mut Ui, branch: &Branch, refs: &RefsSnapshot, actions: &mut Vec<Action>) {
    let name = branch.name.clone();
    if !branch.is_head {
        item(ui, format!("Checkout {name}"), actions, || Action::Run(Operation::Checkout { target: name.clone() }));
        integrate(ui, &name, &name, refs, actions);
        ui.separator();
    }
    item(ui, "Show only this branch", actions, || {
        Action::SetScope(Scope::Only { label: name.clone(), tips: vec![branch.target] })
    });
    item(ui, "New branch from here…", actions, || Action::Open(Dialog::create_branch(branch.target, &name)));
    item(ui, "Create tag here…", actions, || Action::Open(Dialog::create_tag(branch.target, &name)));
    if !branch.is_head {
        item(ui, "Check out in new worktree…", actions, || Action::Open(Dialog::add_worktree(Some(&name), &name)));
    }
    ui.separator();
    if !refs.remotes.is_empty() {
        item(ui, format!("Push {name}…"), actions, || Action::Open(Dialog::push(refs, branch)));
    }
    if branch.upstream.is_some() {
        item(ui, "Stop tracking upstream", actions, || {
            Action::Run(Operation::SetUpstream { branch: name.clone(), upstream: None })
        });
    }
    item(ui, "Rename…", actions, || Action::Open(Dialog::RenameBranch { from: name.clone(), to: name.clone() }));
    if !branch.is_head {
        item(ui, "Delete…", actions, || Action::Open(Dialog::delete_local(refs, &name)));
    }
    ui.separator();
    copy(ui, "Copy branch name", name.clone());
}

pub fn remote_branch(ui: &mut Ui, branch: &RemoteBranch, refs: &RefsSnapshot, actions: &mut Vec<Action>) {
    let full = branch.full_name();
    let exists_locally = refs.branches.iter().any(|b| b.name == branch.name);
    if !exists_locally {
        item(ui, format!("Checkout {full}…"), actions, || {
            Action::Open(Dialog::CheckoutRemote { remote_branch: full.clone(), local_name: branch.name.clone() })
        });
    }
    integrate(ui, &full, &full, refs, actions);
    if let Some(current) = refs.head.branch_name() {
        item(ui, format!("Track {full} from {current}"), actions, || {
            Action::Run(Operation::SetUpstream { branch: current.to_owned(), upstream: Some(full.clone()) })
        });
    }
    ui.separator();
    item(ui, "Show only this branch", actions, || {
        Action::SetScope(Scope::Only { label: full.clone(), tips: vec![branch.target] })
    });
    item(ui, "New branch from here…", actions, || Action::Open(Dialog::create_branch(branch.target, &full)));
    ui.separator();
    item(ui, format!("Delete {full} on remote…"), actions, || Action::Open(Dialog::delete_remote(refs, branch)));
    copy(ui, "Copy branch name", full.clone());
}

pub fn tag(ui: &mut Ui, tag: &Tag, refs: &RefsSnapshot, actions: &mut Vec<Action>) {
    let name = tag.name.clone();
    item(ui, format!("Checkout {name}…"), actions, || detached_checkout("tag", &name, format!("refs/tags/{name}")));
    item(ui, "New branch from here…", actions, || Action::Open(Dialog::create_branch(tag.target, &name)));
    ui.separator();
    for remote in &refs.remotes {
        item(ui, format!("Push to {}", remote.name), actions, || {
            Action::Run(Operation::PushTag { remote: remote.name.clone(), tag: name.clone() })
        });
    }
    item(ui, "Delete…", actions, || Action::Confirm {
        title: "Delete tag".into(),
        message: format!("Delete local tag {name}?"),
        op: Operation::DeleteTag { name: name.clone() },
    });
    for remote in &refs.remotes {
        item(ui, format!("Delete from {}…", remote.name), actions, || Action::Confirm {
            title: "Delete remote tag".into(),
            message: format!("Delete tag {name} from {}?", remote.name),
            op: Operation::DeleteRemoteTag { remote: remote.name.clone(), tag: name.clone() },
        });
    }
    ui.separator();
    copy(ui, "Copy tag name", name.clone());
}

pub fn stash(ui: &mut Ui, stash: &Stash, actions: &mut Vec<Action>) {
    let index = stash.index;
    item(ui, "Apply", actions, || Action::Run(Operation::StashApply { index, pop: false }));
    item(ui, "Pop", actions, || Action::Run(Operation::StashApply { index, pop: true }));
    item(ui, "Delete…", actions, || Action::Confirm {
        title: "Delete stash".into(),
        message: format!("Delete {}? This cannot be undone.", stash.reference()),
        op: Operation::StashDrop { index },
    });
}

pub fn worktree(ui: &mut Ui, worktree: &Worktree, actions: &mut Vec<Action>) {
    let path = worktree.path.clone();
    if !worktree.is_current {
        item(ui, "Open in new tab", actions, || Action::OpenRepository(path.clone()));
    }
    item(ui, "Show in file manager", actions, || Action::Reveal(path.clone()));
    if !worktree.is_main && !worktree.is_current {
        item(ui, "Remove…", actions, || Action::Open(Dialog::RemoveWorktree { path: path.clone(), force: false }));
    }
    ui.separator();
    copy(ui, "Copy path", path.display().to_string());
}

/// Entries for a file in the working copy or a commit.
pub fn file(ui: &mut Ui, path: &str, blame_at: Option<furkst_core::Oid>, actions: &mut Vec<Action>) {
    item(ui, "Open file", actions, || Action::OpenPath(path.into()));
    item(ui, "Show in file manager", actions, || Action::Reveal(path.into()));
    ui.separator();
    item(ui, "File history", actions, || Action::FileHistory(path.to_owned()));
    if let Some(at) = blame_at {
        item(ui, "Blame at this commit", actions, || Action::Blame { path: path.to_owned(), at });
    }
    copy(ui, "Copy path", path.to_owned());
}

pub fn remote(ui: &mut Ui, remote: &Remote, actions: &mut Vec<Action>) {
    item(ui, "Fetch", actions, || Action::Run(Operation::Fetch { remote: Some(remote.name.clone()), prune: true }));
    copy(ui, "Copy URL", remote.url.clone().unwrap_or_default());
    item(ui, "Remove remote…", actions, || Action::Confirm {
        title: "Remove remote".into(),
        message: format!("Remove remote {} and its remote-tracking branches?", remote.name),
        op: Operation::RemoveRemote { name: remote.name.clone() },
    });
}

pub fn remotes_header(ui: &mut Ui, actions: &mut Vec<Action>) {
    item(ui, "Add remote…", actions, || Action::Open(Dialog::AddRemote { name: "origin".into(), url: String::new() }));
}

pub fn worktrees_header(ui: &mut Ui, refs: &RefsSnapshot, actions: &mut Vec<Action>) {
    let start = refs.head.branch_name().unwrap_or("HEAD");
    item(ui, "Add worktree…", actions, || Action::Open(Dialog::add_worktree(None, start)));
    item(ui, "Prune stale worktrees", actions, || Action::Run(Operation::PruneWorktrees));
}

pub fn submodule(ui: &mut Ui, submodule: &Submodule, actions: &mut Vec<Action>) {
    item(ui, "Open folder", actions, || Action::OpenPath(submodule.path.clone().into()));
    item(ui, "Update all submodules", actions, || Action::Run(Operation::UpdateSubmodules));
}

/// The ref badges of commit `id`. Clicking a badge selects the commit; right-clicking a branch or tag badge opens
/// its menu, so refs can be checked out, merged, pushed or deleted (including remote branches) from the graph.
pub fn ref_badges(ui: &mut Ui, id: furkst_core::Oid, refs: &RefsSnapshot, actions: &mut Vec<Action>) {
    for label in refs.labels_for(&id) {
        // A remote branch at the same commit as its local branch is shown by the local one alone.
        if label.kind == RefKind::RemoteBranch && refs.local_counterpart(&label.name).is_some_and(|local| local.target == id) {
            continue;
        }
        let mut response = crate::widgets::ref_badge(ui, label);
        if label.kind == RefKind::LocalBranch
            && let Some(remote) = refs.remote_counterpart(&label.name).filter(|remote| remote.target == id)
        {
            response = response.on_hover_text(format!("Same as {}", remote.full_name()));
        }
        click_ref(&response, label.kind, &label.name, id, refs, actions);
        response.context_menu(|ui| ref_menu(ui, label, refs, actions));
    }
}

fn ref_menu(ui: &mut Ui, label: &furkst_core::RefLabel, refs: &RefsSnapshot, actions: &mut Vec<Action>) {
    match label.kind {
        RefKind::LocalBranch => {
            if let Some(b) = refs.branches.iter().find(|b| b.name == label.name) {
                branch(ui, b, refs, actions);
            }
        }
        RefKind::RemoteBranch => {
            if let Some(b) = refs.remotes.iter().flat_map(|r| &r.branches).find(|b| b.full_name() == label.name) {
                remote_branch(ui, b, refs, actions);
            }
        }
        RefKind::Tag => {
            if let Some(t) = refs.tags.iter().find(|t| t.name == label.name) {
                tag(ui, t, refs, actions);
            }
        }
        RefKind::Head | RefKind::Stash => {
            ui.label(egui::RichText::new(&label.name).weak());
        }
    }
}

/// Entries for a file in a commit's file tree.
pub fn tree_file(ui: &mut Ui, path: &str, at: furkst_core::Oid, actions: &mut Vec<Action>) {
    file(ui, path, Some(at), actions);
    item(ui, "Save as…", actions, || Action::SaveFileAs { path: path.to_owned(), at });
}

/// The git-flow menu: set it up, start branches, or finish the flow branch that is checked out.
pub fn flow(ui: &mut Ui, refs: &RefsSnapshot, actions: &mut Vec<Action>) {
    let Some(flow) = &refs.flow else {
        let main = ["main", "master"]
            .into_iter()
            .find(|name| refs.branches.iter().any(|b| b.name == *name))
            .or(refs.head.branch_name())
            .unwrap_or("main");
        item(ui, "Initialize git-flow…", actions, || Action::Open(Dialog::FlowInit { main: main.into(), develop: "develop".into() }));
        return;
    };
    for kind in furkst_core::FlowKind::ALL {
        item(ui, format!("Start {}…", kind.label()), actions, || Action::Open(Dialog::FlowStart { kind, name: String::new() }));
    }
    if let Some((kind, name)) = refs.head.branch_name().and_then(|branch| flow.classify(branch)) {
        ui.separator();
        item(ui, format!("Finish {} {name}…", kind.label()), actions, || {
            Action::Open(Dialog::FlowFinish { kind, name: name.clone(), style: furkst_core::MergeStyle::MergeCommit })
        });
    }
}

/// Checking out anything but a local branch leaves HEAD detached, so it asks first.
fn detached_checkout(what: &str, label: &str, target: String) -> Action {
    Action::Confirm {
        title: format!("Checkout {what}"),
        message: format!("Checking out {label} will leave you in 'detached HEAD' state. Continue?"),
        op: Operation::Checkout { target },
    }
}

/// What double-clicking a ref does: switch to a local branch; for a remote branch, switch to its local
/// branch or create one tracking it; a tag checks out detached after asking. `None` when it's already checked out.
pub fn checkout_ref(kind: RefKind, name: &str, refs: &RefsSnapshot) -> Option<Action> {
    let switch = |branch: &str| {
        (refs.head.branch_name() != Some(branch)).then(|| Action::Run(Operation::Checkout { target: branch.to_owned() }))
    };
    match kind {
        RefKind::LocalBranch => switch(name),
        RefKind::RemoteBranch => {
            let remote = refs.remotes.iter().flat_map(|r| &r.branches).find(|b| b.full_name() == name)?;
            if refs.branches.iter().any(|b| b.name == remote.name) {
                switch(&remote.name)
            } else {
                Some(Action::Run(Operation::CheckoutRemoteBranch { remote_branch: name.to_owned(), local_name: remote.name.clone() }))
            }
        }
        RefKind::Tag => Some(detached_checkout("tag", name, format!("refs/tags/{name}"))),
        RefKind::Head | RefKind::Stash => None,
    }
}

/// What double-clicking a commit does: switch to a branch on it (local before remote), otherwise check the commit
/// out detached after asking. `None` when HEAD is already there.
pub fn checkout_commit(commit: &CommitInfo, refs: &RefsSnapshot) -> Option<Action> {
    let labels = refs.labels_for(&commit.id);
    [RefKind::LocalBranch, RefKind::RemoteBranch]
        .into_iter()
        .find_map(|kind| labels.iter().filter(|l| l.kind == kind).find_map(|l| checkout_ref(kind, &l.name, refs)))
        .or_else(|| (refs.head.target() != Some(commit.id)).then(|| detached_checkout("commit", &commit.short_id(), commit.id.to_string())))
}

/// Clicking a ref selects its commit; double-clicking checks it out.
pub fn click_ref(response: &egui::Response, kind: RefKind, name: &str, target: furkst_core::Oid, refs: &RefsSnapshot, actions: &mut Vec<Action>) {
    if response.double_clicked() {
        actions.extend(checkout_ref(kind, name, refs));
    } else if response.clicked() {
        actions.push(Action::SelectCommit(target));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use furkst_core::test_support::{commit, oid};
    use furkst_core::{Branch, HeadState, RefLabel, Remote};

    use super::*;

    /// `main` (checked out) at 1, `feature` at 2, and `origin/feature` + `origin/fresh` at 3.
    fn refs() -> RefsSnapshot {
        let branch = |name: &str, at: u8, is_head| Branch {
            name: name.into(),
            target: oid(at),
            upstream: None,
            ahead: 0,
            behind: 0,
            is_head,
        };
        let remote_branch = |name: &str| RemoteBranch { remote: "origin".into(), name: name.into(), target: oid(3) };
        let label = |name: &str, kind, is_head| RefLabel { name: name.into(), kind, is_head };
        RefsSnapshot {
            head: HeadState::Branch { name: "main".into(), target: Some(oid(1)) },
            branches: vec![branch("main", 1, true), branch("feature", 2, false)],
            remotes: vec![Remote { name: "origin".into(), url: None, branches: vec![remote_branch("feature"), remote_branch("fresh")] }],
            tags: Vec::new(),
            stashes: Vec::new(),
            submodules: Vec::new(),
            worktrees: Vec::new(),
            labels: HashMap::from([
                (oid(1), vec![label("main", RefKind::LocalBranch, true)]),
                (oid(2), vec![label("feature", RefKind::LocalBranch, false)]),
                (oid(3), vec![label("origin/fresh", RefKind::RemoteBranch, false)]),
            ]),
            uses_lfs: false,
            lfs_patterns: Vec::new(),
            flow: None,
        }
    }

    fn op(action: Option<Action>) -> Option<Operation> {
        match action? {
            Action::Run(op) | Action::Confirm { op, .. } => Some(op),
            _ => None,
        }
    }

    #[test]
    fn double_clicking_refs() {
        let refs = refs();
        assert!(matches!(op(checkout_ref(RefKind::LocalBranch, "feature", &refs)), Some(Operation::Checkout { target }) if target == "feature"));
        assert!(checkout_ref(RefKind::LocalBranch, "main", &refs).is_none(), "already checked out");
        // A remote branch with a local counterpart switches to the local branch.
        assert!(matches!(op(checkout_ref(RefKind::RemoteBranch, "origin/feature", &refs)), Some(Operation::Checkout { target }) if target == "feature"));
        // Without one, a tracking branch is created.
        assert!(matches!(
            op(checkout_ref(RefKind::RemoteBranch, "origin/fresh", &refs)),
            Some(Operation::CheckoutRemoteBranch { local_name, .. }) if local_name == "fresh"
        ));
    }

    #[test]
    fn branches_pair_with_their_remote_counterparts() {
        let refs = refs();
        assert_eq!(refs.remote_counterpart("feature").map(RemoteBranch::full_name).as_deref(), Some("origin/feature"));
        assert_eq!(refs.local_counterpart("origin/feature").map(|b| b.name.as_str()), Some("feature"));
        assert!(refs.local_counterpart("origin/fresh").is_none(), "no local branch of that name");
        assert!(refs.remote_counterpart("main").is_none(), "nothing on the remote");
    }

    #[test]
    fn deleting_a_branch_offers_its_counterpart_unticked() {
        let refs = refs();
        let Dialog::DeleteBranch { local, remote, delete_local, delete_remote } = Dialog::delete_local(&refs, "feature") else {
            unreachable!()
        };
        assert_eq!((local.as_deref(), remote.clone(), delete_local, delete_remote), (Some("feature"), Some(("origin".into(), "feature".into())), true, false));
        let only_local = Dialog::DeleteBranch { local, remote, delete_local, delete_remote };
        assert!(matches!(only_local.action(), Some(Action::Run(Operation::DeleteBranches { local: Some(_), remote: None }))));
        let fresh = refs.remotes[0].branches.iter().find(|b| b.name == "fresh").unwrap();
        let Dialog::DeleteBranch { local, delete_remote, .. } = Dialog::delete_remote(&refs, fresh) else { unreachable!() };
        assert!(local.is_none() && delete_remote, "nothing local to offer");
    }

    #[test]
    fn double_clicking_commits() {
        let refs = refs();
        assert!(checkout_commit(&commit(1, &[]), &refs).is_none(), "HEAD is already there");
        assert!(matches!(op(checkout_commit(&commit(2, &[]), &refs)), Some(Operation::Checkout { target }) if target == "feature"));
        assert!(matches!(op(checkout_commit(&commit(3, &[]), &refs)), Some(Operation::CheckoutRemoteBranch { .. })));
        assert!(matches!(checkout_commit(&commit(9, &[]), &refs), Some(Action::Confirm { .. })), "bare commits ask before detaching");
    }
}
