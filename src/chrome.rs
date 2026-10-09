use eframe::egui::{self, Color32};

pub const CANVAS: Color32 = Color32::from_rgb(18, 24, 22);
/// Borderless Windows windows need explicit native resize gestures.
pub fn resize_handles(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    use egui::ResizeDirection::*;
    let r = ctx.content_rect();
    let edge = 5.0;
    let corner = 14.0;
    let zones = [
        (
            egui::Rect::from_min_size(r.min, egui::vec2(corner, corner)),
            NorthWest,
            egui::CursorIcon::ResizeNwSe,
        ),
        (
            egui::Rect::from_min_size(
                egui::pos2(r.right() - corner, r.top()),
                egui::vec2(corner, corner),
            ),
            NorthEast,
            egui::CursorIcon::ResizeNeSw,
        ),
        (
            egui::Rect::from_min_size(
                egui::pos2(r.left(), r.bottom() - corner),
                egui::vec2(corner, corner),
            ),
            SouthWest,
            egui::CursorIcon::ResizeNeSw,
        ),
        (
            egui::Rect::from_min_size(
                r.max - egui::vec2(corner, corner),
                egui::vec2(corner, corner),
            ),
            SouthEast,
            egui::CursorIcon::ResizeNwSe,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(r.left() + corner, r.top()),
                egui::pos2(r.right() - corner, r.top() + edge),
            ),
            North,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(r.left() + corner, r.bottom() - edge),
                egui::pos2(r.right() - corner, r.bottom()),
            ),
            South,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(r.left(), r.top() + corner),
                egui::pos2(r.left() + edge, r.bottom() - corner),
            ),
            West,
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(r.right() - edge, r.top() + corner),
                egui::pos2(r.right(), r.bottom() - corner),
            ),
            East,
            egui::CursorIcon::ResizeHorizontal,
        ),
    ];
    for (index, (rect, direction, cursor)) in zones.into_iter().enumerate() {
        let response = egui::Area::new(egui::Id::new(("window-resize", index)))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .constrain(false)
            .movable(false)
            .show(ctx, |ui| {
                ui.allocate_exact_size(rect.size(), egui::Sense::drag()).1
            })
            .inner;
        if response.hovered() || response.dragged() {
            ctx.set_cursor_icon(cursor);
        }
        if response.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
}
pub struct Chrome {
    icons: [egui::TextureHandle; 11],
    minecraft_banner: egui::TextureHandle,
    logo: egui::TextureHandle,
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
            ("back", include_bytes!("assets/back.png").as_slice()),
            ("error", include_bytes!("assets/error.png").as_slice()),
            ("warning", include_bytes!("assets/warning.png").as_slice()),
        ]
        .map(|(name, bytes)| {
            let mut image = image::load_from_memory(bytes)
                .expect("Bundled navigation icon")
                .to_rgba8();
            // The supplied icons are black alpha masks. Render their original
            // outlines in a light tint so they remain readable on dark surfaces.
            if name != "minecraft" {
                for pixel in image.pixels_mut() {
                    pixel.0[..3].fill(255);
                }
            }
            let bounds = image
                .enumerate_pixels()
                .filter(|(_, _, p)| p.0[3] > 16)
                .fold(None::<(u32, u32, u32, u32)>, |b, x| {
                    Some(match b {
                        None => (x.0, x.1, x.0, x.1),
                        Some((l, t, r, b)) => (l.min(x.0), t.min(x.1), r.max(x.0), b.max(x.1)),
                    })
                });
            if let Some((l, t, r, b)) = bounds {
                image = image::imageops::crop_imm(&image, l, t, r - l + 1, b - t + 1).to_image();
            }
            if image.width() > 256 || image.height() > 256 {
                image = image::DynamicImage::ImageRgba8(image)
                    .resize(256, 256, image::imageops::FilterType::Lanczos3)
                    .to_rgba8();
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
        let banner = image::load_from_memory(include_bytes!("assets/minecraft-banner.png"))
            .expect("Bundled Minecraft banner")
            .to_rgba8();
        let minecraft_banner = ctx.load_texture(
            "minecraft-banner",
            egui::ColorImage::from_rgba_unmultiplied(
                [banner.width() as usize, banner.height() as usize],
                banner.as_raw(),
            ),
            egui::TextureOptions::LINEAR,
        );
        let image = image::load_from_memory(include_bytes!("assets/canna-logo.png"))
            .expect("Bundled Canna logo")
            .to_rgba8();
        let logo = ctx.load_texture(
            "canna-logo",
            egui::ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            ),
            egui::TextureOptions::LINEAR,
        );
        Self {
            logo,
            icons,
            minecraft_banner,
        }
    }
    pub fn logo(&self) -> &egui::TextureHandle {
        &self.logo
    }
    pub fn minecraft_banner(&self) -> &egui::TextureHandle {
        &self.minecraft_banner
    }
    pub fn nav(
        &self,
        ui: &mut egui::Ui,
        index: usize,
        label: &str,
        selected: bool,
    ) -> egui::Response {
        let tint = if index == 5 {
            Color32::WHITE
        } else if selected {
            super::GREEN
        } else {
            Color32::from_rgb(193, 210, 198)
        };
        let previous_padding = ui.spacing().button_padding;
        if index >= 9 {
            ui.spacing_mut().button_padding = egui::vec2(4.0, 4.0);
        }
        let response = ui
            .add(
                egui::Button::image(
                    egui::Image::new(&self.icons[index])
                        .fit_to_exact_size(egui::vec2(
                            if index == 6 { 32.0 } else { 24.0 },
                            if index == 6 { 32.0 } else { 24.0 },
                        ))
                        .tint(tint),
                )
                .selected(selected)
                .min_size(if index >= 9 {
                    egui::vec2(32.0, 32.0)
                } else {
                    egui::vec2(48.0, 48.0)
                }),
            )
            .on_hover_text(label);
        ui.spacing_mut().button_padding = previous_padding;
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        response
    }
    pub fn close_game(ui: &mut egui::Ui, game_name: &str) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::click());
        let hovered = response.hovered() || response.has_focus();
        let color = if hovered {
            Color32::WHITE
        } else {
            Color32::from_rgb(244, 158, 160)
        };
        if ui.is_rect_visible(rect) {
            ui.painter().rect(
                rect,
                super::ui_helpers::CONTROL_RADIUS,
                if hovered {
                    Color32::from_rgb(96, 43, 46)
                } else {
                    Color32::from_rgb(57, 35, 37)
                },
                egui::Stroke::new(1.0_f32, Color32::from_rgb(139, 73, 77)),
                egui::StrokeKind::Inside,
            );
            // Standard close-X shape, matching https://lucide.dev/icons/x.
            // Paint vector strokes directly so the control stays sharp at any DPI.
            let center = rect.center() - egui::vec2(0.0, 6.0);
            for diagonal in [egui::vec2(9.0, 9.0), egui::vec2(9.0, -9.0)] {
                let ends = [center - diagonal, center + diagonal];
                ui.painter()
                    .line_segment(ends, egui::Stroke::new(2.4_f32, color));
                for end in ends {
                    ui.painter().circle_filled(end, 1.2, color);
                }
            }
            ui.painter().text(
                rect.center() + egui::vec2(0.0, 13.0),
                egui::Align2::CENTER_CENTER,
                "Close",
                egui::FontId::proportional(10.0),
                color,
            );
        }
        let label = format!("Close {game_name}");
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label)
        });
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("{label}\nStops the game launched by Canna."))
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

