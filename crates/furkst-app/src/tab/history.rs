//! The commit list with its graph, and the details pane underneath.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, Ui, Vec2, pos2};
use furkst_core::{CommitInfo, GraphRow, Half, HistoryQuery, Oid, SearchHit};

use super::{Action, RepoTab, Scope, commit_details, menus};
use crate::settings::Settings;
use crate::{theme, widgets};

const PAGE: usize = 2000;
const ROW_HEIGHT: f32 = 24.0;
const LANE_WIDTH: f32 = 14.0;

pub struct HistoryState {
    pub commits: Vec<CommitInfo>,
    graph: Vec<GraphRow>,
    graph_lanes: usize,
    pub selected: Option<Oid>,
    pub scroll_to_selected: bool,
    pub scope: Scope,
    pub search: String,
    /// Whether the search also matches the paths of the files commits changed.
    pub search_paths: bool,
    /// Whether the search bar is shown (Ctrl+F, or Cmd+F on macOS); `focus_search` puts the cursor in it once.
    search_open: bool,
    focus_search: bool,
    /// Every commit in the whole history matching the search, in walk order; empty without a search.
    hits: Vec<SearchHit>,
    hit_ids: HashSet<Oid>,
    /// Indices of the loaded commits among the hits, ascending.
    matches: Vec<usize>,
    search_generation: u64,
    /// Whether a search is running; its hits replace the current ones.
    search_pending: bool,
    /// Set to stop the running search once a newer one replaces it.
    search_cancel: Arc<AtomicBool>,
    /// A search step (forward, restart) waiting for the search or for more history to load.
    pub seeking: Option<(bool, bool)>,
    limit: usize,
    generation: u64,
    loading: bool,
    visible: std::ops::Range<usize>,
}

impl Default for HistoryState {
    fn default() -> Self {
        Self {
            commits: Vec::new(),
            graph: Vec::new(),
            graph_lanes: 0,
            selected: None,
            scroll_to_selected: false,
            scope: Scope::All,
            search: String::new(),
            search_paths: false,
            search_open: false,
            focus_search: false,
            hits: Vec::new(),
            hit_ids: HashSet::new(),
            matches: Vec::new(),
            search_generation: 0,
            search_pending: false,
            search_cancel: Arc::default(),
            seeking: None,
            limit: PAGE,
            generation: 0,
            loading: false,
            visible: 0..0,
        }
    }
}

impl HistoryState {
    pub fn query(&self, tips: Vec<Oid>) -> HistoryQuery {
        HistoryQuery { tips, limit: self.limit, ..Default::default() }
    }

    pub fn begin_load(&mut self) -> u64 {
        self.loading = true;
        self.generation += 1;
        self.generation
    }

    /// Stores a finished load; returns false if a newer load superseded it.
    pub fn finish_load(&mut self, generation: u64, commits: Vec<CommitInfo>, graph: Vec<GraphRow>) -> bool {
        if generation != self.generation {
            return false;
        }
        self.loading = false;
        self.graph_lanes = graph.iter().map(|r| r.width).max().unwrap_or(0);
        self.commits = commits;
        self.graph = graph;
        self.update_matches();
        true
    }

    pub fn fail_load(&mut self, generation: u64) -> bool {
        let current = generation == self.generation;
        if current {
            self.loading = false;
        }
        current
    }

    fn index_of(&self, id: Oid) -> Option<usize> {
        self.commits.iter().position(|c| c.id == id)
    }

    /// Loads at least as far into the history as the walk `position`, in whole pages.
    pub fn load_until(&mut self, position: usize) {
        self.limit = self.limit.max((position / PAGE + 1) * PAGE);
    }

    pub fn searching(&self) -> bool {
        !self.search.trim().is_empty()
    }

    /// Stops the running search and starts tracking a new one; returns its generation and cancel flag.
    pub fn begin_search(&mut self) -> (u64, Arc<AtomicBool>) {
        self.search_cancel.store(true, Ordering::Relaxed);
        self.search_cancel = Arc::default();
        self.search_generation += 1;
        self.search_pending = true;
        (self.search_generation, Arc::clone(&self.search_cancel))
    }

