use std::collections::{BinaryHeap, HashMap, HashSet};
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError};

use aho_corasick::AhoCorasick;
use gix::bstr::ByteSlice;
use gix::prelude::FindExt;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::error::{GixResultExt, Result};
use crate::repo::{WalkCache, commit_info};
use crate::{ChangeKind, CommitDetails, CommitInfo, FileChange, Oid, RefsSnapshot, Repo};

/// The shortest search text also looked up as an abbreviated hash, like git's minimum abbreviation.
const MIN_HASH_PREFIX: usize = 4;

/// What to list in the history view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryQuery {
    /// Commits to start walking from.
    pub tips: Vec<Oid>,
    pub limit: usize,
    /// Only list commits that changed this path (file history).
    pub path: Option<String>,
}

/// One phrase looked up in every commit's hash, author and message, and in ref names, ignoring case.
#[derive(Clone, Debug, Default)]
pub struct CommitSearch {
    /// Lowercased.
    text: String,
    /// Finds an ASCII `text` in raw bytes without lowercasing them; a non-ASCII one compares lowercased text instead.
    ascii: Option<AhoCorasick>,
    /// Also match commits that changed a file whose path contains the text (slower: diffs every commit).
    paths: bool,
}

/// A commit matching a search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub id: Oid,
    /// How many commits the history walk reaches before this one, so a history `limit` above it lists it.
    pub position: usize,
}

impl CommitSearch {
    pub fn new(text: &str) -> Self {
        let text = text.trim().to_lowercase();
        let ascii = (text.is_ascii() && !text.is_empty()).then(|| {
            AhoCorasick::builder().ascii_case_insensitive(true).build([&text]).expect("a single literal always builds")
        });
        Self { text, ascii, paths: false }
    }

    /// Also matches commits by the paths of the files they changed.
    pub fn in_paths(mut self, paths: bool) -> Self {
        self.paths = paths;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn contains(&self, haystack: &[u8]) -> bool {
        match &self.ascii {
            Some(finder) => finder.is_match(haystack),
            None => haystack.to_str_lossy().to_lowercase().contains(&self.text),
        }
    }

    /// Whether the commit's hash starts with the text, or its author or message contains it.
    fn matches_commit(&self, id: &Oid, raw: &[u8]) -> bool {
        let mut hex = [0; 64];
        if self.text.len() >= MIN_HASH_PREFIX && id.hex_to_buf(&mut hex).starts_with(&self.text) {
            return true;
        }
        let (author, message) = author_and_message(raw);
        self.contains(author) || self.contains(message)
    }

    /// Whether a file `id` changed (compared to its first parent) has the text in its path; stops at the first one.
    fn matches_changed_path(&self, repo: &gix::Repository, id: Oid) -> Result<bool> {
        let commit = repo.find_commit(id)?;
        let mut found = false;
        let diff = first_parent_tree(repo, &commit)?
            .changes()?
            .options(|options| {
                options.track_path().track_rewrites(None);
            })
            .for_each_to_obtain_tree(&commit.tree()?, |change| {
                found = self.contains(change.location());
                Ok(if found { ControlFlow::Break(()) } else { ControlFlow::Continue(()) })
            });
        // Stopping at the first match reports the diff as cancelled.
        if !found {
            diff?;
        }
        Ok(found)
    }

    /// The commits pointed to by a branch, tag or other ref whose name contains the text.
    pub fn ref_targets(&self, refs: &RefsSnapshot) -> HashSet<Oid> {
        if self.is_empty() {
            return HashSet::new();
        }
        let named = |labels: &Vec<crate::RefLabel>| labels.iter().any(|label| self.contains(label.name.as_bytes()));
        refs.labels.iter().filter(|(_, labels)| named(labels)).map(|(id, _)| *id).collect()
    }
}

/// The author's name and email, and the message, of a raw commit object.
fn author_and_message(raw: &[u8]) -> (&[u8], &[u8]) {
    // Header continuation lines (like a signature's) start with a space, so the first empty line ends the headers.
    let (headers, message) = raw.find(b"\n\n").map_or((raw, &b""[..]), |end| (&raw[..end], &raw[end + 2..]));
    let author = headers.lines().find_map(|line| line.strip_prefix(b"author ")).unwrap_or_default();
    // Without the timestamp and time zone, so searching for digits doesn't match every date.
    let author = author.rfind_byte(b'>').map_or(author, |end| &author[..=end]);
    (author, message)
}

impl Repo {
    /// Commits reachable from the query's tips, children always before parents, otherwise newest first
    /// (like `git log --date-order`), which the graph layout relies on.
    pub fn history(&self, query: &HistoryQuery) -> Result<Vec<CommitInfo>> {
        if query.tips.is_empty() {
            return Ok(Vec::new());
        }
        let commits = match (&query.path, self.cached_walk(&query.tips)) {
            // A search walked these tips already, so even a deep page (like one reaching a search match) only
            // reads its commits, on all cores.
            (None, Some(ids)) => self.in_parallel(&ids[..query.limit.min(ids.len())], |repo, _, ids| {
                ids.iter().map(|id| commit_info(&repo.find_commit(*id)?)).collect()
            })?,
            (path, _) => {
                let repo = self.gix();
                let ids: Box<dyn Iterator<Item = Result<Oid>>> = match path {
                    Some(path) => Box::new(FileWalk::new(&repo, path, &query.tips)?),
                    None => Box::new(walk(&repo, &query.tips)?),
                };
                ids.take(query.limit).map(|id| commit_info(&repo.find_commit(id?)?)).collect::<Result<_>>()?
            }
        };
        Ok(children_first(commits))
    }

