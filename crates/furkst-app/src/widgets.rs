//! Small reusable widgets.

use egui::{Align2, Color32, CornerRadius, FontId, Response, RichText, Sense, Ui, Vec2};
use furkst_core::{ChangeKind, FileChange, RefKind, RefLabel, Signature};

use crate::theme;

/// A rounded, tinted label such as a branch name next to a commit.
pub fn badge(ui: &mut Ui, text: &str, color: Color32, strong: bool) -> Response {
    let colors = theme::badge(color, strong, ui.visuals());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(12.0), colors.text);
    let size = galley.size() + Vec2::new(10.0, 4.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::same(4), colors.fill);
    // Always outlined, so the badge stands apart from the row; a hovered one stands out a little more.
    let width = if response.hovered() { 1.5 } else { 1.0 };
    ui.painter().rect_stroke(rect, CornerRadius::same(4), egui::Stroke::new(width, colors.outline), egui::StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, Color32::PLACEHOLDER);
    response
}

pub fn ref_badge(ui: &mut Ui, label: &RefLabel) -> Response {
    let text = match label.kind {
        RefKind::Tag => format!("🏷 {}", label.name),
        _ => label.name.clone(),
    };
    badge(ui, &text, theme::reference(label.kind), label.is_head || label.kind == RefKind::Head)
}

/// The colored one-letter square shown before changed files.
pub fn change_icon(ui: &mut Ui, kind: ChangeKind) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
    ui.painter().rect_filled(rect.shrink(1.0), CornerRadius::same(3), theme::solid(theme::change(kind)));
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, kind.letter(), FontId::monospace(11.0), Color32::WHITE);
    response
}

/// A circle with the person's initials, standing in for an avatar.
/// The author's GitHub picture when avatars are on and the author is a known GitHub user (see [`crate::avatars`]);
/// otherwise colored initials.
pub fn avatar(ui: &mut Ui, sig: &Signature, committer: &Signature, diameter: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::hover());
    let response = response.on_hover_text(format!("{} <{}>", sig.name, sig.email));
    if let Some(picture) = crate::avatars::get(ui.ctx(), sig, committer) {
        round_picture(ui, &picture, rect);
        return response;
    }
    let initials: String = sig.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
    ui.painter().circle_filled(rect.center(), diameter / 2.0, theme::solid(theme::hashed(&sig.email)));
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        initials.to_uppercase(),
        FontId::proportional(diameter * 0.42),
        Color32::WHITE,
    );
    response
}

/// Paints `picture` into `rect`, cropped to a circle.
pub fn round_picture(ui: &Ui, picture: &egui::TextureHandle, rect: egui::Rect) {
    let radius = CornerRadius::same((rect.width() / 2.0).round() as u8);
    egui::Image::from_texture(egui::load::SizedTexture::new(picture.id(), rect.size())).corner_radius(radius).paint_at(ui, rect);
}

/// A full-width clickable row showing a changed file: icon, file name and, with `show_folder`, its dimmed directory. With
/// `expanded`, it leads with the arrow of something that opens under it.
pub fn file_row(ui: &mut Ui, change: &FileChange, selected: bool, show_folder: bool, expanded: Option<bool>) -> Response {
    let height = 22.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, CornerRadius::same(3), ui.visuals().selection.bg_fill);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(3), ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(4.0, 0.0))).layout(
        egui::Layout::left_to_right(egui::Align::Center),
    ));
    let name_color = if selected { ui.visuals().selection.stroke.color } else { ui.visuals().strong_text_color() };
    if let Some(open) = expanded {
        let arrow = child.allocate_exact_size(Vec2::splat(14.0), Sense::hover()).0;
        chevron(&child, arrow, open);
    }
    file_label(&mut child, change, name_color, show_folder);
    response.on_hover_text(&change.path)
}

/// A thin chevron in `rect`: pointing right when closed, down when `open`.
pub fn chevron(ui: &Ui, rect: egui::Rect, open: bool) {
    let c = rect.center();
    let s = rect.width() * 0.18;
    let points = if open {
        [c + Vec2::new(-1.6 * s, -0.8 * s), c + Vec2::new(0.0, 0.8 * s), c + Vec2::new(1.6 * s, -0.8 * s)]
    } else {
        [c + Vec2::new(-0.8 * s, -1.6 * s), c + Vec2::new(0.8 * s, 0.0), c + Vec2::new(-0.8 * s, 1.6 * s)]
    };
    let stroke = egui::Stroke::new(1.5, ui.visuals().text_color());
    ui.painter().add(egui::Shape::line(points.to_vec(), stroke));
}

