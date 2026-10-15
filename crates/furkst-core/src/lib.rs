//! Git backend for Furkst.
//!
//! Everything that gitoxide can do is done in-process through [`gix`]. Operations gitoxide does not
//! implement yet (network transport for push, merges into the worktree, rebase, stash creation, …) are
//! delegated to the `git` executable through the single [`cli`] module.
//!
//! All public types are plain owned data so they can cross thread boundaries freely.

mod blame;
pub mod cli;
mod diff;
mod error;
mod graph;
mod gitflow;
mod history;
mod index_ops;
mod lfs;
mod merging;
mod merge;
mod model;
mod operation;
mod rebase;
mod refs;
mod repo;
mod status;
mod tracked;
mod tree;
mod watch;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use diff::{is_binary, DiffLine, DiffOptions, DiffTarget, FileDiff, Hunk, HunkAction, LineKind, LineSelection};
pub use error::{Error, Result};
pub use gitflow::{FlowConfig, FlowKind};
pub use graph::{GraphLine, GraphRow, Half, MAX_LANES, layout as layout_graph};
pub use history::{CommitSearch, HistoryQuery, SearchHit};
pub use lfs::LfsPointer;
pub use merging::MergeStyle;
pub use merge::{ConflictRegion, ConflictSides, Resolution, Segment, assemble, conflict_count, lines, parse_conflicts};
pub use model::*;
pub use operation::{Operation, ResetMode};
pub use rebase::{RebaseAction, RebaseStep, validate_rebase};
pub use refs::natural_cmp;
pub use repo::Repo;
pub use watch::{Change, Watch};

/// Object identifier used throughout the crate.
pub type Oid = gix::ObjectId;
