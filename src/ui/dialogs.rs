use egui::{self, RichText};

use super::theme::*;
use crate::app::{App, QrPairStatus, QrPairingState};
use crate::capture::ConnectedDevice;

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
    let mut open = app.show_wireless_dialog;

    egui::Window::new("📡 Wireless Debugging")
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(380.0)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(
                RichText::new(format!("Using adb: {}", crate::adb::adb_binary()))
                    .size(11.0)
                    .weak(),
            );

            // === QR Code Pairing ===
            ui.label(RichText::new("Pair with QR code").strong().size(14.0));
            ui.label(RichText::new("On your Android device: Developer Options → Wireless debugging → Pair device with QR code").size(11.0).weak());
            ui.add_space(2.0);

            // Poll mDNS result if active
            if let Some(qr) = &mut app.qr_pairing {
                if matches!(qr.status, QrPairStatus::WaitingScan | QrPairStatus::Pairing(_)) {
                    if let Ok(event) = qr.mdns_rx.try_recv() {
                        match event {
                            crate::adb::QrPairEvent::Status(msg) => {
                                qr.status = QrPairStatus::Pairing(msg)
                            }
                            crate::adb::QrPairEvent::Finished(Ok(msg)) => {
                                qr.status = QrPairStatus::Success(msg);
                                app.device_refresh_pending = true;
                            }
                            crate::adb::QrPairEvent::Finished(Err(msg)) => {
                                qr.status = QrPairStatus::Failed(msg)
                            }
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

                let qr_hint = match &qr.status {
                    QrPairStatus::Success(msg) => wireless_status_hint(true, msg),
                    QrPairStatus::Failed(msg) => wireless_status_hint(false, msg),
                    _ => None,
                };
                if let Some(hint) = qr_hint {
                    ui.label(RichText::new(hint).size(11.0).weak());
                }

                if ui.button("Reset").clicked() {
                    app.qr_pairing = None;
                    app.wireless_status = None;
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
                        qr_texture: Some(texture),
                        mdns_rx,
                        status: QrPairStatus::WaitingScan,
                    });
                    app.wireless_status = None;
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
                    set_wireless_status(app, result);
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
                    let success = result.is_ok();
                    set_wireless_status(app, result);
                    if success {
                        app.device_refresh_pending = true;
                    }
                }
            });

            ui.add_space(2.0);
            ui.collapsing("Recovery tools", |ui| {
                ui.label(
                    RichText::new(
                        "Use these when QR or pairing gets stuck, or when USB works but wireless does not.",
                    )
                    .size(11.0)
                    .weak(),
                );
                ui.add_space(2.0);

                ui.horizontal_wrapped(|ui| {
                    if ui.button("Restart ADB server").clicked() {
                        let result = app.rt.block_on(crate::adb::restart_server());
                        let success = result.is_ok();
                        set_wireless_status(app, result);
                        if success {
                            app.device_refresh_pending = true;
                        }
                    }

                    if ui.button("Refresh devices").clicked() {
                        app.device_refresh_pending = true;
                    }

                    if ui.button("Clear status").clicked() {
                        app.wireless_status = None;
                    }
                });

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
                ui.label(RichText::new("USB-assisted fallback").strong().size(13.0));
                ui.label(
                    RichText::new(
                        "If USB debugging works, CatPane can switch the device into TCP/IP mode and try to connect wirelessly on port 5555.",
                    )
                    .size(11.0)
                    .weak(),
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
                        RichText::new(
                            "Connect an Android device over USB to enable the TCP/IP fallback.",
                        )
                        .weak(),
                    );
                } else {
                    let selected_label = app
                        .wireless_usb_device
                        .as_ref()
                        .and_then(|selected| {
                            usb_devices
                                .iter()
                                .find(|device| device.id == *selected)
                                .map(|device| format!("{} — {}", device.display_name(), device.id))
                        })
                        .unwrap_or_else(|| "Select USB device".to_string());

                    egui::ComboBox::from_id_salt("wireless_usb_device")
                        .selected_text(selected_label)
                        .width(260.0)
                        .show_ui(ui, |ui| {
                            for device in &usb_devices {
                                let label =
                                    format!("{} — {}", device.display_name(), device.id);
                                ui.selectable_value(
                                    &mut app.wireless_usb_device,
                                    Some(device.id.clone()),
                                    label,
                                );
                            }
                        });

                    if ui.button("Enable TCP/IP over USB").clicked() {
                        if let Some(device) = app.wireless_usb_device.clone() {
                            match app.rt.block_on(crate::adb::enable_tcpip_mode(&device, 5555)) {
                                Ok(result) => {
                                    if let Some(host) = result.connect_host {
                                        app.wireless_connect_host = host;
                                    }
                                    app.wireless_status = Some((true, result.message));
                                    app.device_refresh_pending = true;
                                }
                                Err(msg) => {
                                    app.wireless_status = Some((false, msg));
                                }
                            }
                        }
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
                if let Some(hint) = wireless_status_hint(*success, msg) {
                    ui.label(RichText::new(hint).size(11.0).weak());
                }
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
                    .map(|device| (device.id.clone(), device.display_name()))
                    .collect();

                let mut to_disconnect: Option<String> = None;

                for (serial, name) in &device_serials {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("●").color(if is_dark { OD_GREEN } else { OL_GREEN }));
                        ui.label(name);
                        ui.label(RichText::new(serial.as_str()).weak().size(11.0));
                        if app
                            .devices
                            .iter()
                            .any(|device| device.id == *serial && device.supports_disconnect())
                        {
                            if ui.small_button("Disconnect").clicked() {
                                to_disconnect = Some(serial.clone());
                            }
                        }
                    });
                }

                if let Some(serial) = to_disconnect {
                    let result = app.rt.block_on(crate::adb::disconnect_device(&serial));
                    set_wireless_status(app, result);
                    app.device_refresh_pending = true;
                }
            }
        });
    app.show_wireless_dialog = open;
}

