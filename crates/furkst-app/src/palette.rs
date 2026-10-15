//! A Quick Launch style command palette: a text field over a fuzzy-filtered, keyboard-navigable list.
//!
//! Generic over the payload each item carries; the owner decides what running an item means.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Key, Modifiers, RichText, TextStyle};

/// Only this many of the best matches are rendered, so huge item lists stay fast.
const MAX_SHOWN: usize = 50;

pub struct Item<T> {
    pub label: String,
    /// Dimmed text after the label, e.g. a shortcut or a path.
    pub detail: Option<String>,
    pub payload: T,
}

impl<T> Item<T> {
    pub fn new(label: impl Into<String>, payload: T) -> Self {
        Self { label: label.into(), detail: None, payload }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

/// "Open <repo>" item for a recent repository, with its full path as detail.
pub fn open_repo_item<T>(path: &std::path::Path, payload: T) -> Item<T> {
    let name = crate::widgets::path_name(path);
    Item::new(format!("Open {name}"), payload).detail(path.display().to_string())
}

/// What the user did with the palette this frame.
pub enum Event<T> {
    Run(T),
    Close,
}

struct Matched {
    item: usize,
    /// Char indices of the label that matched the query.
    positions: Vec<usize>,
}

pub struct Palette<T> {
    id: egui::Id,
    query: String,
    items: Vec<Item<T>>,
    shown: Vec<Matched>,
    /// The query `shown` was computed for, `None` when the items changed since.
    filtered_for: Option<String>,
    selected: usize,
    scroll_to_selected: bool,
    hint: &'static str,
}

impl<T> Palette<T> {
    pub fn new(id: impl std::hash::Hash + std::fmt::Debug, hint: &'static str, items: Vec<Item<T>>) -> Self {
        Self {
            id: egui::Id::new(id),
            query: String::new(),
            items,
            shown: Vec::new(),
            filtered_for: None,
            selected: 0,
            scroll_to_selected: false,
            hint,
        }
    }

    /// Adds items that arrived after the palette opened.
    pub fn extend(&mut self, items: impl IntoIterator<Item = Item<T>>) {
        self.items.extend(items);
        self.filtered_for = None;
    }

    fn refilter(&mut self) {
        if self.filtered_for.as_deref() == Some(&self.query) {
            return;
        }
        let mut scored: Vec<(i32, Matched)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(item, i)| {
                let (score, positions) = fuzzy_match(&self.query, &i.label)?;
                Some((score, Matched { item, positions }))
            })
            .collect();
        // Stable, so equally good matches keep the order the owner listed them in.
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored.truncate(MAX_SHOWN);
        self.shown = scored.into_iter().map(|(_, m)| m).collect();
        self.filtered_for = Some(self.query.clone());
        self.selected = 0;
        self.scroll_to_selected = true;
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<Event<T>> {
        self.refilter();
        let mut run = None;
        let (up, down, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        let last = self.shown.len().saturating_sub(1);
        if up {
            self.selected = self.selected.saturating_sub(1);
            self.scroll_to_selected = true;
        }
        if down {
            self.selected = (self.selected + 1).min(last);
            self.scroll_to_selected = true;
        }
        if enter && self.selected < self.shown.len() {
            run = Some(self.selected);
        }

        let response = egui::Modal::new(self.id).show(ctx, |ui| {
            ui.set_width(560.0);
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text(self.hint)
                    .font(TextStyle::Heading)
                    .desired_width(f32::INFINITY),
            );
            edit.request_focus();
            ui.add_space(4.0);
            if self.shown.is_empty() {
                ui.label(RichText::new("No matches").weak());
            }
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                for (row, matched) in self.shown.iter().enumerate() {
                    let item = &self.items[matched.item];
                    let selected = row == self.selected;
                    let job = highlighted(ui, item, &matched.positions);
                    let response = ui.add(egui::Button::selectable(selected, job).min_size(egui::vec2(ui.available_width(), 0.0)));
                    if selected && self.scroll_to_selected {
                        response.scroll_to_me(None);
                    }
                    if response.clicked() {
                        run = Some(row);
                    }
                }
            });
        });
        self.scroll_to_selected = false;

        match run {
            Some(row) => {
                let item = self.items.swap_remove(self.shown[row].item);
                Some(Event::Run(item.payload))
            }
            None if response.should_close() => Some(Event::Close),
            None => None,
        }
    }
}

