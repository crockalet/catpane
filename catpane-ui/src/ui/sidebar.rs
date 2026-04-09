use egui::{self, Align, Layout, RichText, ScrollArea, Ui};

use super::theme::*;
use crate::app::{App, DeviceLocationState, QrPairStatus, QrPairingState, SavedLocation, SidebarTab};
use crate::pane::WATCH_COLORS;
use catpane_core::capture::{ConnectedDevice, DevicePlatform};
use catpane_core::crash_detector::CrashType;

const SIDEBAR_CARD_LEFT_MARGIN: f32 = 4.0;
const SIDEBAR_CARD_VERTICAL_MARGIN: f32 = 4.0;
const SIDEBAR_CARD_INNER_MARGIN: f32 = 6.0;
const SIDEBAR_CARD_STROKE_WIDTH: f32 = 1.0;
const SIDEBAR_SECTION_GAP: f32 = 6.0;
const SIDEBAR_WIDTH: f32 = 300.0;
const SIDEBAR_MIN_WIDTH: f32 = 240.0;
const SIDEBAR_MAX_WIDTH: f32 = 420.0;
const SIDEBAR_RAIL_WIDTH: f32 = 40.0;
const SIDEBAR_RAIL_BUTTON_SIZE: f32 = 24.0;

pub fn draw_sidebar(ctx: &egui::Context, app: &mut App) {
    let sidebar_open = app.sidebar_open;
    let backdrop = if ctx.style().visuals.dark_mode {
        OD_BG_BACKDROP
    } else {
        OL_BG_BACKDROP
    };
    let mut panel = egui::SidePanel::left("device_manager_sidebar");
    panel = panel
        .frame(egui::Frame::new().fill(backdrop))
        .resizable(sidebar_open)
        .show_separator_line(false);

    if sidebar_open {
        panel = panel
            .default_width(sidebar_panel_width(
                SIDEBAR_RAIL_WIDTH + SIDEBAR_WIDTH,
                true,
            ))
            .min_width(sidebar_panel_width(
                SIDEBAR_RAIL_WIDTH + SIDEBAR_MIN_WIDTH,
                true,
            ))
            .max_width(sidebar_panel_width(
                SIDEBAR_RAIL_WIDTH + SIDEBAR_MAX_WIDTH,
                true,
            ));
    } else {
        panel = panel
            .default_width(sidebar_panel_width(SIDEBAR_RAIL_WIDTH, false))
            .min_width(sidebar_panel_width(SIDEBAR_RAIL_WIDTH, false))
            .max_width(sidebar_panel_width(SIDEBAR_RAIL_WIDTH, false));
    }

    panel.show(ctx, |ui| {
        let panel_rect = ui.max_rect();
        let card_rect = egui::Rect::from_min_max(
            egui::pos2(
                panel_rect.min.x + SIDEBAR_CARD_LEFT_MARGIN,
                panel_rect.min.y + SIDEBAR_CARD_VERTICAL_MARGIN,
            ),
            egui::pos2(
                panel_rect.max.x,
                panel_rect.max.y - SIDEBAR_CARD_VERTICAL_MARGIN,
            ),
        );

        let (card_fill, card_stroke) = sidebar_card_style(ui);
        ui.painter().rect(
            card_rect,
            8.0,
            card_fill,
            card_stroke,
            egui::StrokeKind::Inside,
        );

        let content_inset = SIDEBAR_CARD_INNER_MARGIN + SIDEBAR_CARD_STROKE_WIDTH;
        let content_rect = egui::Rect::from_min_max(
            egui::pos2(
                card_rect.min.x + content_inset,
                card_rect.min.y + content_inset,
            ),
            egui::pos2(
                card_rect.max.x - content_inset,
                card_rect.max.y - content_inset,
            ),
        );

        let mut card_ui = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
        card_ui.set_clip_rect(card_rect);
        card_ui.set_min_size(content_rect.size());
        let rail_rect = egui::Rect::from_min_max(
            content_rect.min,
            egui::pos2(content_rect.min.x + SIDEBAR_RAIL_WIDTH, content_rect.max.y),
        );
        let mut rail_ui = card_ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rail_rect)
                .layout(Layout::top_down(Align::Center)),
        );
        rail_ui.set_clip_rect(card_rect);
        draw_sidebar_rail(&mut rail_ui, app, sidebar_open);

        if sidebar_open {
            let separator_x = rail_rect.max.x + SIDEBAR_SECTION_GAP * 0.5;
            ui.painter()
                .vline(separator_x, content_rect.y_range(), separator_stroke(ui));

            let body_rect = egui::Rect::from_min_max(
                egui::pos2(rail_rect.max.x + SIDEBAR_SECTION_GAP, content_rect.min.y),
                content_rect.max,
            );
            let mut body_ui = card_ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(body_rect)
                    .layout(Layout::top_down(Align::Min)),
            );
            body_ui.set_clip_rect(card_rect);
            body_ui.set_min_size(body_rect.size());
            draw_sidebar_contents(&mut body_ui, app);
        }
        ui.expand_to_include_rect(card_rect);
    });
}

fn sidebar_panel_width(content_width: f32, include_gap: bool) -> f32 {
    content_width
        + if include_gap {
            SIDEBAR_SECTION_GAP
        } else {
            0.0
        }
        + SIDEBAR_CARD_LEFT_MARGIN
        + SIDEBAR_CARD_INNER_MARGIN * 2.0
        + SIDEBAR_CARD_STROKE_WIDTH * 2.0
}

