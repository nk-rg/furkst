//! Left sidebar: views, branches, remotes, tags, stashes and submodules.

use egui::{CollapsingHeader, RichText, Ui};
use furkst_core::{Operation, RefKind, RefsSnapshot, WorkingStatus};

use super::{Action, Scope, View, menus};
use crate::widgets;

#[derive(Default)]
pub struct SidebarState {
    filter: String,
    /// A branch dropped onto the checked-out one, and where, while its merge/rebase menu is open.
    dropped: Option<(String, egui::Pos2)>,
}

/// Drag-and-drop payload: the name of a branch being dragged in the sidebar.
struct DraggedBranch(String);

pub fn show(
    ui: &mut Ui,
    state: &mut SidebarState,
    refs: Option<&RefsSnapshot>,
    status: &WorkingStatus,
    view: View,
    scope: &Scope,
    actions: &mut Vec<Action>,
) {
    ui.add_space(6.0);
    let changes = status.unstaged.len() + status.staged.len();
    let label = if changes > 0 { format!("📝  Local Changes ({changes})") } else { "📝  Local Changes".into() };
    if nav_button(ui, view == View::LocalChanges, label).clicked() {
        actions.push(Action::ShowView(View::LocalChanges));
    }
    let all = view == View::History && *scope == Scope::All;
    if nav_button(ui, all, "🕘  All Commits").clicked() {
        actions.push(Action::SetScope(Scope::All));
    }
    if nav_button(ui, false, "↻  Reflog").on_hover_text("Everywhere HEAD has been: undo resets, rebases and deletions").clicked() {
        actions.push(Action::ShowReflog);
    }
    if let Scope::Only { label, .. } = scope {
        nav_button(ui, view == View::History, format!("🔎  {label}"));
    }
    ui.separator();
    widgets::filter_box(ui, &mut state.filter, "Filter branches, tags…");
    ui.add_space(2.0);

    let Some(refs) = refs else { return };
    let filter = widgets::TextFilter::new(&state.filter);
    let filtering = filter.is_active();
    let branches: Vec<_> = refs.branches.iter().filter(|b| filter.matches(&b.name)).collect();
    // Remote branches match by their full name, so "origin/fe" finds origin/feature.
    let remotes: Vec<_> = refs
        .remotes
        .iter()
        .map(|r| (r, r.branches.iter().filter(|b| filter.matches(&b.full_name())).collect::<Vec<_>>()))
        .filter(|(_, branches)| !filtering || !branches.is_empty())
        .collect();
    let tags: Vec<_> = refs.tags.iter().filter(|t| filter.matches(&t.name)).collect();
    let stashes: Vec<_> = refs.stashes.iter().filter(|s| filter.matches(&s.message)).collect();
    let worktrees: Vec<_> = refs
        .worktrees
        .iter()
        .filter(|w| filter.matches(&widgets::path_name(&w.path)) || w.branch.as_deref().is_some_and(|b| filter.matches(b)))
        .collect();
    let submodules: Vec<_> = refs.submodules.iter().filter(|s| filter.matches(&s.path)).collect();
    let nothing_matches = [branches.len(), remotes.len(), tags.len(), stashes.len(), worktrees.len(), submodules.len()]
        .iter()
        .all(|n| *n == 0);
    let section = |ui: &mut Ui, title: &str, count: usize, body: &mut dyn FnMut(&mut Ui)| section(ui, title, count, filtering, body);

    let mut dropped = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        if filtering && nothing_matches {
            widgets::section_label(ui, "No matches");
        }
        section(ui, "Branches", branches.len(), &mut |ui| {
            // Only the folders leading to the checked-out branch start open (all of them while filtering). Naming the tree
            // after that branch starts it over that way after switching branches.
            let head = refs.branches.iter().find(|b| b.is_head).map_or("", |b| b.name.as_str());
            let id = format!("branches-{filtering}-{head}");
            let open = |items: &[&furkst_core::Branch]| filtering || items.iter().any(|b| b.is_head);
            widgets::folder_tree(ui, &id, &branches, 0, &open, &|b| b.name.as_str(), &mut |ui, leaf, branch| {
                // The checked-out branch stands out: checked, bold and highlighted.
                let button = if branch.is_head {
                    egui::Button::selectable(true, RichText::new(format!("✔ {leaf}")).strong())
                } else {
                    egui::Button::selectable(false, leaf).frame(false)
                };
                let response = ui
                    .horizontal(|ui| {
                        let response = ui.add(button);
                        ui.label(RichText::new(widgets::ahead_behind(branch)).small().weak());
                        response
                    })
                    .inner;
                let response = response.interact(egui::Sense::click_and_drag());
                if branch.is_head {
                    dropped = dropped.take().or_else(|| drop_target(ui, &response));
                } else {
                    response.dnd_set_drag_payload(DraggedBranch(branch.name.clone()));
                }
                menus::click_ref(&response, RefKind::LocalBranch, &branch.name, branch.target, refs, actions);
                response.context_menu(|ui| menus::branch(ui, branch, refs, actions));
            });
        });

        let header = section(ui, "Remotes", remotes.len(), &mut |ui| {
            for (remote, branches) in &remotes {
                let header = CollapsingHeader::new(RichText::new(format!("☁ {}", remote.name)))
                    .id_salt(("remote", &remote.name, filtering))
                    .default_open(filtering);
                let response = header
                    .show(ui, |ui| {
                        let id = format!("{}-{filtering}", remote.name);
                        widgets::folder_tree(ui, &id, branches, 0, &|_| filtering, &|b| b.name.as_str(), &mut |ui, leaf, branch| {
                            let response = ui.add(egui::Button::selectable(false, leaf).frame(false)).interact(egui::Sense::click_and_drag());
                            response.dnd_set_drag_payload(DraggedBranch(branch.full_name()));
                            menus::click_ref(&response, RefKind::RemoteBranch, &branch.full_name(), branch.target, refs, actions);
                            response.context_menu(|ui| menus::remote_branch(ui, branch, refs, actions));
                        });
                    })
                    .header_response;
                response.on_hover_text(remote.url.as_deref().unwrap_or_default()).context_menu(|ui| menus::remote(ui, remote, actions));
            }
        });
        if let Some(header) = header {
            header.context_menu(|ui| menus::remotes_header(ui, actions));
        }

        section(ui, "Tags", tags.len(), &mut |ui| {
            for tag in &tags {
                let response = ui.add(egui::Button::selectable(false, format!("🏷 {}", tag.name)).frame(false));
                let response = match &tag.message {
                    Some(message) if !message.is_empty() => response.on_hover_text(message),
                    _ => response,
                };
                menus::click_ref(&response, RefKind::Tag, &tag.name, tag.target, refs, actions);
                response.context_menu(|ui| menus::tag(ui, tag, refs, actions));
            }
        });

        section(ui, "Stashes", stashes.len(), &mut |ui| {
            for stash in &stashes {
                let response = ui.add(egui::Button::selectable(false, &stash.message).frame(false).truncate());
                if response.clicked() {
                    actions.push(Action::SelectCommit(stash.id));
                }
                response.on_hover_text(stash.reference()).context_menu(|ui| menus::stash(ui, stash, actions));
            }
        });

        if !refs.worktrees.is_empty()
            && let Some(header) = section(ui, "Worktrees", worktrees.len(), &mut |ui| {
                for worktree in &worktrees {
                    let name = widgets::path_name(&worktree.path);
                    let branch = worktree.branch.as_deref().unwrap_or("detached HEAD");
                    let mut text = RichText::new(format!("🗁 {name} ({branch})"));
                    if worktree.is_current {
                        text = text.strong();
                    }
                    let response = ui.add(egui::Button::selectable(false, text).frame(false).truncate());
                    if response.clicked() && !worktree.is_current {
                        actions.push(Action::OpenRepository(worktree.path.clone()));
                    }
                    let mut tip = worktree.path.display().to_string();
                    if let Some(reason) = &worktree.locked {
                        tip.push_str(&format!("\nLocked {reason}"));
                    }
                    if worktree.prunable {
                        tip.push_str("\nMissing on disk (can be pruned)");
                    }
                    response.on_hover_text(tip).context_menu(|ui| menus::worktree(ui, worktree, actions));
                }
            })
        {
            header.context_menu(|ui| menus::worktrees_header(ui, refs, actions));
        }

        // LFS patterns aren't refs, so the section steps aside while filtering.
        if refs.uses_lfs && !filtering {
            section(ui, "Git LFS", refs.lfs_patterns.len(), &mut |ui| {
                ui.horizontal(|ui| {
                    if ui.small_button("Pull").on_hover_text("Download the LFS content of the checked-out files").clicked() {
                        actions.push(Action::Run(Operation::LfsPull));
                    }
                    if ui.small_button("Fetch").on_hover_text("Download LFS content without changing files").clicked() {
                        actions.push(Action::Run(Operation::LfsFetch));
                    }
                });
                for pattern in &refs.lfs_patterns {
                    ui.label(RichText::new(pattern).monospace());
                }
            });
        }

        if !refs.submodules.is_empty() {
            section(ui, "Submodules", submodules.len(), &mut |ui| {
                for submodule in &submodules {
                    let response = ui.add(egui::Button::selectable(false, format!("📦 {}", submodule.path)).frame(false));
                    response.context_menu(|ui| menus::submodule(ui, submodule, actions));
                }
            });
        }
    });

    if dropped.is_some() {
        state.dropped = dropped;
    }
    drag_preview(ui.ctx());
    drop_menu(ui.ctx(), state, refs, actions);
}

