mod adb;
mod app;
mod filter;
mod log_entry;
mod mcp;
mod pane;
mod ui;

use std::{ffi::OsStr, time::Duration};

use muda::{
    AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};

#[cfg(target_os = "macos")]
use objc2_app_kit::{NSView, NSWindowOcclusionState};
#[cfg(target_os = "macos")]
use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};

const VISIBLE_REPAINT_INTERVAL: Duration = Duration::from_millis(33);
const PID_REPOLL_INTERVAL: Duration = Duration::from_secs(5);
const APP_NAME: &str = "CatPane";

/// Update `app.devices` and reconcile any pane whose saved serial is no longer
/// present (e.g. an mDNS serial replaced by an IP:port serial after dedup).
/// Matches by friendly name so the correct physical device is kept.
fn update_devices(
    app: &mut app::App,
    new_devices: Vec<adb::AdbDevice>,
    rt: &tokio::runtime::Handle,
) {
    // Build friendly-name → new serial map before swapping
    let new_by_name: std::collections::HashMap<String, String> = new_devices
        .iter()
        .map(|d| (d.friendly_name(), d.serial.clone()))
        .collect();

    let old_devices = std::mem::replace(&mut app.devices, new_devices);

    let pane_ids: Vec<_> = app.panes.keys().copied().collect();
    for pid in pane_ids {
        let serial = match app.panes.get(&pid).and_then(|p| p.device.clone()) {
            Some(s) => s,
            None => continue,
        };
        if app.devices.iter().any(|d| d.serial == serial) {
            // Serial still valid — but if the device just reappeared
            // (wasn't in old list), we need to restart logcat because
            // the old process died when the device disconnected.
            let was_present_before = old_devices.iter().any(|d| d.serial == serial);
            if was_present_before {
                continue;
            }
            // Device reappeared — restart logcat
            if let Some(pane) = app.panes.get_mut(&pid) {
                pane.stop_logcat();
                pane.start_logcat(rt);
            }
            continue;
        }
        // Find the old device entry to get its friendly name
        let friendly = match old_devices.iter().find(|d| d.serial == serial) {
            Some(d) => d.friendly_name(),
            None => continue,
        };
        // Look for a device with the same friendly name in the new list
        if let Some(new_serial) = new_by_name.get(&friendly) {
            if let Some(pane) = app.panes.get_mut(&pid) {
                pane.device = Some(new_serial.clone());
                pane.stop_logcat();
                pane.start_logcat(rt);
            }
        }
    }
}

