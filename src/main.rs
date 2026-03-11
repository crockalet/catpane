mod adb;
mod app;
mod filter;
mod log_entry;
mod pane;
mod ui;

use muda::{
    AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};

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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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

        if self.app.device_refresh_pending {
            self.app.device_refresh_pending = false;
            self.app.devices = self.rt_handle.block_on(adb::list_devices());
        }

        // Auto-refresh devices from track-devices
        if let Some(tracker) = &mut self.app.device_tracker {
            while let Ok(devices) = tracker.try_recv() {
                self.app.devices = devices;
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

        ui::draw_ui(ctx, &mut self.app);

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
                if !pane.search_open {
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

fn main() -> eframe::Result<()> {
    // Auto-fork: re-launch as a detached process unless already forked
    if std::env::var("CATPANE_FORKED").is_err() {
        use std::process::Command;
        let exe = std::env::current_exe().expect("Failed to get executable path");
        Command::new(&exe)
            .args(std::env::args().skip(1))
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
            .with_title("catpane"),
        ..Default::default()
    };

    eframe::run_native(
        "catpane",
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
