//! Listing the files git tracks.

use crate::error::Result;
use crate::repo::Repo;

impl Repo {
    /// Paths of all files in the index, sorted, with `/` separators.
    pub fn tracked_files(&self) -> Result<Vec<String>> {
        let index = self.gix().index_or_empty()?;
        let mut paths: Vec<String> = index.entries().iter().map(|e| e.path(&index).to_string()).collect();
        // Conflicted files appear once per stage.
        paths.dedup();
        Ok(paths)
    }
}
