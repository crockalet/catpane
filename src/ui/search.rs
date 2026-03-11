use egui::{self, RichText, Ui};

use crate::app::App;
use crate::pane::PaneId;
use super::theme::*;

pub fn draw_search_bar(ui: &mut Ui, app: &mut App, pane_id: PaneId) {
    let pane = match app.panes.get_mut(&pane_id) {
        Some(p) => p,
        None => return,
    };
    if !pane.search_open { return; }

    let is_dark = ui.visuals().dark_mode;
    let search_bg = if is_dark { OD_BG_LIGHT } else { OL_BG_LIGHT };

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
                    .id(ui.id().with("search_input"))
            );
            if search_resp.changed() {
                pane.update_search();
            }

            let match_count = pane.search_match_indices.len();
            if match_count > 0 {
                ui.label(
                    RichText::new(format!("{}/{}", pane.search_current + 1, match_count))
                        .color(OD_FG_DIM).size(12.0)
                );
                if ui.small_button("▲").clicked() { pane.search_prev(); }
                if ui.small_button("▼").clicked() { pane.search_next(); }
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
