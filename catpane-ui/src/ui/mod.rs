mod dialogs;
mod log_area;
mod search;
mod sidebar;
mod theme;
mod toolbar;

use egui::{self, Key, Ui, Vec2};

use crate::app::App;
use crate::pane::{PaneId, PaneNode, SplitDir};
use sidebar::draw_sidebar;
use theme::*;

pub use theme::configure_fonts;

const DIVIDER_SIZE: f32 = 5.0;
const PANEL_PADDING: f32 = 4.0;

pub fn draw_ui(ctx: &egui::Context, app: &mut App) {
    let tab = ctx.input(|i| i.key_pressed(Key::Tab) && !i.modifiers.command && !i.modifiers.ctrl);
    if tab {
        app.cycle_focus();
    }

    app.poll_qr_pairing();
    draw_sidebar(ctx, app);

    let backdrop = if ctx.style().visuals.dark_mode {
        OD_BG_BACKDROP
    } else {
        OL_BG_BACKDROP
    };

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(backdrop))
        .show(ctx, |ui| {
            let panel_rect = ui.max_rect();
            let padded_rect = egui::Rect::from_min_max(
                egui::pos2(
                    panel_rect.min.x + PANEL_PADDING,
                    panel_rect.min.y + PANEL_PADDING,
                ),
                egui::pos2(
                    panel_rect.max.x - PANEL_PADDING,
                    panel_rect.max.y - PANEL_PADDING,
                ),
            );

            let mut padded_ui = ui.new_child(egui::UiBuilder::new().max_rect(padded_rect));
            padded_ui.set_clip_rect(padded_rect);

            let tree_ids = app.pane_tree.pane_ids();
            if tree_ids.is_empty() {
                return;
            }
            draw_pane_tree(&mut padded_ui, app, &tree_ids);
        });

    if app.show_help {
        dialogs::draw_help_window(ctx, &mut app.show_help);
    }
}

