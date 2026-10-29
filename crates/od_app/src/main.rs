// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod export;
mod fabric_shelf;
mod files;
mod i18n;
mod panels;
mod pdf;
mod selection;
mod theme;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OpenDesign")
            .with_inner_size([1440.0, 880.0])
            .with_min_inner_size([960.0, 600.0]),
        // The 3D view draws into egui's render pass with depth testing; its
        // pipelines are built for exactly these settings.
        depth_buffer: od_render::DEPTH_BITS,
        multisampling: od_render::MSAA_SAMPLES,
        ..Default::default()
    };
    eframe::run_native(
        "OpenDesign",
        options,
        Box::new(|cc| Ok(Box::new(app::OpenDesignApp::new(cc)))),
    )
}