fn sidebar_card_style(ui: &Ui) -> (egui::Color32, egui::Stroke) {
    let is_dark = ui.visuals().dark_mode;
    (
        if is_dark { OD_BG } else { OL_BG },
        egui::Stroke::new(
            SIDEBAR_CARD_STROKE_WIDTH,
            if is_dark { OD_BG_HL } else { OL_BORDER },
        ),
    )
}

fn separator_stroke(ui: &Ui) -> egui::Stroke {
    egui::Stroke::new(
        1.0,
        if ui.visuals().dark_mode {
            OD_BG_HL
        } else {
            OL_BORDER
        },
    )
}

fn draw_sidebar_rail(ui: &mut Ui, app: &mut App, sidebar_open: bool) {
    ui.add_space(8.0);

    let tabs: &[(SidebarTab, &str, &str)] = &[
        (SidebarTab::Devices, egui_phosphor::regular::DEVICE_MOBILE, "Devices"),
        (SidebarTab::Location, egui_phosphor::regular::MAP_PIN, "Location"),
        (SidebarTab::Crashes, egui_phosphor::regular::WARNING_CIRCLE, "Crashes"),
        (SidebarTab::Watches, egui_phosphor::regular::EYE, "Watches"),
    ];

    for &(tab, icon, tooltip) in tabs {
        let is_active = sidebar_open && app.sidebar_tab == tab;
        let button = egui::Button::new(RichText::new(icon).size(14.0)).selected(is_active);
        let response = ui
            .add_sized([SIDEBAR_RAIL_BUTTON_SIZE, SIDEBAR_RAIL_BUTTON_SIZE], button)
            .on_hover_text(tooltip);

        if response.clicked() {
            if !sidebar_open {
                app.sidebar_open = true;
                app.sidebar_tab = tab;
            } else if app.sidebar_tab == tab {
                app.sidebar_open = false;
            } else {
                app.sidebar_tab = tab;
            }
        }
        ui.add_space(2.0);
    }
}

fn draw_sidebar_contents(ui: &mut Ui, app: &mut App) {
    ui.set_width(ui.available_width());

    let focused_pane_number = app
        .pane_tree
        .pane_ids()
        .iter()
        .position(|pane_id| *pane_id == app.focused_pane)
        .map(|index| index + 1)
        .unwrap_or(1);

    let focused_device = app
        .panes
        .get(&app.focused_pane)
        .and_then(|pane| pane.device.as_ref())
        .and_then(|device_id| app.devices.iter().find(|device| device.id == *device_id))
        .map(|device| device.display_name())
        .unwrap_or_else(|| "No device selected".to_string());

    // Header with close button
    ui.horizontal(|ui| {
        ui.heading(tab_heading(app.sidebar_tab));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .small_button(egui_phosphor::regular::CARET_LEFT)
                .on_hover_text("Collapse sidebar")
                .clicked()
            {
                app.sidebar_open = false;
            }
        });
    });
    ui.label(
        RichText::new(format!(
            "Focused pane {focused_pane_number} · {focused_device}"
        ))
        .weak()
        .size(11.0),
    );
    ui.add_space(4.0);

    // Tab bar
    draw_tab_bar(ui, app);
    ui.add_space(4.0);
    ui.separator();

    // Tab content
    match app.sidebar_tab {
        SidebarTab::Devices => {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_devices_tab(ui, app);
                });
        }
        SidebarTab::Location => {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_location_tab(ui, app);
                });
        }
        SidebarTab::Crashes => {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_crashes_tab(ui, app);
                });
        }
        SidebarTab::Watches => {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_watches_tab(ui, app);
                });
        }
    }
}

fn tab_heading(tab: SidebarTab) -> &'static str {
    match tab {
        SidebarTab::Devices => "Devices",
        SidebarTab::Location => "Location",
        SidebarTab::Crashes => "Crashes",
        SidebarTab::Watches => "Watches",
    }
}

fn draw_tab_bar(ui: &mut Ui, app: &mut App) {
    let is_dark = ui.visuals().dark_mode;
    let active_color = if is_dark { OD_BLUE } else { OL_BLUE };
    let inactive_color = if is_dark { OD_FG_DIM } else { OL_FG_DIM };

    // Gather badge counts from focused pane
    let crash_count = app
        .panes
        .get(&app.focused_pane)
        .map(|p| p.crash_reports.len())
        .unwrap_or(0);
    let watch_count = app
        .panes
        .get(&app.focused_pane)
        .map(|p| p.watches.len())
        .unwrap_or(0);

    let tabs: Vec<(SidebarTab, String)> = vec![
        (SidebarTab::Devices, format!("{} Devices", egui_phosphor::regular::DEVICE_MOBILE)),
        (SidebarTab::Location, format!("{} Location", egui_phosphor::regular::MAP_PIN)),
        (
            SidebarTab::Crashes,
            if crash_count > 0 {
                format!("{} {crash_count}", egui_phosphor::regular::WARNING_CIRCLE)
            } else {
                format!("{} Crashes", egui_phosphor::regular::WARNING_CIRCLE)
            },
        ),
        (
            SidebarTab::Watches,
            if watch_count > 0 {
                format!("{} {watch_count}", egui_phosphor::regular::EYE)
            } else {
                format!("{} Watches", egui_phosphor::regular::EYE)
            },
        ),
    ];

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (tab, label) in &tabs {
            let is_active = app.sidebar_tab == *tab;
            let color = if is_active {
                active_color
            } else {
                inactive_color
            };
            let response = ui.selectable_label(is_active, RichText::new(label).color(color).size(11.0));
            if response.clicked() {
                app.sidebar_tab = *tab;
            }
        }
    });
}

