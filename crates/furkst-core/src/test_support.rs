//! Helpers shared by unit tests.

use crate::{CommitInfo, Oid, Signature};

/// An id made of one repeated byte, so tests can name commits by number.
pub fn oid(n: u8) -> Oid {
    Oid::from_bytes_or_panic(&[n; 20])
}

/// A commit numbered `id` with the given parent numbers and empty metadata.
pub fn commit(id: u8, parents: &[u8]) -> CommitInfo {
    CommitInfo {
        id: oid(id),
        parents: parents.iter().map(|p| oid(*p)).collect(),
        author: Signature::default(),
        committer: Signature::default(),
        summary: String::new(),
        message: String::new(),
    }
}
