//! Line-by-line authorship of a file, computed by `gix-blame`.

use std::collections::HashMap;

use gix::bstr::{BStr, ByteSlice};

use crate::error::Result;
use crate::{BlameLine, BlameResult, Oid, Repo};

impl Repo {
    /// Blames `path` as of commit `at`.
    pub fn blame(&self, path: &str, at: Oid) -> Result<BlameResult> {
        let repo = self.gix();
        let options = gix::repository::blame_file::Options {
            rewrites: Some(Default::default()),
            ..Default::default()
        };
        let outcome = repo.blame_file(BStr::new(path), at, options)?;
        let text: Vec<&[u8]> = outcome.blob.lines_with_terminator().collect();

        let mut lines = Vec::with_capacity(text.len());
        let mut commits = HashMap::new();
        for entry in &outcome.entries {
            if let std::collections::hash_map::Entry::Vacant(slot) = commits.entry(entry.commit_id) {
                slot.insert(self.commit_info(entry.commit_id)?);
            }
            let start = entry.start_in_blamed_file as usize;
            for line_no in start..start + entry.len.get() as usize {
                let raw = text.get(line_no).copied().unwrap_or_default();
                lines.push(BlameLine {
                    commit: entry.commit_id,
                    line_no: line_no + 1,
                    text: raw.trim_end_with(|c| c == '\n' || c == '\r').to_str_lossy().into_owned(),
                });
            }
        }
        lines.sort_by_key(|l| l.line_no);
        Ok(BlameResult { lines, commits })
    }
}
