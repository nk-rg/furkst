//! The diff pane shared by the history and local-changes views.

use std::collections::HashSet;

use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Ui, Vec2, pos2};
use furkst_core::{ChangeKind, DiffLine, DiffTarget, FileChange, FileDiff, Hunk, HunkAction, LineKind, LineSelection, Operation};

use super::highlight::{self, Spans};
use super::image_view::ImagePair;
use super::{Action, Loaded};
use crate::settings::Settings;
use crate::theme::DiffColors;
use crate::widgets;

const GUTTER_CHARS: usize = 5;

#[derive(Default)]
pub struct DiffPanel {
    generation: u64,
    key: Option<(DiffTarget, FileChange)>,
    state: State,
    selection: HashSet<(usize, usize)>,
    anchor: Option<(usize, usize)>,
    /// Where the diff is scrolled to: both side-by-side panes follow it, and stepping between changes sets it.
    scroll: Vec2,
}

#[derive(Default)]
enum State {
    #[default]
    Empty,
    Loading,
    Ready(Box<Prepared>),
    Failed(String),
}

/// A diff flattened into rows for virtualized rendering.
struct Prepared {
    diff: FileDiff,
    unified: Vec<Row>,
    split: Vec<SplitRow>,
    longest_line: usize,
    /// Decoded images of an image diff; `None` if they could not be decoded.
    images: Option<ImagePair>,
    /// The rows where each run of changed lines starts, in the unified and the side-by-side layout.
    change_starts: (Vec<usize>, Vec<usize>),
    /// Syntax colors per hunk and line, worked out on first display, for a dark (true) or light theme; `None` inside
    /// when the language isn't known.
    highlights: Option<(bool, Option<Vec<Vec<Spans>>>)>,
}

#[derive(Clone, Copy)]
enum Row {
    Header(usize),
    Line(usize, usize),
}

#[derive(Clone, Copy)]
enum SplitRow {
    Header(usize),
    Pair { hunk: usize, left: Option<usize>, right: Option<usize> },
}

impl Prepared {
    fn new(diff: FileDiff) -> Self {
        let mut unified = Vec::new();
        let mut split = Vec::new();
        let mut longest_line = 0;
        for (h, hunk) in diff.hunks().iter().enumerate() {
            unified.push(Row::Header(h));
            split.push(SplitRow::Header(h));
            unified.extend((0..hunk.lines.len()).map(|l| Row::Line(h, l)));
            longest_line = hunk.lines.iter().map(|l| l.text.len()).max().unwrap_or(0).max(longest_line);
            split.extend(pair_lines(h, hunk));
        }
        let images = match &diff {
            FileDiff::Image { old, new } => ImagePair::decode(old.as_deref(), new.as_deref()),
            _ => None,
        };
        let hunks = diff.hunks();
        let changed = |h: usize, l: Option<usize>| l.is_some_and(|l| hunks[h].lines[l].kind != LineKind::Context);
        let unified_starts = change_starts(unified.iter().map(|row| match *row {
            Row::Line(h, l) => changed(h, Some(l)),
            Row::Header(_) => false,
        }));
        let split_starts = change_starts(split.iter().map(|row| match *row {
            SplitRow::Pair { hunk, left, right } => changed(hunk, left) || changed(hunk, right),
            SplitRow::Header(_) => false,
        }));
        let change_starts = (unified_starts, split_starts);
        Self { diff, unified, split, longest_line, images, change_starts, highlights: None }
    }
}

/// Lines a removed block up with the added block that replaces it, for side-by-side display.
fn pair_lines(h: usize, hunk: &Hunk) -> Vec<SplitRow> {
    let mut rows = Vec::new();
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    let flush = |removed: &mut Vec<usize>, added: &mut Vec<usize>, rows: &mut Vec<SplitRow>| {
        for i in 0..removed.len().max(added.len()) {
            rows.push(SplitRow::Pair { hunk: h, left: removed.get(i).copied(), right: added.get(i).copied() });
        }
        removed.clear();
        added.clear();
    };
    for (l, line) in hunk.lines.iter().enumerate() {
        match line.kind {
            LineKind::Removed => removed.push(l),
            LineKind::Added => added.push(l),
            LineKind::Context => {
                flush(&mut removed, &mut added, &mut rows);
                rows.push(SplitRow::Pair { hunk: h, left: Some(l), right: Some(l) });
            }
        }
    }
    flush(&mut removed, &mut added, &mut rows);
    rows
}