/// Highlights the checked-out branch while a branch hovers over it; returns the branch dropped on it.
fn drop_target(ui: &Ui, response: &egui::Response) -> Option<(String, egui::Pos2)> {
    if response.dnd_hover_payload::<DraggedBranch>().is_some() {
        let stroke = egui::Stroke::new(2.0, ui.visuals().selection.bg_fill);
        ui.painter().rect_stroke(response.rect.expand(2.0), 4.0, stroke, egui::StrokeKind::Outside);
    }
    let dropped = response.dnd_release_payload::<DraggedBranch>()?;
    Some((dropped.0.clone(), ui.ctx().pointer_latest_pos().unwrap_or(response.rect.right_center())))
}

/// The name of the dragged branch, following the pointer.
fn drag_preview(ctx: &egui::Context) {
    let (Some(dragged), Some(pointer)) = (egui::DragAndDrop::payload::<DraggedBranch>(ctx), ctx.pointer_latest_pos()) else {
        return;
    };
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    egui::Area::new(egui::Id::new("dragged-branch"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pointer + egui::vec2(12.0, 8.0))
        .interactable(false)
        .show(ctx, |ui| {
            widgets::badge(ui, &dragged.0, crate::theme::reference(furkst_core::RefKind::LocalBranch), true);
        });
}

/// After a drop, offers to merge the dropped branch into the current one or rebase onto it.
fn drop_menu(ctx: &egui::Context, state: &mut SidebarState, refs: &RefsSnapshot, actions: &mut Vec<Action>) {
    let Some((source, pos)) = &state.dropped else { return };
    let before = actions.len();
    let mut cancel = false;
    let area = egui::Area::new(egui::Id::new("drop-menu")).order(egui::Order::Foreground).fixed_pos(*pos).show(ctx, |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            menus::integrate(ui, source, source, refs, actions);
            ui.separator();
            cancel = ui.button("Cancel").clicked();
        });
    });
    let clicked_elsewhere = ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer();
    if actions.len() > before || cancel || clicked_elsewhere || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.dropped = None;
    }
}

/// A collapsible sidebar section; returns the header response for attaching a context menu. While filtering,
/// sections open (without forgetting how the user left them) and hide when nothing in them matches.
fn section(ui: &mut Ui, title: &str, count: usize, filtering: bool, body: &mut dyn FnMut(&mut Ui)) -> Option<egui::Response> {
    if filtering && count == 0 {
        return None;
    }
    ui.add_space(4.0);
    let header = CollapsingHeader::new(widgets::section_title(&format!("{title}  {count}")))
        .id_salt((title, filtering))
        .default_open(true)
        .show(ui, |ui| {
            if count == 0 {
                widgets::section_label(ui, "none");
            }
            body(ui);
        });
    Some(header.header_response)
}

/// A full-width entry at the top of the sidebar.
fn nav_button(ui: &mut Ui, selected: bool, label: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add_sized([ui.available_width(), 24.0], egui::Button::selectable(selected, label))
}