fn set_wireless_status(app: &mut App, result: Result<String, String>) {
    match result {
        Ok(msg) => app.wireless_status = Some((true, msg)),
        Err(msg) => app.wireless_status = Some((false, msg)),
    }
}

fn is_usb_android_device(device: &ConnectedDevice) -> bool {
    device.supports_wireless_debugging() && !crate::adb::is_tcp_device(&device.id)
}

fn wireless_status_hint(success: bool, message: &str) -> Option<&'static str> {
    if success {
        let lower = message.to_lowercase();
        if lower.contains("automatic connect") {
            return Some(
                "Pairing succeeded, but the wireless connect step still needs help. Try the Connect section or Refresh devices.",
            );
        }
        if lower.contains("tcp/ip") {
            return Some(
                "If the device does not appear immediately, leave USB attached for a moment, then click Refresh devices or try Connect with the filled host:port.",
            );
        }
        return None;
    }

    let lower = message.to_lowercase();
    if lower.contains("protocol fault") {
        return Some(
            "This usually means stale wireless auth or a wedged pairing service. Restart ADB here, then on the device re-open Wireless debugging and re-pair.",
        );
    }
    if lower.contains("unauthorized") {
        return Some(
            "Reconnect over USB and accept the RSA prompt again. If it keeps happening, revoke USB debugging authorizations on the phone and retry.",
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

pub fn draw_ios_simulator_dialog(ctx: &egui::Context, app: &mut App) {
    let is_dark = ctx.style().visuals.dark_mode;

    egui::Window::new("🍎 Boot iOS Simulator")
        .open(&mut app.show_ios_simulator_dialog)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(480.0)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;

            ui.horizontal(|ui| {
                ui.label(RichText::new("Available simulators").strong().size(14.0));
                if ui.small_button("Refresh").clicked() {
                    app.ios_simulator_refresh_pending = true;
                }
            });
            ui.label(
                RichText::new(
                    "Boot a simulator directly from CatPane. Once booted, it will appear in the device picker.",
                )
                .size(11.0)
                .weak(),
            );

            if let Some((success, msg)) = &app.ios_simulator_status {
                let color = if *success {
                    if is_dark { OD_GREEN } else { OL_GREEN }
                } else if is_dark {
                    OD_RED
                } else {
                    OL_RED
                };
                ui.label(RichText::new(msg.as_str()).color(color).size(12.0));
            }

            if let Some(booting_udid) = &app.ios_simulator_booting_udid {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(format!("Booting simulator {booting_udid}..."))
                            .size(12.0),
                    );
                });
            }

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            if app.ios_simulators.is_empty() {
                ui.label(RichText::new("No available simulators found").weak());
                return;
            }

            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for simulator in app.ios_simulators.clone() {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(simulator.name.as_str()).strong());
                                    ui.label(
                                        RichText::new(simulator.runtime.as_str())
                                            .weak()
                                            .size(11.0),
                                    );
                                    ui.label(
                                        RichText::new(format!(
                                            "{} · {}",
                                            simulator.state, simulator.udid
                                        ))
                                        .weak()
                                        .size(11.0),
                                    );
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let is_booted = simulator.state == "Booted";
                                        let is_booting = app
                                            .ios_simulator_booting_udid
                                            .as_ref()
                                            .is_some_and(|booting| booting == &simulator.udid);
                                        let button = ui.add_enabled(
                                            !is_booted && app.ios_simulator_booting_udid.is_none(),
                                            egui::Button::new(if is_booted {
                                                "Booted"
                                            } else if is_booting {
                                                "Booting…"
                                            } else {
                                                "Boot"
                                            }),
                                        );
                                        if button.clicked() {
                                            let (tx, rx) =
                                                tokio::sync::mpsc::channel::<Result<String, String>>(1);
                                            let udid = simulator.udid.clone();
                                            app.ios_simulator_status = None;
                                            app.ios_simulator_booting_udid = Some(udid.clone());
                                            app.ios_simulator_boot_rx = Some(rx);
                                            app.rt.spawn(async move {
                                                let result = crate::ios::boot_simulator(&udid).await;
                                                let _ = tx.send(result).await;
                                            });
                                        }
                                    },
                                );
                            });
                        });
                    }
                });
        });
}
