//! One open repository: its state, background loading, and the actions its views emit.

mod autofetch;
mod changes;
mod commit_details;
mod dialogs;
mod diff_view;
mod file_tree;
mod files;
mod history;
mod highlight;
pub(crate) mod image_view;
mod menus;
mod rebase;
mod quick_launch;
mod selection;
mod sidebar;
mod toolbar;
mod resolver;
mod tools;
mod watcher;

use std::path::PathBuf;
use std::sync::Arc;

use egui::{RichText, Ui};
use furkst_core::{
    BlameResult, CommitDetails, CommitInfo, CommitSearch, DiffTarget, FileChange, FileDiff, GraphRow, Oid, Operation,
    RefsSnapshot, Repo, SearchHit, TreeFile, WorkingStatus,
};

use crate::palette::{self, Palette};
use crate::settings::Settings;
use crate::tasks::{Progress, Tasks};
use crate::widgets;
pub use dialogs::Dialog;

type Loaded<T> = Result<T, String>;

/// How many reflog entries the reflog window lists.
const REFLOG_LIMIT: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    LocalChanges,
    History,
}

/// Which commits the history lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    Only { label: String, tips: Vec<Oid> },
}

/// Everything a view can ask the tab to do. Views never mutate shared state directly.
/// With M, open or close the Changes window: Ctrl+Shift on Windows and Linux, Cmd+Shift on macOS.
const CHANGES_WINDOW_KEYS: egui::Modifiers = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);

/// Which diff pane a diff loads into.
#[derive(Clone, Debug, PartialEq, Eq)]
enum DiffPane {
    /// The pane next to the commit details or the local changes.
    Main,
    /// The file history window's.
    FileVersion,
    /// The one under a file opened in the Commit tab.
    Inline(String),
}

pub enum Action {
    Run(Operation),
    /// Ask before running a destructive operation.
    Confirm { title: String, message: String, op: Operation },
    Open(Dialog),
    ShowView(View),
    SelectCommit(Oid),
    SelectFile(DiffTarget, FileChange),
    SetScope(Scope),
    ReloadHistory,
    /// Open the Changes window (the files and the diff, in a window of its own), or close it.
    ToggleChangesWindow,
    /// Open the file's diff in place under it in the Commit tab, or close it.
    ToggleInlineDiff(DiffTarget, FileChange),
    /// Open `path` at `line` in the `editor`-th of [`crate::editors::installed`].
    RevealInEditor { editor: usize, path: String, line: usize },
    /// Search the history for what the search box holds, then select the first match.
    Search,
    /// Walk the history ahead of a search, so the first keystroke doesn't wait for it.
    PrepareSearch,
    /// Select the next (or previous) commit matching the history search; `restart` searches from the top.
    FindCommit { forward: bool, restart: bool },
    Refresh,
    FileHistory(String),
    /// Show the file history window's file as it changed in this commit.
    SelectVersion(Oid),
    /// Load the file history window's diff again (after the diff options changed).
    ShowVersionDiff(DiffTarget, FileChange),
    /// Open the Conflicts window at this conflicted file, or where it was (at the first conflicted file) for `None`.
    Resolve(Option<FileChange>),
    Blame { path: String, at: Oid },
    /// Show where `HEAD` has been, to recover from resets, rebases and deleted branches.
    ShowReflog,
    SetDetailsTab(file_tree::DetailsTab),
    SelectTreeFile(String),
    /// Writes a file as of the given commit to a location the user picks.
    SaveFileAs { path: String, at: Oid },
    OpenPath(PathBuf),
    Reveal(PathBuf),
    /// Open the interactive rebase editor for the commits after this one.
    /// Open the interactive rebase editor for the commits after this one, or for every commit when `None`.
    InteractiveRebase(Option<Oid>),
    OpenTerminal,
    /// Ask the application to open this repository in a tab (or focus its tab).
    OpenRepository(PathBuf),
}

