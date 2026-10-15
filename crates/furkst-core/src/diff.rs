//! Line diffs and the hunk/line selection logic used to stage, unstage and discard partial changes.
//!
//! Every diff is computed between two [`Side`]s. Displaying a diff and applying a selection of it go through
//! the very same [`hunks`] function, so the hunk and line indices the UI shows always match what gets applied.

use std::collections::HashSet;

use gix::bstr::{BStr, ByteSlice};
use similar::{Algorithm, DiffOp, DiffTag};

use crate::error::Result;
use crate::{FileChange, LfsPointer, Oid, Repo};

/// Files larger than this are not diffed line by line.
const MAX_DIFF_BYTES: usize = 8 * 1024 * 1024;

/// Images larger than this are not loaded for display.
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

const IMAGE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "ico"];

/// Which pair of file versions to compare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffTarget {
    /// A commit against its first parent.
    Commit(Oid),
    /// Index against `HEAD`.
    Staged,
    /// Working tree against the index.
    Unstaged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffOptions {
    pub context_lines: usize,
    pub ignore_whitespace: bool,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self { context_lines: 3, ignore_whitespace: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
    /// Line content without its line terminator.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// Zero-based first line in the old version.
    pub old_start: usize,
    pub old_len: usize,
    /// Zero-based first line in the new version.
    pub new_start: usize,
    pub new_len: usize,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    /// The classic `@@ -a,b +c,d @@` header.
    pub fn header(&self) -> String {
        format!("@@ -{},{} +{},{} @@", self.old_start + 1, self.old_len, self.new_start + 1, self.new_len)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDiff {
    Text { hunks: Vec<Hunk> },
    Binary { old_size: Option<usize>, new_size: Option<usize> },
    /// Both versions of an image file, as raw encoded bytes (`None` if the file doesn't exist on that side).
    Image { old: Option<Vec<u8>>, new: Option<Vec<u8>> },
    /// A Git LFS file whose content isn't in the local cache yet; the pointers still tell its size.
    Lfs { old: Option<LfsPointer>, new: Option<LfsPointer> },
    TooLarge,
}

impl FileDiff {
    pub fn hunks(&self) -> &[Hunk] {
        match self {
            Self::Text { hunks } => hunks,
            _ => &[],
        }
    }
}

/// What to do with a selection of a working-copy diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HunkAction {
    Stage,
    Unstage,
    Discard,
}

impl HunkAction {
    /// The diff the selection refers to.
    pub fn target(self) -> DiffTarget {
        match self {
            Self::Stage | Self::Discard => DiffTarget::Unstaged,
            Self::Unstage => DiffTarget::Staged,
        }
    }
}

/// Selected changed lines, identified by `(hunk index, line index within the hunk)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineSelection(pub HashSet<(usize, usize)>);

impl LineSelection {
    /// Selects every changed line of the given hunks.
    pub fn whole_hunks(hunks: &[Hunk], selected: impl IntoIterator<Item = usize>) -> Self {
        Self(
            selected
                .into_iter()
                .filter_map(|h| hunks.get(h).map(|hunk| (h, hunk)))
                .flat_map(|(h, hunk)| {
                    hunk.lines.iter().enumerate().filter(|(_, l)| l.kind != LineKind::Context).map(move |(i, _)| (h, i))
                })
                .collect(),
        )
    }

    pub fn contains(&self, hunk: usize, line: usize) -> bool {
        self.0.contains(&(hunk, line))
    }
}

/// One version of a file.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Side {
    Empty,
    Tree(Oid),
    Index,
    Worktree,
}

impl DiffTarget {
    pub(crate) fn sides(&self, repo: &gix::Repository) -> Result<(Side, Side)> {
        Ok(match self {
            Self::Commit(id) => {
                let commit = repo.find_commit(*id)?;
                let new = Side::Tree(commit.tree_id()?.detach());
                let old = match commit.parent_ids().next() {
                    Some(parent) => Side::Tree(repo.find_commit(parent)?.tree_id()?.detach()),
                    None => Side::Empty,
                };
                (old, new)
            }
            Self::Staged => (Side::Tree(repo.head_tree_id_or_empty()?.detach()), Side::Index),
            Self::Unstaged => (Side::Index, Side::Worktree),
        })
    }
}

impl Repo {
    /// Line diff of one file of `target`.
    pub fn file_diff(&self, target: &DiffTarget, change: &FileChange, options: DiffOptions) -> Result<FileDiff> {
        let repo = self.gix();
        let (old_side, new_side) = target.sides(&repo)?;
        let old_path = change.old_path.as_deref().unwrap_or(&change.path);
        let old = self.read_side(&repo, old_side, old_path)?;
        let new = self.read_side(&repo, new_side, &change.path)?;
        let (old, new) = match (self.resolve_lfs(old), self.resolve_lfs(new)) {
            (Ok(old), Ok(new)) => (old, new),
            (old, new) => return Ok(FileDiff::Lfs { old: old.err(), new: new.err() }),
        };
        if is_image_path(&change.path) {
            if old.as_deref().into_iter().chain(new.as_deref()).any(|data| data.len() > MAX_IMAGE_BYTES) {
                return Ok(FileDiff::TooLarge);
            }
            return Ok(FileDiff::Image { old, new });
        }
        Ok(diff_contents(old.as_deref(), new.as_deref(), options))
    }

    /// Stages, unstages or discards the selected lines of `path`.
    pub fn apply_selection(
        &self,
        action: HunkAction,
        path: &str,
        selection: &LineSelection,
        options: DiffOptions,
    ) -> Result<()> {
        let repo = self.gix();
        let (old_side, new_side) = action.target().sides(&repo)?;
        let old = self.read_side(&repo, old_side, path)?.unwrap_or_default();
        let new = self.read_side(&repo, new_side, path)?.unwrap_or_default();
        let hunks = hunks(&old, &new, options);
        let result = match action {
            HunkAction::Stage => blend(&old, &new, &hunks, |h, l| selection.contains(h, l)),
            // Reverting the selection is the same as applying everything that is not selected.
            HunkAction::Unstage | HunkAction::Discard => blend(&old, &new, &hunks, |h, l| !selection.contains(h, l)),
        };
        match action {
            HunkAction::Stage | HunkAction::Unstage => self.write_index_blob(&repo, path, &result),
            HunkAction::Discard => self.write_worktree_file(&repo, path, &result),
        }
    }

    /// Reads `path` from `side`, converted to its git (normalized) representation. `None` if it doesn't exist.
    pub(crate) fn read_side(&self, repo: &gix::Repository, side: Side, path: &str) -> Result<Option<Vec<u8>>> {
        match side {
            Side::Empty => Ok(None),
            Side::Tree(tree) => {
                let tree = repo.find_tree(tree)?;
                let Some(entry) = tree.lookup_entry_by_path(path)? else { return Ok(None) };
                if !entry.mode().is_blob_or_symlink() {
                    return Ok(Some(format!("Subproject commit {}\n", entry.object_id()).into_bytes()));
                }
                Ok(Some(entry.object()?.detach().data))
            }
            Side::Index => {
                let index = repo.index_or_empty()?;
                let Some(entry) = index.entry_by_path(BStr::new(path)) else { return Ok(None) };
                if entry.mode.is_submodule() {
                    return Ok(Some(format!("Subproject commit {}\n", entry.id).into_bytes()));
                }
                Ok(Some(repo.find_object(entry.id)?.detach().data))
            }
            Side::Worktree => {
                let full = self.workdir().join(path);
                let Ok(meta) = std::fs::symlink_metadata(&full) else { return Ok(None) };
                if meta.is_symlink() {
                    return Ok(Some(gix::path::into_bstr(std::fs::read_link(&full)?)?.into_owned().into()));
                }
                if meta.is_dir() {
                    return Ok(Some(match gix::open(&full).ok().and_then(|sub| sub.head_id().ok().map(|id| id.detach())) {
                        Some(head) => format!("Subproject commit {head}\n").into_bytes(),
                        None => Vec::new(),
                    }));
                }
                let (mut pipeline, index) = repo.filter_pipeline(None)?;
                let file = std::fs::File::open(&full)?;
                let mut converted = pipeline.convert_to_git(file, std::path::Path::new(path), &index)?;
                let mut buf = Vec::new();
                match converted.as_read() {
                    Some(read) => {
                        read.read_to_end(&mut buf)?;
                    }
                    None => buf.extend_from_slice(converted.as_bytes().unwrap_or_default()),
                }
                Ok(Some(buf))
            }
        }
    }

    /// Writes `content` (in git representation) to the working tree, applying worktree filters like `autocrlf`.
    pub(crate) fn write_worktree_file(&self, repo: &gix::Repository, path: &str, content: &[u8]) -> Result<()> {
        let (mut pipeline, _) = repo.filter_pipeline(None)?;
        let mut converted = pipeline.convert_to_worktree(content, BStr::new(path), Default::default())?;
        let mut buf = Vec::new();
        match converted.as_read() {
            Some(read) => {
                read.read_to_end(&mut buf)?;
            }
            None => buf.extend_from_slice(converted.as_bytes().unwrap_or(content)),
        }
        std::fs::write(self.workdir().join(path), buf)?;
        Ok(())
    }
}

/// Whether `path` names an image format we can preview, judged by its extension.
fn is_image_path(path: &str) -> bool {
    let ext = path.rsplit_once('.').map_or("", |(_, ext)| ext);
    IMAGE_EXTENSIONS.iter().any(|known| ext.eq_ignore_ascii_case(known))
}

/// Whether `data` looks like binary content (a NUL byte near the start).
pub fn is_binary(data: &[u8]) -> bool {
    data[..data.len().min(8000)].contains(&0)
}

pub(crate) fn diff_contents(old: Option<&[u8]>, new: Option<&[u8]>, options: DiffOptions) -> FileDiff {
    let (old_bytes, new_bytes) = (old.unwrap_or_default(), new.unwrap_or_default());
    if is_binary(old_bytes) || is_binary(new_bytes) {
        return FileDiff::Binary { old_size: old.map(<[u8]>::len), new_size: new.map(<[u8]>::len) };
    }
    if old_bytes.len().max(new_bytes.len()) > MAX_DIFF_BYTES {
        return FileDiff::TooLarge;
    }
    FileDiff::Text { hunks: hunks(old_bytes, new_bytes, options) }
}

fn lines(data: &[u8]) -> Vec<&[u8]> {
    data.split_inclusive(|b| *b == b'\n').collect()
}

fn display(line: &[u8]) -> String {
    line.strip_suffix(b"\n").map_or(line, |l| l.strip_suffix(b"\r").unwrap_or(l)).to_str_lossy().into_owned()
}

/// Computes the hunks turning `old` into `new`.
pub(crate) fn hunks(old: &[u8], new: &[u8], options: DiffOptions) -> Vec<Hunk> {
    let (old_lines, new_lines) = (lines(old), lines(new));
    let ops = if options.ignore_whitespace {
        let key = |line: &&[u8]| line.fields_with(|c| c.is_whitespace()).flat_map(|w| w.iter().copied()).collect::<Vec<u8>>();
        similar::capture_diff_slices_by_key(Algorithm::Myers, &old_lines, &new_lines, key)
    } else {
        similar::capture_diff_slices(Algorithm::Myers, &old_lines, &new_lines)
    };
    similar::group_diff_ops(ops, options.context_lines)
        .into_iter()
        .map(|group| build_hunk(&group, &old_lines, &new_lines))
        .collect()
}

fn build_hunk(group: &[DiffOp], old_lines: &[&[u8]], new_lines: &[&[u8]]) -> Hunk {
    let first = group.first().expect("groups are never empty");
    let last = group.last().expect("groups are never empty");
    let mut lines = Vec::new();
    for op in group {
        let (tag, old_range, new_range) = op.as_tag_tuple();
        match tag {
            DiffTag::Equal => lines.extend(old_range.zip(new_range).map(|(o, n)| DiffLine {
                kind: LineKind::Context,
                old_no: Some(o + 1),
                new_no: Some(n + 1),
                text: display(old_lines[o]),
            })),
            DiffTag::Delete | DiffTag::Insert | DiffTag::Replace => {
                lines.extend(old_range.map(|o| DiffLine {
                    kind: LineKind::Removed,
                    old_no: Some(o + 1),
                    new_no: None,
                    text: display(old_lines[o]),
                }));
                lines.extend(new_range.map(|n| DiffLine {
                    kind: LineKind::Added,
                    old_no: None,
                    new_no: Some(n + 1),
                    text: display(new_lines[n]),
                }));
            }
        }
    }
    let old_start = first.old_range().start;
    let new_start = first.new_range().start;
    Hunk {
        old_start,
        old_len: last.old_range().end - old_start,
        new_start,
        new_len: last.new_range().end - new_start,
        lines,
    }
}

/// Builds a version of the file that starts from `old` and applies only the changed lines for which
/// `take(hunk, line)` is true.
fn blend(old: &[u8], new: &[u8], hunks: &[Hunk], take: impl Fn(usize, usize) -> bool) -> Vec<u8> {
    let (old_lines, new_lines) = (lines(old), lines(new));
    let mut out = Vec::with_capacity(old.len().max(new.len()));
    let mut cursor = 0;
    for (h, hunk) in hunks.iter().enumerate() {
        old_lines[cursor..hunk.old_start].iter().for_each(|l| out.extend_from_slice(l));
        for (l, line) in hunk.lines.iter().enumerate() {
            let keep_old = match line.kind {
                LineKind::Context => true,
                LineKind::Removed => !take(h, l),
                LineKind::Added => {
                    if take(h, l) {
                        out.extend_from_slice(new_lines[line.new_no.expect("added lines have a number") - 1]);
                    }
                    false
                }
            };
            if keep_old {
                out.extend_from_slice(old_lines[line.old_no.expect("old lines have a number") - 1]);
            }
        }
        cursor = hunk.old_start + hunk.old_len;
    }
    old_lines[cursor..].iter().for_each(|l| out.extend_from_slice(l));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &[u8] = b"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n";
    const NEW: &[u8] = b"a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nK\nl\nm\n";

    fn opts(context_lines: usize) -> DiffOptions {
        DiffOptions { context_lines, ignore_whitespace: false }
    }

    #[test]
    fn hunks_are_split_by_context() {
        let hunks = hunks(OLD, NEW, opts(1));
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].header(), "@@ -1,3 +1,3 @@");
    }

    #[test]
    fn blend_all_or_nothing() {
        let hunks = hunks(OLD, NEW, opts(1));
        assert_eq!(blend(OLD, NEW, &hunks, |_, _| true), NEW);
        assert_eq!(blend(OLD, NEW, &hunks, |_, _| false), OLD);
    }

    #[test]
    fn blend_single_hunk() {
        let hunks = hunks(OLD, NEW, opts(1));
        let selection = LineSelection::whole_hunks(&hunks, [1]);
        let staged = blend(OLD, NEW, &hunks, |h, l| selection.contains(h, l));
        assert_eq!(staged, b"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nK\nl\nm\n");
        // Reverting the complement of the remaining hunk from NEW gives the same file.
        let unstaged = blend(OLD, NEW, &hunks, |h, l| !LineSelection::whole_hunks(&hunks, [0]).contains(h, l));
        assert_eq!(unstaged, staged);
    }

    #[test]
    fn blend_single_line() {
        let hunks = hunks(b"x\ny\n", b"x\n1\n2\n", opts(3));
        // Lines: x (ctx), y (removed), 1 (added), 2 (added). Take only the removal and "2".
        let staged = blend(b"x\ny\n", b"x\n1\n2\n", &hunks, |_, l| l == 1 || l == 3);
        assert_eq!(staged, b"x\n2\n");
    }

    #[test]
    fn binary_is_detected() {
        assert!(matches!(diff_contents(Some(b"\0abc"), None, opts(3)), FileDiff::Binary { .. }));
    }
}