// ─── Tab 1: Devices ───────────────────────────────────────────────────────────

fn draw_devices_tab(ui: &mut Ui, app: &mut App) {
    egui::CollapsingHeader::new("Connected devices")
        .default_open(true)
        .show(ui, |ui| draw_connected_devices_section(ui, app));

    ui.add_space(6.0);

    egui::CollapsingHeader::new("Wireless debugging")
        .default_open(true)
        .show(ui, |ui| draw_wireless_debugging_section(ui, app));

    #[cfg(target_os = "macos")]
    {
        ui.add_space(6.0);
        egui::CollapsingHeader::new("iOS simulator")
            .default_open(true)
            .show(ui, |ui| draw_ios_simulator_section(ui, app));
    }
}

fn draw_connected_devices_section(ui: &mut Ui, app: &mut App) {
    let is_dark = ui.visuals().dark_mode;
    let devices = app.devices.clone();
    let focused_device_id = app
        .panes
        .get(&app.focused_pane)
        .and_then(|pane| pane.device.clone());

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} connected", devices.len()))
                .weak()
                .size(11.0),
        );
        if ui.small_button("Refresh").clicked() {
            app.device_refresh_pending = true;
        }
    });

    if devices.is_empty() {
        ui.label(RichText::new("No devices connected").weak());
        return;
    }

    let mut selected_device: Option<String> = None;
    let mut to_disconnect: Option<String> = None;

    for device in &devices {
        let is_focused = focused_device_id.as_ref() == Some(&device.id);
        let dot_color = if is_dark { OD_GREEN } else { OL_GREEN };

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").color(dot_color));
                ui.vertical(|ui| {
                    ui.label(RichText::new(device.name.as_str()).strong());
                    ui.label(RichText::new(device.platform.label()).weak().size(11.0));
                    if !device.description.is_empty() {
                        ui.label(RichText::new(device.description.as_str()).weak().size(11.0));
                    }
                    ui.label(
                        RichText::new(device.id.as_str())
                            .weak()
                            .monospace()
                            .size(10.0),
                    );
                });

                ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                    if device.supports_disconnect() && ui.small_button("Disconnect").clicked() {
                        to_disconnect = Some(device.id.clone());
                    }

                    if is_focused {
                        ui.label(
                            RichText::new("Focused pane")
                                .color(if is_dark { OD_CYAN } else { OL_BLUE })
                                .size(11.0),
                        );
                    } else if ui.small_button("Use").clicked() {
                        selected_device = Some(device.id.clone());
                    }
                });
            });
        });
    }

    if let Some(device_id) = selected_device {
        app.set_focused_pane_device(Some(device_id));
    }

    if let Some(device_id) = to_disconnect {
        let result = app
            .rt
            .block_on(catpane_core::adb::disconnect_device(&device_id));
        set_wireless_status(app, result);
        app.device_refresh_pending = true;
    }
}

