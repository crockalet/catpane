use egui::{self, Key, RichText, ScrollArea, Ui};

use super::theme::*;
use crate::app::App;
use crate::pane::PaneId;
use catpane_core::log_entry::LogLevel;

pub fn draw_toolbar(ui: &mut Ui, app: &mut App, pane_id: PaneId) {
    let is_dark = ui.visuals().dark_mode;
    let toolbar_bg = if is_dark { OD_BG_LIGHT } else { OL_BG_LIGHT };
    let mut pending_device_selection: Option<String> = None;

    let toolbar_frame = egui::Frame::new()
        .fill(toolbar_bg)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .corner_radius(4.0);

    toolbar_frame.show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            let pane = match app.panes.get_mut(&pane_id) {
                Some(p) => p,
                None => return,
            };

            let device_label = pane
                .device
                .as_ref()
                .and_then(|device_id| {
                    app.devices
                        .iter()
                        .find(|device| device.id == *device_id)
                        .map(|device| device.display_name())
                })
                .unwrap_or_else(|| "No device".to_string());

            let selected_device = pane
                .device
                .as_ref()
                .and_then(|device_id| app.devices.iter().find(|device| device.id == *device_id))
                .cloned();

            ui.label(RichText::new(egui_phosphor::regular::DEVICE_MOBILE).size(14.0));
            egui::ComboBox::from_id_salt("device_combo")
                .selected_text(&device_label)
                .width(180.0)
                .show_ui(ui, |ui| {
                    for device in &app.devices {
                        let is_selected = pane.device.as_ref() == Some(&device.id);
                        if ui
                            .selectable_label(is_selected, device.display_name())
                            .clicked()
                        {
                            pending_device_selection = Some(device.id.clone());
                        }
                    }
                    ui.separator();
                    if ui.button(format!("{} Refresh devices", egui_phosphor::regular::ARROW_CLOCKWISE)).clicked() {
                        app.device_refresh_pending = true;
                    }
                });

            ui.separator();

            if selected_device
                .as_ref()
                .is_some_and(|device| device.supports_package_filter())
            {
                ui.label(RichText::new(egui_phosphor::regular::PACKAGE).size(14.0));
                let pkg_hint = pane.filter.package.as_deref().unwrap_or("Package...");
                let pkg_resp = ui.add(
                    egui::TextEdit::singleline(&mut pane.package_filter_text)
                        .desired_width(120.0)
                        .hint_text(pkg_hint)
                        .id(ui.id().with("pkg_input")),
                );

                if pkg_resp.gained_focus() && pane.packages.is_empty() && pane.device.is_some() {
                    pane.package_refresh_pending = true;
                    pane.pkg_selection_index = -1;
                }

                if pkg_resp.changed() {
                    pane.pkg_selection_index = 0;
                    if pane.package_filter_text.is_empty() && pane.filter.package.is_some() {
                        pane.filter.package = None;
                        pane.pid_filter = None;
                        pane.rebuild_filtered();
                    }
                }

                let popup_id = ui.id().with("pkg_popup");
                let has_focus = pkg_resp.has_focus();
                let just_lost_focus = pkg_resp.lost_focus();
                let filter_text = pane.package_filter_text.to_lowercase();

                let matching: Vec<String> = if filter_text.is_empty() {
                    pane.packages.iter().take(20).cloned().collect()
                } else {
                    pane.packages
                        .iter()
                        .filter(|p| p.to_lowercase().contains(&filter_text))
                        .take(15)
                        .cloned()
                        .collect()
                };

                if has_focus && !matching.is_empty() {
                    let pressed_down = ui.input(|i| i.key_pressed(Key::ArrowDown));
                    let pressed_up = ui.input(|i| i.key_pressed(Key::ArrowUp));

                    if pressed_down {
                        pane.pkg_selection_index =
                            ((pane.pkg_selection_index + 1) as usize % matching.len()) as i32;
                    }
                    if pressed_up {
                        if pane.pkg_selection_index <= 0 {
                            pane.pkg_selection_index = matching.len() as i32 - 1;
                        } else {
                            pane.pkg_selection_index -= 1;
                        }
                    }
                }

                if just_lost_focus
                    && ui.input(|i| i.key_pressed(Key::Enter))
                    && !matching.is_empty()
                {
                    let idx = if pane.pkg_selection_index >= 0 {
                        pane.pkg_selection_index as usize
                    } else {
                        0
                    };
                    if idx < matching.len() {
                        let pkg = matching[idx].clone();
                        pane.filter.package = Some(pkg.clone());
                        pane.package_filter_text = pkg;
                        pane.pkg_selection_index = -1;
                        pane.package_refresh_pending = true;
                    }
                }

                let mut pkg_cleared = false;
                if has_focus && !matching.is_empty() {
                    let sel_idx = pane.pkg_selection_index;
                    egui::popup_below_widget(
                        ui,
                        popup_id,
                        &pkg_resp,
                        egui::PopupCloseBehavior::CloseOnClickOutside,
                        |ui| {
                            ui.set_min_width(250.0);
                            ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        false,
                                        RichText::new(format!("{} Clear filter", egui_phosphor::regular::CROSS)).color(OD_FG_DIM),
                                    )
                                    .clicked()
                                {
                                    pane.filter.package = None;
                                    pane.pid_filter = None;
                                    pane.package_filter_text.clear();
                                    pane.pkg_selection_index = -1;
                                    pane.rebuild_filtered();
                                    pkg_cleared = true;
                                }
                                ui.separator();
                                for (i, pkg) in matching.iter().enumerate() {
                                    let is_kb_selected = sel_idx >= 0 && i == sel_idx as usize;
                                    let is_current = pane.filter.package.as_ref() == Some(pkg);
                                    let label = if is_kb_selected {
                                        RichText::new(pkg).strong()
                                    } else {
                                        RichText::new(pkg)
                                    };
                                    if ui
                                        .selectable_label(is_current || is_kb_selected, label)
                                        .clicked()
                                    {
                                        pane.filter.package = Some(pkg.clone());
                                        pane.package_filter_text = pkg.clone();
                                        pane.pkg_selection_index = -1;
                                        pane.package_refresh_pending = true;
                                    }
                                }
                            });
                        },
                    );
                    if pkg_cleared {
                        ui.memory_mut(|m| m.close_popup());
                        pkg_resp.surrender_focus();
                    } else {
                        ui.memory_mut(|m| m.open_popup(popup_id));
                    }
                }

                if pkg_resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    pane.filter.package = None;
                    pane.pid_filter = None;
                    pane.package_filter_text.clear();
                    pane.pkg_selection_index = -1;
                    pane.rebuild_filtered();
                    pkg_resp.surrender_focus();
                }
            } else if selected_device
                .as_ref()
                .is_some_and(|device| device.supports_ios_filters())
            {
                ui.label(RichText::new(egui_phosphor::regular::GEAR).size(14.0));
                let process_resp = ui.add(
                    egui::TextEdit::singleline(&mut pane.ios_process_filter_text)
                        .desired_width(120.0)
                        .hint_text("Process"),
                );
                let subsystem_resp = ui.add(
                    egui::TextEdit::singleline(&mut pane.ios_subsystem_filter_text)
                        .desired_width(140.0)
                        .hint_text("Subsystem"),
                );
                let category_resp = ui.add(
                    egui::TextEdit::singleline(&mut pane.ios_category_filter_text)
                        .desired_width(120.0)
                        .hint_text("Category"),
                );
                if process_resp.changed() || subsystem_resp.changed() || category_resp.changed() {
                    pane.apply_ios_filters();
                }
            }

            ui.separator();

            // Log level selector
            let current_level = pane.filter.min_level;
            egui::ComboBox::from_id_salt("level_combo")
                .selected_text(
                    RichText::new(format!("≥ {}", current_level.label()))
                        .color(current_level.color()),
                )
                .width(80.0)
                .show_ui(ui, |ui| {
                    for level in LogLevel::ALL {
                        if ui
                            .selectable_label(
                                pane.filter.min_level == level,
                                RichText::new(level.label()).color(level.color()),
                            )
                            .clicked()
                        {
                            pane.filter.min_level = level;
                            pane.rebuild_filtered();
                        }
                    }
                });

            // Right-aligned: entry count + scroll-to-bottom + pause + clear + word wrap
            ui.separator();

            let total = pane.entries.len();
            let filtered = pane.filtered_indices.len();
            let count_text = if filtered == total {
                format!("{total} lines")
            } else {
                format!("{filtered} of {total}")
            };
            ui.label(
                RichText::new(count_text)
                    .size(11.0)
                    .color(OD_FG_DIM)
                    .monospace(),
            );

            ui.separator();

            // Scroll-to-bottom / follow
            let follow_color = if pane.auto_scroll {
                if is_dark { OD_CYAN } else { OL_BLUE }
            } else {
                if is_dark { OD_FG_DIM } else { OL_FG_DIM }
            };
            if ui
                .add(egui::Button::new(
                    RichText::new(egui_phosphor::regular::ARROW_LINE_DOWN).size(15.0).color(follow_color),
                ))
                .on_hover_text(if pane.auto_scroll {
                    "Following logs (click to stop)"
                } else {
                    "Scroll to bottom & follow"
                })
                .clicked()
            {
                if pane.auto_scroll {
                    pane.auto_scroll = false;
                } else {
                    pane.auto_scroll = true;
                    pane.scroll_to_bottom = true;
                }
            }

            // Word wrap toggle
            let wrap_color = if pane.word_wrap {
                if is_dark { OD_CYAN } else { OL_BLUE }
            } else {
                if is_dark { OD_FG_DIM } else { OL_FG_DIM }
            };
            if ui
                .add(egui::Button::new(
                    RichText::new(egui_phosphor::regular::KEY_RETURN).size(15.0).color(wrap_color),
                ))
                .on_hover_text(if pane.word_wrap {
                    "Word wrap on (click to disable)"
                } else {
                    "Word wrap off (click to enable)"
                })
                .clicked()
            {
                pane.word_wrap = !pane.word_wrap;
                // Reset scroll state — the offset from one mode is invalid for the other
                // (wrap mode has variable-height rows with a 5k cap; no-wrap uses fixed-height
                // virtualized rows over the full entry set).
                pane.scroll_to_bottom = true;
                pane.scroll_offset_y = 0.0;
            }

            // Pause / resume
            let pause_icon = if pane.paused {
                RichText::new(egui_phosphor::regular::PLAY).size(15.0).color(OD_GREEN)
            } else {
                RichText::new(egui_phosphor::regular::PAUSE).size(15.0)
            };
            if ui
                .add(egui::Button::new(pause_icon))
                .on_hover_text(if pane.paused { "Resume" } else { "Pause" })
                .clicked()
            {
                pane.paused = !pane.paused;
            }

            // Clear
            if ui
                .add(egui::Button::new(RichText::new(egui_phosphor::regular::TRASH).size(15.0)))
                .on_hover_text("Clear logs")
                .clicked()
            {
                pane.clear();
            }


        });
    });

    if let Some(device_id) = pending_device_selection {
        app.set_pane_device(pane_id, Some(device_id));
    }
}

