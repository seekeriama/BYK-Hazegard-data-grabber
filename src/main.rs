// Release: không hiện cửa sổ console trên Windows
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use hazegard_gui::app::{self, HazeGardApp};

fn main() -> eframe::Result {
    // Ghi lỗi nghiêm trọng ra file cạnh chương trình (release không có console)
    let crash_path = app::app_dir().join("hazegard_crash.log");
    std::panic::set_hook(Box::new(move |info| {
        let _ = std::fs::write(&crash_path, format!("{info}\n"));
    }));
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Haze Gard")
            .with_inner_size([700.0, 700.0])
            .with_resizable(false)
            .with_maximize_button(false),
        ..Default::default()
    };
    eframe::run_native("Haze Gard", options, Box::new(|cc| Ok(Box::new(HazeGardApp::new(cc)))))
}