    /// Every commit reachable from `tips` that matches `search` or is one of `ref_targets`, in the order `history`
    /// walks them. Commits are matched on all cores from their raw objects. Setting `cancel` stops the search early,
    /// returning what it found so far, for a search a newer one replaced.
    pub fn search_commits(
        &self,
        tips: &[Oid],
        search: &CommitSearch,
        ref_targets: &HashSet<Oid>,
        cancel: &AtomicBool,
    ) -> Result<Vec<SearchHit>> {
        if tips.is_empty() || search.is_empty() {
            return Ok(Vec::new());
        }
        let ids = self.walked(tips)?;
        self.in_parallel(&ids, |repo, start, ids| {
            let mut raw = Vec::new();
            let mut hits = Vec::new();
            for (offset, id) in ids.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                if ref_targets.contains(id)
                    || search.matches_commit(id, repo.objects.find(id, &mut raw).git()?.data)
                    || (search.paths && search.matches_changed_path(repo, *id)?)
                {
                    hits.push(SearchHit { id: *id, position: start + offset });
                }
            }
            Ok(hits)
        })
    }

    /// Walks the history from `tips` ahead of `search_commits`, so the first search doesn't wait for the walk.
    pub fn prepare_search(&self, tips: &[Oid]) -> Result<()> {
        self.walked(tips).map(drop)
    }

    /// Runs `work` over `ids` split across all cores, each part with its own repository handle and the index of its
    /// first id, and joins the parts' results in order.
    fn in_parallel<T: Send>(
        &self,
        ids: &[Oid],
        work: impl Fn(&gix::Repository, usize, &[Oid]) -> Result<Vec<T>> + Sync,
    ) -> Result<Vec<T>> {
        let threads = std::thread::available_parallelism().map_or(1, usize::from);
        let part_len = ids.len().div_ceil(threads).max(1);
        let work = &work;
        let parts: Vec<Result<Vec<T>>> = std::thread::scope(|scope| {
            let workers: Vec<_> = ids
                .chunks(part_len)
                .enumerate()
                .map(|(part, ids)| scope.spawn(move || work(&self.gix(), part * part_len, ids)))
                .collect();
            workers.into_iter().map(|worker| worker.join().expect("worker thread panicked")).collect()
        });
        Ok(parts.into_iter().collect::<Result<Vec<_>>>()?.into_iter().flatten().collect())
    }

