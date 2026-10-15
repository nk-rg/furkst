//! Browsing the complete file tree of a commit.

use gix::bstr::ByteSlice;
use gix::objs::tree::EntryKind;

use crate::diff::Side;
use crate::error::Result;
use crate::{Oid, Repo, TreeFile, TreeFileKind};

impl Repo {
    /// Every file, symlink and submodule in the tree of `commit`, recursively, in git's tree order.
    pub fn tree_files(&self, commit: Oid) -> Result<Vec<TreeFile>> {
        let repo = self.gix();
        let tree = repo.find_commit(commit)?.tree()?;
        let mut recorder = gix::traverse::tree::Recorder::default();
        tree.traverse().depthfirst(&mut recorder)?;
        Ok(recorder
            .records
            .into_iter()
            .filter_map(|entry| {
                let kind = match entry.mode.kind() {
                    EntryKind::Tree => return None,
                    EntryKind::Blob => TreeFileKind::File,
                    EntryKind::BlobExecutable => TreeFileKind::Executable,
                    EntryKind::Link => TreeFileKind::Symlink,
                    EntryKind::Commit => TreeFileKind::Submodule,
                };
                Some(TreeFile { path: entry.filepath.to_str_lossy().into_owned(), kind, id: entry.oid })
            })
            .collect())
    }

    /// The content of `path` as of `commit`; `None` if it doesn't exist there.
    pub fn file_at(&self, commit: Oid, path: &str) -> Result<Option<Vec<u8>>> {
        let repo = self.gix();
        let tree = repo.find_commit(commit)?.tree_id()?.detach();
        self.read_side(&repo, Side::Tree(tree), path)
    }
}
