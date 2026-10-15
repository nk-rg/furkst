//! Floating window for resolving the conflict regions of one file.

use egui::{RichText, Ui};
use furkst_core::{ConflictRegion, FileChange, Operation, Repo, Resolution, Segment, assemble, conflict_count};

use super::Action;
use crate::theme::DiffColors;
use crate::widgets;

pub struct Resolver {
    change: FileChange,
    open: bool,
    /// The parsed file, or why it cannot be resolved here.
    content: Result<Vec<Segment>, String>,
    /// One entry per conflict region, in file order.
    choices: Vec<Option<Resolution>>,
}

impl Resolver {
    pub fn open(repo: &Repo, change: FileChange) -> Self {
        let content = match repo.conflict_segments(&change.path) {
            Ok(segments) if conflict_count(&segments) == 0 => Err("The file has no conflict markers.".to_owned()),
            Ok(segments) => Ok(segments),
            Err(err) => Err(err.to_string()),
        };
        let choices = vec![None; content.as_ref().map_or(0, |s| conflict_count(s))];
        Self { change, open: true, content, choices }
    }
}

pub fn show(ctx: &egui::Context, slot: &mut Option<Resolver>, actions: &mut Vec<Action>) {
    let Some(resolver) = slot else { return };
    let mut save = false;
    egui::Window::new(format!("Resolve conflicts — {}", resolver.change.path))
        .id(egui::Id::new("resolver"))
        .open(&mut resolver.open)
        .default_size([900.0, 600.0])
        .show(ctx, |ui| match &resolver.content {
            Err(message) => {
                widgets::placeholder(ui, &format!("{message}\nUse \"Resolve using mine\" or \"Resolve using theirs\" instead."));
            }
            Ok(segments) => save = body(ui, segments, &mut resolver.choices),
        });
    if save && let Ok(segments) = &resolver.content {
        actions.push(Action::Run(Operation::ResolveMerged {
            change: resolver.change.clone(),
            content: assemble(segments, &resolver.choices),
        }));
        resolver.open = false;
    }
    if !resolver.open {
        *slot = None;
    }
}

/// Returns whether "Save & mark resolved" was clicked.
fn body(ui: &mut Ui, segments: &[Segment], choices: &mut [Option<Resolution>]) -> bool {
    let colors = DiffColors::of(ui.visuals());
    let height = (ui.available_height() - 90.0).max(120.0);
    egui::ScrollArea::vertical().max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        let mut index = 0;
        for segment in segments {
            match segment {
                Segment::Common(text) => {
                    let lines = text.lines().count();
                    egui::CollapsingHeader::new(RichText::new(format!("{lines} unchanged lines")).weak())
                        .id_salt(("common", index, lines))
                        .show(ui, |ui| ui.label(RichText::new(text).monospace()));
                }
                Segment::Conflict(region) => {
                    region_ui(ui, region, &mut choices[index], index, &colors);
                    index += 1;
                }
            }
        }
    });
    ui.separator();
    let resolved = choices.iter().flatten().count();
    egui::CollapsingHeader::new("Preview of the merged result").show(ui, |ui| {
        egui::ScrollArea::both().id_salt("preview").max_height(160.0).show(ui, |ui| {
            ui.label(RichText::new(assemble(segments, choices)).monospace());
        });
    });
    ui.horizontal(|ui| {
        ui.label(format!("{resolved} of {} conflicts resolved", choices.len()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled(resolved == choices.len(), egui::Button::new(RichText::new("Save & mark resolved").strong())).clicked()
        })
        .inner
    })
    .inner
}

fn region_ui(ui: &mut Ui, region: &ConflictRegion, choice: &mut Option<Resolution>, index: usize, colors: &DiffColors) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Conflict {}", index + 1)).strong());
            for resolution in Resolution::ALL {
                if ui.selectable_label(*choice == Some(resolution), resolution.label()).clicked() {
                    *choice = Some(resolution);
                }
            }
        });
        let sides = [
            Some(("Ours", &region.ours_label, &region.ours, colors.removed)),
            region.base.as_ref().map(|base| ("Base", &region.base_label, base, colors.hunk)),
            Some(("Theirs", &region.theirs_label, &region.theirs, colors.added)),
        ];
        let sides: Vec<_> = sides.into_iter().flatten().collect();
        ui.columns(sides.len(), |columns| {
            for (ui, (title, label, text, fill)) in columns.iter_mut().zip(sides.iter()) {
                widgets::section_label(ui, &if label.is_empty() { (*title).to_owned() } else { format!("{title} — {label}") });
                egui::Frame::new().fill(*fill).inner_margin(4.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(text.as_str()).monospace());
                });
            }
        });
    });
}
