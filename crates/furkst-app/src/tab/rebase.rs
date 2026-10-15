//! The interactive rebase editor: reorder commits and choose what happens to each one.

use egui::{RichText, TextEdit, Ui};
use furkst_core::{CommitInfo, Oid, Operation, RebaseAction, RebaseStep, validate_rebase};

struct Entry {
    step: RebaseStep,
    summary: String,
    /// Text of the reword box; only used while the action is [`RebaseAction::Reword`].
    message: String,
}

/// Editable state of the rebase dialog. Entries are oldest first, the order they are replayed in.
pub struct Editor {
    /// The commit the replayed ones follow; `None` replays from the very first commit.
    base: Option<Oid>,
    /// What gets replayed, for the heading.
    replaying: String,
    entries: Vec<Entry>,
}

impl Editor {
    /// `plan` is what [`furkst_core::Repo::rebase_plan`] returned for `base`.
    pub fn new(base: Option<Oid>, replaying: String, plan: Vec<CommitInfo>) -> Self {
        let entries = plan
            .into_iter()
            .map(|c| Entry { step: RebaseStep::new(c.id), summary: c.summary, message: c.message.trim_end().to_owned() })
            .collect();
        Self { base, replaying, entries }
    }

    fn steps(&self) -> Vec<RebaseStep> {
        self.entries
            .iter()
            .map(|e| {
                let message = (e.step.action == RebaseAction::Reword).then(|| e.message.clone());
                RebaseStep { message, ..e.step.clone() }
            })
            .collect()
    }

    /// Why the plan cannot start, or the operation that runs it.
    pub fn operation(&self) -> Result<Operation, String> {
        let steps = self.steps();
        validate_rebase(&steps).map_err(|e| e.to_string())?;
        Ok(Operation::InteractiveRebase { base: self.base, steps })
    }

    pub fn fields(&mut self, ui: &mut Ui) {
        ui.label(format!("Replay {} (oldest first):", self.replaying));
        let mut swap = None;
        let last = self.entries.len().saturating_sub(1);
        egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
            for (i, entry) in self.entries.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    if ui.add_enabled(i > 0, egui::Button::new("⬆").small()).clicked() {
                        swap = Some((i - 1, i));
                    }
                    if ui.add_enabled(i < last, egui::Button::new("⬇").small()).clicked() {
                        swap = Some((i, i + 1));
                    }
                    egui::ComboBox::from_id_salt(("rebase-action", i))
                        .width(70.0)
                        .selected_text(entry.step.action.label())
                        .show_ui(ui, |ui| {
                            for action in RebaseAction::ALL {
                                ui.selectable_value(&mut entry.step.action, action, action.label());
                            }
                        });
                    let mut text = RichText::new(&entry.summary);
                    if entry.step.action == RebaseAction::Drop {
                        text = text.strikethrough().weak();
                    }
                    ui.add(egui::Label::new(text).truncate());
                });
                if entry.step.action == RebaseAction::Reword {
                    ui.add(TextEdit::multiline(&mut entry.message).desired_width(f32::INFINITY).desired_rows(3));
                }
            }
        });
        if let Some((a, b)) = swap {
            self.entries.swap(a, b);
        }
        if let Err(problem) = self.operation() {
            ui.colored_label(ui.visuals().warn_fg_color, problem);
        }
    }
}