impl DiffPanel {
    /// Starts loading a new diff and returns the generation the result must carry.
    pub fn start(&mut self, target: DiffTarget, change: FileChange) -> u64 {
        let key = Some((target, change));
        if self.key != key {
            self.state = State::Loading;
            self.key = key;
            self.scroll = Vec2::ZERO;
        }
        self.selection.clear();
        self.anchor = None;
        self.generation += 1;
        self.generation
    }

    pub fn finish(&mut self, generation: u64, result: Loaded<FileDiff>) {
        if generation == self.generation {
            self.state = match result {
                Ok(diff) => State::Ready(Box::new(Prepared::new(diff))),
                Err(err) => State::Failed(err),
            };
        }
    }

    pub fn clear(&mut self) {
        self.key = None;
        self.state = State::Empty;
        self.selection.clear();
        self.generation += 1;
    }

    pub fn key(&self) -> Option<&(DiffTarget, FileChange)> {
        self.key.as_ref()
    }

    pub fn is_showing(&self, target: &DiffTarget, path: &str) -> bool {
        self.key.as_ref().is_some_and(|(t, c)| t == target && c.path == path)
    }

    /// What can be done with the hunks and lines of the diff shown: staging or discarding a working-copy change,
    /// unstaging a staged one, nothing for a commit's (nor an untracked file's, which is staged whole).
    pub fn hunk_actions(&self) -> &'static [HunkAction] {
        match &self.key {
            Some((DiffTarget::Unstaged, change)) if change.kind != ChangeKind::Untracked => {
                &[HunkAction::Stage, HunkAction::Discard]
            }
            Some((DiffTarget::Staged, _)) => &[HunkAction::Unstage],
            _ => &[],
        }
    }

    /// Draws the pane. `reload` is the action that loads this pane's diff again, after the diff options change. With
    /// `full_view` (whether this is the Changes window), the header has a button that opens or closes that window.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        settings: &mut Settings,
        reload: fn(DiffTarget, FileChange) -> Action,
        full_view: Option<bool>,
        actions: &mut Vec<Action>,
    ) {
        let Some((target, change)) = self.key.clone() else {
            widgets::placeholder(ui, "Select a file to see its changes");
            return;
        };
        let clicks = header(ui, &change, settings, full_view);
        if clicks.reload {
            actions.push(reload(target.clone(), change.clone()));
        }
        if clicks.toggle_full_view {
            actions.push(Action::ToggleChangesWindow);
        }
        ui.separator();
        self.body(ui, settings, clicks.step, false, actions);
    }

    /// The diff drawn in place, without a header or scrolling of its own: the Commit tab shows one under each file
    /// opened there.
    pub fn show_inline(&mut self, ui: &mut Ui, settings: &Settings, actions: &mut Vec<Action>) {
        self.body(ui, settings, None, true, actions);
    }

    /// Everything under the header: the lines, or a note standing for content that has none, scrolled to the change
    /// `step` asks for. `inline` draws every row in place instead of in a scroll area.
    fn body(&mut self, ui: &mut Ui, settings: &Settings, step: Option<bool>, inline: bool, actions: &mut Vec<Action>) {
        let allowed = self.hunk_actions();
        let Some((_, change)) = self.key.clone() else { return };
        // Centered in the pane, or a line of text when inline.
        let note = |ui: &mut Ui, text: &str| {
            if inline {
                ui.label(RichText::new(text).weak());
            } else {
                widgets::placeholder(ui, text);
            }
        };
        let prepared = match &mut self.state {
            State::Empty => return,
            State::Loading => return note(ui, "Loading…"),
            State::Failed(err) => return note(ui, err),
            State::Ready(prepared) => prepared,
        };
        // "<what>: 1.2 MB before, 1.3 MB after" for content that can't be shown line by line.
        let sizes = |ui: &mut Ui, what: &str, old_size: Option<usize>, new_size: Option<usize>| {
            let size = |s: Option<usize>| s.map_or("—".to_owned(), widgets::format_size);
            note(ui, &format!("{what}: {} before, {} after", size(old_size), size(new_size)));
        };
        let binary = |ui: &mut Ui, old_size, new_size| sizes(ui, "Binary file", old_size, new_size);
        match &prepared.diff {
            FileDiff::Binary { old_size, new_size } => return binary(ui, *old_size, *new_size),
            FileDiff::Image { old, new } => {
                return match &mut prepared.images {
                    Some(images) if inline => images.show(ui),
                    Some(images) => egui::ScrollArea::vertical().show(ui, |ui| images.show(ui)).inner,
                    None => binary(ui, old.as_ref().map(Vec::len), new.as_ref().map(Vec::len)),
                };
            }
            FileDiff::Lfs { old, new } => {
                let size = |p: &Option<furkst_core::LfsPointer>| p.as_ref().map(|p| p.size as usize);
                return sizes(ui, "Git LFS object not downloaded yet (use LFS pull)", size(old), size(new));
            }
            FileDiff::TooLarge => return note(ui, "File too large to display"),
            FileDiff::Text { hunks } if hunks.is_empty() => return note(ui, "No changes"),
            FileDiff::Text { .. } => {}
        }

        let options = settings.diff_options();
        // Staging, unstaging or discarding lines; discarding asks first.
        let staging = |action: HunkAction, selection: LineSelection| {
            let op = Operation::ApplySelection { action, path: change.path.clone(), selection, options };
            match action {
                HunkAction::Discard => Action::Confirm {
                    title: "Discard changes".into(),
                    message: format!("Discard the selected changes in {}? This cannot be undone.", change.path),
                    op,
                },
                _ => Action::Run(op),
            }
        };

        if !self.selection.is_empty() && !allowed.is_empty() {
            ui.horizontal(|ui| {
                ui.label(format!("{} lines selected", self.selection.len()));
                for &action in allowed {
                    if ui.button(format!("{} lines", verb(action))).clicked() {
                        actions.push(staging(action, LineSelection(std::mem::take(&mut self.selection))));
                    }
                }
                if ui.button("Clear selection").clicked() {
                    self.selection.clear();
                }
            });
            ui.separator();
        }

        let font = FontId::monospace(settings.diff_font_size);
        let char_width = ui.ctx().fonts_mut(|f| f.glyph_width(&font, 'M'));
        let row_height = ui.ctx().fonts_mut(|f| f.row_height(&font)) + 3.0;
        if let Some(forward) = step {
            let starts = if settings.side_by_side { &prepared.change_starts.1 } else { &prepared.change_starts.0 };
            if let Some(y) = step_to_change(starts, self.scroll.y, row_height, forward) {
                self.scroll.y = y;
            }
        }
        let painter_ctx = RowPainter {
            font,
            char_width,
            colors: DiffColors::of(ui.visuals()),
            // Bright, like code in an editor; numbers and signs stay dim.
            text: ui.visuals().strong_text_color(),
            weak: ui.visuals().weak_text_color(),
        };
        let dark = ui.visuals().dark_mode;
        if prepared.highlights.as_ref().is_none_or(|(for_dark, _)| *for_dark != dark) {
            prepared.highlights = Some((dark, highlight::highlight(&change.path, prepared.diff.hunks(), dark)));
        }
        let colored = prepared.highlights.as_ref().and_then(|(_, colored)| colored.as_ref());
        let spans = |h: usize, l: usize| colored.and_then(|c| c.get(h)?.get(l));
        ui.spacing_mut().item_spacing.y = 0.0;
        let selectable = !allowed.is_empty();
        let mut clicked: Option<((usize, usize), bool)> = None;
        let mut hunk_action: Option<(HunkAction, usize)> = None;
        let hunks = prepared.diff.hunks();

        // Wide enough for the longest line and its number columns, scrolling sideways when the pane is narrower.
        let content_width = |number_columns: usize| (prepared.longest_line + GUTTER_CHARS * number_columns + 4) as f32 * char_width;
        if settings.side_by_side && !inline {
            // Two panes of half the width each, old and new, each scrolling on its own scroll bars but
            // always together, so long lines can be read on both sides.
            let content_width = content_width(1);
            let area = ui.available_rect_before_wrap();
            let middle = area.center().x.floor();
            let halves = [(true, area.with_max_x(middle)), (false, area.with_min_x(middle))];
            let mut offset = self.scroll;
            for (old_side, half) in halves {
                let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(half));
                let output = egui::ScrollArea::both()
                    .id_salt(("split", old_side))
                    .auto_shrink(false)
                    .scroll_offset(self.scroll)
                    .show_rows(ui, row_height, prepared.split.len(), |ui, range| {
                        let width = ui.available_width().max(content_width);
                        for row in &prepared.split[range] {
                            let rect = ui.allocate_exact_size(Vec2::new(width, row_height), Sense::hover()).0;
                            match *row {
                                // The hunk's buttons go on the new side only.
                                SplitRow::Header(h) => {
                                    let actions = if old_side { &[][..] } else { allowed };
                                    hunk_action =
                                        hunk_action.or(hunk_header(ui, rect, &hunks[h], h, actions, &painter_ctx));
                                }
                                SplitRow::Pair { hunk, left, right } => {
                                    let Some(l) = (if old_side { left } else { right }) else {
                                        ui.painter().rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
                                        continue;
                                    };
                                    let line = &hunks[hunk].lines[l];
                                    let selected = self.selection.contains(&(hunk, l));
                                    let row = ui.interact(rect, ui.id().with((hunk, l, old_side)), Sense::click());
                                    let response = row.union(painter_ctx.line(ui, rect, line, spans(hunk, l), selected, Some(old_side)));
                                    if selectable && response.clicked() && line.kind != LineKind::Context {
                                        clicked = Some(((hunk, l), ui.input(|i| i.modifiers.shift)));
                                    }
                                    response.context_menu(|ui| line_menu(ui, &change.path, line, actions));
                                }
                            }
                        }
                    });
                // Whichever pane was scrolled leads; the other follows on the next frame.
                if output.state.offset != self.scroll {
                    offset = output.state.offset;
                }
            }
            ui.allocate_rect(area, Sense::hover());
            if offset != self.scroll {
                self.scroll = offset;
                ui.ctx().request_repaint();
            }
        } else {
            let content_width = content_width(2);
            let mut unified_row = |ui: &mut Ui, row: &Row, width: f32| {
                let (rect, response) = ui.allocate_exact_size(Vec2::new(width, row_height), Sense::click());
                match *row {
                    Row::Header(h) => {
                        hunk_action = hunk_action.or(hunk_header(ui, rect, &hunks[h], h, allowed, &painter_ctx));
                    }
                    Row::Line(h, l) => {
                        let line = &hunks[h].lines[l];
                        let selected = self.selection.contains(&(h, l));
                        let response = response.union(painter_ctx.line(ui, rect, line, spans(h, l), selected, None));
                        if selectable && response.clicked() && line.kind != LineKind::Context {
                            clicked = Some(((h, l), ui.input(|i| i.modifiers.shift)));
                        }
                        response.context_menu(|ui| line_menu(ui, &change.path, line, actions));
                    }
                }
            };
            if inline {
                // Every row, in place: the page around it scrolls.
                let width = ui.available_width().max(content_width);
                prepared.unified.iter().for_each(|row| unified_row(ui, row, width));
            } else {
                let area = egui::ScrollArea::both().auto_shrink(false).scroll_offset(self.scroll);
                let output = area.show_rows(ui, row_height, prepared.unified.len(), |ui, range| {
                    let width = ui.available_width().max(content_width);
                    prepared.unified[range].iter().for_each(|row| unified_row(ui, row, width));
                });
                self.scroll = output.state.offset;
            }
        }

        if let Some((action, h)) = hunk_action {
            actions.push(staging(action, LineSelection::whole_hunks(hunks, [h])));
        }
        if let Some((line, extend)) = clicked {
            click_line(&mut self.selection, &mut self.anchor, hunks, line, extend);
        }
    }

}

