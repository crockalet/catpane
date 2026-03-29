use egui::{self, RichText};

use super::theme::*;
use crate::app::{App, QrPairStatus, QrPairingState};

pub fn draw_help_window(ctx: &egui::Context, show: &mut bool) {
    egui::Window::new("CatPane — Keyboard Shortcuts")
        .open(show)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            let cmd = if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl+"
            };
            let shortcuts = [
                (format!("{cmd}D"), "Split pane right"),
                (format!("{cmd}⇧D"), "Split pane down"),
                (format!("{cmd}W"), "Close pane"),
                (format!("{cmd}N"), "New window"),
                (format!("{cmd}F"), "Find in logs"),
                ("Tab".to_string(), "Cycle pane focus"),
                ("F1".to_string(), "Toggle this help"),
                (String::new(), ""),
                ("Right-click".to_string(), "Include/exclude/like tag"),
                (
                    "Tags".to_string(),
                    "tag:Name  tag-:Excl  tag~:Regex  Name:V *:E",
                ),
            ];

            egui::Grid::new("help_grid").striped(true).show(ui, |ui| {
                for (key, desc) in &shortcuts {
                    if key.is_empty() {
                        ui.label("");
                        ui.label("");
                    } else {
                        ui.label(RichText::new(key).strong().monospace().size(13.0));
                        ui.label(RichText::new(*desc).size(13.0));
                    }
                    ui.end_row();
                }
            });
        });
}