fn draw_wireless_debugging_section(ui: &mut Ui, app: &mut App) {
    let is_dark = ui.visuals().dark_mode;
    let android_devices: Vec<ConnectedDevice> = app
        .devices
        .iter()
        .filter(|device| device.supports_wireless_debugging())
        .cloned()
        .collect();
    let usb_count = android_devices
        .iter()
        .filter(|device| !catpane_core::adb::is_tcp_device(&device.id))
        .count();
    let tcp_count = android_devices.len().saturating_sub(usb_count);

    ui.label(
        RichText::new(format!(
            "{} Android device(s) · {} USB · {} Wi-Fi",
            android_devices.len(),
            usb_count,
            tcp_count
        ))
        .weak()
        .size(11.0),
    );
    ui.label(
        RichText::new(format!("Using adb: {}", catpane_core::adb::adb_binary()))
            .weak()
            .size(11.0),
    );
    ui.add_space(4.0);

    ui.label(RichText::new("Pair with QR code").strong().size(13.0));
    ui.label(
        RichText::new(
            "On your Android device: Developer Options -> Wireless debugging -> Pair device with QR code",
        )
        .weak()
        .size(11.0),
    );

    if let Some(qr) = &app.qr_pairing {
        if let Some(texture) = &qr.qr_texture {
            ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(200.0, 200.0)));
        }
        match &qr.status {
            QrPairStatus::WaitingScan => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new("Scan QR on your device - waiting for pairing...").size(12.0),
                    );
                });
            }
            QrPairStatus::Pairing(message) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(message.as_str()).size(12.0));
                });
            }
            QrPairStatus::Success(message) => {
                ui.label(
                    RichText::new(message.as_str())
                        .color(if is_dark { OD_GREEN } else { OL_GREEN })
                        .size(12.0),
                );
            }
            QrPairStatus::Failed(message) => {
                ui.label(
                    RichText::new(message.as_str())
                        .color(if is_dark { OD_RED } else { OL_RED })
                        .size(12.0),
                );
            }
        }

        if let Some(hint) = match &qr.status {
            QrPairStatus::Success(message) => wireless_status_hint(true, message),
            QrPairStatus::Failed(message) => wireless_status_hint(false, message),
            _ => None,
        } {
            ui.label(RichText::new(hint).weak().size(11.0));
        }
    } else if ui.button("Generate QR Code").clicked() {
        let service_name = format!("ADB_WIFI_{}", catpane_core::adb::random_id(5));
        let password = catpane_core::adb::random_id(5);
        let qr_string = catpane_core::adb::qr_pairing_string(&service_name, &password);
        let qr_image = catpane_core::adb::generate_qr_image(&qr_string, 4);
        let texture = ui
            .ctx()
            .load_texture("qr_pairing", qr_image, egui::TextureOptions::NEAREST);
        let mdns_rx =
            catpane_core::adb::spawn_mdns_pairing_discovery(&app.rt, service_name, password);

        app.qr_pairing = Some(QrPairingState {
            qr_texture: Some(texture),
            mdns_rx,
            status: QrPairStatus::WaitingScan,
        });
        app.wireless_status = None;
    }

    ui.horizontal_wrapped(|ui| {
        if ui.small_button("Refresh devices").clicked() {
            app.device_refresh_pending = true;
        }
        if ui.small_button("Restart ADB").clicked() {
            let result = app.rt.block_on(catpane_core::adb::restart_server());
            let success = result.is_ok();
            set_wireless_status(app, result);
            if success {
                app.device_refresh_pending = true;
            }
        }
        if app.qr_pairing.is_some() && ui.small_button("Reset QR").clicked() {
            app.qr_pairing = None;
        }
        if app.wireless_status.is_some() && ui.small_button("Clear status").clicked() {
            app.wireless_status = None;
        }
    });
    ui.add_space(6.0);

    egui::CollapsingHeader::new("Manual pairing (code)")
        .default_open(false)
        .show(ui, |ui| {
            ui.label(
                RichText::new("Use if QR doesn't work. Enter pairing info from your device.")
                    .weak()
                    .size(11.0),
            );
            egui::Grid::new("sidebar_pair_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Host:Port");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.wireless_pair_host)
                            .hint_text("host:port")
                            .desired_width(180.0),
                    );
                    ui.end_row();

                    ui.label("Pairing Code");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.wireless_pair_code)
                            .hint_text("123456")
                            .desired_width(180.0),
                    );
                    ui.end_row();
                });

            if ui
                .add_enabled(
                    !app.wireless_pair_host.is_empty() && !app.wireless_pair_code.is_empty(),
                    egui::Button::new("Pair"),
                )
                .clicked()
            {
                let host = app.wireless_pair_host.clone();
                let code = app.wireless_pair_code.clone();
                let result = app
                    .rt
                    .block_on(catpane_core::adb::pair_device(&host, &code));
                set_wireless_status(app, result);
            }
        });

    egui::CollapsingHeader::new("Connect to device")
        .default_open(false)
        .show(ui, |ui| {
            ui.label(
                RichText::new(
                    "After pairing, connect using the device's connect port, not the pairing port.",
                )
                .weak()
                .size(11.0),
            );
            ui.horizontal(|ui| {
                ui.label("Host:Port");
                ui.add(
                    egui::TextEdit::singleline(&mut app.wireless_connect_host)
                        .hint_text("host:port")
                        .desired_width(180.0),
                );
            });

            if ui
                .add_enabled(
                    !app.wireless_connect_host.is_empty(),
                    egui::Button::new("Connect"),
                )
                .clicked()
            {
                let host = app.wireless_connect_host.clone();
                let result = app.rt.block_on(catpane_core::adb::connect_device(&host));
                let success = result.is_ok();
                set_wireless_status(app, result);
                if success {
                    app.device_refresh_pending = true;
                }
            }
        });

    egui::CollapsingHeader::new("USB-assisted fallback")
        .default_open(false)
        .show(ui, |ui| {
            ui.label(
                RichText::new(
                    "If USB debugging works, CatPane can switch the device into TCP/IP mode and try port 5555.",
                )
                .weak()
                .size(11.0),
            );

            let usb_devices: Vec<ConnectedDevice> = app
                .devices
                .iter()
                .filter(|device| is_usb_android_device(device))
                .cloned()
                .collect();

            if app
                .wireless_usb_device
                .as_ref()
                .is_none_or(|selected| !usb_devices.iter().any(|device| &device.id == selected))
            {
                app.wireless_usb_device = usb_devices.first().map(|device| device.id.clone());
            }

            if usb_devices.is_empty() {
                ui.label(
                    RichText::new("Connect an Android device over USB to enable TCP/IP fallback.")
                        .weak(),
                );
                return;
            }

            let selected_label = app
                .wireless_usb_device
                .as_ref()
                .and_then(|selected| {
                    usb_devices
                        .iter()
                        .find(|device| device.id == *selected)
                        .map(|device| format!("{} - {}", device.display_name(), device.id))
                })
                .unwrap_or_else(|| "Select USB device".to_string());

            egui::ComboBox::from_id_salt("wireless_usb_device")
                .selected_text(selected_label)
                .width(220.0)
                .show_ui(ui, |ui| {
                    for device in &usb_devices {
                        let label = format!("{} - {}", device.display_name(), device.id);
                        ui.selectable_value(
                            &mut app.wireless_usb_device,
                            Some(device.id.clone()),
                            label,
                        );
                    }
                });

            if ui.button("Enable TCP/IP over USB").clicked()
                && let Some(device) = app.wireless_usb_device.clone()
            {
                match app.rt.block_on(catpane_core::adb::enable_tcpip_mode(&device, 5555)) {
                    Ok(result) => {
                        if let Some(host) = result.connect_host {
                            app.wireless_connect_host = host;
                        }
                        app.wireless_status = Some((true, result.message));
                        app.device_refresh_pending = true;
                    }
                    Err(message) => app.wireless_status = Some((false, message)),
                }
            }
        });

    if let Some((success, message)) = &app.wireless_status {
        let color = if *success {
            if is_dark { OD_GREEN } else { OL_GREEN }
        } else if is_dark {
            OD_RED
        } else {
            OL_RED
        };
        ui.label(RichText::new(message.as_str()).color(color).size(12.0));
        if let Some(hint) = wireless_status_hint(*success, message) {
            ui.label(RichText::new(hint).weak().size(11.0));
        }
    }
}

