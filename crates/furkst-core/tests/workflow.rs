//! End-to-end checks of the gitoxide-backed operations against a throwaway repository.

use std::path::PathBuf;

use furkst_core::{
    ChangeKind, DiffOptions, DiffTarget, FileDiff, HistoryQuery, HunkAction, LineSelection, Operation, RebaseAction, RebaseStep,
    Repo,
};

struct TempRepo {
    dir: PathBuf,
    repo: Repo,
}

impl TempRepo {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("furkst-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = Repo::init(&dir).unwrap();
        repo.cli().run(&["config", "user.name", "Test"]).unwrap();
        repo.cli().run(&["config", "user.email", "test@example.com"]).unwrap();
        repo.cli().run(&["config", "core.autocrlf", "false"]).unwrap();
        repo.reload().unwrap();
        Self { dir, repo }
    }

    fn write(&self, path: &str, content: &str) {
        std::fs::write(self.dir.join(path), content).unwrap();
    }

    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.dir.join(path)).unwrap()
    }

    fn git(&self, args: &[&str]) -> String {
        self.repo.cli().run(args).unwrap()
    }

    fn run(&self, op: Operation) -> String {
        op.run(&self.repo).unwrap()
    }

    fn stage_all(&self) {
        self.run(Operation::Stage(self.repo.status().unwrap().unstaged));
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn stage_commit_and_inspect() {
    let t = TempRepo::new("commit");
    t.write("a.txt", "one\ntwo\nthree\n");

    let status = t.repo.status().unwrap();
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].kind, ChangeKind::Untracked);

    t.run(Operation::Stage(status.unstaged.clone()));
    let status = t.repo.status().unwrap();
    assert!(status.unstaged.is_empty());
    assert_eq!(status.staged[0].kind, ChangeKind::Added);

    t.run(Operation::Commit { message: "first\n\nbody".into(), amend: false });
    assert!(t.repo.status().unwrap().is_clean());
    // git agrees with what gitoxide wrote.
    assert_eq!(t.git(&["status", "--porcelain"]), "");
    assert_eq!(t.git(&["log", "--format=%s"]), "first");

    let refs = t.repo.refs().unwrap();
    let head = refs.head.target().unwrap();
    let history = t.repo.history(&HistoryQuery { tips: refs.all_tips(), limit: 100, ..Default::default() }).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].body(), "body");

    let details = t.repo.commit_details(head).unwrap();
    assert_eq!(details.changes[0].path, "a.txt");
    let diff = t.repo.file_diff(&DiffTarget::Commit(head), &details.changes[0], DiffOptions::default()).unwrap();
    assert_eq!(diff.hunks()[0].lines.len(), 3);

    t.write("a.txt", "one\n2\nthree\n");
    t.stage_all();
    t.run(Operation::Commit { message: "amended".into(), amend: false });
    t.run(Operation::Commit { message: "amended again".into(), amend: true });
    assert_eq!(t.git(&["log", "--format=%s"]), "amended again\nfirst");
}

#[test]
fn partial_staging_and_discarding() {
    let t = TempRepo::new("hunks");
    let original: String = (1..=20).map(|i| format!("line {i}\n")).collect();
    t.write("f.txt", &original);
    t.stage_all();
    t.run(Operation::Commit { message: "base".into(), amend: false });

    let modified = original.replace("line 2\n", "line two\n").replace("line 19\n", "line nineteen\n");
    t.write("f.txt", &modified);
    let change = t.repo.status().unwrap().unstaged.remove(0);
    let options = DiffOptions::default();
    let diff = t.repo.file_diff(&DiffTarget::Unstaged, &change, options).unwrap();
    assert_eq!(diff.hunks().len(), 2);

    // Stage only the second hunk.
    let selection = LineSelection::whole_hunks(diff.hunks(), [1]);
    t.run(Operation::ApplySelection { action: HunkAction::Stage, path: "f.txt".into(), selection, options });
    let cached = t.git(&["diff", "--cached", "--unified=0"]);
    assert!(cached.contains("+line nineteen") && !cached.contains("+line two"), "{cached}");

    // Unstage it again.
    let staged = t.repo.file_diff(&DiffTarget::Staged, &change, options).unwrap();
    let selection = LineSelection::whole_hunks(staged.hunks(), [0]);
    t.run(Operation::ApplySelection { action: HunkAction::Unstage, path: "f.txt".into(), selection, options });
    assert_eq!(t.git(&["diff", "--cached"]), "");

    // Discard the first hunk from the working tree.
    let diff = t.repo.file_diff(&DiffTarget::Unstaged, &change, options).unwrap();
    let selection = LineSelection::whole_hunks(diff.hunks(), [0]);
    t.run(Operation::ApplySelection { action: HunkAction::Discard, path: "f.txt".into(), selection, options });
    assert_eq!(t.read("f.txt"), original.replace("line 19\n", "line nineteen\n"));

    t.run(Operation::Discard(t.repo.status().unwrap().unstaged));
    assert_eq!(t.read("f.txt"), original);
}