/// File name and display toggles. Returns true when the diff needs to be recomputed.
/// What was clicked in the diff header.
#[derive(Default)]
struct HeaderClicks {
    /// A diff option changed, so the diff must be loaded again.
    reload: bool,
    /// The button opening or closing the Changes window (shown when there is a `full_view` to switch).
    toggle_full_view: bool,
    /// Go to the next (true) or previous change.
    step: Option<bool>,
}

/// The arrows to step between changes, the file, and the diff options.
fn header(ui: &mut Ui, change: &FileChange, settings: &mut Settings, full_view: Option<bool>) -> HeaderClicks {
    let mut clicks = HeaderClicks::default();
    ui.horizontal(|ui| {
        // The controls first, from the right, so a long path gets the room left over instead of running under them.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(on) = full_view {
                let tip = if on { "Close this window (Ctrl+Shift+M or Esc)" } else { "Open in the Changes window (Ctrl+Shift+M)" };
                clicks.toggle_full_view = ui.small_button(if on { "🗗" } else { "⛶" }).on_hover_text(tip).clicked();
            }
            ui.toggle_value(&mut settings.side_by_side, "Side by side");
            clicks.reload |= ui.toggle_value(&mut settings.ignore_whitespace, "Ignore whitespace").changed();
            if ui.small_button("+").on_hover_text("More context").clicked() {
                settings.diff_context = (settings.diff_context + 1).min(50);
                clicks.reload = true;
            }
            ui.label(format!("{} lines", settings.diff_context));
            if ui.small_button("−").on_hover_text("Less context").clicked() {
                settings.diff_context = settings.diff_context.saturating_sub(1);
                clicks.reload = true;
            }
            let name_color = ui.visuals().strong_text_color();
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if ui.small_button("⬆").on_hover_text("Previous change").clicked() {
                    clicks.step = Some(false);
                }
                if ui.small_button("⬇").on_hover_text("Next change").clicked() {
                    clicks.step = Some(true);
                }
                widgets::file_label(ui, change, name_color, true);
            })
            .response
            .on_hover_text(&change.path);
        });
    });
    clicks
}