    /// Every commit reachable from `tips` in walk order, remembered for the last tips: the walk takes far longer than
    /// matching (~0.5 s against ~0.05 s on git/git), and every keystroke searches the same tips. The commits reachable
    /// from given tips never change, so the result can't go stale. A search that arrives mid-walk waits to reuse it.
    fn walked(&self, tips: &[Oid]) -> Result<Arc<Vec<Oid>>> {
        let mut cache = self.walk_cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(ids) = walk_for(&cache, tips) {
            return Ok(ids);
        }
        let ids: Arc<Vec<Oid>> = Arc::new(walk(&self.gix(), tips)?.collect::<Result<_>>()?);
        *cache = Some((tips.to_vec(), Arc::clone(&ids)));
        Ok(ids)
    }

    /// The remembered walk from `tips`, if the last one walked started there. Never waits for a walk under way (it
    /// holds the lock): streaming a page is quicker than waiting for a whole walk.
    fn cached_walk(&self, tips: &[Oid]) -> Option<Arc<Vec<Oid>>> {
        let cache = match self.walk_cache.try_lock() {
            Ok(cache) => cache,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
        walk_for(&cache, tips)
    }

    /// How `commit` changed `path` compared to its first parent, for showing one version in the file's history.
    pub fn file_change_at(&self, commit: Oid, path: &str) -> Result<FileChange> {
        let repo = self.gix();
        let commit = repo.find_commit(commit)?;
        let has_file = |tree: gix::Tree<'_>| -> Result<bool> { Ok(tree.lookup_entry_by_path(path)?.is_some()) };
        let (before, after) = (has_file(first_parent_tree(&repo, &commit)?)?, has_file(commit.tree()?)?);
        let kind = match (before, after) {
            (false, _) => ChangeKind::Added,
            (true, false) => ChangeKind::Deleted,
            (true, true) => ChangeKind::Modified,
        };
        Ok(FileChange { path: path.to_owned(), old_path: None, kind })
    }

    /// A commit and the files it changed compared to its first parent; for a stash, also the untracked files it saved.
    pub fn commit_details(&self, id: Oid) -> Result<CommitDetails> {
        let repo = self.gix();
        let commit = repo.find_commit(id)?;
        let changes = tree_changes(&repo, &first_parent_tree(&repo, &commit)?, &commit.tree()?)?;
        let stash_untracked = match stash_untracked_commit(&repo, &commit)? {
            Some(untracked) => {
                let files = tree_changes(&repo, &repo.empty_tree(), &untracked.tree()?)?;
                Some((untracked.id, files.into_iter().map(|f| FileChange { kind: ChangeKind::Untracked, ..f }).collect()))
            }
            None => None,
        };
        Ok(CommitDetails { info: commit_info(&commit)?, changes, stash_untracked })
    }
}

/// The tree a commit is compared with to see what it changed: its first parent's, or an empty one for a root commit.
fn first_parent_tree<'r>(repo: &'r gix::Repository, commit: &gix::Commit<'_>) -> Result<gix::Tree<'r>> {
    Ok(match commit.parent_ids().next() {
        Some(parent) => repo.find_commit(parent)?.tree()?,
        None => repo.empty_tree(),
    })
}

/// The files that differ between two trees, sorted by path.
fn tree_changes(repo: &gix::Repository, old: &gix::Tree<'_>, new: &gix::Tree<'_>) -> Result<Vec<FileChange>> {
    let mut changes: Vec<FileChange> = repo
        .diff_tree_to_tree(old, new, None)?
        .into_iter()
        .filter_map(|change| {
            use gix::object::tree::diff::ChangeDetached as C;
            if change.entry_mode().is_tree() {
                return None;
            }
            Some(match change {
                C::Addition { location, .. } => file_change(location, None, ChangeKind::Added),
                C::Deletion { location, .. } => file_change(location, None, ChangeKind::Deleted),
                C::Modification { location, previous_entry_mode, entry_mode, .. } => {
                    let kind = if previous_entry_mode.kind() == entry_mode.kind() {
                        ChangeKind::Modified
                    } else {
                        ChangeKind::TypeChange
                    };
                    file_change(location, None, kind)
                }
                C::Rewrite { source_location, location, copy, .. } => {
                    let kind = if copy { ChangeKind::Copied } else { ChangeKind::Renamed };
                    file_change(location, Some(source_location), kind)
                }
            })
        })
        .collect();
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

/// Where `git stash --include-untracked` keeps a stash's untracked files: its third parent, a commit of its own.
fn stash_untracked_commit<'r>(repo: &'r gix::Repository, commit: &gix::Commit<'_>) -> Result<Option<gix::Commit<'r>>> {
    let Some(third) = commit.parent_ids().nth(2) else { return Ok(None) };
    let third = repo.find_commit(third)?;
    let untracked = third.parent_ids().next().is_none() && third.message_raw()?.starts_with(b"untracked files on ");
    Ok(untracked.then_some(third))
}

fn file_change(path: impl ToString, old_path: Option<gix::bstr::BString>, kind: ChangeKind) -> FileChange {
    FileChange { path: path.to_string(), old_path: old_path.map(|p| p.to_string()), kind }
}

/// Whether `commit` changed `path` compared to all of its parents (git's default history simplification).
/// Holds back any commit listed before one of its children (possible when a clock was wrong, since the walk goes by
/// date) until the last of them, so the graph layout always sees children first. Otherwise the order is unchanged.
fn children_first(commits: Vec<CommitInfo>) -> Vec<CommitInfo> {
    let listed: HashSet<Oid> = commits.iter().map(|c| c.id).collect();
    let mut unseen_children: HashMap<Oid, usize> = HashMap::new();
    for parent in commits.iter().flat_map(|c| &c.parents).filter(|p| listed.contains(*p)) {
        *unseen_children.entry(*parent).or_default() += 1;
    }
    let mut held: HashMap<Oid, CommitInfo> = HashMap::new();
    let mut ordered = Vec::with_capacity(commits.len());
    for commit in commits {
        if unseen_children.get(&commit.id).is_some_and(|n| *n > 0) {
            held.insert(commit.id, commit);
            continue;
        }
        let mut ready = vec![commit];
        while let Some(commit) = ready.pop() {
            for parent in &commit.parents {
                if let Some(count) = unseen_children.get_mut(parent) {
                    *count -= 1;
                    if *count == 0
                        && let Some(parent) = held.remove(parent)
                    {
                        ready.push(parent);
                    }
                }
            }
            ordered.push(commit);
        }
    }
    ordered
}

/// Every commit reachable from `tips`, newest first. Streams, so a page only reads the commits it lists: a topological
/// sort reads the entire history first, which takes about a second on big repositories without a commit-graph file.
fn walk<'r>(repo: &'r gix::Repository, tips: &[Oid]) -> Result<impl Iterator<Item = Result<Oid>> + 'r> {
    let walk = repo.rev_walk(tips.iter().copied()).sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst)).all()?;
    Ok(walk.map(|info| Ok(info.git()?.id)))
}

