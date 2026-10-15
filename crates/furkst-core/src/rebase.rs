//! Interactive rebase: planning the commit list and running `git rebase -i` with a todo file we generate.

use std::fmt::Write as _;
use std::path::Path;

use crate::error::{Error, Result};
use crate::{CommitInfo, InProgress, Oid, Repo};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RebaseAction {
    #[default]
    Pick,
    /// Keep the commit but replace its message with [`RebaseStep::message`].
    Reword,
    /// Meld into the previous commit, keeping both messages.
    Squash,
    /// Meld into the previous commit, discarding this message.
    Fixup,
    Drop,
}

impl RebaseAction {
    pub const ALL: [Self; 5] = [Self::Pick, Self::Reword, Self::Squash, Self::Fixup, Self::Drop];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pick => "pick",
            Self::Reword => "reword",
            Self::Squash => "squash",
            Self::Fixup => "fixup",
            Self::Drop => "drop",
        }
    }
}

/// One line of the rebase todo list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebaseStep {
    pub commit: Oid,
    pub action: RebaseAction,
    /// The new message for [`RebaseAction::Reword`].
    pub message: Option<String>,
}

impl RebaseStep {
    pub fn new(commit: Oid) -> Self {
        Self { commit, action: RebaseAction::Pick, message: None }
    }
}

/// Checks that `steps` can be run: something must be kept, nothing may be squashed into nothing,
/// and rewords need a message.
pub fn validate_rebase(steps: &[RebaseStep]) -> Result<()> {
    match steps.iter().find(|s| s.action != RebaseAction::Drop) {
        None => return Err(Error::msg("nothing to rebase: every commit is dropped")),
        Some(first) if matches!(first.action, RebaseAction::Squash | RebaseAction::Fixup) => {
            return Err(Error::msg(format!("the first commit cannot be a {}", first.action.label())));
        }
        _ => {}
    }
    if steps.iter().any(|s| s.action == RebaseAction::Reword && s.message.as_deref().is_none_or(|m| m.trim().is_empty())) {
        return Err(Error::msg("a reworded commit needs a message"));
    }
    Ok(())
}

const WORK_DIR: &str = "furkst-rebase";

impl Repo {
    /// The commits an interactive rebase onto `base` would replay (`base..HEAD`, first-parent), oldest first.
    /// `None` plans from the root commit.
    pub fn rebase_plan(&self, base: Option<Oid>) -> Result<Vec<CommitInfo>> {
        let head = self.gix().head_id().map_err(|_| Error::msg("there are no commits to rebase"))?.detach();
        let mut plan = Vec::new();
        let mut next = Some(head);
        while let Some(id) = next {
            if Some(id) == base {
                break;
            }
            let commit = self.commit_info(id)?;
            if commit.parents.len() > 1 {
                return Err(Error::msg(format!(
                    "{} is a merge commit; interactive rebase over merges is not supported",
                    commit.short_id()
                )));
            }
            next = commit.parents.first().copied();
            plan.push(commit);
        }
        if base.is_some() && next.is_none() {
            return Err(Error::msg("the selected commit is not an ancestor of HEAD"));
        }
        plan.reverse();
        Ok(plan)
    }

    pub(crate) fn interactive_rebase(&self, base: Option<Oid>, steps: &[RebaseStep]) -> Result<String> {
        validate_rebase(steps)?;
        let dir = self.git_dir().join(WORK_DIR);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let result = self.run_rebase(base, steps, &dir);
        // A stopped rebase still needs the reword messages when it is continued.
        if self.in_progress() != Some(InProgress::Rebase) {
            let _ = std::fs::remove_dir_all(&dir);
        }
        result
    }

    fn run_rebase(&self, base: Option<Oid>, steps: &[RebaseStep], dir: &Path) -> Result<String> {
        let mut todo = String::new();
        for (index, step) in steps.iter().enumerate() {
            let RebaseStep { commit, action, message } = step;
            let verb = if *action == RebaseAction::Reword { "pick" } else { action.label() };
            writeln!(todo, "{verb} {commit}").expect("writing to a String");
            if *action == RebaseAction::Reword {
                let file = dir.join(format!("message-{index}"));
                std::fs::write(&file, format!("{}\n", message.as_deref().unwrap_or_default().trim_end()))?;
                writeln!(todo, "exec git commit --amend --allow-empty --no-verify -F {}", shell_quote(&file))
                    .expect("writing to a String");
            }
        }
        let todo_file = dir.join("todo");
        std::fs::write(&todo_file, todo)?;

        // git runs `<editor> <its todo file>` through a POSIX shell (also on Windows), so this copies ours over it.
        let editor = format!("cp {}", shell_quote(&todo_file));
        let base = base.map(|b| b.to_string());
        let args = ["rebase", "-i", "--autostash", base.as_deref().unwrap_or("--root")];
        self.cli().run_env(&args, &[("GIT_SEQUENCE_EDITOR", &editor)])
    }
}

/// Single-quotes a path for a POSIX shell, using forward slashes so Git for Windows' sh accepts it.
fn shell_quote(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    format!("'{}'", path.replace('\'', r"'\''"))
}
