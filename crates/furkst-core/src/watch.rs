//! Noticing changes made to a repository outside Furkst, so the UI can refresh on its own.

use std::path::{Path, PathBuf};

use gix::index::entry::Mode;
use notify::{EventKind, RecursiveMode, Watcher as _};

use crate::error::{GixResultExt, Result};
use crate::Repo;

/// What an outside change affects. Ordered by how much needs reloading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    /// Working tree files or the index: the status needs reloading.
    WorkingCopy,
    /// `HEAD`, branches, tags or an in-progress operation: refs, history and status need reloading.
    Refs,
}

/// Watches while alive; dropping it stops watching.
pub struct Watch {
    _watcher: notify::RecommendedWatcher,
}

impl Repo {
    /// Calls `on_change` from a background thread whenever files of the repository change, ignoring paths
    /// git ignores (build output and the like) and git's internal bookkeeping.
    pub fn watch(&self, on_change: impl Fn(Change) + Send + 'static) -> Result<Watch> {
        let repo = self.gix();
        let index = repo.index_or_empty()?;
        let mut excludes = repo.excludes(&index, None, Default::default())?.detach();
        let workdir = self.workdir().to_path_buf();
        let mut git_dirs: Vec<PathBuf> = [repo.git_dir(), repo.common_dir()].map(canonical).to_vec();
        git_dirs.dedup();

        let classifier = Classifier { workdir: workdir.clone(), git_dirs: git_dirs.clone() };
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            let mut is_excluded = |rela: &Path, is_dir: bool| {
                let mode = is_dir.then_some(Mode::DIR);
                excludes.at_path(rela, mode, &repo.objects).is_ok_and(|p| p.is_excluded())
            };
            if let Some(change) = event.paths.iter().filter_map(|p| classifier.classify(p, &mut is_excluded)).max() {
                on_change(change);
            }
        })
        .git()?;
        watcher.watch(&workdir, RecursiveMode::Recursive).git()?;
        // A linked worktree or separate git dir keeps refs outside the working tree.
        for dir in git_dirs.iter().filter(|d| !d.starts_with(&workdir)) {
            watcher.watch(dir, RecursiveMode::Recursive).git()?;
        }
        Ok(Watch { _watcher: watcher })
    }
}

fn canonical(path: &Path) -> PathBuf {
    gix::path::realpath(path).unwrap_or_else(|_| path.to_path_buf())
}

struct Classifier {
    workdir: PathBuf,
    git_dirs: Vec<PathBuf>,
}

impl Classifier {
    /// What a change of `path` affects; `None` for paths that don't matter. `is_excluded(relative, is_dir)` tells
    /// whether git ignores a working-tree path.
    fn classify(&self, path: &Path, is_excluded: &mut impl FnMut(&Path, bool) -> bool) -> Option<Change> {
        if let Some(inside) = self.git_dirs.iter().find_map(|dir| path.strip_prefix(dir).ok()) {
            return git_dir_change(inside);
        }
        let rela = path.strip_prefix(&self.workdir).ok()?;
        // An ignored folder hides everything below it, so check each level from the top.
        let mut prefix = PathBuf::new();
        let mut components = rela.components().peekable();
        while let Some(component) = components.next() {
            prefix.push(component);
            let is_dir = components.peek().is_some() || path.is_dir();
            if is_excluded(&prefix, is_dir) {
                return None;
            }
        }
        (!prefix.as_os_str().is_empty()).then_some(Change::WorkingCopy)
    }
}

/// What a change inside the git directory affects, from its path relative to that directory.
fn git_dir_change(inside: &Path) -> Option<Change> {
    let first = inside.components().next()?.as_os_str().to_str()?;
    match first {
        "HEAD" | "refs" | "packed-refs" | "FETCH_HEAD" | "ORIG_HEAD" | "MERGE_HEAD" | "CHERRY_PICK_HEAD"
        | "REVERT_HEAD" | "rebase-merge" | "rebase-apply" => Some(Change::Refs),
        "index" => Some(Change::WorkingCopy),
        // Objects, logs, lock files and our own temporary files change without anything visible changing.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classifier() -> Classifier {
        Classifier { workdir: PathBuf::from("/repo"), git_dirs: vec![PathBuf::from("/repo/.git")] }
    }

    fn classify(path: &str) -> Option<Change> {
        // Pretend `target/` is gitignored.
        classifier().classify(Path::new(path), &mut |rela, _| rela == Path::new("target"))
    }

    #[test]
    fn working_tree_changes() {
        assert_eq!(classify("/repo/src/main.rs"), Some(Change::WorkingCopy));
        assert_eq!(classify("/repo/target/debug/furkst"), None);
        assert_eq!(classify("/repo"), None);
        assert_eq!(classify("/elsewhere/file"), None);
    }

    #[test]
    fn git_dir_changes() {
        assert_eq!(classify("/repo/.git/HEAD"), Some(Change::Refs));
        assert_eq!(classify("/repo/.git/refs/heads/main"), Some(Change::Refs));
        assert_eq!(classify("/repo/.git/index"), Some(Change::WorkingCopy));
        assert_eq!(classify("/repo/.git/index.lock"), None);
        assert_eq!(classify("/repo/.git/objects/ab/cdef"), None);
        assert_eq!(classify("/repo/.git/logs/HEAD"), None);
    }
}