fn walk_for(cache: &Option<WalkCache>, tips: &[Oid]) -> Option<Arc<Vec<Oid>>> {
    cache.as_ref().filter(|(walked_tips, _)| walked_tips == tips).map(|(_, ids)| Arc::clone(ids))
}

/// The commits that changed one path, newest first, simplified like `git log -- <path>`: at a merge that kept one
/// parent's version of the path, only that parent is followed, so side branches that never touched it aren't walked.
struct FileWalk<'r> {
    repo: &'r gix::Repository,
    path: &'r str,
    /// Commits to visit, by commit time.
    queue: BinaryHeap<(i64, Oid)>,
    queued: HashSet<Oid>,
}

impl<'r> FileWalk<'r> {
    fn new(repo: &'r gix::Repository, path: &'r str, tips: &[Oid]) -> Result<Self> {
        let mut walk = Self { repo, path, queue: BinaryHeap::new(), queued: HashSet::new() };
        for tip in tips {
            walk.enqueue(*tip)?;
        }
        Ok(walk)
    }

    fn enqueue(&mut self, id: Oid) -> Result<()> {
        if self.queued.insert(id) {
            let time = self.repo.find_commit(id)?.time()?.seconds;
            self.queue.push((time, id));
        }
        Ok(())
    }

    fn entry(&self, commit: &gix::Commit<'_>) -> Result<Option<Oid>> {
        Ok(commit.tree()?.lookup_entry_by_path(self.path)?.map(|e| e.object_id()))
    }