pub fn draw_tag_bar(ui: &mut Ui, app: &mut App, pane_id: PaneId) {
    let is_dark = ui.visuals().dark_mode;
    let bar_bg = if is_dark { OD_BG_LIGHT } else { OL_BG_LIGHT };

    let bar_frame = egui::Frame::new()
        .fill(bar_bg)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .corner_radius(4.0);

    let mut save_expr: Option<String> = None;
    let mut apply_history: Option<String> = None;
    let mut clear_tags = false;
    let mut apply_suggestion: Option<String> = None;

    bar_frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(24.0);
            ui.spacing_mut().item_spacing.x = 6.0;

            ui.label(RichText::new(egui_phosphor::regular::TAG).size(13.0));

            let (mut tag_input, has_filters, seen_tags) = {
                let pane = match app.panes.get(&pane_id) {
                    Some(p) => p,
                    None => return,
                };
                (
                    pane.tag_input.clone(),
                    !pane.filter.tag_filters.is_empty(),
                    pane.seen_tags.clone(),
                )
            };

            let prev_tag = app
                .panes
                .get(&pane_id)
                .map(|p| p.prev_tag_input.clone())
                .unwrap_or_default();

            let tag_resp = ui.add(
                egui::TextEdit::singleline(&mut tag_input)
                    .desired_width(ui.available_width() - 100.0)
                    .hint_text("tag:Name  tag-:Exclude  tag~:Regex  Name:V *:E")
                    .id(ui.id().with("tag_input")),
            );

            let text_changed = tag_input != prev_tag;
            if let Some(pane) = app.panes.get_mut(&pane_id) {
                pane.tag_input = tag_input.clone();
                if text_changed {
                    pane.prev_tag_input = tag_input.clone();
                    pane.tag_suggestion_index = 0;
                    pane.apply_tag_filter();
                }
            }

            let suggestion_popup_id = ui.id().with("tag_suggest_popup");
            let tag_has_focus = tag_resp.has_focus();
            let tag_just_lost_focus = tag_resp.lost_focus();

            let suggestions_list: Vec<String> = if !tag_input.is_empty() {
                let partial = extract_partial_tag_value(&tag_input);
                if !partial.is_empty() {
                    let partial_lower = partial.to_lowercase();
                    seen_tags
                        .iter()
                        .filter(|t| {
                            t.to_lowercase().contains(&partial_lower)
                                && t.to_lowercase() != partial_lower
                        })
                        .take(10)
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            if tag_has_focus && !suggestions_list.is_empty() {
                let pane = app.panes.get_mut(&pane_id).unwrap();
                let pressed_down = ui.input(|i| i.key_pressed(Key::ArrowDown));
                let pressed_up = ui.input(|i| i.key_pressed(Key::ArrowUp));

                if pressed_down {
                    pane.tag_suggestion_index =
                        ((pane.tag_suggestion_index + 1) as usize % suggestions_list.len()) as i32;
                }
                if pressed_up {
                    if pane.tag_suggestion_index <= 0 {
                        pane.tag_suggestion_index = suggestions_list.len() as i32 - 1;
                    } else {
                        pane.tag_suggestion_index -= 1;
                    }
                }
            }

            if tag_just_lost_focus && ui.input(|i| i.key_pressed(Key::Enter)) {
                if !suggestions_list.is_empty() {
                    let idx = app
                        .panes
                        .get(&pane_id)
                        .map(|p| p.tag_suggestion_index)
                        .unwrap_or(0);
                    let sel = if idx >= 0 && (idx as usize) < suggestions_list.len() {
                        idx as usize
                    } else {
                        0
                    };
                    apply_suggestion = Some(suggestions_list[sel].clone());
                    if let Some(pane) = app.panes.get_mut(&pane_id) {
                        pane.tag_suggestion_index = -1;
                    }
                } else {
                    save_expr = Some(tag_input.clone());
                }
            }

            if tag_has_focus && !suggestions_list.is_empty() {
                let sel_idx = app
                    .panes
                    .get(&pane_id)
                    .map(|p| p.tag_suggestion_index)
                    .unwrap_or(-1);
                egui::popup_below_widget(
                    ui,
                    suggestion_popup_id,
                    &tag_resp,
                    egui::PopupCloseBehavior::CloseOnClickOutside,
                    |ui| {
                        ui.set_min_width(200.0);
                        for (i, tag) in suggestions_list.iter().enumerate() {
                            let is_kb_selected = sel_idx >= 0 && i == sel_idx as usize;
                            let label = if is_kb_selected {
                                RichText::new(tag).strong()
                            } else {
                                RichText::new(tag)
                            };
                            if ui.selectable_label(is_kb_selected, label).clicked() {
                                apply_suggestion = Some(tag.clone());
                            }
                        }
                    },
                );
                ui.memory_mut(|m| m.open_popup(suggestion_popup_id));
            }

            // History dropdown
            let history_popup_id = ui.id().with("tag_history_popup");
            let history_btn = ui
                .button(RichText::new(egui_phosphor::regular::CARET_DOWN).size(13.0))
                .on_hover_text("Tag history");
            if history_btn.clicked() {
                ui.memory_mut(|m| m.toggle_popup(history_popup_id));
            }

            let history_clone = app.tag_history.clone();
            egui::popup_below_widget(
                ui,
                history_popup_id,
                &history_btn,
                egui::PopupCloseBehavior::CloseOnClickOutside,
                |ui| {
                    ui.set_min_width(300.0);
                    ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                        if history_clone.is_empty() {
                            ui.label(RichText::new("No history yet").color(OD_FG_DIM).size(12.0));
                        } else {
                            for entry in &history_clone {
                                if ui.selectable_label(false, entry).clicked() {
                                    apply_history = Some(entry.clone());
                                }
                            }
                        }
                    });
                },
            );

            let show_clear = !tag_input.is_empty() || has_filters;
            if show_clear {
                if ui
                    .small_button(RichText::new(egui_phosphor::regular::CROSS).size(12.0))
                    .on_hover_text("Clear tag filters")
                    .clicked()
                {
                    clear_tags = true;
                }
            }
        });
    });

    // Apply deferred actions
    if let Some(expr) = save_expr {
        app.save_tag_to_history(&expr);
    }
    if let Some(hist) = apply_history {
        if let Some(pane) = app.panes.get_mut(&pane_id) {
            pane.tag_input = hist;
            pane.prev_tag_input = pane.tag_input.clone();
            pane.apply_tag_filter();
        }
    }
    if clear_tags {
        if let Some(pane) = app.panes.get_mut(&pane_id) {
            pane.tag_input.clear();
            pane.prev_tag_input.clear();
            pane.filter.tag_filters.clear();
            pane.rebuild_filtered();
        }
    }
    if let Some(suggested_tag) = apply_suggestion {
        if let Some(pane) = app.panes.get_mut(&pane_id) {
            replace_partial_tag_value(&mut pane.tag_input, &suggested_tag);
            pane.prev_tag_input = pane.tag_input.clone();
            pane.apply_tag_filter();
        }
    }
}

