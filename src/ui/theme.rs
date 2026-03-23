use egui::{self, Color32, FontData, FontDefinitions, FontFamily, Vec2};

// Layout constants
pub const LOG_ROW_HEIGHT: f32 = 18.0;

// OneDark Dark palette
pub const OD_BG: Color32 = Color32::from_rgb(40, 44, 52);
pub const OD_BG_LIGHT: Color32 = Color32::from_rgb(44, 49, 58);
pub const OD_BG_HL: Color32 = Color32::from_rgb(62, 68, 81);
pub const OD_FG: Color32 = Color32::from_rgb(171, 178, 191);
pub const OD_FG_DIM: Color32 = Color32::from_rgb(92, 99, 112);
pub const OD_BLUE: Color32 = Color32::from_rgb(97, 175, 239);
pub const OD_CYAN: Color32 = Color32::from_rgb(86, 182, 194);
pub const OD_GREEN: Color32 = Color32::from_rgb(152, 195, 121);
pub const OD_RED: Color32 = Color32::from_rgb(224, 108, 117);
#[allow(dead_code)]
pub const OD_YELLOW: Color32 = Color32::from_rgb(229, 192, 123);
#[allow(dead_code)]
pub const OD_PURPLE: Color32 = Color32::from_rgb(198, 120, 221);

// OneDark Light palette
pub const OL_BG: Color32 = Color32::from_rgb(250, 250, 250);
pub const OL_BG_LIGHT: Color32 = Color32::from_rgb(240, 240, 240);
pub const OL_BG_HL: Color32 = Color32::from_rgb(225, 225, 228);
pub const OL_FG: Color32 = Color32::from_rgb(56, 58, 66);
pub const OL_FG_DIM: Color32 = Color32::from_rgb(120, 125, 137);
pub const OL_BLUE: Color32 = Color32::from_rgb(64, 120, 242);
pub const OL_GREEN: Color32 = Color32::from_rgb(80, 161, 79);
pub const OL_RED: Color32 = Color32::from_rgb(228, 86, 73);
pub const OL_BORDER: Color32 = Color32::from_rgb(210, 212, 216);

pub fn configure_fonts(ctx: &egui::Context, is_dark: bool) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "JetBrainsMono".to_owned(),
        FontData::from_static(include_bytes!(
            "../../assets/fonts/JetBrainsMono-Regular.ttf"
        ))
        .into(),
    );
    fonts.font_data.insert(
        "JetBrainsMono-Bold".to_owned(),
        FontData::from_static(include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf")).into(),
    );
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrainsMono".to_owned());
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "JetBrainsMono".to_owned());
    ctx.set_fonts(fonts);

    let mut visuals = if is_dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    if is_dark {
        visuals.panel_fill = OD_BG;
        visuals.window_fill = OD_BG;
        visuals.extreme_bg_color = Color32::from_rgb(30, 34, 42);
        visuals.faint_bg_color = OD_BG_LIGHT;
        visuals.code_bg_color = OD_BG_LIGHT;
        visuals.selection.bg_fill = OD_BG_HL;
        visuals.selection.stroke = egui::Stroke::new(1.0, OD_BLUE);
        for w in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
        ] {
            w.bg_fill = OD_BG_LIGHT;
            w.weak_bg_fill = OD_BG_LIGHT;
            w.fg_stroke = egui::Stroke::new(1.0, OD_FG);
            w.bg_stroke = egui::Stroke::new(1.0, OD_BG_HL);
        }
        for w in [
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            w.bg_fill = OD_BG_HL;
            w.weak_bg_fill = OD_BG_HL;
            w.fg_stroke = egui::Stroke::new(1.0, Color32::WHITE);
            w.bg_stroke = egui::Stroke::new(1.0, OD_BLUE);
        }
        visuals.window_stroke = egui::Stroke::new(1.0, OD_BG_HL);
        visuals.window_shadow = egui::Shadow::NONE;
        visuals.override_text_color = Some(OD_FG);
    } else {
        visuals.panel_fill = OL_BG;
        visuals.window_fill = OL_BG;
        visuals.extreme_bg_color = Color32::WHITE;
        visuals.faint_bg_color = OL_BG_LIGHT;
        visuals.selection.bg_fill = Color32::from_rgb(195, 215, 255);
        visuals.selection.stroke = egui::Stroke::new(1.0, OL_BLUE);
        for w in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
        ] {
            w.bg_fill = OL_BG_LIGHT;
            w.weak_bg_fill = OL_BG_LIGHT;
            w.fg_stroke = egui::Stroke::new(1.0, OL_FG);
            w.bg_stroke = egui::Stroke::new(1.0, OL_BORDER);
        }
        for w in [
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            w.bg_fill = OL_BG_HL;
            w.weak_bg_fill = OL_BG_HL;
            w.fg_stroke = egui::Stroke::new(1.0, OL_FG);
            w.bg_stroke = egui::Stroke::new(1.0, OL_BLUE);
        }
        visuals.override_text_color = Some(OL_FG);
    }
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(6.0, 4.0);
    ctx.set_style(style);
}