#[cfg(target_os = "macos")]
fn draw_ios_simulator_section(ui: &mut Ui, app: &mut App) {
    let is_dark = ui.visuals().dark_mode;
    let booted_count = app
        .devices
        .iter()
        .filter(|device| device.supports_ios_filters())
        .count();

    ui.label(
        RichText::new(format!(
            "{} booted simulator(s) · {} available",
            booted_count,
            app.ios_simulators.len()
        ))
        .weak()
        .size(11.0),
    );

    ui.horizontal_wrapped(|ui| {
        if ui.button("Refresh list").clicked() {
            app.ios_simulator_refresh_pending = true;
        }
        if app.ios_simulator_status.is_some() && ui.small_button("Clear status").clicked() {
            app.ios_simulator_status = None;
        }
    });

    if let Some((success, message)) = &app.ios_simulator_status {
        let color = if *success {
            if is_dark { OD_GREEN } else { OL_GREEN }
        } else if is_dark {
            OD_RED
        } else {
            OL_RED
        };
        ui.label(RichText::new(message.as_str()).color(color).size(12.0));
    }

    if let Some(booting_udid) = &app.ios_simulator_booting_udid {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(format!("Booting {booting_udid}...")).size(12.0));
        });
    }

    if app.ios_simulators.is_empty() {
        ui.label(
            RichText::new("No simulator list loaded yet. Refresh to discover available devices.")
                .weak(),
        );
        return;
    }

    for simulator in app.ios_simulators.clone() {
        draw_ios_simulator_row(ui, app, &simulator);
    }
}

#[cfg(target_os = "macos")]
fn draw_ios_simulator_row(ui: &mut Ui, app: &mut App, simulator: &catpane_core::ios::IosSimulator) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(simulator.name.as_str()).strong());
                ui.label(RichText::new(simulator.runtime.as_str()).weak().size(11.0));
                ui.label(
                    RichText::new(format!("{} · {}", simulator.state, simulator.udid))
                        .weak()
                        .size(10.0),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let is_booted = simulator.state == "Booted";
                let is_booting = app
                    .ios_simulator_booting_udid
                    .as_ref()
                    .is_some_and(|booting| booting == &simulator.udid);
                let response = ui.add_enabled(
                    !is_booted && app.ios_simulator_booting_udid.is_none(),
                    egui::Button::new(if is_booted {
                        "Booted"
                    } else if is_booting {
                        "Booting…"
                    } else {
                        "Boot"
                    }),
                );
                if response.clicked() {
                    start_simulator_boot(app, simulator.udid.clone());
                }
            });
        });
    });
}

#[cfg(target_os = "macos")]
fn start_simulator_boot(app: &mut App, udid: String) {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, String>>(1);
    app.ios_simulator_status = None;
    app.ios_simulator_booting_udid = Some(udid.clone());
    app.ios_simulator_boot_rx = Some(rx);
    app.rt.spawn(async move {
        let result = catpane_core::ios::boot_simulator(&udid).await;
        let _ = tx.send(result).await;
    });
}

// ─── Tab 2: Location ──────────────────────────────────────────────────────────

const BUILTIN_PRESETS: &[(&str, f64, f64)] = &[
    ("Custom", 0.0, 0.0),
    ("San Francisco", 37.7749, -122.4194),
    ("New York", 40.7128, -74.0060),
    ("London", 51.5074, -0.1278),
    ("Tokyo", 35.6762, 139.6503),
    ("Sydney", -33.8688, 151.2093),
];