pub fn draw_wireless_dialog(ctx: &egui::Context, app: &mut App) {
    let is_dark = ctx.style().visuals.dark_mode;

    egui::Window::new("📡 Wireless Debugging")
        .open(&mut app.show_wireless_dialog)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(380.0)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;

            // === QR Code Pairing ===
            ui.label(RichText::new("Pair with QR code").strong().size(14.0));
            ui.label(RichText::new("On your Android device: Developer Options → Wireless debugging → Pair device with QR code").size(11.0).weak());
            ui.add_space(2.0);

            // Poll mDNS result if active
            if let Some(qr) = &mut app.qr_pairing {
                if matches!(qr.status, QrPairStatus::WaitingScan) {
                    if let Ok(result) = qr.mdns_rx.try_recv() {
                        match result {
                            Ok(msg) => qr.status = QrPairStatus::Success(msg),
                            Err(msg) => qr.status = QrPairStatus::Failed(msg),
                        }
                    }
                }
            }

            if let Some(qr) = &app.qr_pairing {
                if let Some(tex) = &qr.qr_texture {
                    let size = egui::vec2(200.0, 200.0);
                    ui.add(egui::Image::new(tex).fit_to_exact_size(size));
                }

                match &qr.status {
                    QrPairStatus::WaitingScan => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Scan QR on your device — waiting for pairing…").size(12.0));
                        });
                    }
                    QrPairStatus::Pairing(msg) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(msg.as_str()).size(12.0));
                        });
                    }
                    QrPairStatus::Success(msg) => {
                        ui.label(RichText::new(msg.as_str())
                            .color(if is_dark { OD_GREEN } else { OL_GREEN }).size(12.0));
                    }
                    QrPairStatus::Failed(msg) => {
                        ui.label(RichText::new(msg.as_str())
                            .color(if is_dark { OD_RED } else { OL_RED }).size(12.0));
                    }
                }

                if ui.button("Reset").clicked() {
                    app.qr_pairing = None;
                }
            } else {
                if ui.button("Generate QR Code").clicked() {
                    let service_name = format!("ADB_WIFI_{}", crate::adb::random_id(5));
                    let password = crate::adb::random_id(5);
                    let qr_string = crate::adb::qr_pairing_string(&service_name, &password);

                    let qr_image = crate::adb::generate_qr_image(&qr_string, 4);
                    let texture = ctx.load_texture(
                        "qr_pairing",
                        qr_image,
                        egui::TextureOptions::NEAREST,
                    );

                    let mdns_rx = crate::adb::spawn_mdns_pairing_discovery(
                        &app.rt,
                        service_name.clone(),
                        password.clone(),
                    );

                    app.qr_pairing = Some(QrPairingState {
                        password,
                        service_name,
                        qr_texture: Some(texture),
                        mdns_rx,
                        status: QrPairStatus::WaitingScan,
                    });
                }
            }

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            // === Manual Pair section ===
            ui.collapsing("Manual pairing (code)", |ui| {
                ui.label(RichText::new("Use if QR doesn't work. Enter pairing info from your device.").size(11.0).weak());
                ui.add_space(2.0);

                egui::Grid::new("pair_grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                    ui.label("Host:Port");
                    ui.add(egui::TextEdit::singleline(&mut app.wireless_pair_host)
                        .hint_text("host:port")
                        .desired_width(200.0));
                    ui.end_row();

                    ui.label("Pairing Code");
                    ui.add(egui::TextEdit::singleline(&mut app.wireless_pair_code)
                        .hint_text("123456")
                        .desired_width(200.0));
                    ui.end_row();
                });

                if ui.add_enabled(
                    !app.wireless_pair_host.is_empty() && !app.wireless_pair_code.is_empty(),
                    egui::Button::new("Pair"),
                ).clicked() {
                    let host = app.wireless_pair_host.clone();
                    let code = app.wireless_pair_code.clone();
                    let result = app.rt.block_on(crate::adb::pair_device(&host, &code));
                    match result {
                        Ok(msg) => app.wireless_status = Some((true, msg)),
                        Err(msg) => app.wireless_status = Some((false, msg)),
                    }
                }
            });

            ui.add_space(2.0);

            // === Connect section ===
            ui.collapsing("Connect to device", |ui| {
                ui.label(RichText::new("After pairing, connect using the IP:port shown under \"Wireless debugging\" (not the pairing port).").size(11.0).weak());
                ui.add_space(2.0);

                ui.horizontal(|ui| {
                    ui.label("Host:Port");
                    ui.add(egui::TextEdit::singleline(&mut app.wireless_connect_host)
                        .hint_text("host:port")
                        .desired_width(200.0));
                });

                if ui.add_enabled(
                    !app.wireless_connect_host.is_empty(),
                    egui::Button::new("Connect"),
                ).clicked() {
                    let host = app.wireless_connect_host.clone();
                    let result = app.rt.block_on(crate::adb::connect_device(&host));
                    match result {
                        Ok(msg) => app.wireless_status = Some((true, msg)),
                        Err(msg) => app.wireless_status = Some((false, msg)),
                    }
                }
            });

            // --- Status ---
            if let Some((success, msg)) = &app.wireless_status {
                ui.add_space(4.0);
                let color = if *success {
                    if is_dark { OD_GREEN } else { OL_GREEN }
                } else {
                    if is_dark { OD_RED } else { OL_RED }
                };
                ui.label(RichText::new(msg.as_str()).color(color).size(12.0));
            }

            // --- Connected devices ---
            ui.add_space(4.0);
            ui.separator();
            ui.label(RichText::new("Connected devices").strong().size(14.0));
            if app.devices.is_empty() {
                ui.label(RichText::new("No devices connected").weak());
            } else {
                let device_serials: Vec<(String, String)> = app
                    .devices
                    .iter()
                    .map(|d| (d.serial.clone(), d.friendly_name()))
                    .collect();

                let mut to_disconnect: Option<String> = None;

                for (serial, name) in &device_serials {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("●").color(if is_dark { OD_GREEN } else { OL_GREEN }));
                        ui.label(name);
                        ui.label(RichText::new(serial.as_str()).weak().size(11.0));
                        if crate::adb::is_tcp_device(serial) {
                            if ui.small_button("Disconnect").clicked() {
                                to_disconnect = Some(serial.clone());
                            }
                        }
                    });
                }

                if let Some(serial) = to_disconnect {
                    let result = app.rt.block_on(crate::adb::disconnect_device(&serial));
                    match result {
                        Ok(msg) => app.wireless_status = Some((true, msg)),
                        Err(msg) => app.wireless_status = Some((false, msg)),
                    }
                    app.device_refresh_pending = true;
                }
            }
        });
}