/// Toggles a line, or with `extend` selects every changed line between the anchor and it.
fn click_line(
    selection: &mut HashSet<(usize, usize)>,
    anchor: &mut Option<(usize, usize)>,
    hunks: &[Hunk],
    (h, l): (usize, usize),
    extend: bool,
) {
    match *anchor {
        Some((ah, al)) if extend && ah == h => {
            let (from, to) = (al.min(l), al.max(l));
            selection.extend((from..=to).filter(|&i| hunks[h].lines[i].kind != LineKind::Context).map(|i| (h, i)));
        }
        _ => {
            if !selection.remove(&(h, l)) {
                selection.insert((h, l));
            }
            *anchor = Some((h, l));
        }
    }
}

fn verb(action: HunkAction) -> &'static str {
    match action {
        HunkAction::Stage => "Stage",
        HunkAction::Unstage => "Unstage",
        HunkAction::Discard => "Discard",
    }
}

/// Draws a hunk header row with its action buttons; returns the clicked action.
fn hunk_header(
    ui: &mut Ui,
    rect: Rect,
    hunk: &Hunk,
    h: usize,
    allowed: &[HunkAction],
    painter: &RowPainter,
) -> Option<(HunkAction, usize)> {
    ui.painter().rect_filled(rect, 0.0, painter.colors.hunk);
    ui.painter().text(
        rect.left_center() + Vec2::new(8.0, 0.0),
        Align2::LEFT_CENTER,
        hunk.header(),
        painter.font.clone(),
        painter.weak,
    );
    let visible = ui.clip_rect().intersect(rect);
    let mut child = ui.new_child(
        egui::UiBuilder::new().max_rect(visible.shrink2(Vec2::new(6.0, 1.0))).layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    allowed
        .iter()
        .rev()
        .find(|&&action| child.small_button(format!("{} hunk", verb(action))).clicked())
        .map(|&action| (action, h))
}

struct RowPainter {
    font: FontId,
    char_width: f32,
    colors: DiffColors,
    text: Color32,
    weak: Color32,
}

impl RowPainter {
    /// Paints one diff line, in its syntax colors when there are `spans`; `side` is `Some(true)` for the old half of a
    /// side-by-side row. The code is selectable text (drag across lines, then Ctrl+C); returns its response.
    fn line(
        &self,
        ui: &mut Ui,
        rect: Rect,
        line: &DiffLine,
        spans: Option<&Spans>,
        selected: bool,
        side: Option<bool>,
    ) -> egui::Response {
        let painter = ui.painter_at(rect);
        let background = match (selected, line.kind) {
            (true, _) => self.colors.selected,
            (false, LineKind::Added) => self.colors.added,
            (false, LineKind::Removed) => self.colors.removed,
            (false, LineKind::Context) => Color32::TRANSPARENT,
        };
        painter.rect_filled(rect, 0.0, background);

        let gutter = GUTTER_CHARS as f32 * self.char_width;
        let number = |n: Option<usize>| n.map(|n| n.to_string()).unwrap_or_default();
        let mut x = rect.left() + 4.0;
        let numbers: Vec<Option<usize>> = match side {
            None => vec![line.old_no, line.new_no],
            Some(true) => vec![line.old_no],
            Some(false) => vec![line.new_no],
        };
        for n in numbers {
            painter.text(pos2(x + gutter, rect.center().y), Align2::RIGHT_CENTER, number(n), self.font.clone(), self.weak);
            x += gutter + self.char_width;
        }
        let sign = match line.kind {
            LineKind::Added => "+",
            LineKind::Removed => "-",
            LineKind::Context => " ",
        };
        painter.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, sign, self.font.clone(), self.weak);
        x += self.char_width * 2.0;
        let text = highlight::display_text(&line.text);
        let mut job = egui::text::LayoutJob::default();
        highlight::append(&mut job, &text, spans, &self.font, self.text);
        let code = Rect::from_min_max(pos2(x, rect.top()), rect.max);
        let layout = egui::Layout::left_to_right(egui::Align::Center);
        ui.scope_builder(egui::UiBuilder::new().max_rect(code).layout(layout), |ui| {
            ui.add(egui::Label::new(job).selectable(true).extend())
        })
        .inner
    }
}

