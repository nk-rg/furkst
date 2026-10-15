//! The "File Tree" tab of the commit details: every file of a commit and a read-only viewer for one of them.

use std::cmp::Ordering;
use std::ops::Range;

use egui::{RichText, TextStyle, Ui};
use furkst_core::{Oid, Repo, TreeFile};

use super::{Action, Loaded, menus};
use crate::widgets;

/// Files larger than this are not shown.
const MAX_VIEW_BYTES: usize = 5 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetailsTab {
    /// Who, when and why, with the files it changed.
    #[default]
    Commit,
    /// The changed files and the selected one's diff.
    Changes,
    /// Every file of the commit and a viewer.
    FileTree,
}

/// What the viewer shows for the selected file.
pub enum FileContent {
    /// The text and the byte range of each of its lines.
    Text { text: String, lines: Vec<Range<usize>> },
    /// Binary or oversized files.
    Placeholder(String),
}

struct Selected {
    path: String,
    content: Option<Loaded<FileContent>>,
}

#[derive(Default)]
pub struct TreeBrowser {
    pub tab: DetailsTab,
    filter: String,
    /// Sorted: folders first, natural order. `None` until loaded.
    files: Option<Vec<TreeFile>>,
    loading: bool,
    /// Indices into `files` that match the filter.
    shown: Vec<usize>,
    selected: Option<Selected>,
}

impl TreeBrowser {
    /// Forgets everything belonging to the previously selected commit.
    pub fn reset(&mut self) {
        self.files = None;
        self.loading = false;
        self.shown.clear();
        self.selected = None;
    }

    /// True once if the tree is wanted but neither loaded nor being loaded.
    pub fn begin_load(&mut self) -> bool {
        let wanted = self.tab == DetailsTab::FileTree && self.files.is_none() && !self.loading;
        self.loading |= wanted;
        wanted
    }

    pub fn finish_tree(&mut self, files: Vec<TreeFile>) {
        self.files = Some(files);
        self.loading = false;
        self.refilter();
    }

    pub fn select(&mut self, path: String) {
        self.selected = Some(Selected { path, content: None });
    }

    pub fn finish_file(&mut self, path: &str, content: Loaded<FileContent>) {
        if let Some(selected) = self.selected.as_mut().filter(|s| s.path == path) {
            selected.content = Some(content);
        }
    }

    pub fn is_selected(&self, path: &str) -> bool {
        self.selected.as_ref().is_some_and(|s| s.path == path)
    }

    fn refilter(&mut self) {
        let filter = widgets::TextFilter::new(&self.filter);
        let files = self.files.as_deref().unwrap_or_default();
        self.shown = (0..files.len()).filter(|&i| filter.matches(&files[i].path)).collect();
    }

