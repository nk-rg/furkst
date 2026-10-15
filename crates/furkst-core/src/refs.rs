use std::collections::HashMap;
use std::path::PathBuf;

use gix::bstr::ByteSlice;
use gix::remote::Direction;

use crate::error::Result;
use crate::repo::commit_info;
use crate::{
    Branch, HeadState, Oid, RefKind, RefLabel, RefsSnapshot, ReflogEntry, Remote, RemoteBranch, Repo, Signature, Stash, Submodule, Tag,
    Worktree,
};

impl Repo {
    /// Branches, remotes, tags, stashes and submodules, as shown in the sidebar.
    pub fn refs(&self) -> Result<RefsSnapshot> {
        let repo = self.gix();
        let head = self.head_state(&repo)?;
        let branches = self.branches(&repo, &head)?;
        let remotes = self.remotes(&repo)?;
        let tags = self.tags(&repo)?;
        let stashes = self.stashes(&repo)?;
        let submodules = self.submodules(&repo)?;
        let worktrees = self.worktrees(&repo)?;

        let mut labels: HashMap<Oid, Vec<RefLabel>> = HashMap::new();
        let mut label = |id: Oid, name: String, kind: RefKind, is_head: bool| {
            labels.entry(id).or_default().push(RefLabel { name, kind, is_head });
        };
        if let HeadState::Detached(id) = head {
            label(id, "HEAD".into(), RefKind::Head, true);
        }
        branches.iter().for_each(|b| label(b.target, b.name.clone(), RefKind::LocalBranch, b.is_head));
        remotes
            .iter()
            .flat_map(|r| &r.branches)
            .for_each(|b| label(b.target, b.full_name(), RefKind::RemoteBranch, false));
        tags.iter().for_each(|t| label(t.target, t.name.clone(), RefKind::Tag, false));

        Ok(RefsSnapshot { head, branches, remotes, tags, stashes, submodules, worktrees, labels, uses_lfs: self.uses_lfs(), lfs_patterns: self.lfs_patterns(), flow: self.flow_config() })
    }

    fn head_state(&self, repo: &gix::Repository) -> Result<HeadState> {
        let head = repo.head()?;
        let target = head.id().map(|id| id.detach());
        Ok(match head.referent_name() {
            Some(name) => HeadState::Branch { name: name.shorten().to_string(), target },
            None => HeadState::Detached(target.unwrap_or_else(|| Oid::null(repo.object_hash()))),
        })
    }

    fn branches(&self, repo: &gix::Repository, head: &HeadState) -> Result<Vec<Branch>> {
        let mut branches = Vec::new();
        for reference in repo.references()?.local_branches()? {
            let mut reference = reference?;
            let name = reference.name().shorten().to_string();
            let target = reference.peel_to_id()?.detach();
            let upstream_ref = repo
                .branch_remote_tracking_ref_name(reference.name(), Direction::Fetch)
                .and_then(Result::ok);
            let upstream_id = upstream_ref
                .as_ref()
                .and_then(|r| repo.try_find_reference(r.as_ref()).ok().flatten())
                .and_then(|mut r| r.peel_to_id().ok())
                .map(|id| id.detach());
            let (ahead, behind) = match upstream_id {
                Some(upstream) => (count_unique(repo, target, upstream)?, count_unique(repo, upstream, target)?),
                None => (0, 0),
            };
            branches.push(Branch {
                is_head: head.branch_name() == Some(name.as_str()),
                upstream: upstream_ref.map(|r| r.as_ref().shorten().to_string()),
                name,
                target,
                ahead,
                behind,
            });
        }
        branches.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(branches)
    }

    fn remotes(&self, repo: &gix::Repository) -> Result<Vec<Remote>> {
        let mut remotes: Vec<Remote> = repo
            .remote_names()
            .into_iter()
            .map(|name| {
                let url = repo
                    .find_remote(name.as_bstr())
                    .ok()
                    .and_then(|r| r.url(Direction::Fetch).map(|u| u.to_bstring().to_string()));
                Remote { name: name.to_string(), url, branches: Vec::new() }
            })
            .collect();
        for reference in repo.references()?.remote_branches()? {
            let mut reference = reference?;
            let short = reference.name().shorten().to_string();
            let Some(remote) = remotes.iter_mut().find(|r| short.starts_with(&format!("{}/", r.name))) else {
                continue;
            };
            let name = short[remote.name.len() + 1..].to_owned();
            if name == "HEAD" {
                continue;
            }
            let target = reference.peel_to_id()?.detach();
            remote.branches.push(RemoteBranch { remote: remote.name.clone(), name, target });
        }
        remotes.iter_mut().for_each(|r| r.branches.sort_by(|a, b| a.name.cmp(&b.name)));
        Ok(remotes)
    }

    fn tags(&self, repo: &gix::Repository) -> Result<Vec<Tag>> {
        let mut tags = Vec::new();
        for reference in repo.references()?.tags()? {
            let mut reference = reference?;
            let name = reference.name().shorten().to_string();
            let message = reference
                .try_id()
                .and_then(|id| id.object().ok())
                .and_then(|object| object.try_into_tag().ok())
                .and_then(|tag| tag.decode().ok().map(|t| t.message.to_str_lossy().trim().to_owned()));
            // Tags may point at trees or blobs; those don't appear in the history.
            let Ok(commit) = reference.peel_to_commit() else { continue };
            tags.push(Tag { name, target: commit.id, message });
        }
        tags.sort_by(|a, b| natural_cmp(&b.name, &a.name));
        Ok(tags)
    }