#[test]
fn branches_tags_and_unstage() {
    let t = TempRepo::new("refs");
    t.write("a.txt", "a\n");
    t.stage_all();
    t.run(Operation::Commit { message: "base".into(), amend: false });
    let head = t.repo.refs().unwrap().head.target().unwrap();

    t.run(Operation::CreateBranch { name: "feature/x".into(), at: head, checkout: false });
    t.run(Operation::RenameBranch { from: "feature/x".into(), to: "feature/y".into() });
    t.run(Operation::CreateTag { name: "v1.0".into(), at: head, message: Some("release".into()) });
    t.run(Operation::CreateTag { name: "light".into(), at: head, message: None });

    let refs = t.repo.refs().unwrap();
    let names: Vec<_> = refs.branches.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"feature/y") && !names.contains(&"feature/x"), "{names:?}");
    let annotated = refs.tags.iter().find(|tag| tag.name == "v1.0").unwrap();
    assert_eq!(annotated.message.as_deref(), Some("release"));
    assert_eq!(refs.labels_for(&head).len(), 4);

    t.run(Operation::DeleteBranch { name: "feature/y".into(), force: false });
    t.run(Operation::DeleteTag { name: "light".into() });
    assert_eq!(t.git(&["branch", "--format=%(refname:short)"]).lines().count(), 1);

    t.write("a.txt", "changed\n");
    t.write("b.txt", "new\n");
    t.stage_all();
    t.run(Operation::Unstage(t.repo.status().unwrap().staged));
    assert_eq!(t.git(&["diff", "--cached"]), "");
    assert_eq!(t.repo.status().unwrap().unstaged.len(), 2);

    t.run(Operation::StashSave { message: "wip".into(), include_untracked: true, keep_index: false });
    let refs = t.repo.refs().unwrap();
    assert_eq!(refs.stashes.len(), 1);
    assert!(refs.stashes[0].message.contains("wip"));
    assert!(t.repo.status().unwrap().is_clean());

    let blame = t.repo.blame("a.txt", head).unwrap();
    assert_eq!(blame.lines.len(), 1);
}

#[test]
fn history_lists_children_before_parents() {
    let t = TempRepo::new("order");
    // Commits created within the same second share a timestamp; ordering must still be topological.
    t.write("a.txt", "a\n");
    t.git(&["add", "."]);
    t.git(&["commit", "-m", "base"]);
    t.git(&["checkout", "-b", "side"]);
    for i in 0..3 {
        t.write("side.txt", &format!("{i}\n"));
        t.git(&["add", "."]);
        t.git(&["commit", "-m", &format!("side {i}")]);
    }
    t.git(&["checkout", "-"]);
    t.write("a.txt", "b\n");
    t.git(&["commit", "-am", "main"]);
    t.git(&["merge", "--no-ff", "side", "-m", "merge"]);

    let refs = t.repo.refs().unwrap();
    let history = t.repo.history(&HistoryQuery { tips: refs.all_tips(), limit: 100, ..Default::default() }).unwrap();
    assert_eq!(history.len(), 6);
    let position = |id| history.iter().position(|c| c.id == id).unwrap();
    for (index, commit) in history.iter().enumerate() {
        assert!(commit.parents.iter().all(|p| position(*p) > index), "{} listed after a parent", commit.summary);
    }
    let rows = furkst_core::layout_graph(&history);
    assert_eq!(rows.iter().map(|r| r.width).max(), Some(2));
}

