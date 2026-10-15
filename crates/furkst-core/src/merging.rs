//! The ways to merge a branch into the checked-out one, named like GitHub's merge button.

use crate::Repo;
use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MergeStyle {
    /// git's default: move the branch forward when possible, otherwise create a merge commit.
    #[default]
    FastForward,
    /// Always record a merge commit, keeping every commit of the branch (`--no-ff`).
    MergeCommit,
    /// Combine all of the branch's changes into one new commit (`--squash`, committed right away).
    Squash,
    /// Replay the branch's commits on top, one by one, without a merge commit; the branch itself is left as it was.
    Rebase,
}

impl MergeStyle {
    pub const ALL: [Self; 4] = [Self::FastForward, Self::MergeCommit, Self::Squash, Self::Rebase];

    pub fn label(self) -> &'static str {
        match self {
            Self::FastForward => "Fast-forward if possible",
            Self::MergeCommit => "Create a merge commit",
            Self::Squash => "Squash and merge",
            Self::Rebase => "Rebase and merge",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::FastForward => "Moves the branch forward when it has no new commits; otherwise creates a merge commit.",
            Self::MergeCommit => "Keeps all commits from the branch and adds a merge commit.",
            Self::Squash => "Combines all commits from the branch into one commit.",
            Self::Rebase => "Adds the branch's commits on top, one by one, without a merge commit.",
        }
    }

    /// Whether the branch's commits end up copied rather than merged, so git no longer sees the branch as merged.
    pub fn copies_commits(self) -> bool {
        matches!(self, Self::Squash | Self::Rebase)
    }
}

impl Repo {
    /// Merges `branch` into the checked-out branch. Like GitHub's "Squash and merge", a squash is committed right
    /// away, with the squashed commits listed in its message.
    pub(crate) fn merge_branch(&self, branch: &str, style: MergeStyle) -> Result<String> {
        let git = self.cli();
        match style {
            MergeStyle::FastForward => git.run(&["merge", "--no-edit", branch]),
            MergeStyle::MergeCommit => git.run(&["merge", "--no-ff", "--no-edit", branch]),
            MergeStyle::Squash => {
                let squashed = git.run(&["log", "--reverse", "--format=* %s", &format!("HEAD..{branch}")])?;
                git.run(&["merge", "--squash", branch])?;
                git.run(&["commit", "-m", &format!("Squashed {branch}\n\n{squashed}")])
            }
            // Nothing new on this side: take the branch's commits as they are, like a fast-forward.
            MergeStyle::Rebase if git.run(&["merge-base", "--is-ancestor", "HEAD", branch]).is_ok() => {
                git.run(&["merge", "--ff-only", branch])
            }
            // Like a rebase that doesn't move the branch: its new commits are applied here, in order.
            MergeStyle::Rebase => git.run(&["cherry-pick", &format!("HEAD..{branch}")]),
        }
    }
}

/// What the stashes Furkst sets changes aside in are named after.
const SET_ASIDE: &str = "Set aside by Furkst to ";

/// Ends the error of an operation that stopped on conflicts while its changes were set aside.
const KEPT_ASIDE: &str = "Your uncommitted changes are set aside in a stash until this is finished or aborted.";

/// Whether git refused an operation because it would overwrite uncommitted changes (to tracked or untracked files).
pub fn blocked_by_local_changes(error: &str) -> bool {
    error.contains("would be overwritten by")
}

/// Whether an operation's `error` says it stopped with uncommitted changes set aside, to put back with
/// [`Operation::RestoreSetAside`](crate::Operation::RestoreSetAside) once it is finished or aborted.
pub fn kept_aside(error: &str) -> bool {
    error.ends_with(KEPT_ASIDE)
}

impl Repo {
    /// Runs `work` (to `what`, as in "merge topic") with the uncommitted changes, untracked files included, stashed,
    /// and puts them back afterwards. When `work` stops half way on conflicts they stay stashed, which its error says.
    pub(crate) fn with_changes_aside(&self, what: &str, work: impl FnOnce() -> Result<String>) -> Result<String> {
        let git = self.cli();
        let top = || git.run(&["rev-parse", "-q", "--verify", "refs/stash"]).ok();
        let before = top();
        git.run(&["stash", "push", "--include-untracked", "-m", &format!("{SET_ASIDE}{what}")])?;
        if top() == before {
            // Nothing was set aside.
            return work();
        }
        match work() {
            Ok(output) => git.run(&["stash", "pop"]).map(|_| output),
            Err(error) => {
                self.reload()?;
                let status = self.status()?;
                if status.has_conflicts() || status.operation.is_some() {
                    Err(Error::msg(format!("{error}\n\n{KEPT_ASIDE}")))
                } else {
                    git.run(&["stash", "pop"])?;
                    Err(error)
                }
            }
        }
    }

    /// Puts back the most recent changes [`with_changes_aside`](Self::with_changes_aside) kept stashed, if any.
    pub(crate) fn restore_set_aside(&self) -> Result<String> {
        let git = self.cli();
        let names = git.run(&["stash", "list", "--format=%gs"])?;
        match names.lines().position(|name| name.contains(SET_ASIDE)) {
            Some(index) => git.run(&["stash", "pop", &format!("stash@{{{index}}}")]),
            None => Ok(String::new()),
        }
    }
}
