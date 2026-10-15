//! Times the operations the UI relies on against any repository, to find what is slow on big ones:
//!
//! ```sh
//! cargo run --release -p furkst-core --example profile -- <repository> [file]
//! ```
//!
//! `file` is used for blame and file history; it defaults to `Makefile` or the first tracked file.

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use furkst_core::{CommitSearch, DiffTarget, HistoryQuery, Repo, Result, layout_graph};

fn time<T>(label: &str, run: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let value = run();
    println!("{label:<40} {:>9.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    value
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: profile <repository> [file]");
    let repo = time("open", || Repo::open(&path))?;

    let refs = time("refs", || repo.refs())?;
    let remote_branches: usize = refs.remotes.iter().map(|r| r.branches.len()).sum();
    println!("    {} branches, {remote_branches} remote branches, {} tags", refs.branches.len(), refs.tags.len());
    let tips = refs.all_tips();
    let head = refs.head.target().expect("a checked-out commit");
    let query = |limit| HistoryQuery { tips: tips.clone(), limit, ..Default::default() };

    let first_page = time("history: first page (2,000 commits)", || repo.history(&query(2_000)))?;
    time("graph layout: 2,000 commits", || layout_graph(&first_page));
    let deep = time("history: 20,000 commits", || repo.history(&query(20_000)))?;
    time("graph layout: 20,000 commits", || layout_graph(&deep));
    let whole = time("history: whole (to jump to the oldest match)", || repo.history(&query(usize::MAX)))?;
    time(&format!("graph layout: {} commits", whole.len()), || layout_graph(&whole));
    for text in ["fix", "JEFF KING", "6dbe1b4", "nobody-matches-this", "señal"] {
        let search = CommitSearch::new(text);
        let refs_hit = search.ref_targets(&refs);
        let hits = time(&format!("search \"{text}\" (whole history)"), || {
            repo.search_commits(&tips, &search, &refs_hit, &AtomicBool::new(false))
        })?;
        println!("    {} hits", hits.len());
    }
    let search = CommitSearch::new("Makefile").in_paths(true);
    let hits = time("search \"Makefile\" in messages and paths", || {
        repo.search_commits(&tips, &search, &Default::default(), &AtomicBool::new(false))
    })?;
    println!("    {} hits", hits.len());
    time("history: whole, reusing the search's walk", || repo.history(&query(usize::MAX)))?;

    let status = time("status", || repo.status())?;
    println!("    {} unstaged, {} staged", status.unstaged.len(), status.staged.len());

    let details = time("commit details (HEAD)", || repo.commit_details(head))?;
    if let Some(change) = details.changes.first() {
        time("diff of HEAD's first changed file", || repo.file_diff(&DiffTarget::Commit(head), change, Default::default()))?;
    }
    if let Some(merge) = first_page.iter().find(|c| c.is_merge()) {
        let merge = time("commit details (a merge commit)", || repo.commit_details(merge.id))?;
        println!("    {} changed files", merge.changes.len());
    }

    let tracked = time("tracked files (index)", || repo.tracked_files())?;
    let tree = time("file tree of HEAD", || repo.tree_files(head))?;
    println!("    {} tracked, {} in HEAD's tree", tracked.len(), tree.len());
    let file = args.next().or_else(|| tracked.iter().find(|f| *f == "Makefile").cloned()).or_else(|| tracked.first().cloned());
    if let Some(file) = file {
        let file_history = HistoryQuery { tips: vec![head], limit: 1_000, path: Some(file.clone()) };
        let commits = time(&format!("file history: {file}"), || repo.history(&file_history))?;
        println!("    {} commits", commits.len());
        let blame = time(&format!("blame: {file}"), || repo.blame(&file, head))?;
        println!("    {} lines", blame.lines.len());
    }
    time("reflog (500)", || repo.reflog(500))?;
    Ok(())
}
