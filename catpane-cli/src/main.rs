use std::{
    collections::HashMap,
    process::{Command, ExitCode, Stdio},
    time::Duration,
};

use catpane_core::{
    capture::{self, ConnectedDevice},
    ios,
};
use catpane_mcp::run_stdio_server;
use catpane_ui::{App as UiApp, SplitDir, configure_fonts, draw_ui};
use clap::{Parser, Subcommand};
use muda::{
    AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};

#[cfg(target_os = "macos")]
use objc2_app_kit::{NSView, NSWindowOcclusionState};
#[cfg(target_os = "macos")]
use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};

const ACTIVE_REPAINT_INTERVAL: Duration = Duration::from_millis(33);
const IDLE_REPAINT_INTERVAL: Duration = Duration::from_millis(250);
const PID_REPOLL_INTERVAL: Duration = Duration::from_secs(5);
const APP_NAME: &str = "CatPane";

#[derive(Parser)]
#[command(
    name = "catpane",
    bin_name = "catpane",
    version,
    about = "CatPane GUI and MCP CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the MCP server over stdio.
    Mcp,
}

/// Update `app.devices` and reconcile any pane whose saved serial is no longer
/// present (e.g. an mDNS serial replaced by an IP:port serial after dedup).
/// Matches by friendly name so the correct physical device is kept.
fn update_devices(
    app: &mut UiApp,
    new_devices: Vec<ConnectedDevice>,
    _rt: &tokio::runtime::Handle,
) {
    let new_by_name: HashMap<String, String> = new_devices
        .iter()
        .map(|device| {
            (
                format!("{}::{}", device.platform.label(), device.name),
                device.id.clone(),
            )
        })
        .collect();

    let old_devices = std::mem::replace(&mut app.devices, new_devices);

    let pane_ids: Vec<_> = app.panes.keys().copied().collect();
    for pane_id in pane_ids {
        let device_id = match app.panes.get(&pane_id).and_then(|pane| pane.device.clone()) {
            Some(device_id) => device_id,
            None => continue,
        };

        if app.devices.iter().any(|device| device.id == device_id) {
            let was_present_before = old_devices.iter().any(|device| device.id == device_id);
            if was_present_before {
                continue;
            }
            app.ensure_pane_capture(pane_id);
            continue;
        }

        let reconnect_key = match old_devices.iter().find(|device| device.id == device_id) {
            Some(device) => format!("{}::{}", device.platform.label(), device.name),
            None => continue,
        };

        if let Some(new_id) = new_by_name.get(&reconnect_key) {
            if let Some(pane) = app.panes.get_mut(&pane_id) {
                pane.device = Some(new_id.clone());
            }
            app.ensure_pane_capture(pane_id);
        }
    }
}

struct CatPaneApp {
    app: UiApp,
    rt_handle: tokio::runtime::Handle,
    _rt: tokio::runtime::Runtime,
    _menu: Menu,
    fonts_configured: bool,
    is_dark: bool,
    copy_requested: bool,
}

