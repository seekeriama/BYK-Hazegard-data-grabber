// Release: không hiện cửa sổ console trên Windows
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use hazegard_gui::app::{self, HazeGardApp};

/// Ghi lỗi ra hazegard_crash.log cạnh chương trình và (trên Windows) hiện hộp thoại báo lỗi,
/// vì bản release không có console nên lỗi sẽ không hiện ở đâu cả.
fn report_fatal(msg: &str) {
    let _ = std::fs::write(app::app_dir().join("hazegard_crash.log"), msg);
    eprintln!("{msg}");
    #[cfg(windows)]
    {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Haze Gard - lỗi")
            .set_description(msg)
            .show();
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| report_fatal(&format!("Lỗi chương trình:\n{info}"))));

    // Cho phép wgpu thử DirectX 12, Vulkan rồi OpenGL (máy cũ / driver kém); chỉ đặt khi người dùng chưa đặt.
    if std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: gọi ở đầu main, trước khi tạo bất kỳ luồng nào.
        unsafe { std::env::set_var("WGPU_BACKEND", "dx12,vulkan,gl") };
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Haze Gard")
            .with_inner_size([700.0, 700.0])
            .with_resizable(false)
            .with_maximize_button(false),
        ..Default::default()
    };
    if let Err(e) = eframe::run_native("Haze Gard", options, Box::new(|cc| Ok(Box::new(HazeGardApp::new(cc))))) {
        report_fatal(&format!(
            "Không khởi động được giao diện đồ họa:\n{e}\n\n\
             Thường do driver card màn hình (DirectX/Vulkan/OpenGL) thiếu hoặc quá cũ, \
             hoặc đang chạy qua Remote Desktop. Hãy cập nhật driver card màn hình, \
             hoặc dùng bản Python (hazegard_gui.py)."
        ));
    }
}