/// A diff line's right-click menu: copy it, or open the file at that line in an installed editor.
fn line_menu(ui: &mut Ui, path: &str, line: &DiffLine, actions: &mut Vec<Action>) {
    if ui.button("Copy line").clicked() {
        ui.ctx().copy_text(line.text.clone());
        ui.close();
    }
    // A removed line has no number in the new version; its old number is the nearest place.
    let number = line.new_no.or(line.old_no).unwrap_or(1);
    ui.menu_button("Reveal line in", |ui| {
        let editors = crate::editors::installed();
        if editors.is_empty() {
            ui.label(RichText::new("No code editors found").weak());
        }
        for (editor, found) in editors.iter().enumerate() {
            if ui.button(&found.name).clicked() {
                actions.push(Action::RevealInEditor { editor, path: path.to_owned(), line: number });
                ui.close();
            }
        }
    });
}

/// The indexes where a run of changed rows begins.
fn change_starts(changed: impl Iterator<Item = bool>) -> Vec<usize> {
    let mut before = false;
    changed
        .enumerate()
        .filter_map(|(row, now)| {
            let starts = now && !before;
            before = now;
            starts.then_some(row)
        })
        .collect()
}

/// Rows of context kept above a change stepped to.
const CONTEXT_ROWS: usize = 2;

