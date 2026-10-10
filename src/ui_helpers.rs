use eframe::egui;

pub const CONTROL_RADIUS: u8 = 10;
pub const SURFACE_RADIUS: u8 = 16;

/// Stop remains usable while another game's preparation is running.
pub fn launch_control(
    ui: &mut egui::Ui,
    owned: bool,
    starting: bool,
    busy: bool,
) -> egui::Response {
    let label = if owned {
        "Stop instance"
    } else if starting {
        "Starting…"
    } else {
        "Launch modded"
    };
    ui.add_enabled(owned || (!starting && !busy), egui::Button::new(label))
}

pub fn filter_options<T: Clone + PartialEq>(
    ui: &mut egui::Ui,
    value: &mut T,
    query: &mut String,
    choices: &[(T, String)],
) -> egui::Response {
    ui.set_min_width(230.0);
    let search = ui.add(
        egui::TextEdit::singleline(query)
            .hint_text("Search options…")
            .desired_width(230.0)
            .char_limit(80),
    );
    let query = query.trim().to_lowercase();
    let mut sorted: Vec<_> = choices
        .iter()
        .filter(|(_, name)| name.to_lowercase().contains(&query))
        .collect();
    sorted.sort_by_key(|(_, name)| name.to_lowercase());
    if sorted.is_empty() {
        ui.label("No matching options");
    }
    for (option, name) in sorted {
        if ui.selectable_value(value, option.clone(), name).clicked() {
            ui.close();
        }
    }
    search
}

pub fn searchable_options<T: Clone + PartialEq>(
    ui: &mut egui::Ui,
    value: &mut T,
    choices: &[(T, String)],
) {
    let id = ui.id().with("option-search");
    let mut query = ui
        .ctx()
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_default();
    filter_options(ui, value, &mut query, choices);
    ui.ctx().data_mut(|d| d.insert_temp(id, query));
}

/// Keep form controls reachable when headers consume a compact viewport.
pub fn responsive_page<R>(
    ui: &mut egui::Ui,
    id: &str,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let width = ui.available_width();
    let height = ui.available_height();
    if height < 580.0 || width < 720.0 {
        egui::ScrollArea::both()
            .id_salt(id)
            .auto_shrink([false, false])
            .max_height(height)
            .show(ui, |ui| {
                ui.set_max_width(width);
                ui.set_max_height(height.max(800.0));
                contents(ui)
            })
            .inner
    } else {
        contents(ui)
    }
}

pub fn mod_art(ui: &mut egui::Ui, item: &crate::model::ModInfo, size: egui::Vec2) -> bool {
    let mut drawn = false;
    use base64::Engine;
    let p = &item.provenance;
    if let Some(data) = p["icon_data"].as_str() {
        let key = egui::Id::new(("mod-art", item.file.as_str(), item.version.as_str()));
        let mut texture = ui
            .ctx()
            .data_mut(|d| d.get_temp::<egui::TextureHandle>(key));
        if texture.is_none()
            && data.len() < 6 * 1024 * 1024
            && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data)
            && let Ok(image) = decode_mod_art(&bytes)
        {
            let pixels = image.to_rgba8();
            if pixels.width() <= 4096 && pixels.height() <= 4096 {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [pixels.width() as usize, pixels.height() as usize],
                    pixels.as_raw(),
                );
                let handle = ui.ctx().load_texture(
                    format!("mod-{}", item.file),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                ui.ctx().data_mut(|d| d.insert_temp(key, handle.clone()));
                texture = Some(handle);
            }
        }
        if let Some(t) = texture {
            ui.add(egui::Image::new(&t).fit_to_exact_size(size));
            drawn = true;
        }
    }
    drawn
}

pub fn mod_links(ui: &mut egui::Ui, item: &crate::model::ModInfo) {
    let p = &item.provenance;
    if let Some(authors) = p["author_links"].as_array() {
        ui.horizontal_wrapped(|ui| {
            ui.label("By");
            for author in authors {
                if let (Some(name), Some(url)) = (author["name"].as_str(), author["url"].as_str())
                    && url.starts_with("https://")
                {
                    ui.hyperlink_to(name, url);
                }
            }
        });
    }
    if let Some(url) = p["source_url"].as_str()
        && url.starts_with("https://")
    {
        ui.hyperlink_to("Original project", url);
    }
}

pub fn mod_credits(ui: &mut egui::Ui, item: &crate::model::ModInfo) {
    mod_art(ui, item, egui::vec2(96.0, 96.0));
    mod_links(ui, item);
    if let Some(notes) = item.provenance["install_notes"].as_str() {
        ui.label(notes);
    }
}