    /// Stores a finished search; returns false if a newer search superseded it.
    pub fn finish_search(&mut self, generation: u64, hits: Vec<SearchHit>) -> bool {
        if generation != self.search_generation {
            return false;
        }
        self.search_pending = false;
        self.hit_ids = hits.iter().map(|hit| hit.id).collect();
        self.hits = hits;
        self.update_matches();
        true
    }

    fn update_matches(&mut self) {
        self.matches = self.commits.iter().enumerate().filter(|(_, c)| self.hit_ids.contains(&c.id)).map(|(i, _)| i).collect();
    }

    fn is_match(&self, index: usize) -> bool {
        self.matches.binary_search(&index).is_ok()
    }

    /// The match after (or before) the selected commit, or the first one when `restart`. Going forward past the
    /// last loaded match loads the history up to the next one first; at the very end it wraps around.
    pub fn find(&self, forward: bool, restart: bool) -> Found {
        if self.search_pending {
            return Found::Searching;
        }
        let current = if restart { None } else { self.selected.and_then(|id| self.index_of(id)) };
        let next = if forward {
            self.matches.iter().find(|&&i| current.is_none_or(|c| i > c))
        } else {
            self.matches.iter().rev().find(|&&i| current.is_some_and(|c| i < c))
        };
        if let Some(&index) = next {
            return Found::Commit(self.commits[index].id);
        }
        // The loaded commits are the start of the walk, so the hits past them aren't loaded yet.
        let unloaded = self.hits.iter().find(|hit| hit.position >= self.commits.len());
        if forward && let Some(hit) = unloaded {
            return Found::NeedsHistoryUntil(hit.position);
        }
        let wrapped = if forward { self.matches.first() } else { self.matches.last() };
        wrapped.map_or(Found::Nothing, |&i| Found::Commit(self.commits[i].id))
    }

    /// "3 of 57" for the selected match, counting the whole history; "Searching…" while a search or a step to the
    /// next match is under way.
    fn match_counter(&self) -> String {
        if self.search_pending || self.seeking.is_some() {
            return "Searching…".to_owned();
        }
        let selected = self.selected.and_then(|id| self.index_of(id)).and_then(|i| self.matches.binary_search(&i).ok());
        match selected {
            Some(position) => format!("{} of {}", position + 1, self.hits.len()),
            None => format!("{} matches", self.hits.len()),
        }
    }
}

/// Where the next search match is.
pub enum Found {
    Commit(Oid),
    /// Past the loaded commits, at this walk position.
    NeedsHistoryUntil(usize),
    /// Not known until the running search finishes.
    Searching,
    Nothing,
}

pub fn show(ui: &mut Ui, tab: &mut RepoTab, settings: &mut Settings, actions: &mut Vec<Action>) {
    if settings.details_collapsed {
        // Just the tab row, so the commits get the whole height.
        egui::Panel::bottom("commit_details_collapsed").show(ui, |ui| commit_details::tab_row(ui, tab, settings, actions));
    } else {
        let height = ui.available_height();
        egui::Panel::bottom("commit_details")
            .resizable(true)
            .default_size(height * 0.55)
            .size_range(120.0..=height - 120.0)
            .show(ui, |ui| commit_details::show(ui, tab, settings, actions));
    }
    egui::CentralPanel::no_frame().show(ui, |ui| {
        search_bar(ui, &mut tab.history, actions);
        commit_list(ui, tab, actions);
    });
}

/// The row above the commits: the search, which Ctrl+F (Cmd+F on macOS) shows and hides (as does Esc), and a note when
/// only one branch is listed. Without either, the row isn't there at all.
fn search_bar(ui: &mut Ui, state: &mut HistoryState, actions: &mut Vec<Action>) {
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
        if state.search_open {
            close_search(state, actions);
        } else {
            state.search_open = true;
            state.focus_search = true;
        }
    }
    if !state.search_open && !matches!(state.scope, Scope::Only { .. }) {
        return;
    }
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        if state.search_open {
            search_controls(ui, state, actions);
        }
        if state.loading {
            ui.spinner();
        }
        if let Scope::Only { label, .. } = &state.scope {
            ui.label(RichText::new(format!("Showing only {label}")).weak());
            if ui.small_button("Show all").clicked() {
                actions.push(Action::SetScope(Scope::All));
            }
        }
    });
    ui.separator();
}