#[test]
fn file_history_matches_git_log() {
    let t = TempRepo::new("file-history");
    let commit = |file: &str, content: &str, message: &str| {
        t.write(file, content);
        t.git(&["add", "."]);
        t.git(&["commit", "-m", message]);
    };
    commit("f.txt", "1\n2\n3\n", "add f");
    // A side branch that never touches f: neither it nor its merge belong to f's history.
    t.git(&["checkout", "-b", "other"]);
    commit("other.txt", "x\n", "other only");
    t.git(&["checkout", "-"]);
    commit("f.txt", "1\n2\n3\n4\n", "main changes f");
    t.git(&["merge", "--no-ff", "other", "-m", "merge other"]);
    // Both sides change f: the merge combines them into a version neither parent had.
    t.git(&["checkout", "-b", "side"]);
    commit("f.txt", "one\n2\n3\n4\n", "side changes f");
    t.git(&["checkout", "-"]);
    commit("f.txt", "1\n2\n3\nfour\n", "main changes f again");
    t.git(&["merge", "--no-ff", "side", "-m", "merge side"]);
    // A merge that just takes the side's version of f.
    t.git(&["checkout", "-b", "late"]);
    commit("f.txt", "one\n2\nthree\nfour\n", "late changes f");
    t.git(&["checkout", "-"]);
    t.git(&["merge", "--no-ff", "late", "-m", "merge late"]);

    let head = t.repo.refs().unwrap().head.target().unwrap();
    let query = HistoryQuery { tips: vec![head], limit: 100, path: Some("f.txt".into()) };
    let mut ours: Vec<String> = t.repo.history(&query).unwrap().iter().map(|c| c.id.to_string()).collect();
    let mut git: Vec<String> = t.git(&["log", "--format=%H", "--", "f.txt"]).lines().map(str::to_owned).collect();
    ours.sort();
    git.sort();
    assert_eq!(ours, git);
    assert_eq!(ours.len(), 6, "add, two main changes, side, merge side, late");

    // Each version shows as added (the first), then modified.
    let versions = t.repo.history(&query).unwrap();
    let kind = |commit: &furkst_core::CommitInfo| t.repo.file_change_at(commit.id, "f.txt").unwrap().kind;
    assert_eq!(kind(versions.last().unwrap()), ChangeKind::Added);
    assert!(versions[..versions.len() - 1].iter().all(|c| kind(c) == ChangeKind::Modified));
}

#[test]
fn search_commits_finds_messages_and_refs_in_walk_order() {
    let t = TempRepo::new("search");
    for (file, message) in [("a", "add parser"), ("b", "docs"), ("c", "Fix PARSER bug"), ("d", "docs again")] {
        t.write(file, file);
        t.git(&["add", "."]);
        t.git(&["commit", "-m", message]);
    }
    t.git(&["tag", "parser-v1", "HEAD~2"]);
    let refs = t.repo.refs().unwrap();
    let tips = refs.all_tips();
    let history = t.repo.history(&HistoryQuery { tips: tips.clone(), limit: 100, ..Default::default() }).unwrap();

    let search = furkst_core::CommitSearch::new("parser");
    let ref_targets = search.ref_targets(&refs);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let hits = t.repo.search_commits(&tips, &search, &ref_targets, &cancel).unwrap();
    let summary = |id| history.iter().find(|c| c.id == id).unwrap().summary.as_str();
    let mut found: Vec<&str> = hits.iter().map(|hit| summary(hit.id)).collect();
    found.sort_unstable();
    assert_eq!(found, ["Fix PARSER bug", "add parser", "docs"], "\"docs\" by its tag");
    // Positions are in the walk, which history only reorders to put children first when commit times tie.
    assert!(hits.windows(2).all(|pair| pair[0].position < pair[1].position));
    assert!(hits.iter().all(|hit| hit.position < history.len()));

    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(t.repo.search_commits(&tips, &search, &ref_targets, &cancel).unwrap().is_empty(), "a cancelled search stops");
}

#[test]
fn rebase_and_merge_replays_commits_without_moving_the_branch() {
    use furkst_core::MergeStyle;
    let t = TempRepo::new("rebase-and-merge");
    let commit = |file: &str, message: &str| {
        t.write(file, message);
        t.git(&["add", "."]);
        t.git(&["commit", "-m", message]);
    };
    commit("base.txt", "base");
    t.git(&["checkout", "-b", "feature"]);
    commit("a.txt", "feature one");
    commit("b.txt", "feature two");
    let feature_tip = t.git(&["rev-parse", "feature"]);
    t.git(&["checkout", "-"]);
    commit("main.txt", "main moves on");

    t.run(Operation::Merge { target: "feature".into(), style: MergeStyle::Rebase });
    let subjects = t.git(&["log", "--format=%s"]);
    assert_eq!(subjects.lines().collect::<Vec<_>>(), ["feature two", "feature one", "main moves on", "base"], "replayed on top");
    assert!(t.git(&["rev-list", "--merges", "HEAD"]).trim().is_empty(), "no merge commit");
    assert_eq!(t.git(&["rev-parse", "feature"]), feature_tip, "the branch is left as it was");

    // Behind the branch: its commits are taken as they are.
    t.git(&["checkout", "-b", "behind", "HEAD~3"]);
    t.run(Operation::Merge { target: "feature".into(), style: MergeStyle::Rebase });
    assert_eq!(t.git(&["rev-parse", "HEAD"]), feature_tip, "fast-forwarded");
}

