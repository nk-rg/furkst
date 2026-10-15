//! Plain data types shared between the backend and the UI.

use std::collections::HashMap;

use crate::Oid;

/// A person and point in time, as recorded in a commit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    pub email: String,
    /// Seconds since the UNIX epoch.
    pub time: i64,
    /// Offset from UTC in seconds.
    pub offset: i32,
}

impl Signature {
    pub(crate) fn from_gix(sig: gix::actor::SignatureRef<'_>) -> Self {
        let sig = sig.trim();
        let time = sig.time().unwrap_or_default();
        Self {
            name: sig.name.to_string(),
            email: sig.email.to_string(),
            time: time.seconds,
            offset: time.offset,
        }
    }

    /// Formats the timestamp in the author's own timezone, e.g. `2026-10-13 18:32`.
    pub fn format_time(&self, pattern: &str) -> String {
        let offset = chrono::FixedOffset::east_opt(self.offset).unwrap_or(chrono::FixedOffset::east_opt(0).expect("zero offset"));
        chrono::DateTime::from_timestamp(self.time, 0)
            .map(|utc| utc.with_timezone(&offset).format(pattern).to_string())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitInfo {
    pub id: Oid,
    pub parents: Vec<Oid>,
    pub author: Signature,
    pub committer: Signature,
    /// First line of the message.
    pub summary: String,
    /// Full message, including the summary.
    pub message: String,
}

impl CommitInfo {
    pub fn short_id(&self) -> String {
        short_id(&self.id)
    }

    /// The message without its summary line.
    pub fn body(&self) -> &str {
        self.message.split_once('\n').map_or("", |(_, body)| body.trim())
    }

    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

pub fn short_id(id: &Oid) -> String {
    id.to_hex_with_len(7).to_string()
}

/// A commit together with the files it touched.
#[derive(Clone, Debug)]
pub struct CommitDetails {
    pub info: CommitInfo,
    pub changes: Vec<FileChange>,
    /// For a stash made with its untracked files: the commit holding them, and those files.
    pub stash_untracked: Option<(Oid, Vec<FileChange>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChangeKind {
    Conflicted,
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChange,
    Untracked,
}

impl ChangeKind {
    /// Single-letter code as shown by `git status --short`.
    pub fn letter(self) -> char {
        match self {
            Self::Conflicted => 'U',
            Self::Modified => 'M',
            Self::Added => 'A',
            Self::Deleted => 'D',
            Self::Renamed => 'R',
            Self::Copied => 'C',
            Self::TypeChange => 'T',
            Self::Untracked => '?',
        }
    }
}

/// One changed path, in a commit or in the working copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    /// Repository-relative path with `/` separators.
    pub path: String,
    /// Previous path for renames and copies.
    pub old_path: Option<String>,
    pub kind: ChangeKind,
}

impl FileChange {
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    pub fn directory(&self) -> &str {
        self.path.rsplit_once('/').map_or("", |(dir, _)| dir)
    }
}

/// The state of the working copy, split into unstaged and staged changes.
#[derive(Clone, Debug, Default)]
pub struct WorkingStatus {
    pub unstaged: Vec<FileChange>,
    pub staged: Vec<FileChange>,
    pub operation: Option<InProgress>,
}

impl WorkingStatus {
    pub fn is_clean(&self) -> bool {
        self.unstaged.is_empty() && self.staged.is_empty()
    }

    pub fn has_conflicts(&self) -> bool {
        self.unstaged.iter().any(|c| c.kind == ChangeKind::Conflicted)
    }
}

/// A multi-step operation that was interrupted and awaits the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InProgress {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Bisect,
    ApplyMailbox,
}

impl InProgress {
    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "Merging",
            Self::Rebase => "Rebasing",
            Self::CherryPick => "Cherry-picking",
            Self::Revert => "Reverting",
            Self::Bisect => "Bisecting",
            Self::ApplyMailbox => "Applying patches",
        }
    }

    /// The `git` sub-command that drives `--continue`/`--abort`, if any.
    pub fn command(self) -> Option<&'static str> {
        match self {
            Self::Merge => Some("merge"),
            Self::Rebase => Some("rebase"),
            Self::CherryPick => Some("cherry-pick"),
            Self::Revert => Some("revert"),
            Self::ApplyMailbox => Some("am"),
            Self::Bisect => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeadState {
    /// On a branch; `target` is `None` for an unborn branch.
    Branch { name: String, target: Option<Oid> },
    Detached(Oid),
}

impl HeadState {
    pub fn branch_name(&self) -> Option<&str> {
        match self {
            Self::Branch { name, .. } => Some(name),
            Self::Detached(_) => None,
        }
    }

    pub fn target(&self) -> Option<Oid> {
        match self {
            Self::Branch { target, .. } => *target,
            Self::Detached(id) => Some(*id),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// Short name, e.g. `feature/login`.
    pub name: String,
    pub target: Oid,
    /// Short name of the tracked remote branch, e.g. `origin/feature/login`.
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub is_head: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteBranch {
    pub remote: String,
    /// Name without the remote prefix.
    pub name: String,
    pub target: Oid,
}

impl RemoteBranch {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.remote, self.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub url: Option<String>,
    pub branches: Vec<RemoteBranch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    /// The commit the tag ultimately points to.
    pub target: Oid,
    /// Message of annotated tags.
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stash {
    pub index: usize,
    pub id: Oid,
    pub message: String,
}

impl Stash {
    pub fn reference(&self) -> String {
        format!("stash@{{{}}}", self.index)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submodule {
    pub name: String,
    pub path: String,
    pub url: Option<String>,
}

/// A checked-out working tree of the repository: the main one or a linked one.
#[derive(Clone, Debug)]
pub struct Worktree {
    pub path: std::path::PathBuf,
    /// Short name of the checked-out branch; `None` when HEAD is detached.
    pub branch: Option<String>,
    pub head: Option<Oid>,
    pub is_main: bool,
    /// The worktree this repository was opened from.
    pub is_current: bool,
    /// Lock reason (possibly empty) if the worktree is locked.
    pub locked: Option<String>,
    /// Its directory is gone, so `git worktree prune` would remove it.
    pub prunable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefKind {
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
    Stash,
}

/// A decoration shown next to a commit in the history list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
    /// True for the branch `HEAD` points to.
    pub is_head: bool,
}

/// Everything shown in the sidebar, captured at one point in time.
#[derive(Clone, Debug)]
pub struct RefsSnapshot {
    pub head: HeadState,
    pub branches: Vec<Branch>,
    pub remotes: Vec<Remote>,
    pub tags: Vec<Tag>,
    pub stashes: Vec<Stash>,
    pub submodules: Vec<Submodule>,
    pub worktrees: Vec<Worktree>,
    /// Whether the repository uses Git LFS.
    pub uses_lfs: bool,
    /// Patterns stored with Git LFS, from `.gitattributes`.
    pub lfs_patterns: Vec<String>,
    /// The git-flow setup, if git-flow is initialized.
    pub flow: Option<crate::FlowConfig>,
    pub labels: HashMap<Oid, Vec<RefLabel>>,
}

impl RefsSnapshot {
    pub fn current_branch(&self) -> Option<&Branch> {
        self.branches.iter().find(|b| b.is_head)
    }

    /// The remote branch that goes with local branch `name`: the one it tracks, or else one of the same name.
    pub fn remote_counterpart(&self, name: &str) -> Option<&RemoteBranch> {
        let branch = self.branches.iter().find(|b| b.name == name)?;
        let mut remote_branches = self.remotes.iter().flat_map(|r| &r.branches);
        match &branch.upstream {
            Some(upstream) => remote_branches.find(|r| &r.full_name() == upstream),
            None => remote_branches.find(|r| r.name == name),
        }
    }

    /// The local branch that goes with remote branch `full_name` (like `origin/feature`): the one tracking it, or else
    /// one of the same name that tracks nothing.
    pub fn local_counterpart(&self, full_name: &str) -> Option<&Branch> {
        let remote = self.remotes.iter().flat_map(|r| &r.branches).find(|r| r.full_name() == full_name)?;
        let tracking = self.branches.iter().find(|b| b.upstream.as_deref() == Some(full_name));
        tracking.or_else(|| self.branches.iter().find(|b| b.upstream.is_none() && b.name == remote.name))
    }

    pub fn labels_for(&self, id: &Oid) -> &[RefLabel] {
        self.labels.get(id).map_or(&[], Vec::as_slice)
    }

    /// Every tip worth walking from when listing "all commits".
    pub fn all_tips(&self) -> Vec<Oid> {
        let mut tips: Vec<Oid> = self.head.target().into_iter().collect();
        tips.extend(self.branches.iter().map(|b| b.target));
        tips.extend(self.remotes.iter().flat_map(|r| r.branches.iter().map(|b| b.target)));
        tips.extend(self.tags.iter().map(|t| t.target));
        tips.sort_unstable();
        tips.dedup();
        tips
    }
}

/// A line of a blamed file.
#[derive(Clone, Debug)]
pub struct BlameLine {
    pub commit: Oid,
    pub line_no: usize,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct BlameResult {
    pub lines: Vec<BlameLine>,
    /// Details of every commit referenced by `lines`.
    pub commits: HashMap<Oid, CommitInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeFileKind {
    File,
    Executable,
    Symlink,
    Submodule,
}

/// One non-directory entry of a commit's tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeFile {
    /// Repository-relative path with `/` separators.
    pub path: String,
    pub kind: TreeFileKind,
    pub id: Oid,
}

/// One movement of `HEAD` recorded in the reflog.
#[derive(Clone, Debug)]
pub struct ReflogEntry {
    /// The commit `HEAD` moved to.
    pub commit: CommitInfo,
    /// Where `HEAD` was before.
    pub previous: Oid,
    /// What moved it, e.g. `commit: Fix typo` or `reset: moving to HEAD~1`.
    pub action: String,
    /// Who moved it and when.
    pub when: Signature,
}
