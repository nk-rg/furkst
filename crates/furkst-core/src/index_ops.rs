//! Index manipulation and commit creation, all done in-process with gitoxide.

use gix::bstr::{BStr, ByteSlice};
use gix::index::entry::{Flags, Mode, Stage, Stat};

use crate::error::{Error, GixResultExt, Result};
use crate::{ChangeKind, FileChange, Oid, Repo};

impl Repo {
    /// Adds the working tree version of each change to the index (`git add`, including deletions).
    pub fn stage(&self, changes: &[FileChange]) -> Result<()> {
        let repo = self.gix();
        let snapshot = repo.index_or_empty()?;
        let (mut pipeline, _) = repo.filter_pipeline(None)?;
        let mut updates = Vec::new();
        for path in paths_of(changes) {
            let object = pipeline.worktree_file_to_object(BStr::new(path), &snapshot)?;
            let stat = match &object {
                Some(_) => Stat::from_fs(&gix::index::fs::Metadata::from_path_no_follow(&self.workdir().join(path))?)
                    .unwrap_or_default(),
                None => Stat::default(),
            };
            updates.push((path, object.map(|(id, kind, _)| (id, Mode::from(kind), stat))));
        }
        self.edit_index(&repo, |index| {
            for (path, update) in updates {
                match update {
                    Some((id, mode, stat)) => upsert(index, path, id, mode, stat),
                    None => remove(index, path),
                }
            }
            Ok(())
        })
    }

    /// Resets the index entries of each change to their `HEAD` version (`git reset -- <paths>`).
    pub fn unstage(&self, changes: &[FileChange]) -> Result<()> {
        let repo = self.gix();
        let head = repo.head_tree_id_or_empty()?;
        let tree = repo.find_tree(head)?;
        let mut updates = Vec::new();
        for path in paths_of(changes) {
            let entry = tree.lookup_entry_by_path(path)?;
            updates.push((path, entry.map(|e| (e.object_id(), Mode::from(e.mode())))));
        }
        self.edit_index(&repo, |index| {
            for (path, entry) in updates {
                match entry {
                    // A zeroed stat makes the next status re-hash the file instead of trusting the cache.
                    Some((id, mode)) => upsert(index, path, id, mode, Stat::default()),
                    None => remove(index, path),
                }
            }
            Ok(())
        })
    }

    /// Throws away unstaged changes: tracked files are restored from the index, untracked ones deleted.
    pub fn discard(&self, changes: &[FileChange]) -> Result<()> {
        let repo = self.gix();
        for change in changes {
            let full = self.workdir().join(&change.path);
            if change.kind == ChangeKind::Untracked {
                if full.is_dir() {
                    std::fs::remove_dir_all(&full)?;
                } else {
                    std::fs::remove_file(&full)?;
                }
                continue;
            }
            match self.read_side(&repo, crate::diff::Side::Index, &change.path)? {
                Some(content) => {
                    if let Some(parent) = full.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    self.write_worktree_file(&repo, &change.path, &content)?;
                }
                None if full.exists() => std::fs::remove_file(&full)?,
                None => {}
            }
        }
        Ok(())
    }

    /// Stores `content` as the staged version of `path`.
    pub(crate) fn write_index_blob(&self, repo: &gix::Repository, path: &str, content: &[u8]) -> Result<()> {
        let id = repo.write_blob(content)?.detach();
        let index = repo.index_or_empty()?;
        let mode = index.entry_by_path(BStr::new(path)).map_or(Mode::FILE, |e| e.mode);
        self.edit_index(repo, |index| {
            upsert(index, path, id, mode, Stat::default());
            Ok(())
        })
    }

    fn edit_index(
        &self,
        repo: &gix::Repository,
        edit: impl FnOnce(&mut gix::index::File) -> Result<()>,
    ) -> Result<()> {
        let snapshot = repo.index_or_empty()?;
        let mut index: gix::index::File = (**snapshot).clone();
        edit(&mut index)?;
        // The cached tree no longer matches the entries; git rebuilds it when needed.
        index.remove_tree();
        index.remove_resolve_undo();
        index.sort_entries();
        index.write(Default::default()).git()
    }