struct CatPaneApp {
    app: app::App,
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
            ui::configure_fonts(ctx, self.is_dark);
            self.fonts_configured = true;
        }

        // Handle native menu events
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            self.handle_menu_event(ctx, &event);
        }

        // If copy was requested via native menu (Cmd+C), inject Copy event so TextEdits can handle it
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
                let devices = self.rt_handle.block_on(adb::list_devices());
                update_devices(&mut self.app, devices, &self.rt_handle);
            }

            // Auto-refresh devices from track-devices
            let mut tracker_devices: Option<Vec<adb::AdbDevice>> = None;
            if let Some(tracker) = &mut self.app.device_tracker {
                while let Ok(devices) = tracker.try_recv() {
                    tracker_devices = Some(devices);
                }
            }
            if let Some(devices) = tracker_devices {
                update_devices(&mut self.app, devices, &self.rt_handle);
            }

            // Auto-restart logcat for panes whose logcat channel died
            // (e.g. device briefly disconnected, adb crashed) but the
            // device is still in the current device list.
            // A 3-second cooldown prevents rapid-fire respawn loops when
            // the device isn't truly ready yet.
            {
                let pane_ids: Vec<_> = self.app.panes.keys().copied().collect();
                for pid in pane_ids {
                    let should_restart = self.app.panes.get(&pid).is_some_and(|p| {
                        p.logcat_handle.is_none()
                            && !p.paused
                            && p.last_logcat_restart.elapsed() >= std::time::Duration::from_secs(3)
                            && p.device.as_ref().is_some_and(|serial| {
                                self.app.devices.iter().any(|d| &d.serial == serial)
                            })
                    });
                    if should_restart {
                        if let Some(pane) = self.app.panes.get_mut(&pid) {
                            pane.start_logcat(&self.rt_handle);
                        }
                    }
                }
            }

            // Per-pane package refresh
            let pane_ids: Vec<_> = self.app.panes.keys().copied().collect();
            for pid in pane_ids {
                let needs_refresh = self
                    .app
                    .panes
                    .get(&pid)
                    .is_some_and(|p| p.package_refresh_pending);
                if !needs_refresh {
                    continue;
                }

                let (device_serial, pkg_name) = {
                    let pane = self.app.panes.get(&pid).unwrap();
                    (pane.device.clone(), pane.filter.package.clone())
                };

                if let Some(pane) = self.app.panes.get_mut(&pid) {
                    pane.package_refresh_pending = false;
                    pane.last_pid_poll = std::time::Instant::now();
                }

                if let Some(device) = device_serial {
                    let packages = self.rt_handle.block_on(adb::list_packages(&device));
                    if let Some(pane) = self.app.panes.get_mut(&pid) {
                        pane.packages = packages;
                    }
                    if let Some(pkg) = pkg_name {
                        let pid_val = self
                            .rt_handle
                            .block_on(adb::get_pid_for_package(&device, &pkg));
                        if let Some(pane) = self.app.panes.get_mut(&pid) {
                            pane.pid_filter = pid_val;
                            pane.rebuild_filtered();
                        }
                    }
                }
            }

            // Periodic PID re-poll: detect when a filtered package has restarted
            let pane_ids: Vec<_> = self.app.panes.keys().copied().collect();
            for pid in pane_ids {
                let should_repoll = self.app.panes.get(&pid).is_some_and(|p| {
                    p.filter.package.is_some()
                        && !p.package_refresh_pending
                        && p.last_pid_poll.elapsed() >= PID_REPOLL_INTERVAL
                });
                if !should_repoll {
                    continue;
                }

                let (device_serial, pkg_name) = {
                    let pane = self.app.panes.get(&pid).unwrap();
                    (pane.device.clone(), pane.filter.package.clone())
                };

                if let Some(pane) = self.app.panes.get_mut(&pid) {
                    pane.last_pid_poll = std::time::Instant::now();
                }

                if let (Some(device), Some(pkg)) = (device_serial, pkg_name) {
                    let new_pid = self
                        .rt_handle
                        .block_on(adb::get_pid_for_package(&device, &pkg));
                    if let Some(pane) = self.app.panes.get_mut(&pid) {
                        if pane.pid_filter != new_pid {
                            pane.pid_filter = new_pid;
                            pane.rebuild_filtered();
                        }
                    }
                }
            }
        }

        ui::draw_ui(ctx, &mut self.app);

        if window_visible {
            ctx.request_repaint_after(VISIBLE_REPAINT_INTERVAL);
        }

        // After render: if Cmd+C was requested and no TextEdit handled it, copy selected log rows
        if self.copy_requested {
            self.copy_requested = false;
            #[allow(deprecated)]
            let nothing_copied = ctx.output(|o| o.copied_text.is_empty());
            if nothing_copied {
                if let Some(pane) = self.app.panes.get(&self.app.focused_pane) {
                    if let Some((lo, hi)) = pane.selected_range() {
                        let lines: Vec<String> = (lo..=hi)
                            .filter_map(|fi| {
                                pane.filtered_indices
                                    .get(fi)
                                    .and_then(|&ei| pane.entries.get(ei))
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

    // eframe gives us the backing NSView; the view/window stay owned by AppKit.
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
            self.app.split_pane(pane::SplitDir::Vertical);
        } else if *id == muda::MenuId::from("split_down") {
            self.app.split_pane(pane::SplitDir::Horizontal);
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
        } else if *id == muda::MenuId::from("wireless_debug") {
            self.app.show_wireless_dialog = !self.app.show_wireless_dialog;
        }
    }
}

fn setup_menu() -> Menu {
    let menu = Menu::new();

    // macOS App menu
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
        &[
            &MenuItem::with_id(
                "new_window",
                "New Window",
                true,
                Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyN)),
            ),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("wireless_debug", "Wireless Debug…", true, None),
        ],
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
        std::process::Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
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

fn should_run_mcp_stdio() -> bool {
    matches!(
        std::env::args_os().nth(1).as_deref(),
        Some(arg) if arg == OsStr::new("mcp")
    )
}

fn run_mcp_mode() -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("Failed to create tokio runtime: {err}"))?;
    let rt_handle = rt.handle().clone();
    rt.block_on(mcp::run_stdio_server(rt_handle))
}

fn main() -> eframe::Result<()> {
    if should_run_mcp_stdio() {
        if let Err(err) = run_mcp_mode() {
            eprintln!("{err}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Auto-fork: re-launch as a detached process unless already forked
    if std::env::var("CATPANE_FORKED").is_err() {
        use std::process::Command;
        let exe = std::env::current_exe().expect("Failed to get executable path");
        Command::new(&exe)
            .args(std::env::args_os().skip(1))
            .env("CATPANE_FORKED", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("Failed to fork process");
        return Ok(());
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    let devices = rt.block_on(adb::list_devices());
    let rt_handle = rt.handle().clone();
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
                app: app::App::new(rt_handle.clone(), devices),
                rt_handle,
                _rt: rt,
                _menu: menu,
                fonts_configured: false,
                is_dark,
                copy_requested: false,
            }))
        }),
    )
}
