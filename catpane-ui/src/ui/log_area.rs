use egui::{self, Color32, RichText, ScrollArea, Ui, Vec2};

use super::theme::*;
use crate::app::App;
use crate::pane::PaneId;

enum TagActionKind {
    Include,
    Exclude,
    Like,
}

struct TagAction {
    pane_id: PaneId,
    tag: String,
    kind: TagActionKind,
}

// In wrap mode, render at most this many rows (most recent) for performance.
const MAX_WRAP_ROWS: usize = 5_000;

pub fn draw_log_area(ui: &mut Ui, pane_id: PaneId, app: &mut App) {
    let pane = match app.panes.get_mut(&pane_id) {
        Some(p) => p,
        None => return,
    };

    let total_rows = pane.filtered_indices.len();
    let is_dark = ui.visuals().dark_mode;
    let word_wrap = pane.word_wrap;

    let search_highlight_fi = if pane.search_open && !pane.search_match_indices.is_empty() {
        Some(
            pane.search_match_indices
                .get(pane.search_current)
                .copied()
                .unwrap_or(0),
        )
    } else {
        None
    };

    let wants_scroll_to_bottom = pane.scroll_to_bottom;
    pane.scroll_to_bottom = false;
    let scroll_to_fi = pane.scroll_to_fi.take();
    let stored_offset = pane.scroll_offset_y;

    let tag_color = if is_dark { OD_BLUE } else { OL_BLUE };
    let ts_color = if is_dark { OD_FG_DIM } else { OL_FG_DIM };
    let msg_color = if is_dark { OD_FG } else { OL_FG };

    let mut pending_tag_action: Option<TagAction> = None;

    if word_wrap {
        // ── Wrap mode: variable-height rows via LayoutJob ──────────────────
        let wrap_start = total_rows.saturating_sub(MAX_WRAP_ROWS);

        let mut target_offset = stored_offset;
        if scroll_to_fi.is_none() && wants_scroll_to_bottom {
            target_offset = f32::MAX;
        }

        let scroll_area = ScrollArea::vertical()
            .id_salt("log_scroll_wrap")
            .auto_shrink([false, false])
            .stick_to_bottom(false)
            .animated(false)
            .vertical_scroll_offset(target_offset);

        let output = scroll_area.show(ui, |ui| {
            if wrap_start > 0 {
                ui.label(
                    RichText::new(format!(
                        "↑ {} older entries hidden in wrap mode",
                        wrap_start
                    ))
                    .size(11.0)
                    .color(ts_color),
                );
            }

            for fi in wrap_start..total_rows {
                if fi >= pane.filtered_indices.len() {
                    break;
                }
                let ei = pane.filtered_indices[fi];
                if ei >= pane.entries.len() {
                    continue;
                }
                let entry = &pane.entries[ei];

                let is_search_current = search_highlight_fi == Some(fi);
                let is_search_match = pane.search_open && pane.filter.matches_search(entry);
                let is_selected = pane.is_row_selected(fi);

                let bg_color: Option<Color32> = if is_search_current {
                    Some(if is_dark {
                        Color32::from_rgb(100, 90, 0)
                    } else {
                        Color32::from_rgb(255, 240, 150)
                    })
                } else if is_selected {
                    Some(if is_dark { OD_BG_HL } else { OL_BG_HL })
                } else if is_search_match {
                    Some(if is_dark {
                        Color32::from_rgb(50, 45, 20)
                    } else {
                        Color32::from_rgb(255, 250, 220)
                    })
                } else {
                    None
                };

                let fmt = |color: Color32| egui::text::TextFormat {
                    font_id: egui::FontId::monospace(12.0),
                    color,
                    background: bg_color.unwrap_or(Color32::TRANSPARENT),
                    ..Default::default()
                };

                let mut job = egui::text::LayoutJob::default();
                job.append(&format!("{} ", entry.timestamp), 0.0, fmt(ts_color));
                job.append(
                    &format!("{} ", entry.level.as_char()),
                    0.0,
                    fmt(entry.level.color()),
                );
                job.append(&format!("{} ", entry.tag), 0.0, fmt(tag_color));
                job.append(&entry.message, 0.0, fmt(msg_color));
                job.wrap.max_width = ui.available_width() - 8.0;

                let row_resp = ui
                    .add(egui::Label::new(job).sense(egui::Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::Default);

                if scroll_to_fi == Some(fi) {
                    row_resp.scroll_to_me(Some(egui::Align::Center));
                }

                if row_resp.clicked() {
                    let shift = ui.input(|i| i.modifiers.shift);
                    if shift && pane.selection_anchor.is_some() {
                        pane.selection_end = Some(fi);
                    } else {
                        pane.selection_anchor = Some(fi);
                        pane.selection_end = None;
                    }
                }

                let entry_tag = entry.tag.clone();
                let entry_line = format!(
                    "{} {} {} {}",
                    entry.timestamp,
                    entry.level.as_char(),
                    entry.tag,
                    entry.message
                );
                let entry_msg = entry.message.clone();
                row_resp.context_menu(|ui| {
                    build_context_menu(
                        ui,
                        pane_id,
                        fi,
                        &pane.filtered_indices,
                        &pane.entries,
                        &pane.selection_anchor,
                        &pane.selection_end,
                        &entry_tag,
                        &entry_line,
                        &entry_msg,
                        &mut pending_tag_action,
                    );
                });
            }
        });
        pane.scroll_offset_y = output.state.offset.y;
    } else {
        // ── No-wrap mode: fixed-height virtualized rows ─────────────────────
        let mut target_offset = stored_offset;
        if let Some(fi) = scroll_to_fi {
            if fi < total_rows {
                let row_spacing = ui.spacing().item_spacing.y;
                let row_h = LOG_ROW_HEIGHT + row_spacing;
                let target_y = fi as f32 * row_h;
                let viewport_height = ui.available_height();
                target_offset = (target_y - viewport_height / 2.0).max(0.0);
            }
        } else if wants_scroll_to_bottom {
            let row_spacing = ui.spacing().item_spacing.y;
            let content_height = total_rows as f32 * (LOG_ROW_HEIGHT + row_spacing);
            let viewport_height = ui.available_height();
            target_offset = (content_height - viewport_height).max(0.0);
        }

        let scroll_area = ScrollArea::vertical()
            .id_salt("log_scroll")
            .auto_shrink([false, false])
            .stick_to_bottom(false)
            .animated(false)
            .vertical_scroll_offset(target_offset);

        let output = scroll_area.show_rows(ui, LOG_ROW_HEIGHT, total_rows, |ui, row_range| {
            for fi in row_range {
                if fi >= pane.filtered_indices.len() {
                    break;
                }
                let ei = pane.filtered_indices[fi];
                if ei >= pane.entries.len() {
                    continue;
                }
                let entry = &pane.entries[ei];

                let is_search_current = search_highlight_fi == Some(fi);
                let is_search_match = pane.search_open && pane.filter.matches_search(entry);
                let is_selected = pane.is_row_selected(fi);

                let bg_color = if is_search_current {
                    Some(if is_dark {
                        Color32::from_rgb(100, 90, 0)
                    } else {
                        Color32::from_rgb(255, 240, 150)
                    })
                } else if is_selected {
                    Some(if is_dark { OD_BG_HL } else { OL_BG_HL })
                } else if is_search_match {
                    Some(if is_dark {
                        Color32::from_rgb(50, 45, 20)
                    } else {
                        Color32::from_rgb(255, 250, 220)
                    })
                } else {
                    None
                };

                let (row_rect, row_resp) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), LOG_ROW_HEIGHT),
                    egui::Sense::click(),
                );

                if let Some(bg) = bg_color {
                    ui.painter().rect_filled(row_rect, 0.0, bg);
                }

                if row_resp.clicked() {
                    let shift = ui.input(|i| i.modifiers.shift);
                    if shift && pane.selection_anchor.is_some() {
                        pane.selection_end = Some(fi);
                    } else {
                        pane.selection_anchor = Some(fi);
                        pane.selection_end = None;
                    }
                }

                let mut text_pos = row_rect.min + Vec2::new(4.0, 1.0);
                let painter = ui
                    .painter()
                    .with_clip_rect(row_rect.intersect(ui.clip_rect()));

                let ts_galley = painter.layout_no_wrap(
                    entry.timestamp.clone(),
                    egui::FontId::monospace(12.0),
                    ts_color,
                );
                painter.galley(text_pos, ts_galley.clone(), Color32::TRANSPARENT);
                text_pos.x += ts_galley.size().x + 8.0;

                let level_galley = painter.layout_no_wrap(
                    entry.level.as_char().to_string(),
                    egui::FontId::monospace(12.0),
                    entry.level.color(),
                );
                painter.galley(text_pos, level_galley.clone(), Color32::TRANSPARENT);
                text_pos.x += level_galley.size().x + 8.0;

                let tag_galley = painter.layout_no_wrap(
                    entry.tag.clone(),
                    egui::FontId::monospace(12.0),
                    tag_color,
                );
                painter.galley(text_pos, tag_galley.clone(), Color32::TRANSPARENT);
                text_pos.x += tag_galley.size().x + 8.0;

                let msg_galley = painter.layout_no_wrap(
                    entry.message.clone(),
                    egui::FontId::monospace(12.0),
                    msg_color,
                );
                painter.galley(text_pos, msg_galley, Color32::TRANSPARENT);

                let entry_tag = entry.tag.clone();
                let entry_line = format!(
                    "{} {} {} {}",
                    entry.timestamp,
                    entry.level.as_char(),
                    entry.tag,
                    entry.message
                );
                let entry_msg = entry.message.clone();
                row_resp.context_menu(|ui| {
                    build_context_menu(
                        ui,
                        pane_id,
                        fi,
                        &pane.filtered_indices,
                        &pane.entries,
                        &pane.selection_anchor,
                        &pane.selection_end,
                        &entry_tag,
                        &entry_line,
                        &entry_msg,
                        &mut pending_tag_action,
                    );
                });
            }
        });
        pane.scroll_offset_y = output.state.offset.y;
    }

    // Detect user scrolling away from bottom
    {
        let user_scrolled_up =
            ui.input(|i| i.smooth_scroll_delta.y > 0.0 || i.raw_scroll_delta.y > 0.0);

        if let Some(pane) = app.panes.get_mut(&pane_id) {
            if user_scrolled_up && pane.auto_scroll {
                pane.auto_scroll = false;
            }
        }
    }

    // Apply pending tag action
    if let Some(action) = pending_tag_action {
        let prefix = match action.kind {
            TagActionKind::Include => "tag:",
            TagActionKind::Exclude => "tag-:",
            TagActionKind::Like => "tag~:",
        };
        let new_filter = format!("{}{}", prefix, action.tag);
        let full_expr = if let Some(pane) = app.panes.get_mut(&action.pane_id) {
            if pane.tag_input.is_empty() {
                pane.tag_input = new_filter;
            } else {
                pane.tag_input = format!("{} {}", pane.tag_input.trim(), new_filter);
            }
            pane.prev_tag_input = pane.tag_input.clone();
            pane.apply_tag_filter();
            Some(pane.tag_input.clone())
        } else {
            None
        };
        if let Some(expr) = full_expr {
            app.save_tag_to_history(&expr);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_context_menu(
    ui: &mut egui::Ui,
    pane_id: PaneId,
    fi: usize,
    filtered_indices: &[usize],
    entries: &[catpane_core::log_entry::LogEntry],
    selection_anchor: &Option<usize>,
    selection_end: &Option<usize>,
    entry_tag: &str,
    entry_line: &str,
    entry_msg: &str,
    pending_tag_action: &mut Option<TagAction>,
) {
    let sel_range = match (selection_anchor, selection_end) {
        (Some(a), Some(b)) => Some((*a.min(b), *a.max(b))),
        (Some(a), None) => Some((*a, *a)),
        _ => None,
    };
    if let Some((lo, hi)) = sel_range {
        if hi > lo {
            let count = hi - lo + 1;
            if ui
                .button(format!("📋 Copy {} selected lines", count))
                .clicked()
            {
                let lines: Vec<String> = (lo..=hi)
                    .filter_map(|f| {
                        filtered_indices
                            .get(f)
                            .and_then(|&e| entries.get(e))
                            .map(|e| {
                                format!(
                                    "{} {} {} {}",
                                    e.timestamp,
                                    e.level.as_char(),
                                    e.tag,
                                    e.message
                                )
                            })
                    })
                    .collect();
                ui.ctx().copy_text(lines.join("\n"));
                ui.close_menu();
            }
            ui.separator();
        }
    }
    if ui.button("📋 Copy line").clicked() {
        ui.ctx().copy_text(entry_line.to_string());
        ui.close_menu();
    }
    if ui.button("📋 Copy message").clicked() {
        ui.ctx().copy_text(entry_msg.to_string());
        ui.close_menu();
    }
    ui.separator();
    ui.label(
        RichText::new(format!("Tag: {}", entry_tag))
            .strong()
            .size(12.0),
    );
    ui.separator();
    if ui
        .button(format!("Include tag:\"{}\"", entry_tag))
        .clicked()
    {
        *pending_tag_action = Some(TagAction {
            pane_id,
            tag: entry_tag.to_string(),
            kind: TagActionKind::Include,
        });
        ui.close_menu();
    }
    if ui
        .button(format!("Exclude tag-:\"{}\"", entry_tag))
        .clicked()
    {
        *pending_tag_action = Some(TagAction {
            pane_id,
            tag: entry_tag.to_string(),
            kind: TagActionKind::Exclude,
        });
        ui.close_menu();
    }
    if ui.button(format!("Like tag~:\"{}\"", entry_tag)).clicked() {
        *pending_tag_action = Some(TagAction {
            pane_id,
            tag: entry_tag.to_string(),
            kind: TagActionKind::Like,
        });
        ui.close_menu();
    }
    let _ = fi; // suppress unused warning; fi used by caller for selection
}