    /// Writes the index as a tree object, refusing to do so while conflicts are unresolved.
    pub(crate) fn write_tree(&self, repo: &gix::Repository) -> Result<Oid> {
        let index = repo.index_or_empty()?;
        let mut editor = repo.edit_tree(repo.empty_tree().id)?;
        for entry in index.entries() {
            let path = entry.path(&index);
            if entry.stage() != Stage::Unconflicted {
                return Err(Error::msg(format!("resolve the conflict in {path} before committing")));
            }
            if entry.flags.contains(Flags::INTENT_TO_ADD) {
                continue;
            }
            let mode = entry.mode.to_tree_entry_mode().ok_or_else(|| Error::msg(format!("unsupported entry {path}")))?;
            editor.upsert(path, mode.kind(), entry.id)?;
        }
        Ok(editor.write()?.detach())
    }

    /// Commits the index. With `amend`, replaces the `HEAD` commit, keeping its author.
    ///
    /// Repositories that use commit hooks, signing, or are in the middle of a merge are committed through `git`
    /// so hooks run and the operation state is cleaned up exactly as git would.
    pub fn commit(&self, message: &str, amend: bool) -> Result<String> {
        if self.commit_needs_git() {
            let mut args = vec!["commit", "-m", message];
            if amend {
                args.push("--amend");
            }
            return self.cli().run(&args);
        }
        let repo = self.gix();
        let tree = self.write_tree(&repo)?;
        let head = repo.head()?;
        let head_id = head.id().map(|id| id.detach());
        let id = match (amend, head_id) {
            (true, Some(head_id)) => {
                let previous = repo.find_commit(head_id)?;
                let author = previous.author()?;
                let committer = repo.committer().ok_or_else(|| Error::msg("committer identity is not configured"))??;
                let parents: Vec<Oid> = previous.parent_ids().map(|p| p.detach()).collect();
                let commit = repo.new_commit_as(committer, author, message, tree, parents)?;
                self.move_head(&repo, head_id, commit.id, &format!("commit (amend): {}", summary(message)))?;
                commit.id
            }
            (true, None) => return Err(Error::msg("there is no commit to amend")),
            (false, parent) => repo.commit("HEAD", message, tree, parent)?.detach(),
        };
        Ok(format!("Committed {}", crate::short_id(&id)))
    }

    /// Points `HEAD` (or the branch it refers to) from `from` to `to`, recording `log_message` in the reflog.
    pub(crate) fn move_head(&self, repo: &gix::Repository, from: Oid, to: Oid, log_message: &str) -> Result<()> {
        use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
        repo.edit_reference(RefEdit {
            change: Change::Update {
                log: LogChange { mode: RefLog::AndReference, force_create_reflog: false, message: log_message.into() },
                expected: PreviousValue::MustExistAndMatch(gix::refs::Target::Object(from)),
                new: gix::refs::Target::Object(to),
            },
            name: "HEAD".try_into().git()?,
            deref: true,
        })?;
        Ok(())
    }

    fn commit_needs_git(&self) -> bool {
        if self.in_progress().is_some() {
            return true;
        }
        let repo = self.gix();
        let config = repo.config_snapshot();
        if config.boolean("commit.gpgSign").unwrap_or(false) {
            return true;
        }
        let hooks_dir = config
            .trusted_path("core.hooksPath")
            .ok()
            .flatten()
            .map(|p| self.workdir().join(p))
            .unwrap_or_else(|| repo.common_dir().join("hooks"));
        ["pre-commit", "prepare-commit-msg", "commit-msg", "post-commit"]
            .iter()
            .any(|hook| hooks_dir.join(hook).is_file())
    }
}

fn summary(message: &str) -> &str {
    message.lines().next().unwrap_or_default()
}

/// Every path touched by `changes`, including the sources of renames.
fn paths_of(changes: &[FileChange]) -> impl Iterator<Item = &str> {
    changes.iter().flat_map(|c| std::iter::once(c.path.as_str()).chain(c.old_path.as_deref()))
}

fn upsert(index: &mut gix::index::File, path: &str, id: Oid, mode: Mode, stat: Stat) {
    remove_conflict_stages(index, path);
    match index.entry_index_by_path_and_stage(BStr::new(path), Stage::Unconflicted) {
        Some(i) => {
            let entry = &mut index.entries_mut()[i];
            entry.id = id;
            entry.mode = mode;
            entry.stat = stat;
            entry.flags.remove(Flags::INTENT_TO_ADD);
        }
        None => index.dangerously_push_entry(stat, id, Flags::empty(), mode, BStr::new(path)),
    }
}

fn remove(index: &mut gix::index::File, path: &str) {
    index.remove_entries(|_, p, _| p == path.as_bytes().as_bstr());
}

fn remove_conflict_stages(index: &mut gix::index::File, path: &str) {
    index.remove_entries(|_, p, e| p == path.as_bytes().as_bstr() && e.stage() != Stage::Unconflicted);
}