fn draw_location_tab(ui: &mut Ui, app: &mut App) {
    let is_dark = ui.visuals().dark_mode;

    let focused_device = app
        .panes
        .get(&app.focused_pane)
        .and_then(|pane| pane.device.as_ref())
        .and_then(|device_id| app.devices.iter().find(|d| d.id == *device_id))
        .cloned();

    let device_hint = match &focused_device {
        Some(d) if d.platform == DevicePlatform::IosSimulator => "iOS Simulator",
        Some(d) if d.platform == DevicePlatform::Android && catpane_core::adb::is_emulator(&d.id) => {
            "Android Emulator"
        }
        Some(d) if d.platform == DevicePlatform::Android => "Android (physical – not supported)",
        _ => "No device selected",
    };

    ui.label(
        RichText::new(format!("Target: {device_hint}"))
            .weak()
            .size(11.0),
    );
    ui.add_space(4.0);

    let device_id = focused_device.as_ref().map(|d| d.id.clone());

    // Build combined preset list: built-in + saved
    let mut all_presets: Vec<(String, f64, f64, bool)> = BUILTIN_PRESETS
        .iter()
        .map(|(name, lat, lon)| (name.to_string(), *lat, *lon, false))
        .collect();
    for loc in &app.saved_locations {
        all_presets.push((loc.name.clone(), loc.lat, loc.lon, true));
    }

    // Get or create per-device location state
    if let Some(ref dev_id) = device_id {
        if !app.device_locations.contains_key(dev_id) {
            app.device_locations
                .insert(dev_id.clone(), DeviceLocationState::default());
        }
    }

    if let Some(dev_id) = &device_id {
        let loc_state = app.device_locations.get_mut(dev_id).unwrap();

        egui::ComboBox::from_id_salt("location_preset_tab")
            .selected_text(loc_state.preset.as_str())
            .width(180.0)
            .show_ui(ui, |ui| {
                for (name, lat, lon, _is_custom) in &all_presets {
                    if ui
                        .selectable_value(&mut loc_state.preset, name.clone(), name.as_str())
                        .clicked()
                        && name != "Custom"
                    {
                        loc_state.lat = format!("{lat}");
                        loc_state.lon = format!("{lon}");
                    }
                }
            });

        ui.add_space(4.0);

        egui::Grid::new("location_grid_tab")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Lat:");
                ui.add(
                    egui::TextEdit::singleline(&mut loc_state.lat)
                        .hint_text("37.7749")
                        .desired_width(150.0),
                );
                ui.end_row();

                ui.label("Lon:");
                ui.add(
                    egui::TextEdit::singleline(&mut loc_state.lon)
                        .hint_text("-122.4194")
                        .desired_width(150.0),
                );
                ui.end_row();
            });

        ui.add_space(4.0);

        let task_running = app.location_pending.is_some();
        let can_set = focused_device.as_ref().is_some_and(|d| {
            d.platform == DevicePlatform::IosSimulator
                || (d.platform == DevicePlatform::Android && catpane_core::adb::is_emulator(&d.id))
        });

        // Read lat/lon from state for the button actions
        let lat_str = loc_state.lat.clone();
        let lon_str = loc_state.lon.clone();
        let status = loc_state.status.clone();

        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    can_set && !task_running,
                    egui::Button::new("Set Location"),
                )
                .clicked()
            {
                let lat = match lat_str.trim().parse::<f64>() {
                    Ok(v) => v,
                    Err(_) => {
                        if let Some(state) = app.device_locations.get_mut(dev_id) {
                            state.status = Some((false, "Invalid latitude".to_string()));
                        }
                        return;
                    }
                };
                let lon = match lon_str.trim().parse::<f64>() {
                    Ok(v) => v,
                    Err(_) => {
                        if let Some(state) = app.device_locations.get_mut(dev_id) {
                            state.status = Some((false, "Invalid longitude".to_string()));
                        }
                        return;
                    }
                };

                if let Some(device) = &focused_device {
                    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, String>>(1);
                    app.location_pending = Some((dev_id.clone(), rx));
                    if let Some(state) = app.device_locations.get_mut(dev_id) {
                        state.status = None;
                    }
                    let device_id = device.id.clone();
                    let platform = device.platform;
                    app.rt.spawn(async move {
                        let result = match platform {
                            DevicePlatform::IosSimulator => {
                                catpane_core::ios::set_simulator_location(&device_id, lat, lon).await
                            }
                            DevicePlatform::Android => {
                                catpane_core::adb::set_emulator_location(&device_id, lat, lon, None)
                                    .await
                            }
                        };
                        let _ = tx.send(result).await;
                    });
                }
            }

            let can_clear = focused_device
                .as_ref()
                .is_some_and(|d| d.platform == DevicePlatform::IosSimulator);
            if ui
                .add_enabled(
                    can_clear && !task_running,
                    egui::Button::new("Clear"),
                )
                .on_hover_text("Clear spoofed location (iOS Simulator only)")
                .clicked()
            {
                if let Some(device) = &focused_device {
                    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, String>>(1);
                    app.location_pending = Some((dev_id.clone(), rx));
                    if let Some(state) = app.device_locations.get_mut(dev_id) {
                        state.status = None;
                    }
                    let device_id = device.id.clone();
                    app.rt.spawn(async move {
                        let result =
                            catpane_core::ios::clear_simulator_location(&device_id).await;
                        let _ = tx.send(result).await;
                    });
                }
            }

            if status.is_some() && ui.small_button(egui_phosphor::regular::CROSS).clicked() {
                if let Some(state) = app.device_locations.get_mut(dev_id) {
                    state.status = None;
                }
            }
        });

        if task_running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Setting location…").size(12.0));
            });
        }

        if let Some((success, message)) = &status {
            let color = if *success {
                if is_dark { OD_GREEN } else { OL_GREEN }
            } else if is_dark {
                OD_RED
            } else {
                OL_RED
            };
            ui.label(RichText::new(message.as_str()).color(color).size(12.0));
        }
    } else {
        ui.label(RichText::new("Select a device to set location.").weak());
    }

    // ── Saved Locations section ──
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);
    ui.label(RichText::new("Saved Locations").strong().size(13.0));

    // Save current location
    if device_id.is_some() {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut app.save_location_name)
                    .hint_text("Location name")
                    .desired_width(120.0),
            );
            if ui
                .add_enabled(
                    !app.save_location_name.trim().is_empty(),
                    egui::Button::new("Save Current"),
                )
                .clicked()
            {
                if let Some(dev_id) = &device_id {
                    if let Some(state) = app.device_locations.get(dev_id) {
                        if let (Ok(lat), Ok(lon)) = (
                            state.lat.trim().parse::<f64>(),
                            state.lon.trim().parse::<f64>(),
                        ) {
                            app.saved_locations.push(SavedLocation {
                                name: app.save_location_name.trim().to_string(),
                                lat,
                                lon,
                            });
                            app.save_location_name.clear();
                            app.persist_saved_locations();
                        }
                    }
                }
            }
        });
    }

    if app.saved_locations.is_empty() {
        ui.label(RichText::new("No saved locations yet.").weak().size(11.0));
    } else {
        let mut to_delete: Option<usize> = None;
        let mut to_use: Option<usize> = None;

        for (i, loc) in app.saved_locations.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{} ({:.4}, {:.4})", loc.name, loc.lat, loc.lon))
                        .size(11.0),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button(egui_phosphor::regular::CROSS).on_hover_text("Delete").clicked() {
                        to_delete = Some(i);
                    }
                    if ui.small_button("Use").clicked() {
                        to_use = Some(i);
                    }
                });
            });
        }

        if let Some(idx) = to_use {
            let loc = &app.saved_locations[idx];
            if let Some(dev_id) = &device_id {
                let state = app
                    .device_locations
                    .entry(dev_id.clone())
                    .or_default();
                state.lat = format!("{}", loc.lat);
                state.lon = format!("{}", loc.lon);
                state.preset = loc.name.clone();
            }
        }

        if let Some(idx) = to_delete {
            app.saved_locations.remove(idx);
            app.persist_saved_locations();
        }
    }
}

