//! Git-flow branching (feature, release and hotfix branches), done with plain git commands and stored in the
//! same `gitflow.*` configuration keys as the git-flow extension, so both can be used on one repository.

use crate::error::{Error, Result};
use crate::{MergeStyle, Repo};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowKind {
    Feature,
    Release,
    Hotfix,
}

impl FlowKind {
    pub const ALL: [Self; 3] = [Self::Feature, Self::Release, Self::Hotfix];

    pub fn label(self) -> &'static str {
        match self {
            Self::Feature => "feature",
            Self::Release => "release",
            Self::Hotfix => "hotfix",
        }
    }

    /// Releases and hotfixes ship: finishing them merges into the production branch and tags a version.
    fn ships(self) -> bool {
        self != Self::Feature
    }
}

/// A repository's git-flow setup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowConfig {
    /// The production branch, e.g. `main`.
    pub main: String,
    /// The integration branch, e.g. `develop`.
    pub develop: String,
    pub feature_prefix: String,
    pub release_prefix: String,
    pub hotfix_prefix: String,
    /// Prepended to release and hotfix names to form tags, often empty or `v`.
    pub version_tag_prefix: String,
}

impl FlowConfig {
    pub fn prefix(&self, kind: FlowKind) -> &str {
        match kind {
            FlowKind::Feature => &self.feature_prefix,
            FlowKind::Release => &self.release_prefix,
            FlowKind::Hotfix => &self.hotfix_prefix,
        }
    }

    /// The branch a new `kind` branch starts from: production for hotfixes, development otherwise.
    pub fn base(&self, kind: FlowKind) -> &str {
        if kind == FlowKind::Hotfix { &self.main } else { &self.develop }
    }

    /// The flow branch `branch` belongs to, as `(kind, name)`, e.g. `feature/login` → `(Feature, "login")`.
    pub fn classify(&self, branch: &str) -> Option<(FlowKind, String)> {
        FlowKind::ALL
            .into_iter()
            .find_map(|kind| branch.strip_prefix(self.prefix(kind)).filter(|n| !n.is_empty()).map(|n| (kind, n.to_owned())))
    }
}

impl Repo {
    /// The git-flow setup, or `None` if git-flow hasn't been initialized here.
    pub fn flow_config(&self) -> Option<FlowConfig> {
        let repo = self.gix();
        let config = repo.config_snapshot();
        let get = |key: &str, default: &str| config.string(key).map_or_else(|| default.to_owned(), |v| v.to_string());
        let develop = config.string("gitflow.branch.develop")?.to_string();
        Some(FlowConfig {
            main: get("gitflow.branch.master", "main"),
            develop,
            feature_prefix: get("gitflow.prefix.feature", "feature/"),
            release_prefix: get("gitflow.prefix.release", "release/"),
            hotfix_prefix: get("gitflow.prefix.hotfix", "hotfix/"),
            version_tag_prefix: get("gitflow.prefix.versiontag", ""),
        })
    }

    /// Sets up git-flow with `main` and `develop`, creating `develop` from `main` if needed.
    pub(crate) fn flow_init(&self, main: &str, develop: &str) -> Result<String> {
        let git = self.cli();
        if self.resolve(develop).is_err() {
            git.run(&["branch", develop, main])?;
        }
        for (key, value) in [
            ("gitflow.branch.master", main),
            ("gitflow.branch.develop", develop),
            ("gitflow.prefix.feature", "feature/"),
            ("gitflow.prefix.release", "release/"),
            ("gitflow.prefix.hotfix", "hotfix/"),
            ("gitflow.prefix.versiontag", ""),
        ] {
            git.run(&["config", key, value])?;
        }
        Ok(format!("Git-flow initialized: {main} for releases, {develop} for development"))
    }

    /// Creates and checks out `<prefix><name>` from develop (features, releases) or main (hotfixes).
    pub(crate) fn flow_start(&self, kind: FlowKind, name: &str) -> Result<String> {
        let flow = self.require_flow()?;
        let base = flow.base(kind);
        let branch = format!("{}{name}", flow.prefix(kind));
        self.cli().run(&["checkout", "-b", &branch, base])?;
        Ok(format!("Started {branch} from {base}"))
    }

    /// Merges a flow branch where it belongs, tags shipped versions, and deletes the branch. Features merge with
    /// `style`; releases and hotfixes always get a merge commit, since a squash would disconnect main from develop.
    pub(crate) fn flow_finish(&self, kind: FlowKind, name: &str, style: MergeStyle) -> Result<String> {
        let flow = self.require_flow()?;
        let branch = format!("{}{name}", flow.prefix(kind));
        let style = if kind.ships() { MergeStyle::MergeCommit } else { style };
        let git = self.cli();
        if kind.ships() {
            self.merge_into(&flow.main, &branch, style)?;
            let tag = format!("{}{name}", flow.version_tag_prefix);
            git.run(&["tag", "-a", &tag, "-m", &format!("{} {name}", kind.label())])?;
        }
        self.merge_into(&flow.develop, &branch, style)?;
        // Once its commits are copied, git doesn't consider the branch merged, so it needs a forced delete.
        git.run(&["branch", if style.copies_commits() { "-D" } else { "-d" }, &branch])?;
        Ok(format!("Finished {branch}"))
    }

    /// Checks out `target` and merges `branch` into it.
    fn merge_into(&self, target: &str, branch: &str, style: MergeStyle) -> Result<String> {
        self.cli().run(&["checkout", target])?;
        self.merge_branch(branch, style)
    }

    fn require_flow(&self) -> Result<FlowConfig> {
        self.flow_config().ok_or_else(|| Error::msg("git-flow is not initialized in this repository"))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_flow_branches() {
        let flow = FlowConfig {
            main: "main".into(),
            develop: "develop".into(),
            feature_prefix: "feature/".into(),
            release_prefix: "release/".into(),
            hotfix_prefix: "hotfix/".into(),
            version_tag_prefix: "v".into(),
        };
        assert_eq!(flow.classify("feature/login"), Some((FlowKind::Feature, "login".into())));
        assert_eq!(flow.classify("release/1.2.0"), Some((FlowKind::Release, "1.2.0".into())));
        assert_eq!(flow.classify("develop"), None);
        assert_eq!(flow.classify("feature/"), None);
    }
}