/// The item's label with matched characters emphasized, followed by its dimmed detail.
fn highlighted<T>(ui: &egui::Ui, item: &Item<T>, positions: &[usize]) -> LayoutJob {
    let font = FontId::proportional(14.0);
    let plain = TextFormat { font_id: font.clone(), color: ui.visuals().text_color(), ..Default::default() };
    let accent = TextFormat {
        color: ui.visuals().hyperlink_color,
        underline: egui::Stroke::new(1.0, ui.visuals().hyperlink_color),
        ..plain.clone()
    };
    let mut job = LayoutJob::default();
    for (index, c) in item.label.chars().enumerate() {
        let format = if positions.binary_search(&index).is_ok() { &accent } else { &plain };
        job.append(c.encode_utf8(&mut [0; 4]), 0.0, format.clone());
    }
    if let Some(detail) = &item.detail {
        let weak = TextFormat { font_id: FontId::proportional(12.0), color: Color32::GRAY, ..plain };
        job.append(&format!("   {detail}"), 0.0, weak);
    }
    job
}

/// Case-insensitive subsequence match of `query` in `label`.
///
/// Returns a score (higher is better) and the matched char positions. Consecutive matches and matches at
/// word starts score higher, and shorter labels win ties. An empty query matches everything with score 0.
pub fn fuzzy_match(query: &str, label: &str) -> Option<(i32, Vec<usize>)> {
    let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let chars: Vec<char> = label.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let (n, m) = (chars.len(), query.len());

    // Cheap rejection before the quadratic scoring.
    let mut wanted = query.iter();
    let mut next = wanted.next();
    for c in &lower {
        if Some(c) == next {
            next = wanted.next();
        }
    }
    next.is_none().then_some(())?;

    let word_start = |i: usize| {
        i == 0
            || !chars[i - 1].is_alphanumeric()
            || (chars[i - 1].is_lowercase() && chars[i].is_uppercase())
    };
    // best[j][i]: best score with query[j] matched at label position i; parent[j][i]: where query[j - 1] matched.
    let mut best = vec![vec![i32::MIN; n]; m];
    let mut parent = vec![vec![usize::MAX; n]; m];
    for j in 0..m {
        // Best of best[j - 1][k] for k < i - 1, tracked with its position.
        let mut earlier: Option<(i32, usize)> = None;
        for i in j..n {
            if j > 0 && i >= 2 && best[j - 1][i - 2] != i32::MIN && earlier.is_none_or(|(s, _)| best[j - 1][i - 2] > s) {
                earlier = Some((best[j - 1][i - 2], i - 2));
            }
            if lower[i] != query[j] {
                continue;
            }
            let bonus = if word_start(i) { 8 } else { 0 };
            if j == 0 {
                // Matching late in the label is slightly worse.
                best[0][i] = 1 + bonus - (i as i32).min(8) / 2;
                continue;
            }
            let adjacent = (i > 0 && best[j - 1][i - 1] != i32::MIN).then(|| (best[j - 1][i - 1] + 6, i - 1));
            let gapped = earlier.map(|(s, k)| (s - 1, k));
            if let Some((score, from)) = [gapped, adjacent].into_iter().flatten().max_by_key(|(s, _)| *s) {
                best[j][i] = score + 1 + bonus;
                parent[j][i] = from;
            }
        }
    }
    let (mut at, score) = (0..n).rev().filter(|&i| best[m - 1][i] != i32::MIN).map(|i| (i, best[m - 1][i])).max_by_key(|(_, s)| *s)?;
    let mut positions = vec![0; m];
    for j in (0..m).rev() {
        positions[j] = at;
        at = parent[j][at];
    }
    Some((score * 4 - n as i32 / 4, positions))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(query: &str, label: &str) -> i32 {
        fuzzy_match(query, label).unwrap_or_else(|| panic!("{query} should match {label}")).0
    }

    #[test]
    fn requires_a_subsequence_ignoring_case() {
        assert!(fuzzy_match("PUL", "Pull…").is_some());
        assert!(fuzzy_match("lup", "Pull…").is_none());
        assert!(fuzzy_match("xyz", "Fetch").is_none());
        assert_eq!(fuzzy_match("", "anything"), Some((0, vec![])));
    }

    #[test]
    fn reports_matched_positions() {
        assert_eq!(fuzzy_match("pl", "Pull").unwrap().1, [0, 2]);
        // Prefers the word start over an earlier scattered match.
        assert_eq!(fuzzy_match("m", "some main").unwrap().1, [5]);
    }

    #[test]
    fn prefers_consecutive_matches() {
        assert!(score("fet", "Fetch") > score("fet", "Foo extra stuff"));
        assert!(score("pull", "Pull…") > score("pull", "Push + upload all"));
    }

    #[test]
    fn prefers_word_starts() {
        assert!(score("cb", "Checkout branch") > score("cb", "Scribble"));
        assert!(score("fh", "File history") > score("fh", "Refresh"));
    }

    #[test]
    fn prefers_shorter_labels() {
        assert!(score("main", "main") > score("main", "maintenance-branch"));
    }

    #[test]
    fn handles_multibyte_labels() {
        assert_eq!(fuzzy_match("é", "Ée").unwrap().1, [0]);
        assert!(fuzzy_match("ab", "a…b").is_some());
    }
}