// ─── Tab 3: Crashes ───────────────────────────────────────────────────────────

fn crash_type_label(ct: CrashType) -> &'static str {
    match ct {
        CrashType::JavaException => "Java Exception",
        CrashType::NativeCrash => "Native Crash",
        CrashType::Anr => "ANR",
        CrashType::IosCrash => "iOS Crash",
    }
}

fn draw_crashes_tab(ui: &mut Ui, app: &mut App) {
    let is_dark = ui.visuals().dark_mode;

    let crash_reports: Vec<_> = app
        .panes
        .get(&app.focused_pane)
        .map(|p| p.crash_reports.clone())
        .unwrap_or_default();

    let count = crash_reports.len();
    ui.label(
        RichText::new(format!("{} Crashes ({count})", egui_phosphor::regular::WARNING_CIRCLE))
            .strong()
            .size(13.0),
    );
    ui.add_space(4.0);

    if crash_reports.is_empty() {
        ui.label(RichText::new("No crashes detected").weak());
        return;
    }

    // Show newest first
    for (display_idx, report) in crash_reports.iter().rev().enumerate() {
        let original_idx = crash_reports.len() - 1 - display_idx;

        let badge_color = if is_dark { OD_RED } else { OL_RED };
        let type_label = crash_type_label(report.crash_type);

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(type_label).color(badge_color).strong().size(11.0));
                if !report.timestamp.is_empty() {
                    ui.label(RichText::new(&report.timestamp).weak().size(10.0));
                }
            });

            // Summary (headline)
            let summary = if report.headline.len() > 80 {
                format!("{}…", &report.headline[..80])
            } else {
                report.headline.clone()
            };
            ui.label(RichText::new(&summary).size(11.0));

            // Expandable detail
            let is_expanded = app.expanded_crashes.contains(&original_idx);
            ui.horizontal(|ui| {
                let toggle_text = if is_expanded {
                    format!("{} Hide details", egui_phosphor::regular::CARET_DOWN)
                } else {
                    format!("{} Show details", egui_phosphor::regular::CARET_RIGHT)
                };
                if ui.small_button(toggle_text).clicked() {
                    if is_expanded {
                        app.expanded_crashes.remove(&original_idx);
                    } else {
                        app.expanded_crashes.insert(original_idx);
                    }
                }

                // Copy button
                if ui.small_button(format!("{} Copy", egui_phosphor::regular::COPY)).clicked() {
                    let mut text = format!("[{}] {}\n{}\n", type_label, report.timestamp, report.headline);
                    // Gather context lines from pane entries
                    if let Some(pane) = app.panes.get(&app.focused_pane) {
                        let start = report.first_index;
                        let end = report.last_index.min(pane.entries.len().saturating_sub(1));
                        for i in start..=end {
                            if let Some(entry) = pane.entries.get(i) {
                                text.push_str(&format!(
                                    "{} {} {}: {}\n",
                                    entry.timestamp,
                                    entry.level.as_char(),
                                    entry.tag,
                                    entry.message
                                ));
                            }
                        }
                    }
                    ui.ctx().copy_text(text);
                }
            });

            if is_expanded {
                ui.add_space(2.0);
                // Show context lines from pane entries
                if let Some(pane) = app.panes.get(&app.focused_pane) {
                    let crash_indices = &pane.crash_line_indices;
                    let start = report.first_index;
                    let end = report.last_index.min(pane.entries.len().saturating_sub(1));
                    for i in start..=end {
                        if let Some(entry) = pane.entries.get(i) {
                            let is_crash_line = crash_indices.contains(&i);
                            let line_text = format!(
                                "{} {} {}: {}",
                                entry.timestamp,
                                entry.level.as_char(),
                                entry.tag,
                                entry.message
                            );
                            let color = if is_crash_line {
                                if is_dark { OD_RED } else { OL_RED }
                            } else if is_dark {
                                OD_FG_DIM
                            } else {
                                OL_FG_DIM
                            };
                            ui.label(
                                RichText::new(&line_text)
                                    .color(color)
                                    .monospace()
                                    .size(10.0),
                            );
                        }
                    }
                }
            }
        });
        ui.add_space(2.0);
    }

    ui.add_space(4.0);
    if ui.button("Clear All Crashes").clicked() {
        if let Some(pane) = app.panes.get_mut(&app.focused_pane) {
            pane.crash_reports.clear();
            pane.crash_line_indices.clear();
            pane.crash_nav_index = None;
        }
        app.expanded_crashes.clear();
    }
}

