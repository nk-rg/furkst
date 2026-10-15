//! Every repository-mutating action the UI can trigger, as data.
//!
//! The UI only builds [`Operation`] values and hands them to a background worker; [`Operation::run`] is the
//! single dispatch point. Branch and tag management, staging and committing go through gitoxide; the rest
//! uses the `git` executable because gitoxide has no implementation of it yet.

use std::path::PathBuf;

use gix::refs::transaction::PreviousValue;

use crate::error::{Error, GixResultExt, Result};
use crate::{DiffOptions, FileChange, FlowKind, HunkAction, InProgress, LineSelection, MergeStyle, Oid, RebaseStep, Repo, short_id};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

impl ResetMode {
    fn flag(self) -> &'static str {
        match self {
            Self::Soft => "--soft",
            Self::Mixed => "--mixed",
            Self::Hard => "--hard",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Operation {
    // Working copy (gitoxide).
    Stage(Vec<FileChange>),
    Unstage(Vec<FileChange>),
    Discard(Vec<FileChange>),
    ApplySelection { action: HunkAction, path: String, selection: LineSelection, options: DiffOptions },
    Commit { message: String, amend: bool },
    /// Resolves a conflicted file by taking one side entirely, then marks it resolved.
    ResolveConflict { change: FileChange, ours: bool },
    /// Replaces a conflicted file with the merged `content` (worktree form), then marks it resolved.
    ResolveMerged { change: FileChange, content: String },
    /// Writes `path` as it was in `commit` to `destination`.
    SaveFileAt { commit: Oid, path: String, destination: PathBuf },

    // References (gitoxide).
    CreateBranch { name: String, at: Oid, checkout: bool },
    RenameBranch { from: String, to: String },
    DeleteBranch { name: String, force: bool },
    CreateTag { name: String, at: Oid, message: Option<String> },
    DeleteTag { name: String },

    // Everything below shells out to git.
    Checkout { target: String },
    CheckoutRemoteBranch { remote_branch: String, local_name: String },
    Fetch { remote: Option<String>, prune: bool },
    Pull { rebase: bool },
    Push { remote: String, branch: String, set_upstream: bool, force: bool, tags: bool },
    PushTag { remote: String, tag: String },
    DeleteRemoteBranch { remote: String, branch: String },
    /// A local branch, the remote branch that goes with it, or both, as chosen in the delete dialog. `remote` is (remote,
    /// branch).
    DeleteBranches { local: Option<String>, remote: Option<(String, String)> },
    DeleteRemoteTag { remote: String, tag: String },
    SetUpstream { branch: String, upstream: Option<String> },
    Merge { target: String, style: MergeStyle },
    Rebase { onto: String },
    CherryPick { commit: Oid },
    Revert { commit: Oid },
    Reset { target: Oid, mode: ResetMode },
    Continue(InProgress),
    Abort(InProgress),
    StashSave { message: String, include_untracked: bool, keep_index: bool },
    StashApply { index: usize, pop: bool },
    StashDrop { index: usize },
    AddRemote { name: String, url: String },
    RemoveRemote { name: String },
    UpdateSubmodules,
    /// Downloads the Git LFS content of the checked-out files.
    LfsPull,
    /// Downloads Git LFS content for the current branch without touching the working tree.
    LfsFetch,
    /// Stores files matching `pattern` with Git LFS from now on (updates `.gitattributes`).
    LfsTrack { pattern: String },
    /// Sets up git-flow with the given production and development branches.
    FlowInit { main: String, develop: String },
    /// Starts a git-flow feature, release or hotfix branch.
    FlowStart { kind: FlowKind, name: String },
    /// Finishes a git-flow branch: merges it where it belongs (tagging releases and hotfixes) and deletes it.
    FlowFinish { kind: FlowKind, name: String, style: MergeStyle },
    /// Interactive rebase onto `base` (`None` = from the root) replaying `steps` in order.
    InteractiveRebase { base: Option<Oid>, steps: Vec<RebaseStep> },
    /// Adds a linked worktree at `path` checking out `branch`; with `create` the branch is new, started at `start`.
    AddWorktree { path: PathBuf, branch: String, create: bool, start: Option<String> },
    RemoveWorktree { path: PathBuf, force: bool },
    PruneWorktrees,
}

impl Operation {
    /// Short human readable description for the activity log.
    pub fn describe(&self) -> String {
        match self {
            Self::Stage(c) => format!("Stage {}", files(c)),
            Self::Unstage(c) => format!("Unstage {}", files(c)),
            Self::Discard(c) => format!("Discard changes in {}", files(c)),
            Self::ApplySelection { action, path, .. } => format!("{action:?} lines in {path}"),
            Self::Commit { amend: true, .. } => "Amend last commit".into(),
            Self::Commit { .. } => "Commit".into(),
            Self::ResolveConflict { change, ours } => {
                format!("Resolve {} using the {} version", change.path, if *ours { "current" } else { "incoming" })
            }
            Self::ResolveMerged { change, .. } => format!("Resolve conflicts in {}", change.path),
            Self::SaveFileAt { path, destination, .. } => format!("Save {path} to {}", destination.display()),
            Self::CreateBranch { name, .. } => format!("Create branch {name}"),
            Self::RenameBranch { from, to } => format!("Rename branch {from} to {to}"),
            Self::DeleteBranch { name, .. } => format!("Delete branch {name}"),
            Self::CreateTag { name, .. } => format!("Create tag {name}"),
            Self::DeleteTag { name } => format!("Delete tag {name}"),
            Self::Checkout { target } => format!("Checkout {target}"),
            Self::CheckoutRemoteBranch { remote_branch, .. } => format!("Checkout {remote_branch}"),
            Self::Fetch { remote: Some(remote), .. } => format!("Fetch {remote}"),
            Self::Fetch { remote: None, .. } => "Fetch all remotes".into(),
            Self::Pull { .. } => "Pull".into(),
            Self::Push { remote, branch, .. } => format!("Push {branch} to {remote}"),
            Self::PushTag { remote, tag } => format!("Push tag {tag} to {remote}"),
            Self::DeleteRemoteBranch { remote, branch } => format!("Delete {remote}/{branch}"),
            Self::DeleteBranches { local, remote } => {
                let remote = remote.as_ref().map(|(remote, branch)| format!("{remote}/{branch}"));
                format!("Delete {}", local.iter().chain(remote.iter()).cloned().collect::<Vec<_>>().join(" and "))
            }
            Self::DeleteRemoteTag { remote, tag } => format!("Delete tag {tag} from {remote}"),
            Self::SetUpstream { branch, .. } => format!("Set upstream of {branch}"),
            Self::Merge { target, .. } => format!("Merge {target}"),
            Self::Rebase { onto } => format!("Rebase onto {onto}"),
            Self::CherryPick { commit } => format!("Cherry-pick {}", short_id(commit)),
            Self::Revert { commit } => format!("Revert {}", short_id(commit)),
            Self::Reset { target, mode } => format!("Reset ({mode:?}) to {}", short_id(target)),
            Self::Continue(op) => format!("Continue {}", op.label().to_lowercase()),
            Self::Abort(op) => format!("Abort {}", op.label().to_lowercase()),
            Self::StashSave { .. } => "Stash changes".into(),
            Self::StashApply { index, pop: true } => format!("Pop stash@{{{index}}}"),
            Self::StashApply { index, pop: false } => format!("Apply stash@{{{index}}}"),
            Self::StashDrop { index } => format!("Drop stash@{{{index}}}"),
            Self::AddRemote { name, .. } => format!("Add remote {name}"),
            Self::RemoveRemote { name } => format!("Remove remote {name}"),
            Self::UpdateSubmodules => "Update submodules".into(),
            Self::LfsPull => "Pull Git LFS content".into(),
            Self::LfsFetch => "Fetch Git LFS content".into(),
            Self::LfsTrack { pattern } => format!("Track {pattern} with Git LFS"),
            Self::FlowInit { .. } => "Initialize git-flow".into(),
            Self::FlowStart { kind, name } => format!("Start {} {name}", kind.label()),
            Self::FlowFinish { kind, name, .. } => format!("Finish {} {name}", kind.label()),
            Self::InteractiveRebase { .. } => "Interactive rebase".into(),
            Self::AddWorktree { path, branch, .. } => format!("Check out {branch} in worktree {}", path.display()),
            Self::RemoveWorktree { path, .. } => format!("Remove worktree {}", path.display()),
            Self::PruneWorktrees => "Prune worktrees".into(),
        }
    }

    /// Whether the history and branch list may have changed (as opposed to only the working copy).
    pub fn changes_refs(&self) -> bool {
        !matches!(
            self,
            Self::Stage(_) | Self::Unstage(_) | Self::Discard(_) | Self::ApplySelection { .. } | Self::ResolveMerged { .. } | Self::SaveFileAt { .. }
                | Self::LfsPull
                | Self::LfsFetch
                | Self::LfsTrack { .. }
        )
    }

    /// Whether it can stop half way on conflicts, leaving them to resolve.
    pub fn may_conflict(&self) -> bool {
        matches!(
            self,
            Self::Merge { .. }
                | Self::Rebase { .. }
                | Self::InteractiveRebase { .. }
                | Self::CherryPick { .. }
                | Self::Revert { .. }
                | Self::Pull { .. }
                | Self::StashApply { .. }
                | Self::FlowFinish { .. }
                | Self::Continue(_)
        )
    }

    /// Executes the operation and returns output worth showing to the user.
    pub fn run(self, repo: &Repo) -> Result<String> {
        self.run_with_progress(repo, &|_| {})
    }

    /// Like [`run`](Self::run), reporting git's progress lines (fetch, pull, push…) as they arrive.
    pub fn run_with_progress(self, repo: &Repo, on_progress: &dyn Fn(&str)) -> Result<String> {
        let output = self.execute(repo, on_progress)?;
        // Start afresh after every change: git may have changed config (remotes, upstreams) behind gitoxide's back, and
        // gitoxide only rereads a rewritten index when its modification time moved on, which a write within the same
        // timestamp tick (coarse on Windows) doesn't do, so the next read could see the index as it was before.
        repo.reload()?;
        Ok(output)
    }

    fn execute(self, repo: &Repo, on_progress: &dyn Fn(&str)) -> Result<String> {
        let git = |args: &[&str]| repo.cli().run_with_progress(args, on_progress);
        match self {
            Self::Stage(changes) => repo.stage(&changes).map(|()| String::new()),
            Self::Unstage(changes) => repo.unstage(&changes).map(|()| String::new()),
            Self::Discard(changes) => repo.discard(&changes).map(|()| String::new()),
            Self::ApplySelection { action, path, selection, options } => {
                repo.apply_selection(action, &path, &selection, options).map(|()| String::new())
            }
            Self::Commit { message, amend } => repo.commit(&message, amend),
            Self::ResolveConflict { change, ours } => {
                git(&["checkout", if ours { "--ours" } else { "--theirs" }, "--", &change.path])?;
                repo.stage(&[change]).map(|()| String::new())
            }
            Self::ResolveMerged { change, content } => repo.resolve_with_content(&change, &content).map(|()| String::new()),
            Self::SaveFileAt { commit, path, destination } => {
                let data = repo.file_at(commit, &path)?.ok_or_else(|| Error::msg(format!("{path} does not exist in this commit")))?;
                std::fs::write(destination, data)?;
                Ok(String::new())
            }
            Self::CreateBranch { name, at, checkout } => {
                repo.gix().reference(format!("refs/heads/{name}"), at, PreviousValue::MustNotExist, "branch: Created")?;
                if checkout { git(&["checkout", &name]) } else { Ok(String::new()) }
            }
            Self::RenameBranch { from, to } => repo.rename_branch(&from, &to).map(|()| String::new()),
            Self::DeleteBranch { name, force } => repo.delete_branch(&name, force).map(|()| String::new()),
            Self::CreateTag { name, at, message } => {
                let gix = repo.gix();
                match message.filter(|m| !m.trim().is_empty()) {
                    Some(message) => {
                        let tagger = gix.committer().ok_or_else(|| Error::msg("committer identity is not configured"))??;
                        gix.tag(&name, at, gix::object::Kind::Commit, Some(tagger), message, PreviousValue::MustNotExist)?;
                    }
                    None => {
                        gix.tag_reference(&name, at, PreviousValue::MustNotExist)?;
                    }
                }
                Ok(String::new())
            }
            Self::DeleteTag { name } => {
                repo.gix().find_reference(&format!("refs/tags/{name}"))?.delete()?;
                Ok(String::new())
            }
            Self::Checkout { target } => git(&["checkout", &target]),
            Self::CheckoutRemoteBranch { remote_branch, local_name } => {
                git(&["checkout", "-b", &local_name, "--track", &remote_branch])
            }
            Self::Fetch { remote, prune } => {
                let mut args = vec!["fetch", "--progress"];
                if prune {
                    args.push("--prune");
                }
                match &remote {
                    Some(remote) => args.push(remote),
                    None => args.push("--all"),
                }
                git(&args)
            }
            Self::Pull { rebase } => git(&["pull", "--progress", if rebase { "--rebase" } else { "--no-rebase" }]),
            Self::Push { remote, branch, set_upstream, force, tags } => {
                let mut args = vec!["push", "--progress"];
                if set_upstream {
                    args.push("--set-upstream");
                }
                if force {
                    args.push("--force-with-lease");
                }
                if tags {
                    args.push("--tags");
                }
                args.extend([remote.as_str(), branch.as_str()]);
                git(&args)
            }
            Self::PushTag { remote, tag } => git(&["push", &remote, &format!("refs/tags/{tag}")]),
            Self::DeleteRemoteBranch { remote, branch } => git(&["push", &remote, "--delete", &branch]),
            Self::DeleteBranches { local, remote } => {
                let local = local.map(|name| Self::DeleteBranch { name, force: true });
                let remote = remote.map(|(remote, branch)| Self::DeleteRemoteBranch { remote, branch });
                let outputs = local.into_iter().chain(remote).map(|op| op.execute(repo, on_progress)).collect::<Result<Vec<_>>>()?;
                Ok(outputs.join("
"))
            }
            Self::DeleteRemoteTag { remote, tag } => git(&["push", &remote, "--delete", &format!("refs/tags/{tag}")]),
            Self::SetUpstream { branch, upstream: Some(upstream) } => {
                git(&["branch", &format!("--set-upstream-to={upstream}"), &branch])
            }
            Self::SetUpstream { branch, upstream: None } => git(&["branch", "--unset-upstream", &branch]),
            Self::Merge { target, style } => repo.merge_branch(&target, style),
            // Uncommitted changes are set aside for the rebase and put back afterwards, rather than refusing to start.
            Self::Rebase { onto } => git(&["rebase", "--autostash", &onto]),
            Self::CherryPick { commit } => git(&["cherry-pick", &commit.to_string()]),
            Self::Revert { commit } => git(&["revert", "--no-edit", &commit.to_string()]),
            Self::Reset { target, mode } => git(&["reset", mode.flag(), &target.to_string()]),
            Self::Continue(op) => match op.command() {
                Some(command) => git(&[command, "--continue"]),
                None => Err(Error::msg(format!("{} cannot be continued from here", op.label()))),
            },
            Self::Abort(op) => match op {
                InProgress::Bisect => git(&["bisect", "reset"]),
                op => git(&[op.command().expect("all but bisect have a command"), "--abort"]),
            },
            Self::StashSave { message, include_untracked, keep_index } => {
                let mut args = vec!["stash", "push"];
                if include_untracked {
                    args.push("--include-untracked");
                }
                if keep_index {
                    args.push("--keep-index");
                }
                if !message.trim().is_empty() {
                    args.extend(["-m", message.as_str()]);
                }
                git(&args)
            }
            Self::StashApply { index, pop } => {
                git(&["stash", if pop { "pop" } else { "apply" }, &format!("stash@{{{index}}}")])
            }
            Self::StashDrop { index } => git(&["stash", "drop", &format!("stash@{{{index}}}")]),
            Self::AddRemote { name, url } => git(&["remote", "add", &name, &url]),
            Self::RemoveRemote { name } => git(&["remote", "remove", &name]),
            Self::UpdateSubmodules => git(&["submodule", "update", "--init", "--recursive"]),
            Self::LfsPull => git(&["lfs", "pull"]),
            Self::LfsFetch => git(&["lfs", "fetch"]),
            Self::LfsTrack { pattern } => git(&["lfs", "track", &pattern]),
            Self::FlowInit { main, develop } => repo.flow_init(&main, &develop),
            Self::FlowStart { kind, name } => repo.flow_start(kind, &name),
            Self::FlowFinish { kind, name, style } => repo.flow_finish(kind, &name, style),
            Self::InteractiveRebase { base, steps } => repo.interactive_rebase(base, &steps),
            Self::AddWorktree { path, branch, create, start } => {
                let path = path.to_string_lossy();
                let mut args = vec!["worktree", "add"];
                if create {
                    args.extend(["-b", &branch, &path]);
                    args.extend(start.as_deref());
                } else {
                    args.extend([&*path, &branch]);
                }
                git(&args)
            }
            Self::RemoveWorktree { path, force } => {
                let path = path.to_string_lossy();
                let mut args = vec!["worktree", "remove"];
                if force {
                    args.push("--force");
                }
                args.push(&path);
                git(&args)
            }
            Self::PruneWorktrees => git(&["worktree", "prune"]),
        }
    }
}

impl Repo {
    fn rename_branch(&self, from: &str, to: &str) -> Result<()> {
        use gix::refs::transaction::{Change, LogChange, RefEdit, RefLog};
        let repo = self.gix();
        let mut old = repo.find_reference(&format!("refs/heads/{from}"))?;
        let target = old.peel_to_id()?.detach();
        let new_name: gix::refs::FullName = format!("refs/heads/{to}").try_into().git()?;
        repo.reference(new_name.clone(), target, PreviousValue::MustNotExist, format!("branch: renamed {from} to {to}"))?;
        if repo.head_name()?.is_some_and(|head| head.shorten() == from) {
            repo.edit_reference(RefEdit {
                change: Change::Update {
                    log: LogChange { mode: RefLog::Only, force_create_reflog: false, message: "rename branch".into() },
                    expected: PreviousValue::Any,
                    new: gix::refs::Target::Symbolic(new_name),
                },
                name: "HEAD".try_into().git()?,
                deref: false,
            })?;
        }
        old.delete()?;
        // Keep the tracking configuration by renaming the config section, like `git branch -m` does.
        let _ = self.cli().run(&["config", "--rename-section", &format!("branch.{from}"), &format!("branch.{to}")]);
        Ok(())
    }

    fn delete_branch(&self, name: &str, force: bool) -> Result<()> {
        let repo = self.gix();
        if repo.head_name()?.is_some_and(|head| head.shorten() == name) {
            return Err(Error::msg(format!("cannot delete the checked-out branch {name}")));
        }
        let mut reference = repo.find_reference(&format!("refs/heads/{name}"))?;
        if !force {
            let tip = reference.peel_to_id()?.detach();
            if let Some(head) = repo.head()?.id()
                && repo.merge_base(tip, head)?.is_none_or(|base| base.detach() != tip)
            {
                return Err(Error::msg(format!("{name} is not fully merged into HEAD; use force delete")));
            }
        }
        reference.delete()?;
        Ok(())
    }
}

fn files(changes: &[FileChange]) -> String {
    match changes {
        [one] => one.path.clone(),
        many => format!("{} files", many.len()),
    }
}