#[test]
fn deletes_a_branch_locally_on_the_remote_or_both() {
    let t = TempRepo::new("delete-branches");
    let remote = t.dir.join("remote.git");
    t.git(&["init", "-q", "--bare", remote.to_str().unwrap()]);
    t.git(&["remote", "add", "origin", remote.to_str().unwrap()]);
    t.write("a.txt", "a");
    t.git(&["add", "a.txt"]);
    t.git(&["commit", "-m", "base"]);
    t.git(&["branch", "topic"]);
    t.git(&["push", "-q", "origin", "topic"]);
    let remote_has = || !t.git(&["ls-remote", "--heads", "origin", "topic"]).trim().is_empty();
    let local_has = || !t.git(&["branch", "--list", "topic"]).trim().is_empty();

    t.run(Operation::DeleteBranches { local: Some("topic".into()), remote: None });
    assert!(!local_has() && remote_has(), "only the local branch goes");
    t.run(Operation::DeleteBranches { local: None, remote: Some(("origin".into(), "topic".into())) });
    assert!(!remote_has(), "then the remote one");
}

#[test]
fn search_commits_by_changed_paths() {
    let t = TempRepo::new("search-paths");
    for (file, message) in [("src/main.rs", "start"), ("docs/guide.md", "write"), ("src/lexer.rs", "more")] {
        std::fs::create_dir_all(t.dir.join(file).parent().unwrap()).unwrap();
        t.write(file, file);
        t.git(&["add", "."]);
        t.git(&["commit", "-m", message]);
    }
    let tips = t.repo.refs().unwrap().all_tips();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let search = |text: &str, paths: bool| {
        let search = furkst_core::CommitSearch::new(text).in_paths(paths);
        t.repo.search_commits(&tips, &search, &Default::default(), &cancel).unwrap().len()
    };
    assert_eq!(search("SRC/", false), 0, "no message mentions it");
    assert_eq!(search("SRC/", true), 2, "the two commits that changed files under src/");
    assert_eq!(search("guide", true), 1);
}