    fn stashes(&self, repo: &gix::Repository) -> Result<Vec<Stash>> {
        Ok(reflog_lines(repo, "refs/stash")?
            .into_iter()
            .enumerate()
            .map(|(index, line)| Stash { index, id: line.new_oid, message: line.message.to_str_lossy().into_owned() })
            .collect())
    }

    /// Where `HEAD` has been, newest first, up to `limit` entries. Commits pruned since are skipped.
    pub fn reflog(&self, limit: usize) -> Result<Vec<ReflogEntry>> {
        let repo = self.gix();
        let mut time = gix::date::parse::TimeBuf::default();
        let mut entries = Vec::new();
        for line in reflog_lines(&repo, "HEAD")?.into_iter().take(limit) {
            let Ok(commit) = repo.find_commit(line.new_oid) else { continue };
            entries.push(ReflogEntry {
                commit: commit_info(&commit)?,
                previous: line.previous_oid,
                action: line.message.to_str_lossy().into_owned(),
                when: Signature::from_gix(line.signature.to_ref(&mut time)),
            });
        }
        Ok(entries)
    }

    /// The main worktree followed by the linked ones; empty when there are no linked worktrees.
    fn worktrees(&self, repo: &gix::Repository) -> Result<Vec<Worktree>> {
        let linked = repo.worktrees()?;
        if linked.is_empty() {
            return Ok(Vec::new());
        }
        let canonical = |path: PathBuf| gix::path::realpath(&path).unwrap_or(path);
        let describe = |repo: &gix::Repository| -> (Option<String>, Option<Oid>) {
            repo.head().map_or((None, None), |head| {
                (head.referent_name().map(|name| name.shorten().to_string()), head.id().map(|id| id.detach()))
            })
        };
        let mut all = Vec::new();
        let main = repo.main_repo()?;
        if let Some(path) = main.workdir() {
            let (branch, head) = describe(&main);
            all.push(Worktree {
                path: canonical(path.to_path_buf()),
                branch,
                head,
                is_main: true,
                is_current: false,
                locked: None,
                prunable: false,
            });
        }
        for proxy in linked {
            let Ok(path) = proxy.base() else { continue };
            let (locked, prunable) = (proxy.lock_reason().map(|r| r.to_string()), proxy.is_prunable());
            let (branch, head) = proxy.into_repo_with_possibly_inaccessible_worktree().map_or((None, None), |r| describe(&r));
            all.push(Worktree { path: canonical(path), branch, head, is_main: false, is_current: false, locked, prunable });
        }
        // Compared as the file system resolves them: one path can be spelled several ways (Windows' 8.3 short names).
        let here = std::fs::canonicalize(self.workdir()).ok();
        for worktree in &mut all {
            worktree.is_current = worktree.path == self.workdir()
                || here.is_some() && std::fs::canonicalize(&worktree.path).ok() == here;
        }
        Ok(all)
    }

    fn submodules(&self, repo: &gix::Repository) -> Result<Vec<Submodule>> {
        let Some(modules) = repo.submodules()? else { return Ok(Vec::new()) };
        Ok(modules
            .filter_map(|sm| {
                Some(Submodule {
                    name: sm.name().to_string(),
                    path: sm.path().ok()?.to_string(),
                    url: sm.url().ok().map(|u| u.to_bstring().to_string()),
                })
            })
            .collect())
    }
}

/// Number of commits reachable from `from` but not from `exclude`.
fn count_unique(repo: &gix::Repository, from: Oid, exclude: Oid) -> Result<usize> {
    if from == exclude {
        return Ok(0);
    }
    let mut count = 0;
    for info in repo.rev_walk([from]).with_hidden([exclude]).all()? {
        info?;
        count += 1;
    }
    Ok(count)
}

/// Orders `v1.10` after `v1.9`, ignoring case except as a tie-breaker.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn chunks(s: &str) -> Vec<(bool, &str)> {
        let mut out = Vec::new();
        let mut start = 0;
        let bytes = s.as_bytes();
        for i in 1..=bytes.len() {
            if i == bytes.len() || bytes[i].is_ascii_digit() != bytes[start].is_ascii_digit() {
                out.push((bytes[start].is_ascii_digit(), &s[start..i]));
                start = i;
            }
        }
        out
    }
    let (ca, cb) = (chunks(a), chunks(b));
    for ((da, a), (db, b)) in ca.iter().zip(&cb) {
        let ord = if *da && *db {
            a.len().cmp(&b.len()).then_with(|| a.cmp(b))
        } else {
            a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b))
        };
        if ord.is_ne() {
            return ord;
        }
    }
    ca.len().cmp(&cb.len())
}

/// The reflog of reference `name`, newest first; empty if it has none.
fn reflog_lines(repo: &gix::Repository, name: &str) -> Result<Vec<gix::refs::log::Line>> {
    let Some(reference) = repo.try_find_reference(name)? else { return Ok(Vec::new()) };
    let mut log = reference.log_iter();
    let Some(lines) = log.rev()? else { return Ok(Vec::new()) };
    lines.map(|line| Ok(line?)).collect()
}

#[cfg(test)]
mod tests {
    use super::natural_cmp;

    #[test]
    fn natural_order() {
        let mut tags = vec!["v1.10", "v1.9", "v1.2.3", "v2"];
        tags.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(tags, ["v1.2.3", "v1.9", "v1.10", "v2"]);
    }
}