// ─── Tab 4: Watches ───────────────────────────────────────────────────────────

fn draw_watches_tab(ui: &mut Ui, app: &mut App) {
    ui.label(
        RichText::new("Watches highlight log lines matching a pattern. Add patterns below to monitor specific events.")
            .weak()
            .size(11.0),
    );
    ui.add_space(6.0);

    ui.label(RichText::new("Add Watch").strong().size(13.0));

    let focused_pane = app.focused_pane;
    let Some(pane) = app.panes.get_mut(&focused_pane) else {
        ui.label(RichText::new("No focused pane").weak());
        return;
    };

    let watch_response = ui.add(
        egui::TextEdit::singleline(&mut pane.watch_input)
            .desired_width(ui.available_width() - 80.0)
            .hint_text("e.g. Exception, OOM, ANR")
            .id(ui.id().with("sidebar_watch_input")),
    );
    let enter_pressed =
        watch_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let add_clicked = ui.button("Add Watch").clicked();

    if enter_pressed || add_clicked {
        let pattern = pane.watch_input.trim().to_string();
        if !pattern.is_empty() {
            let name = pattern.clone();
            pane.add_watch(name, pattern);
            pane.watch_input.clear();
        }
    }

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(4.0);
    ui.label(RichText::new("Active Watches").strong().size(13.0));

    if pane.watches.is_empty() {
        ui.label(
            RichText::new("No active watches. Add a pattern above to start monitoring.")
                .weak()
                .size(11.0),
        );
        return;
    }

    let mut watch_to_remove: Option<usize> = None;
    for i in 0..pane.watches.len() {
        let watch = &pane.watches[i];
        let (r, g, b) = WATCH_COLORS[watch.color_index % WATCH_COLORS.len()];
        let color = egui::Color32::from_rgb(r, g, b);

        ui.horizontal(|ui| {
            ui.label(RichText::new("●").color(color));
            ui.label(
                RichText::new(format!("{} ({})", watch.name, watch.match_count)).size(12.0),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button(egui_phosphor::regular::CROSS).on_hover_text("Remove watch").clicked() {
                    watch_to_remove = Some(i);
                }
            });
        });
    }
    if let Some(idx) = watch_to_remove {
        pane.remove_watch(idx);
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn set_wireless_status(app: &mut App, result: Result<String, String>) {
    match result {
        Ok(message) => app.wireless_status = Some((true, message)),
        Err(message) => app.wireless_status = Some((false, message)),
    }
}

fn is_usb_android_device(device: &ConnectedDevice) -> bool {
    device.supports_wireless_debugging() && !catpane_core::adb::is_tcp_device(&device.id)
}

fn wireless_status_hint(success: bool, message: &str) -> Option<&'static str> {
    if success {
        let lower = message.to_lowercase();
        if lower.contains("automatic connect") {
            return Some(
                "Pairing succeeded, but the connect step still needs help. Try Connect or Refresh devices.",
            );
        }
        if lower.contains("tcp/ip") {
            return Some(
                "If the device does not appear immediately, leave USB attached briefly, then refresh devices or retry Connect.",
            );
        }
        return None;
    }

    let lower = message.to_lowercase();
    if lower.contains("protocol fault") {
        return Some(
            "This usually means stale wireless auth or a wedged pairing service. Restart ADB, then re-open Wireless debugging on the device and pair again.",
        );
    }
    if lower.contains("unauthorized") {
        return Some(
            "Reconnect over USB and accept the RSA prompt again. If needed, revoke USB debugging authorizations on the phone and retry.",
        );
    }
    if lower.contains("timed out") || lower.contains("mdns") {
        return Some(
            "Keep the phone on the QR pairing screen and make sure the Mac and phone are on the same Wi-Fi without VPN or client isolation.",
        );
    }
    if lower.contains("failed to connect")
        || lower.contains("unable to connect")
        || lower.contains("connection refused")
    {
        return Some(
            "Double-check that you are using the device's connect port, not the pairing port. If USB works, try the TCP/IP fallback below.",
        );
    }
    if lower.contains("more than one device") {
        return Some(
            "Disconnect extra Android devices or use the USB-assisted fallback selector to target the exact device you want.",
        );
    }
    None
}
