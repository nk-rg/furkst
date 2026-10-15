use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use gix::bstr::ByteSlice;

use crate::cli::GitCli;
use crate::error::{Error, Result};
use crate::{CommitInfo, InProgress, Oid, Signature};

/// An opened repository with a working tree.
///
/// Cheap to share between threads; every call creates its own thread-local gitoxide handle.
/// A history walk: the tips it started from, and every commit it reached, in walk order.
pub(crate) type WalkCache = (Vec<Oid>, Arc<Vec<Oid>>);

pub struct Repo {
    sync: RwLock<gix::ThreadSafeRepository>,
    workdir: PathBuf,
    cli: GitCli,
    /// The last history walk searched; see `Repo::walked`.
    pub(crate) walk_cache: Mutex<Option<WalkCache>>,
}

impl Repo {
    /// Opens the repository containing `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let sync = gix::ThreadSafeRepository::discover(path.as_ref())?;
        let workdir = sync
            .work_dir()
            .ok_or_else(|| Error::msg("bare repositories are not supported"))?
            .to_path_buf();
        let workdir = gix::path::realpath(&workdir).unwrap_or(workdir);
        Ok(Self { sync: RwLock::new(sync), cli: GitCli::new(&workdir), workdir, walk_cache: Mutex::default() })
    }

    /// Creates a new, empty repository at `path` and opens it.
    pub fn init(path: impl AsRef<Path>) -> Result<Self> {
        gix::init(path.as_ref())?;
        Self::open(path)
    }

    /// Clones `url` into `path` and opens the result, reporting git's progress lines to `on_progress`.
    pub fn clone_from(url: &str, path: impl AsRef<Path>, on_progress: &dyn Fn(&str)) -> Result<Self> {
        let path = path.as_ref();
        let parent = path.parent().ok_or_else(|| Error::msg("invalid clone destination"))?;
        GitCli::new(parent).run_with_progress(&["clone", "--progress", "--recurse-submodules", "--", url, &path.to_string_lossy()], on_progress)?;
        Self::open(path)
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// Display name: the name of the working tree directory.
    pub fn name(&self) -> String {
        self.workdir
            .file_name()
            .map_or_else(|| self.workdir.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    pub fn cli(&self) -> &GitCli {
        &self.cli
    }

    /// Re-reads configuration and caches after something outside gitoxide changed the repository.
    pub fn reload(&self) -> Result<()> {
        let fresh = gix::ThreadSafeRepository::open(self.git_dir())?;
        *self.sync.write().expect("repository lock poisoned") = fresh;
        Ok(())
    }

    pub(crate) fn git_dir(&self) -> PathBuf {
        self.sync.read().expect("repository lock poisoned").git_dir().to_path_buf()
    }

    /// A thread-local gitoxide handle.
    pub(crate) fn gix(&self) -> gix::Repository {
        let mut repo = self.sync.read().expect("repository lock poisoned").to_thread_local();
        repo.object_cache_size_if_unset(32 * 1024 * 1024);
        repo
    }

    /// The operation the repository is in the middle of, if any.
    pub fn in_progress(&self) -> Option<InProgress> {
        use gix::state::InProgress as S;
        Some(match self.gix().state()? {
            S::Merge => InProgress::Merge,
            S::CherryPick | S::CherryPickSequence => InProgress::CherryPick,
            S::Revert | S::RevertSequence => InProgress::Revert,
            S::Bisect => InProgress::Bisect,
            S::ApplyMailbox => InProgress::ApplyMailbox,
            S::ApplyMailboxRebase | S::Rebase | S::RebaseInteractive => InProgress::Rebase,
        })
    }

    /// Reads a single commit.
    pub fn commit_info(&self, id: Oid) -> Result<CommitInfo> {
        commit_info(&self.gix().find_commit(id)?)
    }

    /// Resolves a revision like `HEAD~2` or `origin/main` to a commit id.
    pub fn resolve(&self, spec: &str) -> Result<Oid> {
        let repo = self.gix();
        let id = repo.rev_parse_single(spec)?;
        Ok(id.object()?.peel_to_commit()?.id)
    }

    /// The configured identity, used to pre-fill the commit form.
    pub fn identity(&self) -> Option<Signature> {
        let repo = self.gix();
        let sig = repo.committer()?.ok()?;
        Some(Signature::from_gix(sig))
    }

    /// Message of an in-progress merge, cherry-pick or revert, used to pre-fill the commit form.
    pub fn pending_message(&self) -> Option<String> {
        message_file(&self.git_dir().join("MERGE_MSG"))
    }

    /// The configured `commit.template`, used to pre-fill the commit form.
    pub fn commit_template(&self) -> Option<String> {
        let path = self.gix().config_snapshot().trusted_path("commit.template").ok()??;
        message_file(&self.workdir.join(path))
    }
}

/// A commit message file without git's `#` comment lines; `None` if missing or empty.
fn message_file(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join("\n");
    Some(text.trim().to_owned()).filter(|t| !t.is_empty())
}

pub(crate) fn commit_info(commit: &gix::Commit<'_>) -> Result<CommitInfo> {
    let message = commit.message_raw_sloppy().to_str_lossy().trim_end().to_owned();
    let summary = message.lines().next().unwrap_or_default().to_owned();
    Ok(CommitInfo {
        id: commit.id,
        parents: commit.parent_ids().map(|id| id.detach()).collect(),
        author: Signature::from_gix(commit.author()?),
        committer: Signature::from_gix(commit.committer()?),
        summary,
        message,
    })
}