impl eframe::App for CatPaneApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !self.fonts_configured {
            configure_fonts(ctx, self.is_dark);
            self.fonts_configured = true;
        }

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            self.handle_menu_event(ctx, &event);
        }

        if self.copy_requested {
            ctx.input_mut(|input| {
                input.events.push(egui::Event::Copy);
            });
        }

        let window_visible = window_should_update(ctx, frame);

        if window_visible {
            self.app.poll_all();

            if self.app.device_refresh_pending {
                self.app.device_refresh_pending = false;
                let devices = self.rt_handle.block_on(capture::list_devices());
                update_devices(&mut self.app, devices, &self.rt_handle);
            }

            if self.app.ios_simulator_refresh_pending {
                self.app.ios_simulator_refresh_pending = false;
                self.app.ios_simulators = self.rt_handle.block_on(ios::list_available_simulators());
            }

            let boot_result = if let Some(rx) = &mut self.app.ios_simulator_boot_rx {
                match rx.try_recv() {
                    Ok(result) => Some(result),
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => None,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        Some(Err("Simulator boot task ended unexpectedly".to_string()))
                    }
                }
            } else {
                None
            };

            if let Some(result) = boot_result {
                self.app.ios_simulator_boot_rx = None;
                self.app.ios_simulator_booting_udid = None;
                match result {
                    Ok(message) => {
                        self.app.ios_simulator_status = Some((true, message));
                        self.app.ios_simulator_refresh_pending = true;
                        self.app.device_refresh_pending = true;
                    }
                    Err(message) => {
                        self.app.ios_simulator_status = Some((false, message));
                        self.app.ios_simulator_refresh_pending = true;
                    }
                }
            }

            let location_result = if let Some((_, rx)) = &mut self.app.location_pending {
                match rx.try_recv() {
                    Ok(result) => Some(result),
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => None,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        Some(Err("Location task ended unexpectedly".to_string()))
                    }
                }
            } else {
                None
            };

            if let Some(result) = location_result {
                let device_id = self.app.location_pending.take().map(|(id, _)| id);
                if let Some(device_id) = device_id {
                    let state = self.app.device_locations.entry(device_id).or_default();
                    match result {
                        Ok(message) => state.status = Some((true, message)),
                        Err(message) => state.status = Some((false, message)),
                    }
                }
            }

            let network_result = if let Some((_, rx)) = &mut self.app.network_pending {
                match rx.try_recv() {
                    Ok(result) => Some(result),
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => None,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        Some(Err("Network task ended unexpectedly".to_string()))
                    }
                }
            } else {
                None
            };

            if let Some(result) = network_result {
                let device_id = self.app.network_pending.take().map(|(id, _)| id);
                if let Some(device_id) = device_id {
                    let state = self.app.device_networks.entry(device_id).or_default();
                    match result {
                        Ok(message) => state.status = Some((true, message)),
                        Err(message) => state.status = Some((false, message)),
                    }
                }
            }

            let mut tracker_devices: Option<Vec<ConnectedDevice>> = None;
            if let Some(tracker) = &mut self.app.device_tracker {
                while let Ok(devices) = tracker.try_recv() {
                    tracker_devices = Some(devices);
                }
            }
            if let Some(devices) = tracker_devices {
                update_devices(&mut self.app, devices, &self.rt_handle);
            }

            {
                let pane_ids: Vec<_> = self.app.panes.keys().copied().collect();
                for pane_id in pane_ids {
                    let should_restart = self.app.panes.get(&pane_id).is_some_and(|pane| {
                        pane.capture_rx.is_none()
                            && !pane.paused
                            && pane.last_capture_restart.elapsed()
                                >= std::time::Duration::from_secs(3)
                            && pane.device.as_ref().is_some_and(|device_id| {
                                self.app
                                    .devices
                                    .iter()
                                    .any(|device| &device.id == device_id)
                            })
                    });
                    if should_restart {
                        self.app.ensure_pane_capture(pane_id);
                    }
                }
            }

            let pane_ids: Vec<_> = self.app.panes.keys().copied().collect();
            for pane_id in pane_ids {
                let needs_refresh = self
                    .app
                    .panes
                    .get(&pane_id)
                    .is_some_and(|pane| pane.package_refresh_pending);
                if !needs_refresh {
                    continue;
                }

                let (device_id, package_name) = {
                    let pane = self.app.panes.get(&pane_id).unwrap();
                    (pane.device.clone(), pane.filter.package.clone())
                };

                if let Some(pane) = self.app.panes.get_mut(&pane_id) {
                    pane.package_refresh_pending = false;
                    pane.last_pid_poll = std::time::Instant::now();
                }

                if let Some(device_id) = device_id {
                    let packages = self
                        .rt_handle
                        .block_on(capture::list_packages(&device_id, &self.app.devices));
                    if let Some(pane) = self.app.panes.get_mut(&pane_id) {
                        pane.packages = packages;
                    }
                    if let Some(package_name) = package_name {
                        let pid_value = self.rt_handle.block_on(capture::get_pid_for_package(
                            &device_id,
                            &package_name,
                            &self.app.devices,
                        ));
                        if let Some(pane) = self.app.panes.get_mut(&pane_id) {
                            pane.pid_filter = pid_value;
                            pane.rebuild_filtered();
                        }
                    }
                }
            }

            let pane_ids: Vec<_> = self.app.panes.keys().copied().collect();
            for pane_id in pane_ids {
                let should_repoll = self.app.panes.get(&pane_id).is_some_and(|pane| {
                    pane.filter.package.is_some()
                        && !pane.package_refresh_pending
                        && pane.last_pid_poll.elapsed() >= PID_REPOLL_INTERVAL
                        && pane.device.as_ref().is_some_and(|device_id| {
                            self.app.devices.iter().any(|device| {
                                device.id == *device_id && device.supports_package_filter()
                            })
                        })
                });
                if !should_repoll {
                    continue;
                }

                let (device_id, package_name) = {
                    let pane = self.app.panes.get(&pane_id).unwrap();
                    (pane.device.clone(), pane.filter.package.clone())
                };

                if let Some(pane) = self.app.panes.get_mut(&pane_id) {
                    pane.last_pid_poll = std::time::Instant::now();
                }

                if let (Some(device_id), Some(package_name)) = (device_id, package_name) {
                    let new_pid = self.rt_handle.block_on(capture::get_pid_for_package(
                        &device_id,
                        &package_name,
                        &self.app.devices,
                    ));
                    if let Some(pane) = self.app.panes.get_mut(&pane_id) {
                        if pane.pid_filter != new_pid {
                            pane.pid_filter = new_pid;
                            pane.rebuild_filtered();
                        }
                    }
                }
            }
        }

        draw_ui(ctx, &mut self.app);

        if window_visible {
            let repaint_interval = if self.app.needs_live_repaint() {
                ACTIVE_REPAINT_INTERVAL
            } else {
                IDLE_REPAINT_INTERVAL
            };
            ctx.request_repaint_after(repaint_interval);
        }

        if self.copy_requested {
            self.copy_requested = false;
            #[allow(deprecated)]
            let nothing_copied = ctx.output(|output| output.copied_text.is_empty());
            if nothing_copied {
                if let Some(pane) = self.app.panes.get(&self.app.focused_pane) {
                    if let Some((lo, hi)) = pane.selected_range() {
                        let lines: Vec<String> = (lo..=hi)
                            .filter_map(|filtered_index| {
                                pane.filtered_indices
                                    .get(filtered_index)
                                    .and_then(|&entry_index| pane.entries.get(entry_index))
                                    .map(|entry| {
                                        format!(
                                            "{} {} {} {}",
                                            entry.timestamp,
                                            entry.level.as_char(),
                                            entry.tag,
                                            entry.message
                                        )
                                    })
                            })
                            .collect();
                        if !lines.is_empty() {
                            ctx.copy_text(lines.join("\n"));
                        }
                    }
                }
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.app.save_session();
    }
}