fn decode_mod_art(bytes: &[u8]) -> image::ImageResult<image::DynamicImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    reader.decode()
}

pub fn apply_corner_radii(visuals: &mut egui::Visuals) {
    let widgets = &mut visuals.widgets;
    for widget in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(CONTROL_RADIUS);
    }
    visuals.window_corner_radius = egui::CornerRadius::same(SURFACE_RADIUS);
    visuals.menu_corner_radius = egui::CornerRadius::same(SURFACE_RADIUS);
}

/// Context menus for containers without adding a click layer over their buttons.
pub fn context_menu(response: &egui::Response, contents: impl FnOnce(&mut egui::Ui)) {
    let secondary = response.contains_pointer()
        && response
            .ctx
            .input(|input| input.pointer.button_clicked(egui::PointerButton::Secondary));
    egui::Popup::menu(response)
        .open_memory(secondary.then_some(egui::SetOpenCommand::Bool(true)))
        .at_pointer_fixed()
        .show(contents);
}

#[cfg(test)]
mod page_scroll_tests {
    use super::*;
    #[test]
    fn launch_control_is_disabled_while_starting_and_stop_stays_available() {
        let ctx = egui::Context::default();
        for (owned, starting, busy, expected) in [
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, true),
            (true, false, true, true),
        ] {
            let mut enabled = false;
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    enabled = launch_control(ui, owned, starting, busy).enabled();
                });
            });
            assert_eq!(enabled, expected);
        }
    }
    #[test]
    fn option_search_sorts_labels_without_changing_selection() {
        let ctx = egui::Context::default();
        let choices = vec![
            ("z".to_owned(), "Zulu".to_owned()),
            ("a".into(), "alpha".into()),
            ("b".into(), "Beta".into()),
        ];
        let mut selected = "b".to_owned();
        let mut query = String::new();
        let mut search_rect = egui::Rect::NOTHING;
        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
            events,
            ..Default::default()
        };
        let mut frame = |events| {
            ctx.run(input(events), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    search_rect = filter_options(ui, &mut selected, &mut query, &choices).rect;
                });
            })
        };
        frame(vec![]);
        let output = frame(vec![]);
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if choices.iter().any(|(_, name)| name == t.galley.text()) => {
                    Some(t.galley.text().to_owned())
                }
                _ => None,
            })
            .collect();
        assert_eq!(labels, vec!["alpha", "Beta", "Zulu"]);
        drop(frame);
        let point = search_rect.center();
        for pressed in [true, false] {
            let _ = ctx.run(
                input(vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]),
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        filter_options(ui, &mut selected, &mut query, &choices);
                    });
                },
            );
        }
        let output = ctx.run(input(vec![egui::Event::Text("ALPHA".into())]), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                filter_options(ui, &mut selected, &mut query, &choices);
            });
        });
        assert_eq!(query, "ALPHA");
        assert_eq!(selected, "b", "Typing must not change the chosen filter");
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if choices.iter().any(|(_, name)| name == t.galley.text()) => {
                    Some(t.galley.text().to_owned())
                }
                _ => None,
            })
            .collect();
        assert_eq!(labels, vec!["alpha"]);
    }
    #[test]
    fn compact_page_scrolls_controls_that_would_be_below_the_window() {
        let ctx = egui::Context::default();
        let mut last = egui::Rect::NOTHING;
        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 420.0),
            )),
            events,
            ..Default::default()
        };
        for _ in 0..2 {
            let _ = ctx.run(input(vec![]), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    responsive_page(ui, "small-mods", |ui| {
                        for n in 0..40 {
                            last = ui.button(format!("Mod control {n}")).rect;
                        }
                    })
                });
            });
        }
        let before = last.top();
        assert!(before > 420.0);
        for _ in 0..12 {
            let _ = ctx.run(
                input(vec![
                    egui::Event::PointerMoved(egui::pos2(250.0, 250.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -250.0),
                        modifiers: Default::default(),
                    },
                ]),
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        responsive_page(ui, "small-mods", |ui| {
                            for n in 0..40 {
                                last = ui.button(format!("Mod control {n}")).rect;
                            }
                        })
                    });
                },
            );
        }
        assert!(
            last.top() < before,
            "Controls must actually move with wheel input: before={before}, after={}",
            last.top()
        );
        assert!(last.bottom() <= 420.0, "The last control must be reachable");
    }
}