/// A changed file's icon, name and (with `show_folder`) dimmed folder, and where it was renamed from, in a left-to-right
/// `ui`. The name comes first, so when space runs out the folder is what gets cut short.
pub fn file_label(ui: &mut Ui, change: &FileChange, name_color: Color32, show_folder: bool) {
    ui.spacing_mut().item_spacing.x = 6.0;
    change_icon(ui, change.kind);
    ui.add(egui::Label::new(RichText::new(change.file_name()).color(name_color)).truncate().selectable(false));
    let mut location = if show_folder { change.directory().to_owned() } else { String::new() };
    if let Some(old) = &change.old_path {
        location = format!("{location}  (renamed from {old})").trim_start().to_owned();
    }
    if !location.is_empty() {
        ui.add(egui::Label::new(RichText::new(location).weak()).truncate().selectable(false));
    }
}

/// How section titles look, in the sidebar and above lists: capitals, a little spaced out and dimmed, and large enough
/// (unlike egui's 9-point "small" text) to read as the same font as everything else.
pub fn section_title(text: &str) -> RichText {
    RichText::new(text.to_uppercase()).size(11.0).extra_letter_spacing(0.6).weak()
}

/// A section title on its own, above a list.
pub fn section_label(ui: &mut Ui, text: &str) {
    ui.label(section_title(text));
}

/// Text that can be clicked to copy `value` to the clipboard.
pub fn copyable(ui: &mut Ui, text: RichText, value: &str) {
    if ui.add(egui::Label::new(text).sense(Sense::click())).on_hover_text("Click to copy").clicked() {
        ui.ctx().copy_text(value.to_owned());
    }
}

pub fn format_date(sig: &Signature) -> String {
    sig.format_time("%d %b %Y %H:%M")
}

/// A centered, dimmed message for empty panes.
pub fn placeholder(ui: &mut Ui, text: &str) {
    ui.centered_and_justified(|ui| ui.label(RichText::new(text).weak()));
}

/// Paints a git-branch symbol (two commits on a line, a third forking off) inside `rect`.
pub fn paint_branch_glyph(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    let glyph = BranchGlyph::fit(rect);
    let stroke = egui::Stroke::new(glyph.stroke.max(1.2), color);
    painter.line_segment(glyph.trunk, stroke);
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(glyph.fork, false, Color32::TRANSPARENT, stroke));
    for center in glyph.nodes {
        painter.circle_filled(center, glyph.radius, color);
    }
}

/// The shape of the branch symbol, shared by the painted glyph and the generated app icon.
pub struct BranchGlyph {
    /// The straight line between the two commits on the left.
    pub trunk: [egui::Pos2; 2],
    /// The cubic curve forking off to the commit on the right.
    pub fork: [egui::Pos2; 4],
    pub nodes: [egui::Pos2; 3],
    pub stroke: f32,
    pub radius: f32,
}

impl BranchGlyph {
    /// The symbol centered in `rect`.
    pub fn fit(rect: egui::Rect) -> Self {
        let s = rect.width().min(rect.height());
        let c = rect.center();
        let (left, right) = (c.x - s * 0.22, c.x + s * 0.22);
        let (top, bottom) = (c.y - s * 0.32, c.y + s * 0.32);
        let p = egui::pos2;
        Self {
            trunk: [p(left, top), p(left, bottom)],
            fork: [p(right, top), p(right, c.y), p(left, c.y), p(left, c.y + s * 0.18)],
            nodes: [p(left, top), p(left, bottom), p(right, top)],
            stroke: s * 0.09,
            radius: s * 0.11,
        }
    }
}

/// An inline branch symbol sized to the current text.
pub fn branch_icon(ui: &mut Ui) -> Response {
    let size = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_branch_glyph(ui.painter(), rect, ui.visuals().text_color());
    response
}