fn draw_pane_tree(ui: &mut Ui, app: &mut App, _ids: &[PaneId]) {
    let tree_rect = ui.available_rect_before_wrap();
    ui.allocate_rect(tree_rect, egui::Sense::hover());

    let mut tree_ui = ui.new_child(egui::UiBuilder::new().max_rect(tree_rect));
    tree_ui.set_clip_rect(tree_rect);
    if let Some(rect) = draw_node(&mut tree_ui, app, &clone_tree_structure(&app.pane_tree)) {
        paint_focus_ring(&tree_ui, rect);
    }
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

fn draw_node(ui: &mut Ui, app: &mut App, snap: &TreeSnap) -> Option<egui::Rect> {
    match snap {
        TreeSnap::Leaf(id) => {
            let pane_rect = ui.max_rect();
            let mut pane_ui = ui.new_child(egui::UiBuilder::new().max_rect(pane_rect));
            pane_ui.set_clip_rect(ui.clip_rect().intersect(pane_rect));
            draw_pane_panel(&mut pane_ui, app, *id)
        }
        TreeSnap::Split { dir, first, second } => {
            let split_key = egui::Id::new(("split_ratio", first_leaf_id(snap)));
            let ratio: f32 = ui.ctx().data(|d| d.get_temp(split_key).unwrap_or(0.5_f32));

            // Claim the full available rect so the parent layout advances correctly.
            let avail_rect = ui.available_rect_before_wrap();
            let avail = avail_rect.size();
            ui.allocate_rect(avail_rect, egui::Sense::hover());

            match dir {
                SplitDir::Vertical => {
                    let usable = avail.x.max(100.0);
                    let first_w = (usable * ratio).clamp(50.0, usable - 50.0);
                    let second_w = usable - first_w;

                    let first_rect =
                        egui::Rect::from_min_size(avail_rect.min, Vec2::new(first_w, avail.y));
                    let div_rect = egui::Rect::from_center_size(
                        avail_rect.min + Vec2::new(first_w, avail.y * 0.5),
                        Vec2::new(DIVIDER_SIZE, avail.y),
                    );
                    let second_rect = egui::Rect::from_min_size(
                        avail_rect.min + Vec2::new(first_w, 0.0),
                        Vec2::new(second_w, avail.y),
                    );

                    let div_id = ui.id().with(("vdiv", first_leaf_id(snap)));
                    let div_resp = ui.interact(div_rect, div_id, egui::Sense::drag());
                    if div_resp.hovered() || div_resp.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    if div_resp.dragged() {
                        if let Some(pointer) = div_resp.interact_pointer_pos() {
                            let new_first_w =
                                (pointer.x - avail_rect.min.x).clamp(50.0, usable - 50.0);
                            ui.ctx()
                                .data_mut(|d| d.insert_temp(split_key, new_first_w / usable));
                        }
                    }

                    let mut first_ui = ui.new_child(egui::UiBuilder::new().max_rect(first_rect));
                    first_ui.set_clip_rect(ui.clip_rect().intersect(first_rect));
                    let first_focused_rect = draw_node(&mut first_ui, app, first);

                    let mut second_ui = ui.new_child(egui::UiBuilder::new().max_rect(second_rect));
                    second_ui.set_clip_rect(ui.clip_rect().intersect(second_rect));
                    let second_focused_rect = draw_node(&mut second_ui, app, second);
                    let focused_rect = first_focused_rect.or(second_focused_rect);

                    ui.painter().rect_filled(div_rect, 0.0, split_gap_fill(ui));
                    if let Some(rect) = focused_rect {
                        paint_focus_ring(ui, rect);
                    }
                    focused_rect
                }
                SplitDir::Horizontal => {
                    let usable = avail.y.max(100.0);
                    let first_h = (usable * ratio).clamp(50.0, usable - 50.0);
                    let second_h = usable - first_h;

                    let first_rect =
                        egui::Rect::from_min_size(avail_rect.min, Vec2::new(avail.x, first_h));
                    let div_rect = egui::Rect::from_center_size(
                        avail_rect.min + Vec2::new(avail.x * 0.5, first_h),
                        Vec2::new(avail.x, DIVIDER_SIZE),
                    );
                    let second_rect = egui::Rect::from_min_size(
                        avail_rect.min + Vec2::new(0.0, first_h),
                        Vec2::new(avail.x, second_h),
                    );

                    let div_id = ui.id().with(("hdiv", first_leaf_id(snap)));
                    let div_resp = ui.interact(div_rect, div_id, egui::Sense::drag());
                    if div_resp.hovered() || div_resp.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                    if div_resp.dragged() {
                        if let Some(pointer) = div_resp.interact_pointer_pos() {
                            let new_first_h =
                                (pointer.y - avail_rect.min.y).clamp(50.0, usable - 50.0);
                            ui.ctx()
                                .data_mut(|d| d.insert_temp(split_key, new_first_h / usable));
                        }
                    }

                    let mut first_ui = ui.new_child(egui::UiBuilder::new().max_rect(first_rect));
                    first_ui.set_clip_rect(ui.clip_rect().intersect(first_rect));
                    let first_focused_rect = draw_node(&mut first_ui, app, first);

                    let mut second_ui = ui.new_child(egui::UiBuilder::new().max_rect(second_rect));
                    second_ui.set_clip_rect(ui.clip_rect().intersect(second_rect));
                    let second_focused_rect = draw_node(&mut second_ui, app, second);
                    let focused_rect = first_focused_rect.or(second_focused_rect);

                    ui.painter().rect_filled(div_rect, 0.0, split_gap_fill(ui));
                    if let Some(rect) = focused_rect {
                        paint_focus_ring(ui, rect);
                    }
                    focused_rect
                }
            }
        }
    }
}

fn split_gap_fill(ui: &Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        OD_BG_BACKDROP
    } else {
        OL_BG_BACKDROP
    }
}

fn draw_pane_panel(ui: &mut Ui, app: &mut App, pane_id: PaneId) -> Option<egui::Rect> {
    let is_focused = app.focused_pane == pane_id;
    let is_dark = ui.visuals().dark_mode;
    let pane_bg = if is_dark { OD_BG } else { OL_BG };
    let border_color = if is_dark { OD_BG_HL } else { OL_BORDER };

    let frame = egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, border_color))
        .corner_radius(8.0)
        .inner_margin(6.0)
        .fill(pane_bg);

    let frame_resp = frame.show(ui, |ui| {
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
    is_focused.then_some(pane_rect)
}

fn paint_focus_ring(ui: &Ui, pane_rect: egui::Rect) {
    let focus_color = if ui.visuals().dark_mode {
        OD_CYAN
    } else {
        OL_BLUE
    };
    ui.painter().rect_stroke(
        pane_rect,
        8.0,
        egui::Stroke::new(1.0, focus_color),
        egui::StrokeKind::Inside,
    );
}