    /// Visits one commit and queues the parents to follow; `true` if it changed the path.
    fn visit(&mut self, id: Oid) -> Result<bool> {
        let commit = self.repo.find_commit(id)?;
        let own = self.entry(&commit)?;
        let parents: Vec<Oid> = commit.parent_ids().map(|p| p.detach()).collect();
        let mut differs = Vec::with_capacity(parents.len());
        for parent in &parents {
            if self.entry(&self.repo.find_commit(*parent)?)? == own {
                // Same version as this parent: the path's history continues there alone.
                self.enqueue(*parent)?;
                return Ok(false);
            }
            differs.push(*parent);
        }
        for parent in differs {
            self.enqueue(parent)?;
        }
        // Differs from every parent, or a root commit that added it.
        Ok(!parents.is_empty() || own.is_some())
    }
}

impl Iterator for FileWalk<'_> {
    type Item = Result<Oid>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((_, id)) = self.queue.pop() {
            match self.visit(id) {
                Ok(true) => return Some(Ok(id)),
                Ok(false) => {}
                Err(error) => return Some(Err(error)),
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{commit, oid};

    fn ids(commits: &[CommitInfo]) -> Vec<Oid> {
        commits.iter().map(|c| c.id).collect()
    }

    const RAW: &[u8] = b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
        parent 1111111111111111111111111111111111111111\n\
        author Ana Mu\xc3\xb1oz <ana@example.com> 1700000000 +0100\n\
        committer Bob Builder <bob@example.com> 1700000099 +0100\n\
        gpgsig -----BEGIN PGP SIGNATURE-----\n \n signature\n -----END PGP SIGNATURE-----\n\
        \n\
        Fix the Se\xc3\x91al parser\n\nBody text\n";

    #[test]
    fn splits_a_raw_commit_into_author_and_message() {
        let (author, message) = author_and_message(RAW);
        assert_eq!(author, "Ana Muñoz <ana@example.com>".as_bytes(), "without the timestamp");
        assert_eq!(message, "Fix the SeÑal parser\n\nBody text\n".as_bytes(), "past the signature's blank line");
    }

    #[test]
    fn matches_hash_prefix_author_or_message_ignoring_case() {
        let id = oid(0xab);
        let hex = id.to_string();
        let matches = |text: &str| CommitSearch::new(text).matches_commit(&id, RAW);
        assert!(matches(&hex[..4].to_uppercase()), "a hash prefix");
        assert!(!matches(&hex[..3]), "too short to be a hash");
        assert!(!matches(&hex[1..6]), "the middle of a hash");
        assert!(matches("ANA@EXAMPLE"), "the author's email");
        assert!(matches("muÑoz"), "non-ASCII case");
        assert!(matches(" señal "), "trimmed, non-ASCII case");
        assert!(matches("body text"));
        assert!(!matches("bob"), "not the committer");
        assert!(!matches("1700000000"), "not the date");
        assert!(!matches("fix parser"), "a phrase, not separate words");
    }

    #[test]
    fn keeps_date_order_when_children_already_come_first() {
        let commits = vec![commit(4, &[3, 2]), commit(3, &[1]), commit(2, &[1]), commit(1, &[])];
        let expected = ids(&commits);
        assert_eq!(ids(&children_first(commits)), expected);
    }

    #[test]
    fn moves_a_parent_with_a_later_date_below_its_children() {
        // 1's clock was ahead, so a date walk lists it before its children 3 and 2.
        let commits = vec![commit(4, &[3, 2]), commit(1, &[]), commit(3, &[1]), commit(2, &[1])];
        assert_eq!(ids(&children_first(commits)), [4, 3, 2, 1].map(oid));
    }
}