/// The search box, its options and the way through its matches. Closing it clears the search, so nothing stays dimmed.
fn search_controls(ui: &mut Ui, state: &mut HistoryState, actions: &mut Vec<Action>) {
    let input = ui
        .add(egui::TextEdit::singleline(&mut state.search).hint_text("🔍 Hash, author, message or ref").desired_width(260.0))
        .on_hover_text("Searches the whole history, ignoring case");
    if std::mem::take(&mut state.focus_search) {
        input.request_focus();
    }
    // A single-line field gives up focus on Enter; keep it so Enter can be pressed again for the next match.
    let enter = input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if enter {
        input.request_focus();
    }
    if input.gained_focus() {
        actions.push(Action::PrepareSearch);
    }
    let paths = ui
        .toggle_value(&mut state.search_paths, "Files")
        .on_hover_text("Also find commits that changed a file whose path contains the text (slower)");
    if input.changed() || (paths.changed() && state.searching()) {
        actions.push(Action::Search);
    }
    if state.searching() {
        let previous = ui.small_button("⬆").on_hover_text("Previous match (Shift+Enter)").clicked();
        let next = ui.small_button("⬇").on_hover_text("Next match (Enter)").clicked();
        if previous || next || enter {
            let forward = !(previous || enter && ui.input(|i| i.modifiers.shift));
            actions.push(Action::FindCommit { forward, restart: false });
        }
        ui.label(RichText::new(state.match_counter()).weak());
    }
    if widgets::escaped(ui, &input) {
        close_search(state, actions);
    }
}

/// Hides the search and clears it, so nothing stays dimmed.
fn close_search(state: &mut HistoryState, actions: &mut Vec<Action>) {
    state.search_open = false;
    if state.searching() {
        state.search.clear();
        actions.push(Action::Search);
    }
}

fn commit_list(ui: &mut Ui, tab: &mut RepoTab, actions: &mut Vec<Action>) {
    let Some(refs) = tab.refs.as_ref() else { return widgets::placeholder(ui, "Loading…") };
    let state = &mut tab.history;
    if state.commits.is_empty() {
        let text = if state.loading { "Loading…" } else { "No commits" };
        return widgets::placeholder(ui, text);
    }

    let mut scroll = egui::ScrollArea::vertical().auto_shrink(false);
    if state.scroll_to_selected {
        state.scroll_to_selected = false;
        if let Some(index) = state.selected.and_then(|id| state.index_of(id))
            && !state.visible.contains(&index)
        {
            scroll = scroll.vertical_scroll_offset((index as f32 * ROW_HEIGHT - 100.0).max(0.0));
        }
    }

    // Rows must touch so graph lines connect.
    ui.spacing_mut().item_spacing.y = 0.0;
    let graph_width = state.graph_lanes.max(1) as f32 * LANE_WIDTH + 8.0;
    let head = refs.head.target();
    let selection_fill = ui.visuals().selection.bg_fill;
    let output = scroll.show_rows(ui, ROW_HEIGHT, state.commits.len(), |ui, range| {
        state.visible = range.clone();
        for index in range {
            let commit = &state.commits[index];
            let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
            let selected = state.selected == Some(commit.id);
            if selected {
                ui.painter().rect_filled(rect, CornerRadius::ZERO, selection_fill);
            } else if response.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::ZERO, ui.visuals().widgets.hovered.weak_bg_fill);
            }
            if let Some(row) = state.graph.get(index) {
                paint_graph(ui, rect, row, head == Some(commit.id), commit.is_merge());
            }
            let text_rect = Rect::from_min_max(pos2(rect.left() + graph_width, rect.top()), rect.max);
            let dimmed = state.searching() && !state.is_match(index);
            commit_row(ui, text_rect, commit, refs, selected, dimmed, actions);

            if response.clicked() {
                actions.push(Action::SelectCommit(commit.id));
            }
            if response.double_clicked()
                && let Some(checkout) = menus::checkout_commit(commit, refs)
            {
                actions.push(checkout);
            }
            response.context_menu(|ui| menus::commit(ui, commit, refs, actions));
        }
    });

    // Load the next page once the end of the list scrolls into view.
    if !state.loading && state.commits.len() >= state.limit && output.inner_rect.height() > 0.0 && state.visible.end + 50 >= state.commits.len() {
        state.limit += PAGE;
        actions.push(Action::ReloadHistory);
    }
    keyboard_navigation(ui, state, actions);
}

