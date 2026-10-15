use std::collections::BTreeMap;

use gix::status::index_worktree::iter::Summary;

use crate::error::Result;
use crate::{ChangeKind, FileChange, Repo, WorkingStatus};

impl Repo {
    /// Staged and unstaged changes of the working copy, like `git status`.
    pub fn status(&self) -> Result<WorkingStatus> {
        let repo = self.gix();
        let mut iter = repo
            .status(gix::progress::Discard)?
            .untracked_files(gix::status::UntrackedFiles::Files)
            // Unstaged deletions + additions stay separate; renames are shown once staged.
            .index_worktree_rewrites(None)
            .into_iter(None)?;

        let mut unstaged = BTreeMap::new();
        let mut staged = BTreeMap::new();
        for item in iter.by_ref() {
            match item? {
                gix::status::Item::IndexWorktree(item) => {
                    let Some(summary) = item.summary() else { continue };
                    let kind = match summary {
                        Summary::Removed => ChangeKind::Deleted,
                        Summary::Added => ChangeKind::Untracked,
                        Summary::Modified => ChangeKind::Modified,
                        Summary::TypeChange => ChangeKind::TypeChange,
                        Summary::Renamed => ChangeKind::Renamed,
                        Summary::Copied => ChangeKind::Copied,
                        Summary::IntentToAdd => ChangeKind::Added,
                        Summary::Conflict => ChangeKind::Conflicted,
                    };
                    let path = item.rela_path().to_string();
                    unstaged.insert(path.clone(), FileChange { path, old_path: None, kind });
                }
                gix::status::Item::TreeIndex(change) => {
                    use gix::diff::index::ChangeRef;
                    let (kind, old_path) = match &change {
                        ChangeRef::Addition { .. } => (ChangeKind::Added, None),
                        ChangeRef::Deletion { .. } => (ChangeKind::Deleted, None),
                        ChangeRef::Modification { .. } => (ChangeKind::Modified, None),
                        ChangeRef::Rewrite { source_location, copy, .. } => (
                            if *copy { ChangeKind::Copied } else { ChangeKind::Renamed },
                            Some(source_location.to_string()),
                        ),
                    };
                    let path = change.location().to_string();
                    staged.insert(path.clone(), FileChange { path, old_path, kind });
                }
            }
        }
        // Like `git status`, save the timestamps of files found unchanged, so the next status needn't re-read them (it
        // would after a clone or checkout, taking seconds on big repositories). Only if no one changed the index since it
        // was read, so nothing staged meanwhile is lost; a failure (git holding the lock) just leaves the next status slow.
        if let Some(mut outcome) = iter.into_outcome()
            && outcome.has_changes()
            && unchanged_on_disk(&outcome.worktree_index)
        {
            let _ = outcome.write_changes();
        }
        // Conflicted paths only belong to the unstaged list.
        staged.retain(|path, _| unstaged.get(path).is_none_or(|c: &FileChange| c.kind != ChangeKind::Conflicted));

        Ok(WorkingStatus {
            unstaged: unstaged.into_values().collect(),
            staged: staged.into_values().collect(),
            operation: self.in_progress(),
        })
    }
}

/// Whether the index file on disk is still the one `index` was read from, by its trailing checksum.
fn unchanged_on_disk(index: &gix::worktree::IndexPersistedOrInMemory) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let gix::worktree::IndexPersistedOrInMemory::Persisted(index) = index else { return false };
    let Some(checksum) = index.checksum() else { return false };
    let expected = checksum.as_slice();
    let read_tail = || -> std::io::Result<Vec<u8>> {
        let mut file = std::fs::File::open(index.path())?;
        file.seek(SeekFrom::End(-(expected.len() as i64)))?;
        let mut tail = vec![0; expected.len()];
        file.read_exact(&mut tail)?;
        Ok(tail)
    };
    read_tail().is_ok_and(|tail| tail == expected)
}