/// The scroll offset showing the change after (or before) the one at the top of the view, scrolled to `offset_y`, with a
/// little context above it; `None` past the last (or first) change.
fn step_to_change(starts: &[usize], offset_y: f32, row_height: f32, forward: bool) -> Option<f32> {
    let top = (offset_y / row_height).round() as usize + CONTEXT_ROWS;
    let row = if forward { starts.iter().find(|&&row| row > top) } else { starts.iter().rev().find(|&&row| row < top) }?;
    Some(row.saturating_sub(CONTEXT_ROWS) as f32 * row_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_where_runs_of_changes_start() {
        let rows = [false, true, true, false, false, true, false, true];
        assert_eq!(change_starts(rows.into_iter()), [1, 5, 7]);
    }

    #[test]
    fn steps_between_changes_keeping_context_above() {
        let starts = [4, 20, 40];
        let at = |row: usize| row as f32 * 10.0;
        assert_eq!(step_to_change(&starts, 0.0, 10.0, true), Some(at(2)), "the first change, two rows down");
        assert_eq!(step_to_change(&starts, at(2), 10.0, true), Some(at(18)));
        assert_eq!(step_to_change(&starts, at(18), 10.0, false), Some(at(2)));
        assert_eq!(step_to_change(&starts, at(38), 10.0, true), None, "past the last change");
        assert_eq!(step_to_change(&starts, at(2), 10.0, false), None, "before the first change");
    }
}
