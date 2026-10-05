use eframe::egui;

pub const CONTROL_RADIUS: u8 = 10;
pub const SURFACE_RADIUS: u8 = 16;

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
