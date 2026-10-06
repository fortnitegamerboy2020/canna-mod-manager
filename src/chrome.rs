use eframe::egui::{self, Color32};

pub const CANVAS: Color32 = Color32::from_rgb(18, 24, 22);
pub struct Chrome {
    icons: [egui::TextureHandle; 8],
}
impl Chrome {
    pub fn new(ctx: &egui::Context) -> Self {
        let icons = [
            ("library", include_bytes!("assets/play.png").as_slice()),
            (
                "discover",
                include_bytes!("assets/telescope.png").as_slice(),
            ),
            ("console", include_bytes!("assets/command.png").as_slice()),
            ("settings", include_bytes!("assets/settings.png").as_slice()),
            ("skins", include_bytes!("assets/skins.png").as_slice()),
            (
                "minecraft",
                include_bytes!("assets/minecraft-logo.png").as_slice(),
            ),
            ("website", include_bytes!("assets/web.png").as_slice()),
            (
                "downloads",
                include_bytes!("assets/download.png").as_slice(),
            ),
        ]
        .map(|(name, bytes)| {
            let mut image = image::load_from_memory(bytes)
                .expect("Bundled navigation icon")
                .to_rgba8();
            // The supplied icons are black alpha masks. Render their original
            // outlines in a light tint so they remain readable on dark surfaces.
            for pixel in image.pixels_mut() {
                pixel.0[..3].fill(255);
            }
            ctx.load_texture(
                name,
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            )
        });
        Self { icons }
    }
    pub fn nav(
        &self,
        ui: &mut egui::Ui,
        index: usize,
        label: &str,
        selected: bool,
    ) -> egui::Response {
        let tint = if selected {
            super::GREEN
        } else {
            Color32::from_rgb(193, 210, 198)
        };
        let response = ui
            .add(
                egui::Button::image(
                    egui::Image::new(&self.icons[index])
                        .fit_to_exact_size(egui::vec2(24.0, 24.0))
                        .tint(tint),
                )
                .selected(selected)
                .min_size(egui::vec2(48.0, 48.0)),
            )
            .on_hover_text(label);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        response
    }
    pub fn packs(ui: &mut egui::Ui, selected: bool) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::click());
        let visuals = ui.style().interact_selectable(&response, selected);
        ui.painter().rect(
            rect,
            super::ui_helpers::CONTROL_RADIUS,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let color = if selected {
            super::GREEN
        } else {
            Color32::from_rgb(193, 210, 198)
        };
        for offset in [5.0, 0.0, -5.0] {
            let center = rect.center() + egui::vec2(0.0, offset);
            ui.painter().rect(
                egui::Rect::from_center_size(center, egui::vec2(22.0, 12.0)),
                3,
                CANVAS,
                egui::Stroke::new(1.7_f32, color),
                egui::StrokeKind::Inside,
            );
        }
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Modpacks"));
        response.on_hover_text("Modpacks")
    }
}

pub fn title_bar(ctx: &egui::Context) -> [egui::Rect; 2] {
    let mut controls = [egui::Rect::NOTHING; 2];
    egui::TopBottomPanel::top("window_controls")
        .exact_height(44.0)
        .frame(
            egui::Frame::new()
                .fill(CANVAS)
                .inner_margin(egui::Margin::symmetric(16, 8)),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (index, (label, color, command)) in [
                    (
                        "Close",
                        Color32::from_rgb(232, 104, 107),
                        egui::ViewportCommand::Close,
                    ),
                    (
                        "Minimize",
                        Color32::from_rgb(235, 192, 93),
                        egui::ViewportCommand::Minimized(true),
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(30.0, 18.0), egui::Sense::click());
                    controls[index] = rect;
                    ui.painter().rect_filled(
                        rect,
                        super::ui_helpers::CONTROL_RADIUS,
                        if response.hovered() {
                            color.gamma_multiply(1.15)
                        } else {
                            color
                        },
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label)
                    });
                    if response.clicked() {
                        ctx.send_viewport_cmd(command);
                    }
                    response.on_hover_text(label);
                }
                let (_, drag) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 28.0),
                    egui::Sense::click_and_drag(),
                );
                if drag.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(
                        !ctx.input(|i| i.viewport().maximized.unwrap_or(false)),
                    ));
                }
            });
        });
    controls
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn window_controls_send_close_and_minimize_commands() {
        for index in 0..2 {
            let ctx = egui::Context::default();
            let input = |events| egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1240.0, 820.0),
                )),
                events,
                ..Default::default()
            };
            let mut controls = [egui::Rect::NOTHING; 2];
            let _ = ctx.run(input(vec![]), |ctx| {
                controls = title_bar(ctx);
            });
            assert!(
                controls[1].right() < controls[0].left(),
                "Yellow is immediately left of red"
            );
            let point = controls[index].center();
            let _ = ctx.run(
                input(vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ]),
                |ctx| {
                    title_bar(ctx);
                },
            );
            let output = ctx.run(
                input(vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }]),
                |ctx| {
                    title_bar(ctx);
                },
            );
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .iter()
                    .any(|command| {
                        if index == 0 {
                            matches!(command, egui::ViewportCommand::Close)
                        } else {
                            matches!(command, egui::ViewportCommand::Minimized(true))
                        }
                    })
            );
        }
    }
}
