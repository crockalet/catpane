use egui::{self, RichText};

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