/// The last component of `path` (a repository's folder name), or the whole path if it has none.
pub fn path_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Human-readable file size, e.g. `12.3 KB`.
pub fn format_size(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// Renders items whose names contain `/` as nested folders, like a branch tree. A folder starts expanded when `open`
/// says so for the items it holds.
pub fn folder_tree<T>(
    ui: &mut Ui,
    id: &str,
    items: &[&T],
    depth: usize,
    open: &dyn Fn(&[&T]) -> bool,
    name: &dyn Fn(&T) -> &str,
    leaf: &mut dyn FnMut(&mut Ui, &str, &T),
) {
    let segment = |item: &T| name(item).split('/').nth(depth).unwrap_or_default().to_owned();
    let is_leaf = |item: &T| name(item).split('/').count() == depth + 1;
    let mut i = 0;
    while i < items.len() {
        let item = items[i];
        if is_leaf(item) {
            leaf(ui, &segment(item), item);
            i += 1;
            continue;
        }
        // Group every following item sharing this folder.
        let folder = segment(item);
        let end = items[i..].iter().position(|other| is_leaf(other) || segment(other) != folder).map_or(items.len(), |n| i + n);
        egui::CollapsingHeader::new(format!("🗀 {folder}")).id_salt((id, depth, &folder)).default_open(open(&items[i..end])).show(ui, |ui| {
            folder_tree(ui, id, &items[i..end], depth + 1, open, name, leaf);
        });
        i = end;
    }
}

/// `⬆2 ⬇1`-style counts of commits a branch is ahead of / behind its upstream; empty when in sync.
pub fn ahead_behind(branch: &furkst_core::Branch) -> String {
    let counts = [("⬆", branch.ahead), ("⬇", branch.behind)];
    counts.iter().filter(|(_, n)| *n > 0).map(|(arrow, n)| format!("{arrow}{n}")).collect::<Vec<_>>().join(" ")
}

/// A menu entry that queues `make()` into `queue` when clicked, then closes the menu.
pub fn menu_item<T>(ui: &mut Ui, label: impl Into<String>, queue: &mut Vec<T>, make: impl FnOnce() -> T) {
    if ui.button(label.into()).clicked() {
        queue.push(make());
        ui.close();
    }
}

/// A case-insensitive "contains" filter, as typed into a [`filter_box`].
pub struct TextFilter(String);

impl TextFilter {
    pub fn new(text: &str) -> Self {
        Self(text.trim().to_lowercase())
    }

    pub fn is_active(&self) -> bool {
        !self.0.is_empty()
    }

    pub fn matches(&self, text: &str) -> bool {
        text.to_lowercase().contains(&self.0)
    }
}

/// Whether Escape was pressed in the text field `field`. egui drops a field's focus on Escape before the caller gets to
/// look, so a field that just lost it counts too.
pub fn escaped(ui: &Ui, field: &Response) -> bool {
    (field.has_focus() || field.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape))
}

/// A full-width filter field with a clear button; Escape clears it too. Returns whether the text changed.
pub fn filter_box(ui: &mut Ui, text: &mut String, hint: &str) -> bool {
    let field = egui::TextEdit::singleline(text).hint_text(format!("🔍 {hint}")).desired_width(f32::INFINITY);
    let field = ui.add(field.margin(egui::Margin { right: 22, ..egui::Margin::symmetric(4, 2) }));
    let escape = escaped(ui, &field);
    let mut clear = false;
    if !text.is_empty() {
        let spot = egui::Rect::from_center_size(field.rect.right_center() - Vec2::new(12.0, 0.0), Vec2::splat(18.0));
        clear = ui.put(spot, egui::Button::new("×").frame(false)).on_hover_text("Clear (Esc)").clicked();
    }
    if (clear || escape) && !text.is_empty() {
        text.clear();
        return true;
    }
    field.changed()
}

/// What resizing never takes a panel or the content beside it below, so dragging a divider stops instead of
/// collapsing either side.
const MIN_PANE_WIDTH: f32 = 200.0;

/// A resizable panel on the left of `ui`: its divider moves between [`MIN_PANE_WIDTH`] and the point that leaves as
/// much for what is beside it.
pub fn left_panel(ui: &Ui, id: impl Into<egui::Id>, default_width: f32) -> egui::Panel {
    let max = (ui.available_width() - MIN_PANE_WIDTH).max(MIN_PANE_WIDTH);
    egui::Panel::left(id).resizable(true).default_size(default_width).size_range(MIN_PANE_WIDTH..=max)
}

#[cfg(test)]
mod glyph_tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    /// Every non-ASCII character in the app's code (outside comments): symbols in labels, icons in buttons.
    fn characters_in_source(dir: &Path, found: &mut BTreeSet<(char, String)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                characters_in_source(&path, found);
            } else if path.extension().is_some_and(|e| e == "rs") {
                for (number, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
                    let code = line.split("//").next().unwrap_or_default();
                    for c in code.chars().filter(|c| !c.is_ascii()) {
                        found.insert((c, format!("{}:{}", path.file_name().unwrap().to_string_lossy(), number + 1)));
                    }
                }
            }
        }
    }

    #[test]
    fn every_symbol_drawn_has_a_glyph_in_the_built_in_fonts() {
        // The interface font and its fallbacks, as egui chains them (its own `has_glyph` misreports characters that
        // the font with the replacement box also has).
        let chain = [epaint_default_fonts::UBUNTU_LIGHT, epaint_default_fonts::NOTO_EMOJI_REGULAR, epaint_default_fonts::EMOJI_ICON];
        let faces: Vec<ttf_parser::Face<'_>> = chain.iter().map(|font| ttf_parser::Face::parse(font, 0).unwrap()).collect();
        let mut found = BTreeSet::new();
        characters_in_source(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut found);
        let missing: Vec<_> = found
            .into_iter()
            .filter(|(c, _)| faces.iter().all(|face| face.glyph_index(*c).is_none()))
            .map(|(c, at)| format!("{c} (U+{:04X}) at {at}", c as u32))
            .collect();
        assert!(missing.is_empty(), "these would draw as empty boxes:\n{}", missing.join("\n"));
    }
}
