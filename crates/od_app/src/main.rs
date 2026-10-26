// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod files;
mod panels;
mod theme;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OpenDesign")
            .with_inner_size([1440.0, 880.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "OpenDesign",
        options,
        Box::new(|cc| Ok(Box::new(app::OpenDesignApp::new(cc)))),
    )
}
