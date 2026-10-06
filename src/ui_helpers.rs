use eframe::egui;

pub const CONTROL_RADIUS: u8 = 10;
pub const SURFACE_RADIUS: u8 = 16;

pub fn mod_credits(ui: &mut egui::Ui, item: &crate::model::ModInfo) {
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
            ui.add(egui::Image::new(&t).fit_to_exact_size(egui::vec2(96.0, 96.0)));
        }
    }
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
    if let Some(notes) = p["install_notes"].as_str() {
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