fn extract_partial_tag_value(input: &str) -> &str {
    if let Some((start, end)) = current_tag_value_range(input) {
        &input[start..end]
    } else {
        ""
    }
}

fn replace_partial_tag_value(input: &mut String, replacement: &str) {
    let Some((start, end)) = current_tag_value_range(input) else {
        return;
    };
    let suffix = input[end..].to_string();
    let append_level_separator = should_append_level_separator(input);

    input.truncate(start);
    input.push_str(replacement);
    input.push_str(&suffix);
    if append_level_separator {
        input.push(':');
    }
}

fn current_tag_value_range(input: &str) -> Option<(usize, usize)> {
    let token_start = input
        .rfind(char::is_whitespace)
        .map_or(0, |pos| pos.saturating_add(1));
    let token = &input[token_start..];
    if token.is_empty() {
        return None;
    }

    let prefixes = ["tag-:", "tag~:", "tag:"];
    for prefix in &prefixes {
        if token.starts_with(prefix) {
            let start = token_start + prefix.len();
            return (start < input.len()).then_some((start, input.len()));
        }
    }

    let tag_end = token.find(':').unwrap_or(token.len());
    let tag = &token[..tag_end];
    if tag.is_empty() || tag == "*" {
        None
    } else {
        Some((token_start, token_start + tag.len()))
    }
}

