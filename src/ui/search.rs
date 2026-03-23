use egui::{self, RichText, Ui};

use super::theme::*;
use crate::app::App;
use crate::pane::PaneId;

pub fn draw_search_bar(ui: &mut Ui, app: &mut App, pane_id: PaneId) {
    let pane = match app.panes.get_mut(&pane_id) {
        Some(p) => p,
        None => return,
    };
    if !pane.search_open {
        return;
    }

    let is_dark = ui.visuals().dark_mode;
    let search_bg = if is_dark { OD_BG_LIGHT } else { OL_BG_LIGHT };

    let should_focus = pane.search_focus_requested;
    pane.search_focus_requested = false;

    let search_frame = egui::Frame::new()
        .fill(search_bg)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .corner_radius(4.0);

    search_frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(26.0);
            ui.label(RichText::new("🔍").size(14.0));

            let search_resp = ui.add(
                egui::TextEdit::singleline(&mut pane.search_input)
                    .desired_width(250.0)
                    .hint_text("Search logs...")
                    .id(ui.id().with("search_input")),
            );

            if should_focus {
                search_resp.request_focus();
            }

            if search_resp.changed() {
                pane.update_search();
                if let Some(&fi) = pane.search_match_indices.first() {
                    pane.scroll_to_fi = Some(fi);
                    pane.auto_scroll = false;
                }
            }

            // Enter / Shift+Enter: navigate matches while keeping focus
            if search_resp.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                let shift = ui.input(|i| i.modifiers.shift);
                let target = if shift {
                    pane.search_prev()
                } else {
                    pane.search_next()
                };
                if let Some(fi) = target {
                    pane.scroll_to_fi = Some(fi);
                    pane.auto_scroll = false;
                }
                search_resp.request_focus();
            }

            let match_count = pane.search_match_indices.len();
            if match_count > 0 {
                ui.label(
                    RichText::new(format!("{}/{}", pane.search_current + 1, match_count))
                        .color(OD_FG_DIM)
                        .size(12.0),
                );
                if ui.small_button("▲").clicked() {
                    if let Some(fi) = pane.search_prev() {
                        pane.scroll_to_fi = Some(fi);
                        pane.auto_scroll = false;
                    }
                }
                if ui.small_button("▼").clicked() {
                    if let Some(fi) = pane.search_next() {
                        pane.scroll_to_fi = Some(fi);
                        pane.auto_scroll = false;
                    }
                }
            } else if !pane.search_input.is_empty() {
                ui.label(RichText::new("no matches").color(OD_RED).size(12.0));
            }

            if ui.small_button("✕").clicked() {
                pane.search_open = false;
                pane.search_input.clear();
                pane.filter.set_search("");
                pane.search_match_indices.clear();
            }
        });
    });
    ui.add_space(2.0);
}