enum Msg {
    Refs(Loaded<RefsSnapshot>),
    Status(Loaded<WorkingStatus>),
    History { generation: u64, result: Loaded<(Vec<CommitInfo>, Vec<GraphRow>)> },
    Search { generation: u64, result: Loaded<Vec<SearchHit>> },
    /// The history walk a search needs is ready.
    SearchPrepared,
    Details(Oid, Loaded<CommitDetails>),
    Diff { pane: DiffPane, generation: u64, result: Loaded<FileDiff> },
    Operation {
        description: String,
        result: Loaded<String>,
        changes_refs: bool,
        is_commit: bool,
        is_fetch: bool,
        /// Whether it continued or aborted what was in progress.
        finishes: bool,
        /// The conflicts it stopped on, when it failed because of them.
        stopped: Option<resolver::Stopped>,
        /// The same operation with uncommitted changes set aside, for one git may refuse because of them.
        stash_retry: Option<Operation>,
    },
    /// A background fetch: failures only go to the git output log.
    AutoFetch(Loaded<String>),
    FileHistory { path: String, result: Loaded<Vec<CommitInfo>> },
    Blame { path: String, result: Loaded<BlameResult> },
    Reflog(Loaded<Vec<furkst_core::ReflogEntry>>),
    TrackedFiles(Loaded<Vec<String>>),
    Tree(Oid, Loaded<Vec<TreeFile>>),
    TreeFile { commit: Oid, path: String, result: Loaded<file_tree::FileContent> },
}

struct LogEntry {
    description: String,
    output: Loaded<String>,
    /// Whether it failed by stopping on conflicts, which is no error but the next step.
    on_conflicts: bool,
}

pub struct RepoTab {
    pub repo: Arc<Repo>,
    tasks: Tasks<Msg>,
    view: View,
    refs: Option<RefsSnapshot>,
    status: WorkingStatus,
    history: history::HistoryState,
    details: Option<CommitDetails>,
    diff: diff_view::DiffPanel,
    /// The diffs shown under the files opened in the Commit tab, by path.
    inline_diffs: std::collections::BTreeMap<String, diff_view::DiffPanel>,
    tree: file_tree::TreeBrowser,
    commit_form: changes::CommitForm,
    selections: changes::Selections,
    sidebar: sidebar::SidebarState,
    dialog: Option<Dialog>,
    tools: tools::Tools,
    /// The Conflicts window, while open.
    conflicts: Option<resolver::ConflictsWindow>,
    /// Asking what to do about the conflicts an operation just stopped on.
    conflict_prompt: Option<resolver::Stopped>,
    /// Whether the operation in progress set uncommitted changes aside, to put back once it is finished or aborted.
    changes_set_aside: bool,
    log: Vec<LogEntry>,
    show_log: bool,
    auto_fetch: autofetch::AutoFetch,
    watcher: watcher::Watcher,
    /// The latest progress line of a running git operation.
    progress: Progress,
    /// Git operations (checkout, pull, commit…) under way, as opposed to quick background loads.
    operations: usize,
    /// Whether the Changes window is open (Ctrl+Shift+M): the changed files and the diff, big, in a window of their own.
    changes_window: bool,
    error: Option<String>,
    quick_launch: Option<Palette<Action>>,
    open_request: Option<PathBuf>,
}

impl Action {
    pub fn fetch_all(settings: &Settings) -> Self {
        Self::Run(fetch_all(settings))
    }
}

/// Fetches every remote, pruning as configured.
fn fetch_all(settings: &Settings) -> Operation {
    Operation::Fetch { remote: None, prune: settings.fetch_prune }
}

impl RepoTab {
    pub fn new(ctx: &egui::Context, repo: Repo) -> Self {
        let repo = Arc::new(repo);
        let template = repo.commit_template();
        let mut tab = Self {
            watcher: watcher::Watcher::new(ctx, &repo),
            progress: Progress::new(ctx),
            operations: 0,
            changes_window: false,
            repo,
            tasks: Tasks::new(ctx),
            view: View::History,
            refs: None,
            status: WorkingStatus::default(),
            history: history::HistoryState::default(),
            details: None,
            diff: diff_view::DiffPanel::default(),
            inline_diffs: Default::default(),
            tree: file_tree::TreeBrowser::default(),
            commit_form: changes::CommitForm::with_template(template),
            selections: changes::Selections::default(),
            sidebar: sidebar::SidebarState::default(),
            dialog: None,
            tools: tools::Tools::default(),
            conflicts: None,
            conflict_prompt: None,
            changes_set_aside: false,
            log: Vec::new(),
            show_log: false,
            auto_fetch: autofetch::AutoFetch::new(std::time::Instant::now()),
            error: None,
            quick_launch: None,
            open_request: None,
        };
        tab.refresh();
        tab
    }

    /// A repository the user asked to open from within this tab, if any.
    pub fn take_open_request(&mut self) -> Option<PathBuf> {
        self.open_request.take()
    }

    pub fn title(&self) -> String {
        self.repo.name()
    }