    /// The tab strip above the file list.
    pub fn tabs(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            for (tab, label) in [(DetailsTab::Commit, "Commit"), (DetailsTab::Changes, "Changes"), (DetailsTab::FileTree, "File Tree")] {
                if ui.selectable_label(self.tab == tab, label).clicked() {
                    actions.push(Action::SetDetailsTab(tab));
                }
            }
        });
    }

    /// The filter box and the collapsible tree of files of `commit`.
    pub fn list(&mut self, ui: &mut Ui, commit: Oid, actions: &mut Vec<Action>) {
        if self.files.is_none() {
            return widgets::section_label(ui, "Loading…");
        }
        if widgets::filter_box(ui, &mut self.filter, "Filter files") {
            self.refilter();
        }
        ui.add_space(2.0);
        let files = self.files.as_deref().unwrap_or_default();
        let shown: Vec<&TreeFile> = self.shown.iter().map(|&i| &files[i]).collect();
        // A huge tree is only expanded on demand, unless a filter narrowed it down.
        let filtering = !self.filter.is_empty();
        let open = filtering || shown.len() < 500;
        let id = format!("tree-{filtering}-{open}");
        widgets::folder_tree(ui, &id, &shown, 0, &|_| open, &|f| f.path.as_str(), &mut |ui, name, file| {
            let response = ui.add(egui::Button::selectable(self.is_selected(&file.path), format!("📄 {name}")).frame(false));
            if response.clicked() {
                actions.push(Action::SelectTreeFile(file.path.clone()));
            }
            response.context_menu(|ui| menus::tree_file(ui, &file.path, commit, actions));
        });
    }

    /// The selected file's content, virtualized so large files stay fast.
    pub fn show_content(&self, ui: &mut Ui) {
        let Some(selected) = &self.selected else { return widgets::placeholder(ui, "Select a file") };
        match &selected.content {
            None => widgets::placeholder(ui, "Loading…"),
            Some(Err(error)) => widgets::placeholder(ui, error),
            Some(Ok(FileContent::Placeholder(text))) => widgets::placeholder(ui, text),
            Some(Ok(FileContent::Text { text, lines })) => {
                ui.add_space(4.0);
                ui.label(RichText::new(&selected.path).strong());
                ui.separator();
                let row_height = ui.text_style_height(&TextStyle::Monospace).max(ui.spacing().interact_size.y);
                let digits = lines.len().to_string().len();
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::both().auto_shrink(false).show_rows(ui, row_height, lines.len(), |ui, range| {
                    for i in range {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 12.0;
                            ui.add(egui::Label::new(RichText::new(format!("{:>digits$}", i + 1)).monospace().weak()).selectable(false));
                            ui.add(
                                egui::Label::new(RichText::new(&text[lines[i].clone()]).monospace())
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        });
                    }
                });
            }
        }
    }
}

/// Lists the files of `commit`, sorted for display.
pub fn load_tree(repo: &Repo, commit: Oid) -> Loaded<Vec<TreeFile>> {
    let mut files = repo.tree_files(commit).map_err(|e| e.to_string())?;
    files.sort_by(|a, b| tree_order(&a.path, &b.path));
    Ok(files)
}

/// Reads `path` at `commit` for the viewer.
pub fn load_file(repo: &Repo, commit: Oid, path: &str) -> Loaded<FileContent> {
    let data = repo.file_at(commit, path).map_err(|e| e.to_string())?.ok_or("The file does not exist in this commit")?;
    let size = widgets::format_size(data.len());
    if furkst_core::is_binary(&data) {
        return Ok(FileContent::Placeholder(format!("Binary file ({size})")));
    }
    if data.len() > MAX_VIEW_BYTES {
        return Ok(FileContent::Placeholder(format!("File too large to display ({size})")));
    }
    let text = String::from_utf8_lossy(&data).into_owned();
    let mut lines = Vec::new();
    let mut start = 0;
    for (end, _) in text.match_indices('\n') {
        lines.push(start..start + text[start..end].trim_end_matches('\r').len());
        start = end + 1;
    }
    if start < text.len() {
        lines.push(start..text.len());
    }
    Ok(FileContent::Text { text, lines })
}

/// Path order where, at every level, folders come before files and names compare naturally.
pub fn tree_order(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.split('/').peekable(), b.split('/').peekable());
    while let (Some(p), Some(q)) = (x.next(), y.next()) {
        let (p_folder, q_folder) = (x.peek().is_some(), y.peek().is_some());
        if p_folder != q_folder {
            return q_folder.cmp(&p_folder);
        }
        match furkst_core::natural_cmp(p, q) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_first_and_natural() {
        let mut paths = vec!["b.txt", "src/a10.rs", "src/a2.rs", "Zed/x", "a/b/c", "a.txt", "a/z"];
        paths.sort_by(|a, b| tree_order(a, b));
        assert_eq!(paths, ["a/b/c", "a/z", "src/a2.rs", "src/a10.rs", "Zed/x", "a.txt", "b.txt"]);
    }
}