fn keyboard_navigation(ui: &Ui, state: &HistoryState, actions: &mut Vec<Action>) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let step = ui.input(|i| i.key_pressed(egui::Key::ArrowDown) as isize - i.key_pressed(egui::Key::ArrowUp) as isize);
    if step == 0 {
        return;
    }
    let current = state.selected.and_then(|id| state.index_of(id)).unwrap_or(0);
    let next = current.saturating_add_signed(step).min(state.commits.len().saturating_sub(1));
    actions.push(Action::SelectCommit(state.commits[next].id));
}

/// Summary with ref badges on the left; author, date and hash in fixed columns on the right.
/// `dimmed` greys out commits that don't match an active search.
fn commit_row(
    ui: &mut Ui,
    rect: Rect,
    commit: &CommitInfo,
    refs: &furkst_core::RefsSnapshot,
    selected: bool,
    dimmed: bool,
    actions: &mut Vec<Action>,
) {
    const AUTHOR: f32 = 150.0;
    const DATE: f32 = 130.0;
    const HASH: f32 = 70.0;
    let columns = AUTHOR + DATE + HASH;
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    let (strong, weak) = if selected {
        (visuals.selection.stroke.color, visuals.selection.stroke.color)
    } else if dimmed {
        let faint = visuals.weak_text_color().gamma_multiply(0.6);
        (faint, faint)
    } else {
        (visuals.strong_text_color(), visuals.weak_text_color())
    };
    let font = FontId::proportional(13.0);
    let y = rect.center().y;

    let message_rect = Rect::from_min_max(rect.min, pos2((rect.right() - columns).max(rect.left()), rect.bottom()));
    // A stable id per commit keeps badge popups open across frames.
    let builder = egui::UiBuilder::new().id_salt(("commit-row", commit.id)).max_rect(message_rect);
    let mut child = ui.new_child(builder.layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.set_clip_rect(message_rect.intersect(ui.clip_rect()));
    child.spacing_mut().item_spacing.x = 4.0;
    menus::ref_badges(&mut child, commit.id, refs, actions);
    child.add(egui::Label::new(RichText::new(&commit.summary).color(strong)).truncate().selectable(false));

    let mut x = rect.right() - columns;
    // With avatars on, the author's picture leads the author column.
    let author_indent = match crate::avatars::get(ui.ctx(), &commit.author, &commit.committer) {
        Some(picture) => {
            const SIDE: f32 = 16.0;
            widgets::round_picture(ui, &picture, Rect::from_center_size(pos2(x + SIDE / 2.0, y), Vec2::splat(SIDE)));
            SIDE + 5.0
        }
        None => 0.0,
    };
    for (text, width, indent, color) in [
        (commit.author.name.clone(), AUTHOR, author_indent, weak),
        (widgets::format_date(&commit.author), DATE, 0.0, weak),
        (commit.short_id(), HASH, 0.0, weak),
    ] {
        let clip = Rect::from_min_max(pos2(x, rect.top()), pos2(x + width - 6.0, rect.bottom()));
        painter.with_clip_rect(clip).text(pos2(x + indent, y), Align2::LEFT_CENTER, text, font.clone(), color);
        x += width;
    }
}

fn lane_x(rect: Rect, column: usize) -> f32 {
    rect.left() + 6.0 + LANE_WIDTH * (column as f32 + 0.5)
}

fn paint_graph(ui: &Ui, rect: Rect, row: &GraphRow, is_head: bool, is_merge: bool) {
    let painter = ui.painter_at(rect);
    let mid = rect.center().y;
    for line in &row.lines {
        let (y0, y1) = match line.half {
            Half::Top => (rect.top(), mid),
            Half::Bottom => (mid, rect.bottom()),
        };
        let (x0, x1) = (lane_x(rect, line.from.into()), lane_x(rect, line.to.into()));
        let stroke = Stroke::new(2.0, theme::lane(line.color as usize));
        if x0 == x1 {
            painter.line_segment([pos2(x0, y0), pos2(x1, y1)], stroke);
        } else {
            // A smooth S-curve between lanes.
            let curve = egui::epaint::CubicBezierShape::from_points_stroke(
                [pos2(x0, y0), pos2(x0, (y0 + y1) / 2.0), pos2(x1, (y0 + y1) / 2.0), pos2(x1, y1)],
                false,
                Color32::TRANSPARENT,
                stroke,
            );
            painter.add(curve);
        }
    }
    let center = pos2(lane_x(rect, row.column), mid);
    let color = theme::lane(row.color);
    let radius = if is_merge { 3.5 } else { 5.0 };
    painter.circle_filled(center, radius, color);
    if is_head {
        painter.circle_stroke(center, radius + 2.5, Stroke::new(1.5, color));
    }
}

#[cfg(test)]
mod tests {
    use furkst_core::test_support::{commit, oid};

    use super::*;

    /// Six loaded commits (6 newest, at walk positions 0 to 5) of which the even ones match, plus `unloaded` matches
    /// further down the history.
    fn state(unloaded: &[(u8, usize)]) -> HistoryState {
        let mut state = HistoryState {
            commits: (1..=6u8).rev().map(|n| commit(n, &[])).collect(),
            search: "fix".into(),
            ..Default::default()
        };
        let loaded = [(6, 0), (4, 2), (2, 4)];
        let hits = loaded.iter().chain(unloaded).map(|&(n, position)| SearchHit { id: oid(n), position }).collect();
        let (generation, _) = state.begin_search();
        assert!(state.finish_search(generation, hits));
        state
    }

    fn found(state: &HistoryState, forward: bool, restart: bool) -> Option<Oid> {
        match state.find(forward, restart) {
            Found::Commit(id) => Some(id),
            _ => None,
        }
    }

    #[test]
    fn steps_through_matches_and_wraps_around() {
        let mut state = state(&[]);
        assert_eq!(found(&state, true, true), Some(oid(6)), "typing jumps to the first match");
        state.selected = Some(oid(6));
        assert_eq!(found(&state, true, false), Some(oid(4)));
        state.selected = Some(oid(2));
        assert_eq!(found(&state, true, false), Some(oid(6)), "wraps after the last match");
        assert_eq!(found(&state, false, false), Some(oid(4)));
        state.selected = Some(oid(5));
        assert_eq!(found(&state, false, false), Some(oid(6)), "previous from a non-match");
        assert_eq!(state.match_counter(), "3 matches");
        state.selected = Some(oid(4));
        assert_eq!(state.match_counter(), "2 of 3");
    }

    #[test]
    fn loads_history_up_to_the_next_match_instead_of_wrapping() {
        let mut state = state(&[(9, 4_500), (8, 7_000)]);
        state.selected = Some(oid(2));
        assert!(matches!(state.find(true, false), Found::NeedsHistoryUntil(4_500)));
        assert_eq!(state.match_counter(), "3 of 5", "counts matches beyond the loaded history");
        state.load_until(4_500);
        assert_eq!(state.limit, 3 * PAGE, "loads whole pages, in one go");
    }

    #[test]
    fn waits_for_a_running_search_and_ignores_superseded_ones() {
        let mut state = state(&[]);
        let (old, old_cancel) = state.begin_search();
        let (new, _) = state.begin_search();
        assert!(old_cancel.load(Ordering::Relaxed), "a newer search stops the older one");
        assert!(matches!(state.find(true, true), Found::Searching));
        assert_eq!(state.match_counter(), "Searching…");
        assert!(!state.finish_search(old, vec![SearchHit { id: oid(5), position: 1 }]));
        assert!(state.finish_search(new, vec![SearchHit { id: oid(5), position: 1 }]));
        assert_eq!(found(&state, true, true), Some(oid(5)));
    }
}