    /// Whether a git operation is under way. Not the quick loads that follow one (refs, history, details…): Refresh
    /// waiting on those would flicker on and off between them.
    pub fn operation_running(&self) -> bool {
        self.operations > 0
    }

    /// Reloads refs and the working copy status; history follows once refs arrive.
    pub fn refresh(&mut self) {
        self.spawn(|repo| Msg::Refs(repo.refs().map_err(|e| e.to_string())));
        self.refresh_status();
    }

    fn refresh_status(&mut self) {
        self.spawn(|repo| Msg::Status(repo.status().map_err(|e| e.to_string())));
    }

    /// Reloads what changed on disk outside Furkst, once the burst of changes has settled.
    fn apply_outside_changes(&mut self, ctx: &egui::Context) {
        match self.watcher.due(ctx, self.tasks.is_busy()) {
            Some(furkst_core::Change::Refs) => self.refresh(),
            Some(furkst_core::Change::WorkingCopy) => self.refresh_status(),
            None => {}
        }
    }

    fn spawn(&mut self, job: impl FnOnce(&Repo) -> Msg + Send + 'static) {
        let repo = Arc::clone(&self.repo);
        self.tasks.spawn(move || job(&repo));
    }

    pub fn ui(&mut self, ui: &mut Ui, settings: &mut Settings) {
        self.poll(settings);
        self.schedule_auto_fetch(ui.ctx(), settings);
        self.apply_outside_changes(ui.ctx());
        let mut actions = Vec::new();

        egui::Panel::top("toolbar").show(ui, |ui| {
            toolbar::show(ui, self, settings, &mut actions);
        });
        if self.status.operation.is_some() || self.status.has_conflicts() {
            egui::Panel::top("in_progress")
                .frame(egui::Frame::new())
                .show(ui, |ui| changes::in_progress_banner(ui, &self.status, &mut actions));
        }
        egui::Panel::bottom("statusbar").show(ui, |ui| self.status_bar(ui));
        if ui.input_mut(|i| i.consume_key(CHANGES_WINDOW_KEYS, egui::Key::M)) {
            actions.push(Action::ToggleChangesWindow);
        }
        widgets::left_panel(ui, "sidebar", 240.0).show(ui, |ui| {
            sidebar::show(ui, &mut self.sidebar, self.refs.as_ref(), &self.status, self.view, &self.history.scope, &mut actions);
        });
        egui::CentralPanel::no_frame().show(ui, |ui| match self.view {
            View::History => history::show(ui, self, settings, &mut actions),
            View::LocalChanges => changes::show(ui, self, settings, &mut actions),
        });
        if self.changes_window {
            self.changes_window(ui.ctx(), settings, &mut actions);
        }

        if let Some(dialog) = &mut self.dialog {
            match dialogs::show(ui.ctx(), dialog, self.refs.as_ref()) {
                dialogs::Outcome::Open => {}
                dialogs::Outcome::Cancel => self.dialog = None,
                dialogs::Outcome::Submit(action) => {
                    self.dialog = None;
                    actions.push(action);
                }
            }
        }
        tools::show(ui.ctx(), &mut self.tools, self.refs.as_ref(), settings, &mut actions);
        self.quick_launch(ui.ctx(), settings, &mut actions);
        self.conflicts_window(ui.ctx(), settings, &mut actions);
        self.error_window(ui.ctx());
        self.log_window(ui.ctx());

        for action in actions {
            self.handle(action, settings);
        }
    }

