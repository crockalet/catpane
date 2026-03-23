mod dialogs;
mod log_area;
mod search;
mod theme;
mod toolbar;

use egui::{self, Key, Ui, Vec2};

use crate::app::App;
use crate::pane::{PaneId, PaneNode, SplitDir};
use theme::*;

pub use theme::configure_fonts;

const DIVIDER_SIZE: f32 = 5.0;

pub fn draw_ui(ctx: &egui::Context, app: &mut App) {
    let tab = ctx.input(|i| i.key_pressed(Key::Tab) && !i.modifiers.command && !i.modifiers.ctrl);
    if tab {
        app.cycle_focus();
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        let tree_ids = app.pane_tree.pane_ids();
        if tree_ids.is_empty() {
            return;
        }
        draw_pane_tree(ui, app, &tree_ids);
    });

    if app.show_help {
        dialogs::draw_help_window(ctx, &mut app.show_help);
    }

    if app.show_wireless_dialog {
        dialogs::draw_wireless_dialog(ctx, app);
    }
}

fn draw_pane_tree(ui: &mut Ui, app: &mut App, _ids: &[PaneId]) {
    draw_node(ui, app, &clone_tree_structure(&app.pane_tree));
}

#[derive(Clone)]
enum TreeSnap {
    Leaf(PaneId),
    Split {
        dir: SplitDir,
        first: Box<TreeSnap>,
        second: Box<TreeSnap>,
    },
}

fn clone_tree_structure(node: &PaneNode) -> TreeSnap {
    match node {
        PaneNode::Leaf(id) => TreeSnap::Leaf(*id),
        PaneNode::Split { dir, first, second } => TreeSnap::Split {
            dir: *dir,
            first: Box::new(clone_tree_structure(first)),
            second: Box::new(clone_tree_structure(second)),
        },
    }
}

/// Returns the PaneId of the leftmost/topmost leaf in the subtree.
/// Used as a stable key for storing split ratios in egui's temp data.
fn first_leaf_id(snap: &TreeSnap) -> PaneId {
    match snap {
        TreeSnap::Leaf(id) => *id,
        TreeSnap::Split { first, .. } => first_leaf_id(first),
    }
}

