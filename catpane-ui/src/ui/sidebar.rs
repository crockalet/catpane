use egui::{self, Align, Layout, RichText, ScrollArea, Ui};

use super::theme::*;
use crate::app::{App, QrPairStatus, QrPairingState};
use catpane_core::capture::{ConnectedDevice, DevicePlatform};

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

    let button = egui::Button::new(RichText::new("📱").size(14.0)).selected(sidebar_open);

    if ui
        .add_sized([SIDEBAR_RAIL_BUTTON_SIZE, SIDEBAR_RAIL_BUTTON_SIZE], button)
        .on_hover_text("Open device manager")
        .clicked()
    {
        app.sidebar_open = true;
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

    ui.horizontal(|ui| {
        ui.heading("Device Manager");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .small_button("⮜")
                .on_hover_text("Collapse device manager")
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
    ui.separator();

    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
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

            ui.add_space(6.0);
            egui::CollapsingHeader::new("📍 Location Spoofing")
                .default_open(false)
                .show(ui, |ui| draw_location_spoofing_section(ui, app));
        });
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

fn draw_location_spoofing_section(ui: &mut Ui, app: &mut App) {
    let is_dark = ui.visuals().dark_mode;

    let focused_device = app
        .panes
        .get(&app.focused_pane)
        .and_then(|pane| pane.device.as_ref())
        .and_then(|device_id| app.devices.iter().find(|d| d.id == *device_id));

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

    let presets = [
        ("Custom", 0.0, 0.0),
        ("San Francisco", 37.7749, -122.4194),
        ("New York", 40.7128, -74.0060),
        ("London", 51.5074, -0.1278),
        ("Tokyo", 35.6762, 139.6503),
        ("Sydney", -33.8688, 151.2093),
    ];

    egui::ComboBox::from_id_salt("location_preset")
        .selected_text(app.location_preset.as_str())
        .width(180.0)
        .show_ui(ui, |ui| {
            for (name, lat, lon) in &presets {
                if ui
                    .selectable_value(&mut app.location_preset, name.to_string(), *name)
                    .clicked()
                    && *name != "Custom"
                {
                    app.location_lat = format!("{}", lat);
                    app.location_lon = format!("{}", lon);
                }
            }
        });

    ui.add_space(4.0);

    egui::Grid::new("location_grid")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Lat:");
            ui.add(
                egui::TextEdit::singleline(&mut app.location_lat)
                    .hint_text("37.7749")
                    .desired_width(150.0),
            );
            ui.end_row();

            ui.label("Lon:");
            ui.add(
                egui::TextEdit::singleline(&mut app.location_lon)
                    .hint_text("-122.4194")
                    .desired_width(150.0),
            );
            ui.end_row();
        });

    ui.add_space(4.0);

    let task_running = app.location_rx.is_some();
    let can_set = focused_device.is_some_and(|d| {
        d.platform == DevicePlatform::IosSimulator
            || (d.platform == DevicePlatform::Android && catpane_core::adb::is_emulator(&d.id))
    });

    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                can_set && !task_running,
                egui::Button::new("Set Location"),
            )
            .clicked()
        {
            let lat = match app.location_lat.trim().parse::<f64>() {
                Ok(v) => v,
                Err(_) => {
                    app.location_status = Some((false, "Invalid latitude".to_string()));
                    return;
                }
            };
            let lon = match app.location_lon.trim().parse::<f64>() {
                Ok(v) => v,
                Err(_) => {
                    app.location_status = Some((false, "Invalid longitude".to_string()));
                    return;
                }
            };

            if let Some(device) = &focused_device {
                let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, String>>(1);
                app.location_rx = Some(rx);
                app.location_status = None;
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
                app.location_rx = Some(rx);
                app.location_status = None;
                let device_id = device.id.clone();
                app.rt.spawn(async move {
                    let result =
                        catpane_core::ios::clear_simulator_location(&device_id).await;
                    let _ = tx.send(result).await;
                });
            }
        }

        if app.location_status.is_some() && ui.small_button("✕").clicked() {
            app.location_status = None;
        }
    });

    if task_running {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("Setting location…").size(12.0));
        });
    }

    if let Some((success, message)) = &app.location_status {
        let color = if *success {
            if is_dark { OD_GREEN } else { OL_GREEN }
        } else if is_dark {
            OD_RED
        } else {
            OL_RED
        };
        ui.label(RichText::new(message.as_str()).color(color).size(12.0));
    }
}

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