fn window_should_update(ctx: &egui::Context, frame: &eframe::Frame) -> bool {
    if ctx
        .input(|input| input.viewport().minimized)
        .unwrap_or(false)
    {
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        window_is_visible_on_macos(frame)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = frame;
        true
    }
}

#[cfg(target_os = "macos")]
fn window_is_visible_on_macos(frame: &eframe::Frame) -> bool {
    let Ok(window_handle) = frame.window_handle() else {
        return true;
    };

    let RawWindowHandle::AppKit(handle) = window_handle.as_raw() else {
        return true;
    };

    let Some(ns_view) = (unsafe { handle.ns_view.as_ptr().cast::<NSView>().as_ref() }) else {
        return true;
    };

    let Some(window) = ns_view.window() else {
        return true;
    };

    !window.isMiniaturized()
        && window
            .occlusionState()
            .contains(NSWindowOcclusionState::Visible)
}

impl CatPaneApp {
    fn handle_menu_event(&mut self, ctx: &egui::Context, event: &MenuEvent) {
        let id = &event.id;
        if *id == muda::MenuId::from("new_window") {
            self.app.spawn_new_window();
        } else if *id == muda::MenuId::from("copy") {
            self.copy_requested = true;
        } else if *id == muda::MenuId::from("find") {
            if let Some(pane) = self.app.panes.get_mut(&self.app.focused_pane) {
                pane.search_open = !pane.search_open;
                if pane.search_open {
                    pane.search_focus_requested = true;
                } else {
                    pane.search_input.clear();
                    pane.filter.set_search("");
                    pane.search_match_indices.clear();
                }
            }
        } else if *id == muda::MenuId::from("split_right") {
            self.app.split_pane(SplitDir::Vertical);
        } else if *id == muda::MenuId::from("split_down") {
            self.app.split_pane(SplitDir::Horizontal);
        } else if *id == muda::MenuId::from("close_pane") {
            if self.app.pane_tree.count() > 1 {
                let pane_id = self.app.focused_pane;
                self.app.close_pane(pane_id);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else if *id == muda::MenuId::from("next_pane") {
            self.app.cycle_focus();
        } else if *id == muda::MenuId::from("pause_resume") {
            if let Some(pane) = self.app.panes.get_mut(&self.app.focused_pane) {
                pane.paused = !pane.paused;
            }
        } else if *id == muda::MenuId::from("clear_logs") {
            if let Some(pane) = self.app.panes.get_mut(&self.app.focused_pane) {
                pane.clear();
            }
        } else if *id == muda::MenuId::from("show_help") {
            self.app.show_help = !self.app.show_help;
        }
    }
}

fn setup_menu() -> Menu {
    let menu = Menu::new();

    let app_menu = Submenu::with_items(
        "CatPane",
        true,
        &[
            &PredefinedMenuItem::about(
                None,
                Some(AboutMetadata {
                    version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    ..Default::default()
                }),
            ),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ],
    )
    .unwrap();

    let file_menu = Submenu::with_items(
        "File",
        true,
        &[&MenuItem::with_id(
            "new_window",
            "New Window",
            true,
            Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyN)),
        )],
    )
    .unwrap();