pub struct TitleBar {
    #[cfg_attr(not(test), allow(dead_code))] // Geometry for headless click tests.
    pub controls: [egui::Rect; 3],
    pub account_clicked: bool,
}
pub fn title_bar(ctx: &egui::Context, account_label: &str) -> TitleBar {
    let mut controls = [egui::Rect::NOTHING; 3];
    let mut account_clicked = false;
    egui::TopBottomPanel::top("window_controls")
        .exact_height(44.0)
        .frame(
            egui::Frame::new()
                .fill(CANVAS)
                .inner_margin(egui::Margin::symmetric(16, 8)),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.horizontal(|ui| {
                account_clicked = ui.button(account_label).clicked();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (index, (label, color, command)) in [
                        (
                            "Close",
                            Color32::from_rgb(232, 104, 107),
                            egui::ViewportCommand::Close,
                        ),
                        (
                            if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
                                "Restore"
                            } else {
                                "Maximize"
                            },
                            Color32::from_rgb(103, 194, 113),
                            egui::ViewportCommand::Maximized(
                                !ctx.input(|i| i.viewport().maximized.unwrap_or(false)),
                            ),
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
        });
    TitleBar {
        controls,
        account_clicked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn border_drags_request_native_resize() {
        for (point, direction) in [
            (egui::pos2(1238.0, 400.0), egui::ResizeDirection::East),
            (egui::pos2(1235.0, 815.0), egui::ResizeDirection::SouthEast),
            (egui::pos2(2.0, 400.0), egui::ResizeDirection::West),
        ] {
            let ctx = egui::Context::default();
            let input = |events| egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1240.0, 820.0),
                )),
                events,
                ..Default::default()
            };
            for _ in 0..2 {
                let _ = ctx.run(input(vec![]), resize_handles);
            }
            let mut commands = Vec::new();
            for events in [
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
                vec![egui::Event::PointerMoved(point + egui::vec2(20.0, 20.0))],
            ] {
                let output = ctx.run(input(events), resize_handles);
                commands.extend(
                    output.viewport_output[&egui::ViewportId::ROOT]
                        .commands
                        .clone(),
                );
            }
            assert!(
                commands
                    .iter()
                    .any(|c| matches!(c, egui::ViewportCommand::BeginResize(d) if *d==direction)),
                "{direction:?}: {commands:?}"
            );
        }
    }
    #[test]
    fn window_controls_send_close_maximize_restore_and_minimize_commands() {
        for (index, maximized) in [(0, false), (1, false), (1, true), (2, false)] {
            let ctx = egui::Context::default();
            let input = |events| {
                let mut input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1240.0, 820.0),
                    )),
                    events,
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .maximized = Some(maximized);
                input
            };
            let mut controls = [egui::Rect::NOTHING; 3];
            let _ = ctx.run(input(vec![]), |ctx| {
                controls = title_bar(ctx, "Log in").controls;
            });
            assert!(
                controls[2].right() < controls[1].left()
                    && controls[1].right() < controls[0].left(),
                "Yellow, green, red from left to right"
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
                    title_bar(ctx, "Log in");
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
                    title_bar(ctx, "Account");
                },
            );
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .iter()
                    .any(|command| {
                        if index == 0 {
                            matches!(command, egui::ViewportCommand::Close)
                        } else if index==1 {
                            matches!(command, egui::ViewportCommand::Maximized(value) if *value == !maximized)
                        } else {
                            matches!(command, egui::ViewportCommand::Minimized(true))
                        }
                    })
            );
        }
    }
}