fn draw_node(ui: &mut Ui, app: &mut App, snap: &TreeSnap) {
    match snap {
        TreeSnap::Leaf(id) => {
            draw_pane_panel(ui, app, *id);
        }
        TreeSnap::Split { dir, first, second } => {
            let split_key = egui::Id::new(("split_ratio", first_leaf_id(snap)));
            let ratio: f32 = ui.ctx().data(|d| d.get_temp(split_key).unwrap_or(0.5_f32));
            let is_dark = ui.visuals().dark_mode;

            let div_normal = if is_dark { OD_BG_HL } else { OL_BORDER };
            let div_active = if is_dark { OD_CYAN } else { OL_BLUE };

            // Claim the full available rect so the parent layout advances correctly.
            let avail_rect = ui.available_rect_before_wrap();
            let avail = avail_rect.size();
            ui.allocate_rect(avail_rect, egui::Sense::hover());

            match dir {
                SplitDir::Vertical => {
                    let usable = (avail.x - DIVIDER_SIZE).max(100.0);
                    let first_w = (usable * ratio).clamp(50.0, usable - 50.0);
                    let second_w = usable - first_w;

                    let first_rect =
                        egui::Rect::from_min_size(avail_rect.min, Vec2::new(first_w, avail.y));
                    let div_rect = egui::Rect::from_min_size(
                        avail_rect.min + Vec2::new(first_w, 0.0),
                        Vec2::new(DIVIDER_SIZE, avail.y),
                    );
                    let second_rect = egui::Rect::from_min_size(
                        avail_rect.min + Vec2::new(first_w + DIVIDER_SIZE, 0.0),
                        Vec2::new(second_w, avail.y),
                    );

                    let div_id = ui.id().with(("vdiv", first_leaf_id(snap)));
                    let div_resp = ui.interact(div_rect, div_id, egui::Sense::drag());
                    let color = if div_resp.hovered() || div_resp.dragged() {
                        div_active
                    } else {
                        div_normal
                    };
                    ui.painter().rect_filled(div_rect, 0.0, color);
                    if div_resp.hovered() || div_resp.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    if div_resp.dragged() {
                        let new_ratio =
                            ((first_w + div_resp.drag_delta().x) / usable).clamp(0.1, 0.9);
                        ui.ctx().data_mut(|d| d.insert_temp(split_key, new_ratio));
                    }

                    let mut first_ui = ui.new_child(egui::UiBuilder::new().max_rect(first_rect));
                    draw_node(&mut first_ui, app, first);

                    let mut second_ui = ui.new_child(egui::UiBuilder::new().max_rect(second_rect));
                    draw_node(&mut second_ui, app, second);
                }
                SplitDir::Horizontal => {
                    let usable = (avail.y - DIVIDER_SIZE).max(100.0);
                    let first_h = (usable * ratio).clamp(50.0, usable - 50.0);
                    let second_h = usable - first_h;

                    let first_rect =
                        egui::Rect::from_min_size(avail_rect.min, Vec2::new(avail.x, first_h));
                    let div_rect = egui::Rect::from_min_size(
                        avail_rect.min + Vec2::new(0.0, first_h),
                        Vec2::new(avail.x, DIVIDER_SIZE),
                    );
                    let second_rect = egui::Rect::from_min_size(
                        avail_rect.min + Vec2::new(0.0, first_h + DIVIDER_SIZE),
                        Vec2::new(avail.x, second_h),
                    );

                    let div_id = ui.id().with(("hdiv", first_leaf_id(snap)));
                    let div_resp = ui.interact(div_rect, div_id, egui::Sense::drag());
                    let color = if div_resp.hovered() || div_resp.dragged() {
                        div_active
                    } else {
                        div_normal
                    };
                    ui.painter().rect_filled(div_rect, 0.0, color);
                    if div_resp.hovered() || div_resp.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                    if div_resp.dragged() {
                        let new_ratio =
                            ((first_h + div_resp.drag_delta().y) / usable).clamp(0.1, 0.9);
                        ui.ctx().data_mut(|d| d.insert_temp(split_key, new_ratio));
                    }

                    let mut first_ui = ui.new_child(egui::UiBuilder::new().max_rect(first_rect));
                    draw_node(&mut first_ui, app, first);

                    let mut second_ui = ui.new_child(egui::UiBuilder::new().max_rect(second_rect));
                    draw_node(&mut second_ui, app, second);
                }
            }
        }
    }
}

fn draw_pane_panel(ui: &mut Ui, app: &mut App, pane_id: PaneId) {
    let is_focused = app.focused_pane == pane_id;
    let is_dark = ui.visuals().dark_mode;
    let pane_bg = if is_dark { OD_BG } else { OL_BG };
    let border_color = if is_dark { OD_BG_HL } else { OL_BORDER };

    let frame = egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, border_color))
        .inner_margin(6.0)
        .fill(pane_bg);

    let frame_resp = frame.show(ui, |ui| {
        if is_focused {
            let rect = ui.max_rect();
            let indicator_color = if is_dark { OD_CYAN } else { OL_BLUE };
            let top_line = egui::Rect::from_min_max(
                rect.left_top(),
                egui::pos2(rect.right(), rect.top() + 2.0),
            );
            ui.painter().rect_filled(top_line, 0.0, indicator_color);
        }

        ui.push_id(pane_id, |ui| {
            toolbar::draw_toolbar(ui, app, pane_id);
            toolbar::draw_tag_bar(ui, app, pane_id);
            ui.add_space(2.0);
            ui.separator();
            ui.add_space(2.0);
            search::draw_search_bar(ui, app, pane_id);
            log_area::draw_log_area(ui, pane_id, app);
        });
    });

    let pane_rect = frame_resp.response.rect;
    let clicked_in_pane = ui.input(|i| {
        i.pointer.primary_clicked()
            && i.pointer
                .interact_pos()
                .is_some_and(|pos| pane_rect.contains(pos))
    });
    if clicked_in_pane {
        app.focused_pane = pane_id;
    }
}
