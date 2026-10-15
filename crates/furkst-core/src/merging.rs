//! The ways to merge a branch into the checked-out one, named like GitHub's merge button.

use crate::Repo;
use crate::error::Result;

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