fn should_append_level_separator(input: &str) -> bool {
    let token_start = input
        .rfind(char::is_whitespace)
        .map_or(0, |pos| pos.saturating_add(1));
    let token = &input[token_start..];

    !token.is_empty()
        && !token.contains(':')
        && !["tag-:", "tag~:", "tag:"]
            .iter()
            .any(|prefix| token.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::{extract_partial_tag_value, replace_partial_tag_value};

    #[test]
    fn extracts_partial_value_for_tag_level_token() {
        assert_eq!(extract_partial_tag_value("CallMan:V"), "CallMan");
        assert_eq!(extract_partial_tag_value("*:E"), "");
    }

    #[test]
    fn preserves_level_suffix_when_autocompleting_tag_level_token() {
        let mut input = String::from("CallMan:V");
        replace_partial_tag_value(&mut input, "CallManagerService");
        assert_eq!(input, "CallManagerService:V");
    }

    #[test]
    fn appends_level_separator_for_bare_tag_level_completion() {
        let mut input = String::from("CallMan");
        replace_partial_tag_value(&mut input, "CallManagerService");
        assert_eq!(input, "CallManagerService:");
    }

    #[test]
    fn keeps_existing_prefixed_tag_autocomplete_behavior() {
        let mut input = String::from("tag:CallMan");
        replace_partial_tag_value(&mut input, "CallManagerService");
        assert_eq!(input, "tag:CallManagerService");
    }
}
