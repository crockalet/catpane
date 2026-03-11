mod theme;
mod toolbar;
mod search;
mod log_area;
mod dialogs;

use egui::{self, Key, Ui, Vec2};

use crate::app::App;
use crate::pane::{PaneId, PaneNode, SplitDir};
use theme::*;

pub use theme::configure_fonts;

pub fn draw_ui(ctx: &egui::Context, app: &mut App) {
    ctx.request_repaint();
    app.poll_all();

    let tab = ctx.input(|i| i.key_pressed(Key::Tab) && !i.modifiers.command && !i.modifiers.ctrl);
    if tab { app.cycle_focus(); }

    egui::CentralPanel::default().show(ctx, |ui| {
        let tree_ids = app.pane_tree.pane_ids();
        if tree_ids.is_empty() { return; }
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
    Split { dir: SplitDir, first: Box<TreeSnap>, second: Box<TreeSnap> },
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

fn draw_node(ui: &mut Ui, app: &mut App, snap: &TreeSnap) {
    match snap {
        TreeSnap::Leaf(id) => {
            draw_pane_panel(ui, app, *id);
        }
        TreeSnap::Split { dir, first, second } => {
            match dir {
                SplitDir::Vertical => {
                    ui.columns(2, |cols| {
                        draw_node(&mut cols[0], app, first);
                        draw_node(&mut cols[1], app, second);
                    });
                }
                SplitDir::Horizontal => {
                    let available = ui.available_height();
                    let half = available / 2.0;
                    ui.allocate_ui(Vec2::new(ui.available_width(), half), |ui| {
                        draw_node(ui, app, first);
                    });
                    ui.allocate_ui(Vec2::new(ui.available_width(), half), |ui| {
                        draw_node(ui, app, second);
                    });
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
            && i.pointer.interact_pos().is_some_and(|pos| pane_rect.contains(pos))
    });
    if clicked_in_pane {
        app.focused_pane = pane_id;
    }
}