    let edit_menu = Submenu::with_items(
        "Edit",
        true,
        &[
            &MenuItem::with_id(
                "copy",
                "Copy",
                true,
                Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyC)),
            ),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(
                "find",
                "Find",
                true,
                Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyF)),
            ),
        ],
    )
    .unwrap();

    let view_menu = Submenu::with_items(
        "View",
        true,
        &[
            &MenuItem::with_id(
                "split_right",
                "Split Pane Right",
                true,
                Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyD)),
            ),
            &MenuItem::with_id(
                "split_down",
                "Split Pane Down",
                true,
                Some(Accelerator::new(
                    Some(Modifiers::SUPER | Modifiers::SHIFT),
                    Code::KeyD,
                )),
            ),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(
                "close_pane",
                "Close Pane",
                true,
                Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyW)),
            ),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("next_pane", "Next Pane", true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("pause_resume", "Pause / Resume", true, None),
            &MenuItem::with_id("clear_logs", "Clear Logs", true, None),
        ],
    )
    .unwrap();

    let window_menu = Submenu::with_items(
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::fullscreen(None),
        ],
    )
    .unwrap();

    let help_menu = Submenu::with_items(
        "Help",
        true,
        &[&MenuItem::with_id(
            "show_help",
            "Keyboard Shortcuts",
            true,
            Some(Accelerator::new(None, Code::F1)),
        )],
    )
    .unwrap();

    menu.append(&app_menu).unwrap();
    menu.append(&file_menu).unwrap();
    menu.append(&edit_menu).unwrap();
    menu.append(&view_menu).unwrap();
    menu.append(&window_menu).unwrap();
    menu.append(&help_menu).unwrap();

    menu
}

fn is_system_dark_mode() -> bool {
    #[cfg(target_os = "macos")]
    {
        Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .eq_ignore_ascii_case("dark")
            })
            .unwrap_or(true)
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

fn run_mcp_mode() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("Failed to create Tokio runtime: {err}"))?;
    let runtime_handle = runtime.handle().clone();
    runtime.block_on(run_stdio_server(runtime_handle))
}

fn run_gui_mode() -> Result<(), String> {
    if std::env::var("CATPANE_FORKED").is_err() {
        let executable = std::env::current_exe()
            .map_err(|err| format!("Failed to get executable path: {err}"))?;
        Command::new(&executable)
            .args(std::env::args_os().skip(1))
            .env("CATPANE_FORKED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("Failed to fork process: {err}"))?;
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("Failed to create Tokio runtime: {err}"))?;

    let devices = runtime.block_on(capture::list_devices());
    let runtime_handle = runtime.handle().clone();
    let is_dark = is_system_dark_mode();
    let menu = setup_menu();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 700.0])
            .with_title(APP_NAME),
        ..Default::default()
    };

    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |_cc| {
            #[cfg(target_os = "macos")]
            menu.init_for_nsapp();

            Ok(Box::new(CatPaneApp {
                app: UiApp::new(runtime_handle.clone(), devices),
                rt_handle: runtime_handle,
                _rt: runtime,
                _menu: menu,
                fonts_configured: false,
                is_dark,
                copy_requested: false,
            }))
        }),
    )
    .map_err(|err| err.to_string())
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Some(Commands::Mcp) => run_mcp_mode(),
        None => run_gui_mode(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}