    /// Toggles on Ctrl/Cmd+P and shows the palette while it is open.
    fn quick_launch(&mut self, ctx: &egui::Context, settings: &Settings, actions: &mut Vec<Action>) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::P)) {
            if self.quick_launch.is_some() || self.dialog.is_some() {
                self.quick_launch = None;
            } else {
                let items = quick_launch::items(self.refs.as_ref(), settings, self.repo.workdir());
                let id = ("quick-launch", self.repo.workdir());
                self.quick_launch = Some(Palette::new(id, "Type a command, branch, tag or file…", items));
                self.spawn(|repo| Msg::TrackedFiles(repo.tracked_files().map_err(|e| e.to_string())));
            }
        }
        let Some(palette) = &mut self.quick_launch else { return };
        match palette.show(ctx) {
            None => {}
            Some(palette::Event::Close) => self.quick_launch = None,
            Some(palette::Event::Run(action)) => {
                self.quick_launch = None;
                actions.push(action);
            }
        }
    }

    fn handle(&mut self, action: Action, settings: &Settings) {
        match action {
            Action::Run(op) => self.run(op),
            Action::Confirm { title, message, op } => self.dialog = Some(Dialog::Confirm { title, message, op }),
            Action::Open(dialog) => self.dialog = Some(dialog),
            Action::ShowView(view) => {
                if self.view != view {
                    self.view = view;
                    self.diff.clear();
                    self.selections.clear();
                }
                if view == View::LocalChanges {
                    self.refresh_status();
                }
            }
            Action::SelectCommit(id) => {
                self.view = View::History;
                self.select_commit(id);
            }
            Action::SelectFile(target, change) => self.load_diff(DiffPane::Main, target, change, settings),
            Action::SelectVersion(id) => self.select_version(id, settings),
            Action::ShowVersionDiff(target, change) => self.load_diff(DiffPane::FileVersion, target, change, settings),
            Action::SetScope(scope) => {
                self.view = View::History;
                self.history.scope = scope;
                self.tips_changed();
            }
            Action::ReloadHistory => self.load_history(),
            Action::ToggleChangesWindow => self.changes_window = !self.changes_window,
            Action::ToggleInlineDiff(target, change) => {
                if self.inline_diffs.remove(&change.path).is_none() {
                    self.inline_diffs.insert(change.path.clone(), diff_view::DiffPanel::default());
                    self.load_diff(DiffPane::Inline(change.path.clone()), target, change, settings);
                }
            }
            Action::RevealInEditor { editor, path, line } => {
                if let Some(found) = crate::editors::installed().get(editor)
                    && let Err(e) = found.open(&self.repo.workdir().join(&path), line)
                {
                    self.error = Some(format!("Couldn't start {}: {e}", found.name));
                }
            }
            Action::Search => {
                self.search_history();
                self.history.seeking = self.history.searching().then_some((true, true));
            }
            Action::PrepareSearch => {
                if let Some(tips) = self.history_tips() {
                    self.spawn(move |repo| {
                        // A failure resurfaces in the search itself.
                        let _ = repo.prepare_search(&tips);
                        Msg::SearchPrepared
                    });
                }
            }
            Action::FindCommit { forward, restart } => self.find_commit(forward, restart),
            Action::Refresh => self.refresh(),
            Action::FileHistory(path) => {
                self.tools.open_file_history(&path);
                self.spawn(move |repo| {
                    let query = furkst_core::HistoryQuery {
                        tips: repo.refs().map(|r| r.head.target().into_iter().collect()).unwrap_or_default(),
                        limit: 1000,
                        path: Some(path.clone()),
                    };
                    Msg::FileHistory { result: repo.history(&query).map_err(|e| e.to_string()), path }
                });
            }
            Action::Resolve(change) => {
                let change = change.or_else(|| self.status.conflicted().next().cloned());
                self.conflicts.get_or_insert_with(Default::default).select(&self.repo, change);
            }
            Action::Blame { path, at } => {
                self.tools.open_blame(&path);
                self.spawn(move |repo| Msg::Blame { result: repo.blame(&path, at).map_err(|e| e.to_string()), path });
            }
            Action::ShowReflog => {
                self.tools.open_reflog();
                self.spawn(|repo| Msg::Reflog(repo.reflog(REFLOG_LIMIT).map_err(|e| e.to_string())));
            }
            Action::SetDetailsTab(tab) => {
                self.tree.tab = tab;
                self.load_tree();
            }
            Action::SelectTreeFile(path) => {
                self.tree.select(path.clone());
                if let Some(commit) = self.history.selected {
                    self.spawn(move |repo| Msg::TreeFile {
                        commit,
                        result: file_tree::load_file(repo, commit, &path),
                        path,
                    });
                }
            }
            Action::SaveFileAs { path, at } => {
                let name = path.rsplit('/').next().unwrap_or(&path);
                if let Some(destination) = rfd::FileDialog::new().set_title("Save file as").set_file_name(name).save_file() {
                    self.run(Operation::SaveFileAt { commit: at, path, destination });
                }
            }
            Action::OpenPath(path) => crate::platform::open(&self.repo.workdir().join(path)),
            Action::Reveal(path) => crate::platform::reveal(&self.repo.workdir().join(path)),
            Action::OpenTerminal => crate::platform::terminal(self.repo.workdir()),
            Action::OpenRepository(path) => self.open_request = Some(path),
            Action::InteractiveRebase(base) => self.open_rebase_editor(base),
        }
    }

    fn open_rebase_editor(&mut self, base: Option<Oid>) {
        let replaying = |repo: &Repo| -> furkst_core::Result<String> {
            Ok(match base {
                Some(base) => {
                    let info = repo.commit_info(base)?;
                    format!("the commits after {} {}", info.short_id(), info.summary)
                }
                None => "every commit, from the first one".to_owned(),
            })
        };
        let editor = replaying(&self.repo).and_then(|what| Ok(rebase::Editor::new(base, what, self.repo.rebase_plan(base)?)));
        match editor {
            Ok(editor) => self.dialog = Some(Dialog::Rebase(editor)),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn run(&mut self, op: Operation) {
        let description = op.describe();
        let changes_refs = op.changes_refs();
        let is_commit = matches!(op, Operation::Commit { .. });
        let is_fetch = matches!(op, Operation::Fetch { .. });
        let finishes = matches!(op, Operation::Continue(_) | Operation::Abort(_));
        let may_conflict = op.may_conflict();
        let stash_retry = op.with_stash();
        self.operations += 1;
        let progress = self.progress.clone();
        self.spawn(move |repo| {
            let result = op.run_with_progress(repo, &|line| progress.report(line)).map_err(|e| e.to_string());
            let stopped = result.as_ref().err().filter(|_| may_conflict).and_then(|error| resolver::Stopped::after_failure(repo, error));
            Msg::Operation { result, description, changes_refs, is_commit, is_fetch, finishes, stopped, stash_retry }
        });
    }

    /// Selects the next history search match, once the search finished and the history is loaded far enough.
    fn find_commit(&mut self, forward: bool, restart: bool) {
        match self.history.find(forward, restart) {
            history::Found::Commit(id) => self.select_commit(id),
            history::Found::NeedsHistoryUntil(position) => {
                self.history.seeking = Some((forward, restart));
                self.history.load_until(position);
                self.load_history();
            }
            history::Found::Searching => self.history.seeking = Some((forward, restart)),
            history::Found::Nothing => {}
        }
    }

    /// The Changes window: the files changed by the selected commit (or the local changes, with their staging controls)
    /// beside the diff, in a big window of its own, like an editor in VS Code. It follows the same selection as the main
    /// window. Its close button, Ctrl+Shift+M or Esc closes it.
    fn changes_window(&mut self, ctx: &egui::Context, settings: &mut Settings, actions: &mut Vec<Action>) {
        let builder = egui::ViewportBuilder::default()
            .with_title(format!("Changes — {}", self.title()))
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([700.0, 400.0])
            .with_icon(crate::icon::window_icon());
        let id = egui::ViewportId::from_hash_of(("changes", self.repo.workdir()));
        ctx.show_viewport_immediate(id, builder, |ui, _| {
            let typing = ui.ctx().egui_wants_keyboard_input();
            let close = ui.input_mut(|i| {
                i.viewport().close_requested()
                    || i.consume_key(CHANGES_WINDOW_KEYS, egui::Key::M)
                    || (!typing && i.key_pressed(egui::Key::Escape))
            });
            if close {
                actions.push(Action::ToggleChangesWindow);
            }
            widgets::left_panel(ui, "changes_window_files", 360.0).show(ui, |ui| match self.view {
                View::History => {
                    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| match &self.details {
                        Some(details) => {
                            commit_details::changed_files(ui, details, &self.diff, actions);
                        }
                        None => widgets::placeholder(ui, "Select a commit"),
                    });
                }
                View::LocalChanges => changes::file_lists(ui, self, actions),
            });
            egui::CentralPanel::default().show(ui, |ui| match self.view {
                View::History => self.diff.show(ui, settings, Action::SelectFile, Some(true), actions),
                View::LocalChanges => changes::diff(ui, self, settings, true, actions),
            });
        });
    }

    /// The prompt after an operation stopped on conflicts, and the Conflicts window while it is open.
    fn conflicts_window(&mut self, ctx: &egui::Context, settings: &Settings, actions: &mut Vec<Action>) {
        if let Some(stopped) = &self.conflict_prompt
            && resolver::prompt(ctx, stopped, actions)
        {
            self.conflict_prompt = None;
        }
        let name = self.title();
        let Some(window) = &mut self.conflicts else { return };
        let id = egui::ViewportId::from_hash_of(("conflicts", self.repo.workdir()));
        let font = egui::FontId::monospace(settings.diff_font_size);
        if resolver::show(ctx, window, &self.status, &name, id, &font, actions) {
            self.conflicts = None;
        }
    }

    /// Shows how the file history window's file changed in `id`.
    fn select_version(&mut self, id: Oid, settings: &Settings) {
        let Some(path) = self.tools.file_history_path() else { return };
        match self.repo.file_change_at(id, &path) {
            Ok(change) => {
                self.tools.select_version(id);
                self.load_diff(DiffPane::FileVersion, DiffTarget::Commit(id), change, settings);
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn select_commit(&mut self, id: Oid) {
        if self.history.selected == Some(id) && self.details.is_some() {
            return;
        }
        self.history.selected = Some(id);
        self.history.scroll_to_selected = true;
        self.details = None;
        self.diff.clear();
        self.tree.reset();
        self.inline_diffs.clear();
        self.spawn(move |repo| Msg::Details(id, repo.commit_details(id).map_err(|e| e.to_string())));
    }

    /// Loads the selected commit's file tree if the File Tree tab needs it.
    fn load_tree(&mut self) {
        if let Some(commit) = self.history.selected
            && self.tree.begin_load()
        {
            self.spawn(move |repo| Msg::Tree(commit, file_tree::load_tree(repo, commit)));
        }
    }

    /// The pane a diff loads into; `None` for a file closed in the Commit tab since.
    fn diff_panel(&mut self, pane: &DiffPane) -> Option<&mut diff_view::DiffPanel> {
        match pane {
            DiffPane::Main => Some(&mut self.diff),
            DiffPane::FileVersion => Some(self.tools.version_diff()),
            DiffPane::Inline(path) => self.inline_diffs.get_mut(path),
        }
    }

    fn load_diff(&mut self, pane: DiffPane, target: DiffTarget, change: FileChange, settings: &Settings) {
        let Some(panel) = self.diff_panel(&pane) else { return };
        let generation = panel.start(target.clone(), change.clone());
        let options = settings.diff_options();
        self.spawn(move |repo| Msg::Diff {
            pane,
            generation,
            result: repo.file_diff(&target, &change, options).map_err(|e| e.to_string()),
        });
    }

    /// After the working copy changed, reloads the selected file's diff from whichever list now holds it
    /// (e.g. a fully staged file moves from unstaged to staged), or clears it if it is gone.
    fn follow_selected_file(&mut self, settings: &Settings) {
        let Some((target, change)) = self.diff.key() else { return };
        let find = |files: &[FileChange]| files.iter().find(|c| c.path == change.path).cloned();
        let (unstaged, staged) = (find(&self.status.unstaged), find(&self.status.staged));
        let next = match target {
            DiffTarget::Unstaged => unstaged.map(|c| (DiffTarget::Unstaged, c)).or(staged.map(|c| (DiffTarget::Staged, c))),
            DiffTarget::Staged => staged.map(|c| (DiffTarget::Staged, c)).or(unstaged.map(|c| (DiffTarget::Unstaged, c))),
            DiffTarget::Commit(_) => return,
        };
        match next {
            Some((target, change)) => {
                self.selections.select_only(&target, &change.path);
                self.load_diff(DiffPane::Main, target, change, settings);
            }
            None => self.diff.clear(),
        }
    }

    /// Where the history the scope lists starts; unknown until refs are loaded.
    fn history_tips(&self) -> Option<Vec<Oid>> {
        match (&self.history.scope, &self.refs) {
            (Scope::Only { tips, .. }, _) => Some(tips.clone()),
            (Scope::All, Some(refs)) => Some(refs.all_tips()),
            (Scope::All, None) => None,
        }
    }

    /// Reloads the history, and the search over it, after its tips or scope changed.
    fn tips_changed(&mut self) {
        self.load_history();
        if self.history.searching() {
            self.search_history();
        }
    }

    /// Searches the whole history the scope lists for the search box's text, replacing a running search.
    fn search_history(&mut self) {
        let search = CommitSearch::new(&self.history.search).in_paths(self.history.search_paths);
        let (generation, cancel) = self.history.begin_search();
        let Some(tips) = self.history_tips().filter(|_| !search.is_empty()) else {
            self.history.finish_search(generation, Vec::new());
            return;
        };
        let ref_targets = self.refs.as_ref().map(|refs| search.ref_targets(refs)).unwrap_or_default();
        self.spawn(move |repo| Msg::Search {
            generation,
            result: repo.search_commits(&tips, &search, &ref_targets, &cancel).map_err(|e| e.to_string()),
        });
    }

    fn load_history(&mut self) {
        let Some(tips) = self.history_tips() else { return };
        let query = self.history.query(tips);
        let generation = self.history.begin_load();
        self.spawn(move |repo| Msg::History {
            generation,
            result: repo
                .history(&query)
                .map(|commits| {
                    let graph = furkst_core::layout_graph(&commits);
                    (commits, graph)
                })
                .map_err(|e| e.to_string()),
        });
    }

    fn schedule_auto_fetch(&mut self, ctx: &egui::Context, settings: &Settings) {
        let has_remotes = self.refs.as_ref().is_some_and(|r| !r.remotes.is_empty());
        match self.auto_fetch.tick(std::time::Instant::now(), settings.auto_fetch_minutes, self.tasks.is_busy(), has_remotes) {
            autofetch::Decision::Fetch => {
                let op = fetch_all(settings);
                self.spawn(move |repo| Msg::AutoFetch(op.run(repo).map_err(|e| e.to_string())));
            }
            autofetch::Decision::Wait(Some(after)) => ctx.request_repaint_after(after),
            autofetch::Decision::Wait(None) => {}
        }
    }

    fn poll(&mut self, settings: &mut Settings) {
        for msg in self.tasks.poll() {
            match msg {
                Msg::Refs(Ok(refs)) => {
                    let tips_changed = self.refs.as_ref().is_none_or(|old| old.all_tips() != refs.all_tips() || old.labels != refs.labels);
                    self.refs = Some(refs);
                    if tips_changed {
                        self.tips_changed();
                    }
                }
                Msg::Status(Ok(status)) => {
                    if self.status.operation.is_none()
                        && let Some(message) = status.operation.and_then(|_| self.repo.pending_message())
                    {
                        self.commit_form.prefill(&message);
                    }
                    self.status = status;
                    self.selections.retain(&self.status);
                    if self.view == View::LocalChanges && !self.selections.is_multiple() {
                        self.follow_selected_file(settings);
                    }
                }
                Msg::History { generation, result: Ok((commits, graph)) } => {
                    let current = self.history.finish_load(generation, commits, graph);
                    if current && let Some((forward, restart)) = self.history.seeking.take() {
                        self.find_commit(forward, restart);
                    } else if current && self.history.selected.is_none() {
                        let head = self.refs.as_ref().and_then(|r| r.head.target());
                        if let Some(first) = head.or_else(|| self.history.commits.first().map(|c| c.id)) {
                            self.select_commit(first);
                        }
                    }
                }
                Msg::Search { generation, result } => match result {
                    Ok(hits) => {
                        if self.history.finish_search(generation, hits)
                            && let Some((forward, restart)) = self.history.seeking.take()
                        {
                            self.find_commit(forward, restart);
                        }
                    }
                    Err(e) => {
                        if self.history.finish_search(generation, Vec::new()) {
                            self.error = Some(e);
                        }
                    }
                },
                Msg::SearchPrepared => {}
                Msg::Details(id, Ok(details)) if self.history.selected == Some(id) => {
                    if let Some(first) = details.changes.first().cloned() {
                        self.load_diff(DiffPane::Main, DiffTarget::Commit(id), first, settings);
                    }
                    self.details = Some(details);
                    self.load_tree();
                }
                Msg::Tree(id, result) if self.history.selected == Some(id) => match result {
                    Ok(files) => self.tree.finish_tree(files),
                    Err(e) => self.error = Some(e),
                },
                Msg::TreeFile { commit, path, result } if self.history.selected == Some(commit) => {
                    self.tree.finish_file(&path, result);
                }
                Msg::Diff { pane, generation, result } => {
                    if let Some(panel) = self.diff_panel(&pane) {
                        panel.finish(generation, result);
                    }
                }
                Msg::Operation { description, result, changes_refs, is_commit, is_fetch, finishes, stopped, stash_retry } => {
                    self.operations -= 1;
                    // The operation's own file changes are covered by the refresh below.
                    self.watcher.discard();
                    self.progress.clear();
                    if is_fetch && result.is_ok() {
                        self.auto_fetch.succeeded(std::time::Instant::now());
                    }
                    let on_conflicts = stopped.is_some();
                    if result.as_ref().is_err_and(|error| furkst_core::kept_aside(error)) {
                        self.changes_set_aside = true;
                    }
                    match (&result, stopped) {
                        // The Conflicts window, when open, takes the new conflicts in (the next commit of a rebase).
                        (Err(_), Some(_)) if self.conflicts.is_some() => {}
                        // Stopping on conflicts is expected, not an error: say so and offer the ways on.
                        (Err(_), Some(stopped)) => self.conflict_prompt = Some(stopped),
                        // Refused because of uncommitted changes: offer to set them aside.
                        (Err(error), None) if stash_retry.is_some() && furkst_core::blocked_by_local_changes(error) => {
                            if let Some(op) = stash_retry {
                                self.dialog = Some(Dialog::StashFirst { error: error.clone(), op });
                            }
                        }
                        (Err(error), None) => self.error = Some(error.clone()),
                        (Ok(_), _) => {
                            if is_commit {
                                self.commit_form.clear();
                            }
                            // Whatever was in progress is over.
                            if is_commit || finishes {
                                self.conflicts = None;
                                if std::mem::take(&mut self.changes_set_aside) {
                                    self.run(Operation::RestoreSetAside);
                                }
                            }
                        }
                    }
                    self.log.push(LogEntry { description, output: result, on_conflicts });
                    if changes_refs {
                        self.refresh();
                    } else {
                        self.refresh_status();
                    }
                }
                Msg::AutoFetch(result) => {
                    let succeeded = result.is_ok();
                    self.log.push(LogEntry { description: "Auto-fetch".into(), output: result, on_conflicts: false });
                    if succeeded {
                        self.auto_fetch.succeeded(std::time::Instant::now());
                        self.refresh();
                    }
                }
                Msg::FileHistory { path, result } => {
                    self.tools.file_history_loaded(&path, result);
                    if let Some(newest) = self.tools.newest_version() {
                        self.select_version(newest, settings);
                    }
                }
                Msg::Blame { path, result } => self.tools.blame_loaded(&path, result),
                Msg::Reflog(result) => self.tools.reflog_loaded(result),
                Msg::TrackedFiles(Ok(paths)) => {
                    if let Some(palette) = &mut self.quick_launch {
                        palette.extend(quick_launch::file_items(paths));
                    }
                }
                Msg::TrackedFiles(Err(e)) | Msg::Refs(Err(e)) | Msg::Status(Err(e)) | Msg::Details(_, Err(e)) => self.error = Some(e),
                Msg::History { generation, result: Err(e) } => {
                    if self.history.fail_load(generation) {
                        self.error = Some(e);
                    }
                }
                Msg::Details(..) | Msg::Tree(..) | Msg::TreeFile { .. } => {}
            }
        }
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if self.tasks.is_busy() {
                ui.spinner();
            }
            match (self.progress.current(), self.log.last()) {
                (Some(line), _) => ui.label(RichText::new(line).weak()),
                (None, Some(LogEntry { description, on_conflicts: true, .. })) => {
                    ui.label(RichText::new(format!("{description}: stopped on conflicts")).color(ui.visuals().warn_fg_color))
                }
                (None, Some(LogEntry { description, output: Ok(_), .. })) => {
                    ui.label(RichText::new(format!("✔ {description}")).weak())
                }
                (None, Some(LogEntry { description, output: Err(_), .. })) => {
                    ui.label(RichText::new(format!("✖ {description}")).color(ui.visuals().error_fg_color))
                }
                (None, None) => ui.label(RichText::new(self.repo.workdir().display().to_string()).weak()),
            };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Git output").clicked() {
                    self.show_log = !self.show_log;
                }
                let now = std::time::Instant::now();
                if let Some(text) = self.auto_fetch.last_success_text(now) {
                    ui.label(RichText::new(format!("Last fetch {text}")).weak().small());
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs(30));
                }
            });
        });
    }

    fn error_window(&mut self, ctx: &egui::Context) {
        let Some(error) = &self.error else { return };
        let mut close = false;
        egui::Modal::new(egui::Id::new("error").with(self.repo.workdir())).show(ctx, |ui| {
            ui.set_max_width(560.0);
            ui.heading("Something went wrong");
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                ui.label(RichText::new(error).monospace());
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(error.clone());
                }
                close = ui.button("Close").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if close {
            self.error = None;
        }
    }

    fn log_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("Git output").open(&mut self.show_log).default_size([560.0, 360.0]).show(ctx, |ui| {
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink(false).show(ui, |ui| {
                for entry in &self.log {
                    let (icon, text) = match &entry.output {
                        Ok(out) => ("✔", out),
                        Err(err) => ("✖", err),
                    };
                    ui.label(RichText::new(format!("{icon} {}", entry.description)).strong());
                    if !text.is_empty() {
                        ui.label(RichText::new(text).monospace().small());
                    }
                    ui.separator();
                }
            });
        });
    }
}
