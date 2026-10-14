use eframe::egui::{self, Color32, FontFamily, FontId, RichText, Stroke, TextStyle};

pub const ACCENT: Color32 = Color32::from_rgb(112, 82, 232);
pub const MINT: Color32 = Color32::from_rgb(151, 234, 201);
pub fn surface(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(30, 32, 43)
    } else {
        Color32::WHITE
    }
}
pub fn muted(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(157, 165, 187)
    } else {
        Color32::from_rgb(112, 119, 139)
    }
}
pub fn border(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(49, 53, 69)
    } else {
        Color32::from_rgb(232, 233, 240)
    }
}
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".into(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/Inter.ttf")).into(),
    );
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .unwrap()
        .insert(0, "Inter".into());
    ctx.set_fonts(fonts);
}
pub fn style(ctx: &egui::Context) {
    let dark = ctx.style().visuals.dark_mode;
    let mut s = (*ctx.style()).clone();
    s.text_styles
        .insert(TextStyle::Body, FontId::proportional(15.0));
    s.text_styles
        .insert(TextStyle::Button, FontId::proportional(14.0));
    s.text_styles
        .insert(TextStyle::Small, FontId::proportional(12.0));
    s.text_styles
        .insert(TextStyle::Heading, FontId::proportional(23.0));
    s.spacing.item_spacing = egui::vec2(10.0, 10.0);
    s.spacing.button_padding = egui::vec2(16.0, 11.0);
    s.spacing.interact_size = egui::vec2(36.0, 36.0);
    s.spacing.indent = 18.0;
    let bg = if dark {
        Color32::from_rgb(21, 23, 32)
    } else {
        Color32::from_rgb(247, 248, 252)
    };
    let card = if dark {
        Color32::from_rgb(30, 32, 43)
    } else {
        Color32::WHITE
    };
    let edge = if dark {
        Color32::from_rgb(49, 53, 69)
    } else {
        Color32::from_rgb(232, 233, 240)
    };
    let text = if dark {
        Color32::from_rgb(238, 240, 247)
    } else {
        Color32::from_rgb(34, 39, 58)
    };
    s.visuals.panel_fill = bg;
    s.visuals.window_fill = card;
    s.visuals.extreme_bg_color = bg;
    s.visuals.faint_bg_color = bg;
    s.visuals.override_text_color = Some(text);
    s.visuals.window_stroke = Stroke::new(1.0_f32, edge);
    s.visuals.window_corner_radius = 16.into();
    s.visuals.selection.bg_fill = ACCENT;
    s.visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    s.visuals.hyperlink_color = ACCENT;
    for w in [
        &mut s.visuals.widgets.inactive,
        &mut s.visuals.widgets.hovered,
        &mut s.visuals.widgets.active,
    ] {
        w.corner_radius = 9.into();
        w.bg_stroke = Stroke::new(1.0_f32, edge);
        w.fg_stroke = Stroke::new(1.0_f32, text);
    }
    s.visuals.widgets.inactive.bg_fill = card;
    s.visuals.widgets.inactive.weak_bg_fill = card;
    s.visuals.widgets.hovered.bg_fill = if dark {
        Color32::from_rgb(45, 42, 66)
    } else {
        Color32::from_rgb(242, 238, 255)
    };
    s.visuals.widgets.active.bg_fill = if dark {
        Color32::from_rgb(57, 48, 85)
    } else {
        Color32::from_rgb(231, 222, 255)
    };
    s.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, edge);
    s.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, text);
    ctx.set_style(s);
}
pub fn card(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(surface(ui))
        .stroke(Stroke::new(1.0_f32, border(ui)))
        .corner_radius(16)
        .inner_margin(22.0)
}
pub fn primary(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).color(Color32::WHITE).strong())
        .fill(ACCENT)
        .stroke(Stroke::NONE)
        .corner_radius(9)
}
pub fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).strong().color(muted(ui)));
}
pub fn subtitle(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(14.0).color(muted(ui)));
}
pub fn app_icon() -> egui::IconData {
    egui::IconData {
        rgba: include_bytes!("../assets/icons/openairecorder-64.rgba").to_vec(),
        width: 64,
        height: 64,
    }
}
pub fn logo(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 10, ACCENT);
        for (i, h) in [10.0, 20.0, 14.0].iter().enumerate() {
            let x = rect.left() + 9.0 + i as f32 * 6.0;
            ui.painter().rect_filled(
                egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(3.0, *h)),
                2,
                MINT,
            );
        }
        ui.label(RichText::new("OpenAIRecorder").size(17.0).strong());
    });
}
pub fn waveform(ui: &mut egui::Ui, size: egui::Vec2, level: Option<f32>) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let n = (rect.width() / 8.0) as usize;
    for i in 0..n {
        let envelope = (i as f32 / n.max(1) as f32 * std::f32::consts::PI).sin();
        let pulse = ((i as f32 * 1.7).sin().abs() * 0.75 + 0.25) * envelope;
        let h =
            5.0 + pulse * (rect.height() - 8.0) * level.map(|v| v.clamp(0.0, 1.0)).unwrap_or(0.8);
        let bar = egui::Rect::from_center_size(
            egui::pos2(rect.left() + i as f32 * 8.0 + 3.0, rect.center().y),
            egui::vec2(3.0, h),
        );
        ui.painter().rect_filled(
            bar,
            2,
            if i % 4 == 0 {
                MINT
            } else {
                Color32::from_rgb(167, 150, 245)
            },
        );
    }
}
pub fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    let color = if color == ACCENT && ui.visuals().dark_mode {
        Color32::from_rgb(185, 168, 255)
    } else {
        color
    };
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(color).strong());
        });
}
