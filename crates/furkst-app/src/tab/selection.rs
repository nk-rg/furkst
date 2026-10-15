//! Multi-selection of rows in a file list, keyed by path so it survives status refreshes.

use std::collections::HashSet;

#[derive(Default)]
pub struct Selection {
    paths: HashSet<String>,
    /// Where a shift-click or shift-arrow range starts.
    anchor: Option<String>,
    /// The row keyboard navigation moves from.
    cursor: Option<String>,
}

impl Selection {
    pub fn contains(&self, path: &str) -> bool {
        self.paths.contains(path)
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The only selected path, if exactly one is selected.
    pub fn single(&self) -> Option<&str> {
        if self.paths.len() == 1 { self.paths.iter().next().map(String::as_str) } else { None }
    }

    /// Plain click: selects just `path` and makes it the anchor.
    pub fn select_only(&mut self, path: &str) {
        self.paths.clear();
        self.paths.insert(path.to_owned());
        self.anchor = Some(path.to_owned());
        self.cursor = Some(path.to_owned());
    }

    /// Ctrl/Cmd+click: toggles `path` and makes it the anchor.
    pub fn toggle(&mut self, path: &str) {
        if !self.paths.remove(path) {
            self.paths.insert(path.to_owned());
        }
        self.anchor = Some(path.to_owned());
        self.cursor = Some(path.to_owned());
    }

    /// Shift+click: selects the range between the anchor and `path`, in `order`.
    pub fn extend_to(&mut self, order: &[&str], path: &str) {
        let anchor = self.anchor.clone().unwrap_or_else(|| path.to_owned());
        let (Some(from), Some(to)) = (order.iter().position(|p| *p == anchor), order.iter().position(|p| *p == path))
        else {
            return self.select_only(path);
        };
        self.paths = order[from.min(to)..=from.max(to)].iter().map(|p| (*p).to_owned()).collect();
        self.anchor = Some(anchor);
        self.cursor = Some(path.to_owned());
    }

    pub fn select_all(&mut self, order: &[&str]) {
        self.paths = order.iter().map(|p| (*p).to_owned()).collect();
        self.anchor = order.first().map(|p| (*p).to_owned());
        self.cursor = order.last().map(|p| (*p).to_owned());
    }

    /// Up/Down arrow: moves the cursor by `delta` rows (clamped), extending the selection when `extend`.
    /// Returns the path the cursor landed on.
    pub fn step<'a>(&mut self, order: &[&'a str], delta: isize, extend: bool) -> Option<&'a str> {
        let last = order.len().checked_sub(1)?;
        let current = self.cursor.as_deref().and_then(|c| order.iter().position(|p| *p == c));
        let next = match current {
            Some(i) => i.saturating_add_signed(delta).min(last),
            None if delta < 0 => last,
            None => 0,
        };
        let path = order[next];
        if extend { self.extend_to(order, path) } else { self.select_only(path) }
        Some(path)
    }

    /// Drops paths that are no longer in `order` (after the file list changed).
    pub fn retain(&mut self, order: &[&str]) {
        self.paths.retain(|p| order.contains(&p.as_str()));
        if self.anchor.as_ref().is_some_and(|a| !self.paths.contains(a)) {
            self.anchor = self.paths.iter().next().cloned();
        }
        if self.cursor.as_ref().is_some_and(|c| !self.paths.contains(c)) {
            self.cursor = self.anchor.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDER: [&str; 5] = ["a", "b", "c", "d", "e"];

    fn sorted(s: &Selection) -> Vec<&str> {
        let mut v: Vec<_> = s.paths.iter().map(String::as_str).collect();
        v.sort();
        v
    }

    #[test]
    fn click_selects_one() {
        let mut s = Selection::default();
        s.select_only("b");
        s.select_only("c");
        assert_eq!(sorted(&s), ["c"]);
        assert_eq!(s.single(), Some("c"));
    }

    #[test]
    fn toggle_adds_and_removes() {
        let mut s = Selection::default();
        s.select_only("a");
        s.toggle("c");
        assert_eq!(sorted(&s), ["a", "c"]);
        assert_eq!(s.single(), None);
        s.toggle("a");
        assert_eq!(sorted(&s), ["c"]);
    }

    #[test]
    fn shift_selects_range_from_anchor_either_direction() {
        let mut s = Selection::default();
        s.select_only("b");
        s.extend_to(&ORDER, "d");
        assert_eq!(sorted(&s), ["b", "c", "d"]);
        s.extend_to(&ORDER, "a");
        assert_eq!(sorted(&s), ["a", "b"]);
    }

    #[test]
    fn shift_without_anchor_selects_one() {
        let mut s = Selection::default();
        s.extend_to(&ORDER, "c");
        assert_eq!(sorted(&s), ["c"]);
    }

    #[test]
    fn select_all() {
        let mut s = Selection::default();
        s.select_all(&ORDER);
        assert_eq!(s.len(), 5);
    }

    #[test]
    fn arrows_move_and_clamp() {
        let mut s = Selection::default();
        assert_eq!(s.step(&ORDER, 1, false), Some("a"));
        assert_eq!(s.step(&ORDER, 1, false), Some("b"));
        assert_eq!(s.step(&ORDER, -5, false), Some("a"));
        assert_eq!(sorted(&s), ["a"]);
        let mut s = Selection::default();
        assert_eq!(s.step(&ORDER, -1, false), Some("e"));
        assert_eq!(s.step(&ORDER, 1, false), Some("e"));
        assert_eq!(s.step(&[], 1, false), None);
    }

    #[test]
    fn shift_arrows_extend_and_shrink() {
        let mut s = Selection::default();
        s.select_only("b");
        s.step(&ORDER, 1, true);
        s.step(&ORDER, 1, true);
        assert_eq!(sorted(&s), ["b", "c", "d"]);
        s.step(&ORDER, -1, true);
        assert_eq!(sorted(&s), ["b", "c"]);
    }

    #[test]
    fn retain_drops_vanished_paths() {
        let mut s = Selection::default();
        s.select_only("a");
        s.toggle("c");
        s.retain(&["a", "b"]);
        assert_eq!(sorted(&s), ["a"]);
        s.retain(&[]);
        assert_eq!(s.len(), 0);
        assert_eq!(s.cursor, None);
    }
}
