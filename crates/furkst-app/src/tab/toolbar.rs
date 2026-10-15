//! The row of big buttons at the top of a repository tab.

use egui::{Align2, FontId, RichText, Sense, Ui, Vec2};
use furkst_core::Operation;

use super::{Action, Dialog, RepoTab, menus};
use crate::settings::Settings;
use crate::widgets;

enum Icon {
    Glyph(&'static str),
    Branch,
}

pub fn show(ui: &mut Ui, tab: &RepoTab, settings: &Settings, actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let refs = tab.refs.as_ref();
        let current = refs.and_then(|r| r.current_branch());
        let has_remotes = refs.is_some_and(|r| !r.remotes.is_empty());

        if button(ui, Icon::Glyph("⟳"), "Fetch", has_remotes).on_hover_text("Fetch all remotes").clicked() {
            actions.push(Action::fetch_all(settings));
        }
        let can_pull = current.is_some_and(|b| b.upstream.is_some());
        if button(ui, Icon::Glyph("⬇"), "Pull", can_pull).clicked() {
            actions.push(Action::Open(Dialog::pull(settings)));
        }
        let push = refs.and_then(Dialog::push_current).filter(|_| has_remotes);
        if button(ui, Icon::Glyph("⬆"), "Push", push.is_some()).clicked()
            && let Some(push) = push
        {
            actions.push(Action::Open(push));
        }
        ui.separator();
        let head = refs.and_then(|r| r.head.target());
        let new_branch = refs.and_then(Dialog::create_branch_at_head);
        if button(ui, Icon::Branch, "Branch", new_branch.is_some()).clicked()
            && let Some(new_branch) = new_branch
        {
            actions.push(Action::Open(new_branch));
        }
        if button(ui, Icon::Glyph("📥"), "Stash", !tab.status.is_clean()).clicked() {
            actions.push(Action::Open(Dialog::stash()));
        }
        let has_stash = refs.is_some_and(|r| !r.stashes.is_empty());
        if button(ui, Icon::Glyph("📤"), "Pop", has_stash).on_hover_text("Pop the latest stash").clicked() {
            actions.push(Action::Run(Operation::StashApply { index: 0, pop: true }));
        }
        if let Some(refs) = refs {
            let flow = button(ui, Icon::Glyph("⚑"), "Flow", true).on_hover_text("Git-flow: feature, release and hotfix branches");
            egui::Popup::menu(&flow).show(|ui| menus::flow(ui, refs, actions));
        }
        ui.separator();
        if button(ui, Icon::Glyph("🖳"), "Terminal", true).clicked() {
            actions.push(Action::OpenTerminal);
        }
        if button(ui, Icon::Glyph("🗁"), "Explorer", true).clicked() {
            actions.push(Action::OpenPath(Default::default()));
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button(ui, Icon::Glyph("↻"), "Refresh", !tab.operation_running()).on_hover_text("F5").clicked() {
                actions.push(Action::Refresh);
            }
            if let Some(branch) = current {
                ui.label(RichText::new(format!("{}  {}", branch.name, widgets::ahead_behind(branch)).trim_end()).strong());
                widgets::branch_icon(ui);
            } else if let Some(head) = head {
                ui.label(RichText::new(format!("Detached at {}", furkst_core::short_id(&head))).strong());
            }
        });
    });
    ui.add_space(2.0);
}

/// An icon above a caption, as on the toolbar.
fn button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool) -> egui::Response {
    let size = Vec2::new(58.0, 44.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let visuals = ui.style().interact(&response);
    if enabled && response.hovered() {
        ui.painter().rect_filled(rect, 6.0, visuals.weak_bg_fill);
    }
    let color = if enabled { ui.visuals().text_color() } else { ui.visuals().weak_text_color().gamma_multiply(0.6) };
    let icon_center = rect.center_top() + Vec2::new(0.0, 14.0);
    match icon {
        Icon::Glyph(glyph) => {
            ui.painter().text(icon_center, Align2::CENTER_CENTER, glyph, FontId::proportional(18.0), color);
        }
        Icon::Branch => widgets::paint_branch_glyph(ui.painter(), egui::Rect::from_center_size(icon_center, Vec2::splat(20.0)), color),
    }
    ui.painter().text(rect.center_bottom() - Vec2::new(0.0, 10.0), Align2::CENTER_CENTER, label, FontId::proportional(11.5), color);
    response
}