#[test]
fn stash_details_include_its_untracked_files() {
    let t = TempRepo::new("stash-details");
    t.write("tracked.txt", "one
");
    t.git(&["add", "."]);
    t.git(&["commit", "-m", "base"]);
    t.write("tracked.txt", "two
");
    t.write("new.txt", "brand new
");
    t.run(Operation::StashSave { message: "wip".into(), include_untracked: true, keep_index: false });

    let stash = t.repo.refs().unwrap().stashes[0].id;
    let details = t.repo.commit_details(stash).unwrap();
    let paths = |files: &[furkst_core::FileChange]| files.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
    assert_eq!(paths(&details.changes), ["tracked.txt"]);
    let (untracked_commit, untracked) = details.stash_untracked.expect("stashed with its untracked files");
    assert_eq!(paths(&untracked), ["new.txt"]);
    assert_eq!(untracked[0].kind, ChangeKind::Untracked);
    let diff = t.repo.file_diff(&DiffTarget::Commit(untracked_commit), &untracked[0], DiffOptions::default()).unwrap();
    let FileDiff::Text { hunks, .. } = diff else { panic!("a text diff") };
    assert!(hunks[0].lines.iter().any(|l| l.text.contains("brand new")));

    // A stash without untracked files, and an ordinary commit, have none.
    t.write("tracked.txt", "three
");
    t.run(Operation::StashSave { message: "plain".into(), include_untracked: false, keep_index: false });
    assert!(t.repo.commit_details(t.repo.refs().unwrap().stashes[0].id).unwrap().stash_untracked.is_none());
}

/// Repo with commits a, b, c, d (each adding a file); returns their ids oldest first.
fn linear_history(t: &TempRepo) -> Vec<furkst_core::Oid> {
    for name in ["a", "b", "c", "d"] {
        t.write(&format!("{name}.txt"), name);
        t.stage_all();
        t.run(Operation::Commit { message: name.into(), amend: false });
    }
    t.repo.rebase_plan(None).unwrap().iter().map(|c| c.id).collect()
}

fn steps(ids: &[furkst_core::Oid], actions: &[RebaseAction]) -> Vec<RebaseStep> {
    ids.iter().zip(actions).map(|(id, a)| RebaseStep { action: *a, ..RebaseStep::new(*id) }).collect()
}

fn lf(s: String) -> String {
    s.replace("\r\n", "\n")
}

#[test]
fn interactive_rebase_keeps_uncommitted_changes() {
    use RebaseAction::*;
    let t = TempRepo::new("rebase-autostash");
    let ids = linear_history(&t);
    t.write("a.txt", "edited, not committed");
    let s = steps(&[ids[1], ids[2], ids[3]], &[Pick, Drop, Pick]);
    t.run(Operation::InteractiveRebase { base: Some(ids[0]), steps: s });
    assert_eq!(lf(t.git(&["log", "--format=%s"])), "d
b
a");
    assert_eq!(t.read("a.txt"), "edited, not committed", "set aside and put back");
    assert!(t.repo.refs().unwrap().stashes.is_empty(), "no stash left behind");
}

#[test]
fn interactive_rebase_reorder_and_drop() {
    use RebaseAction::*;
    let t = TempRepo::new("rebase-reorder");
    let ids = linear_history(&t);
    let plan = t.repo.rebase_plan(Some(ids[0])).unwrap();
    assert_eq!(plan.iter().map(|c| c.summary.as_str()).collect::<Vec<_>>(), ["b", "c", "d"]);

    // d, b, (c dropped)
    let s = steps(&[ids[3], ids[1], ids[2]], &[Pick, Pick, Drop]);
    t.run(Operation::InteractiveRebase { base: Some(ids[0]), steps: s });
    assert_eq!(lf(t.git(&["log", "--format=%s"])), "b\nd\na");
    assert!(!t.dir.join("c.txt").exists());
    assert!(!t.dir.join(".git/furkst-rebase").exists());
    assert!(t.repo.in_progress().is_none());
    assert!(Operation::InteractiveRebase { base: None, steps: Vec::new() }.run(&t.repo).is_err());
}

#[test]
fn interactive_rebase_squash_and_fixup() {
    use RebaseAction::*;
    let t = TempRepo::new("rebase-squash");
    let ids = linear_history(&t);
    t.run(Operation::InteractiveRebase { base: Some(ids[0]), steps: steps(&ids[1..], &[Pick, Fixup, Squash]) });
    assert_eq!(t.git(&["rev-list", "--count", "HEAD"]), "2");
    // Fixup drops its message, squash keeps it.
    let message = lf(t.git(&["log", "-1", "--format=%B"]));
    assert!(message.starts_with('b') && message.contains('d') && !message.contains('c'), "{message}");
    assert_eq!(t.read("d.txt"), "d");

    let bad = steps(&ids, &[Squash, Pick, Pick, Pick]);
    assert!(Operation::InteractiveRebase { base: None, steps: bad }.run(&t.repo).is_err());
}

#[test]
fn interactive_rebase_reword_from_root() {
    use RebaseAction::*;
    let t = TempRepo::new("rebase-reword");
    let ids = linear_history(&t);
    let mut s = steps(&ids, &[Reword, Pick, Reword, Pick]);
    s[0].message = Some("first\n\nwith body".into());
    s[2].message = Some("third".into());
    t.run(Operation::InteractiveRebase { base: None, steps: s });
    assert_eq!(lf(t.git(&["log", "--format=%s"])), "d\nthird\nb\nfirst");
    assert_eq!(t.git(&["log", "-1", "--format=%b", "HEAD~3"]), "with body");
    assert!(!t.dir.join(".git/furkst-rebase").exists());
}

#[test]
fn rebase_plan_refuses_merges() {
    let t = TempRepo::new("rebase-merge");
    let ids = linear_history(&t);
    t.git(&["checkout", "-q", "-b", "side", &ids[0].to_string()]);
    t.write("s.txt", "s");
    t.stage_all();
    t.run(Operation::Commit { message: "side".into(), amend: false });
    t.git(&["checkout", "-q", "-"]);
    t.git(&["merge", "--no-ff", "--no-edit", "-q", "side"]);
    let err = t.repo.rebase_plan(Some(ids[0])).unwrap_err().to_string();
    assert!(err.contains("merge commit"), "{err}");
}

#[test]
fn tracked_files_lists_the_index() {
    let t = TempRepo::new("tracked");
    std::fs::create_dir_all(t.dir.join("src")).unwrap();
    t.write("b.txt", "b");
    t.write("src/a.rs", "a");
    t.write("untracked.txt", "u");
    t.git(&["add", "b.txt", "src/a.rs"]);
    assert_eq!(t.repo.tracked_files().unwrap(), ["b.txt", "src/a.rs"]);
}

#[test]
fn resolve_merge_conflict_with_mixed_choices() {
    use furkst_core::{Resolution, Segment, assemble, conflict_count};

    let t = TempRepo::new("resolve-merged");
    let base: String = (1..=12).map(|i| format!("line {i}\n")).collect();
    t.write("a.txt", &base);
    t.git(&["add", "."]);
    t.git(&["commit", "-m", "base"]);
    t.git(&["checkout", "-b", "topic"]);
    t.write("a.txt", &base.replace("line 2\n", "topic 2\n").replace("line 11\n", "topic 11\n"));
    t.git(&["commit", "-am", "topic"]);
    t.git(&["checkout", "-"]);
    t.write("a.txt", &base.replace("line 2\n", "main 2\n").replace("line 11\n", "main 11\n"));
    t.git(&["commit", "-am", "main"]);
    // The merge stops with conflicts, which `git` reports as a failure.
    assert!(t.repo.cli().run(&["merge", "topic"]).is_err());

    let status = t.repo.status().unwrap();
    let change = status.unstaged.iter().find(|c| c.kind == ChangeKind::Conflicted).cloned().unwrap();
    let segments = t.repo.conflict_segments("a.txt").unwrap();
    assert_eq!(conflict_count(&segments), 2);
    assert!(matches!(segments[0], Segment::Common(_)));

    let content = assemble(&segments, &[Some(Resolution::Theirs), Some(Resolution::OursThenTheirs)]);
    t.run(Operation::ResolveMerged { change, content });

    let expected = base.replace("line 2\n", "topic 2\n").replace("line 11\n", "main 11\ntopic 11\n");
    assert_eq!(t.read("a.txt"), expected);
    let status = t.repo.status().unwrap();
    assert!(!status.has_conflicts());
    assert!(status.unstaged.is_empty(), "{:?}", status.unstaged);
    assert!(status.staged.iter().any(|c| c.path == "a.txt"));
}

#[test]
fn worktrees_add_list_remove() {
    let t = TempRepo::new("worktrees");
    t.write("a.txt", "a\n");
    t.git(&["add", "."]);
    t.git(&["commit", "-m", "base"]);
    assert!(t.repo.refs().unwrap().worktrees.is_empty());

    let linked = t.dir.with_file_name(format!("furkst-test-worktrees-linked-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&linked);
    t.run(Operation::AddWorktree { path: linked.clone(), branch: "feature".into(), create: true, start: Some("HEAD".into()) });
    assert!(linked.join("a.txt").exists());

    let refs = t.repo.refs().unwrap();
    assert_eq!(refs.worktrees.len(), 2);
    assert!(refs.worktrees[0].is_main && refs.worktrees[0].is_current);
    let entry = &refs.worktrees[1];
    assert_eq!(entry.branch.as_deref(), Some("feature"));
    assert!(!entry.is_main && !entry.is_current && entry.locked.is_none() && !entry.prunable);

    // Seen from the linked worktree, that one is current.
    let from_linked = furkst_core::Repo::open(&linked).unwrap().refs().unwrap();
    assert_eq!(from_linked.worktrees.iter().filter(|w| w.is_current).count(), 1);
    assert!(from_linked.worktrees[1].is_current);

    t.run(Operation::RemoveWorktree { path: entry.path.clone(), force: false });
    assert!(!linked.exists());
    assert!(t.repo.refs().unwrap().worktrees.is_empty());
    t.run(Operation::PruneWorktrees);
}

#[test]
fn committed_image_yields_image_diff() {
    let t = TempRepo::new("image");
    let bytes = vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3];
    std::fs::write(t.dir.join("logo.PNG"), &bytes).unwrap();
    t.stage_all();
    t.run(Operation::Commit { message: "add logo".into(), amend: false });

    let head = t.repo.refs().unwrap().head.target().unwrap();
    let details = t.repo.commit_details(head).unwrap();
    let diff = t.repo.file_diff(&DiffTarget::Commit(head), &details.changes[0], DiffOptions::default()).unwrap();
    assert_eq!(diff, FileDiff::Image { old: None, new: Some(bytes) });
}

#[test]
fn tree_files_and_file_at() {
    let t = TempRepo::new("tree");
    std::fs::create_dir_all(t.dir.join("src/deep")).unwrap();
    t.write("a.txt", "one\n");
    t.write("src/main.rs", "fn main() {}\n");
    t.write("src/deep/x.txt", "x\n");
    t.stage_all();
    t.run(Operation::Commit { message: "first".into(), amend: false });
    let first = t.repo.refs().unwrap().head.target().unwrap();
    t.write("a.txt", "two\n");
    t.stage_all();
    t.run(Operation::Commit { message: "second".into(), amend: false });
    let second = t.repo.refs().unwrap().head.target().unwrap();

    let mut paths: Vec<_> = t.repo.tree_files(second).unwrap().into_iter().map(|f| f.path).collect();
    paths.sort();
    assert_eq!(paths, ["a.txt", "src/deep/x.txt", "src/main.rs"]);
    assert_eq!(t.repo.file_at(first, "a.txt").unwrap().unwrap(), b"one\n");
    assert_eq!(t.repo.file_at(second, "a.txt").unwrap().unwrap(), b"two\n");
    assert!(t.repo.file_at(second, "missing").unwrap().is_none());
}

#[test]
fn watch_reports_outside_changes_but_not_ignored_files() {
    use std::sync::mpsc;
    use std::time::Duration;

    let t = TempRepo::new("watch");
    t.write(".gitignore", "target/\n");
    t.write("a.txt", "a\n");
    t.stage_all();
    t.run(Operation::Commit { message: "base".into(), amend: false });
    std::fs::create_dir_all(t.dir.join("target/debug")).unwrap();

    let (tx, rx) = mpsc::channel();
    let _watch = t.repo.watch(move |change| {
        let _ = tx.send(change);
    })
    .unwrap();
    let settle = |rx: &mpsc::Receiver<furkst_core::Change>| {
        std::thread::sleep(Duration::from_millis(500));
        rx.try_iter().max()
    };
    settle(&rx);

    t.write("target/debug/build.log", "noise\n");
    assert_eq!(settle(&rx), None, "ignored paths must not trigger a refresh");

    t.write("a.txt", "changed\n");
    assert_eq!(settle(&rx), Some(furkst_core::Change::WorkingCopy));

    t.git(&["branch", "feature"]);
    assert_eq!(settle(&rx), Some(furkst_core::Change::Refs));
}

#[test]
fn commit_template_is_read_without_comments() {
    let t = TempRepo::new("template");
    assert_eq!(t.repo.commit_template(), None);
    t.write(".gitmessage", "# Explain why, not what\nfeat: \n\nRefs: #\n");
    t.git(&["config", "commit.template", ".gitmessage"]);
    t.repo.reload().unwrap();
    assert_eq!(t.repo.commit_template().as_deref(), Some("feat: \n\nRefs: #"));
}

#[test]
fn reflog_records_head_movements_newest_first() {
    let t = TempRepo::new("reflog");
    for name in ["one", "two"] {
        t.write(&format!("{name}.txt"), name);
        t.stage_all();
        t.git(&["commit", "-q", "-m", name]);
    }
    t.git(&["reset", "-q", "--hard", "HEAD~1"]);

    let reflog = t.repo.reflog(100).unwrap();
    let actions: Vec<_> = reflog.iter().map(|e| e.action.split(':').next().unwrap()).collect();
    assert_eq!(actions, ["reset", "commit", "commit (initial)"]);
    // The commit undone by the reset is still reachable through the reflog.
    assert_eq!(reflog[1].commit.summary, "two");
    assert_eq!(reflog[0].previous, reflog[1].commit.id);
    assert_eq!(t.repo.reflog(1).unwrap().len(), 1);
}

#[test]
fn lfs_files_diff_by_content_or_pointer() {
    let has_lfs = std::process::Command::new("git").args(["lfs", "version"]).output().is_ok_and(|o| o.status.success());
    if !has_lfs {
        eprintln!("git-lfs not installed; skipping");
        return;
    }
    let t = TempRepo::new("lfs");
    t.git(&["lfs", "install", "--local"]);
    t.run(Operation::LfsTrack { pattern: "*.png".into() });
    assert!(t.repo.uses_lfs());
    std::fs::write(t.dir.join("logo.png"), [0x89, b'P', b'N', b'G', 1, 2, 3]).unwrap();
    t.git(&["add", "."]);
    t.git(&["commit", "-q", "-m", "logo"]);

    // The blob in git is a pointer; the diff shows the real content from the local LFS cache.
    let head = t.repo.refs().unwrap().head.target().unwrap();
    let details = t.repo.commit_details(head).unwrap();
    let logo = details.changes.iter().find(|c| c.path == "logo.png").unwrap();
    let diff = t.repo.file_diff(&DiffTarget::Commit(head), logo, DiffOptions::default()).unwrap();
    assert_eq!(diff, FileDiff::Image { old: None, new: Some(vec![0x89, b'P', b'N', b'G', 1, 2, 3]) });

    // A modified working-tree file goes through git-lfs' clean filter like `git add` would.
    std::fs::write(t.dir.join("logo.png"), [0x89, b'P', b'N', b'G', 9]).unwrap();
    let change = t.repo.status().unwrap().unstaged.remove(0);
    let diff = t.repo.file_diff(&DiffTarget::Unstaged, &change, DiffOptions::default()).unwrap();
    assert!(matches!(diff, FileDiff::Image { old: Some(_), new: Some(_) }), "{diff:?}");

    // Without the cached object, the pointer still tells what the file is.
    std::fs::remove_dir_all(t.dir.join(".git/lfs/objects")).unwrap();
    let diff = t.repo.file_diff(&DiffTarget::Commit(head), logo, DiffOptions::default()).unwrap();
    assert!(matches!(diff, FileDiff::Lfs { old: None, new: Some(ref p) } if p.size == 7), "{diff:?}");
}

#[test]
fn git_flow_feature_and_release() {
    use furkst_core::{FlowKind, MergeStyle};

    let t = TempRepo::new("flow");
    t.write("a.txt", "a\n");
    t.stage_all();
    t.run(Operation::Commit { message: "base".into(), amend: false });
    let main = t.repo.refs().unwrap().head.branch_name().unwrap().to_owned();
    assert!(t.repo.flow_config().is_none());

    t.run(Operation::FlowInit { main: main.clone(), develop: "develop".into() });
    let flow = t.repo.flow_config().unwrap();
    assert_eq!((flow.main.as_str(), flow.develop.as_str()), (main.as_str(), "develop"));

    t.run(Operation::FlowStart { kind: FlowKind::Feature, name: "login".into() });
    assert_eq!(t.repo.refs().unwrap().head.branch_name(), Some("feature/login"));
    t.write("login.txt", "login\n");
    t.stage_all();
    t.run(Operation::Commit { message: "Add login".into(), amend: false });
    t.run(Operation::FlowFinish { kind: FlowKind::Feature, name: "login".into(), style: MergeStyle::MergeCommit });
    assert_eq!(t.git(&["log", "-1", "--format=%s", "develop"]), "Merge branch 'feature/login' into develop");
    assert!(t.repo.resolve("feature/login").is_err());

    t.run(Operation::FlowStart { kind: FlowKind::Release, name: "1.0".into() });
    t.write("VERSION", "1.0
");
    t.stage_all();
    t.run(Operation::Commit { message: "Bump version".into(), amend: false });
    t.run(Operation::FlowFinish { kind: FlowKind::Release, name: "1.0".into(), style: MergeStyle::Squash });
    assert_eq!(t.git(&["describe", "--tags", &main]), "1.0");
    assert_eq!(t.git(&["log", "-1", "--format=%s", "develop"]), "Merge branch 'release/1.0' into develop");
    assert!(t.repo.resolve("release/1.0").is_err());
    assert!(t.git(&["show", &format!("{main}:login.txt")]).contains("login"));
}

#[test]
fn squash_and_merge_commits_one_commit_listing_the_branch() {
    use furkst_core::{FlowKind, MergeStyle};

    let t = TempRepo::new("squash");
    t.write("a.txt", "a\n");
    t.stage_all();
    t.run(Operation::Commit { message: "base".into(), amend: false });
    let main = t.repo.refs().unwrap().head.branch_name().unwrap().to_owned();

    // Plain merge, GitHub's "Squash and merge": one commit, no merge commit, commits listed.
    t.git(&["checkout", "-q", "-b", "topic"]);
    for name in ["one", "two"] {
        t.write(&format!("{name}.txt"), name);
        t.stage_all();
        t.run(Operation::Commit { message: format!("Add {name}"), amend: false });
    }
    t.git(&["checkout", "-q", &main]);
    t.run(Operation::Merge { target: "topic".into(), style: MergeStyle::Squash });
    assert_eq!(t.git(&["log", "--format=%s", "-2"]), "Squashed topic\nbase");
    assert_eq!(t.git(&["log", "-1", "--format=%b"]), "* Add one\n* Add two");

    // A git-flow feature finished with a squash lands on develop as one commit and its branch is removed.
    t.run(Operation::FlowInit { main: main.clone(), develop: "develop".into() });
    t.run(Operation::FlowStart { kind: FlowKind::Feature, name: "search".into() });
    t.write("search.txt", "search");
    t.stage_all();
    t.run(Operation::Commit { message: "Add search".into(), amend: false });
    t.run(Operation::FlowFinish { kind: FlowKind::Feature, name: "search".into(), style: MergeStyle::Squash });
    assert!(!t.git(&["log", "-1", "--format=%p", "develop"]).contains(' '), "one parent: not a merge commit");
    assert!(t.git(&["log", "-1", "--format=%s", "develop"]).starts_with("Squashed feature/search"));
    assert!(t.repo.resolve("feature/search").is_err());
}
