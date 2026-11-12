// Release: không hiện cửa sổ console trên Windows
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Khởi động an toàn: chương trình chạy giao diện trong một tiến trình con. Nếu driver đồ họa làm
//! tiến trình con sập/thoát lỗi ngay lúc khởi động, tiến trình chính tự thử kiểu đồ họa tiếp theo
//! (DirectX 12 -> Vulkan -> OpenGL) và ghi nhật ký vào hazegard_startup.log.

use chrono::Local;
use hazegard_gui::app::{self, HazeGardApp};
use std::io::Write;
use std::process::{Command, exit};
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "HAZEGARD_CHILD_BACKEND";
const EXIT_GRAPHICS_ERROR: i32 = 3;
/// Thoát lỗi trong khoảng thời gian này sau khi mở -> coi là lỗi khởi động đồ họa
const STARTUP_WINDOW: Duration = Duration::from_secs(20);

fn startup_log(line: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(app::app_dir().join("hazegard_startup.log")) {
        let _ = writeln!(f, "[{}] {line}", Local::now().format("%Y-%m-%d %H:%M:%S"));
    }
}

/// Ghi lỗi ra hazegard_crash.log và (trên Windows) hiện hộp thoại báo lỗi.
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

/// Chạy giao diện. Trả về mã thoát.
fn run_gui(as_child: bool) -> i32 {
    if let Ok(b) = std::env::var("HAZEGARD_TEST_ABORT_BACKEND") {
        // chỉ dùng khi thử nghiệm: giả lập driver làm sập tiến trình
        if std::env::var("WGPU_BACKEND").map(|v| v == b).unwrap_or(false) {
            std::process::abort();
        }
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Haze Gard")
            .with_inner_size([560.0, 640.0])
            .with_min_inner_size([470.0, 560.0])
            .with_resizable(true),
        ..Default::default()
    };
    match eframe::run_native("Haze Gard", options, Box::new(|cc| Ok(Box::new(HazeGardApp::new(cc))))) {
        Ok(()) => 0,
        Err(e) => {
            if as_child {
                startup_log(&format!("giao diện lỗi: {e}"));
                EXIT_GRAPHICS_ERROR
            } else {
                report_fatal(&format!(
                    "Không khởi động được giao diện đồ họa:\n{e}\n\nHãy cập nhật driver card màn hình, \
                     hoặc dùng bản Python (hazegard_gui.py)."
                ));
                1
            }
        }
    }
}

fn launcher() -> i32 {
    let backends: Vec<String> = std::env::var("HAZEGARD_BACKENDS")
        .unwrap_or_else(|_| "dx12,vulkan,gl".into())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            startup_log(&format!("không xác định được đường dẫn chương trình ({e}); chạy trực tiếp"));
            return run_gui(false);
        }
    };
    let mut attempts: Vec<String> = vec![];
    for be in &backends {
        startup_log(&format!("thử kiểu đồ họa: {be}"));
        let t0 = Instant::now();
        let status = Command::new(&exe).env(CHILD_ENV, be).env("WGPU_BACKEND", be).status();
        let elapsed = t0.elapsed();
        match status {
            Ok(s) if s.success() => {
                startup_log(&format!("{be}: đóng bình thường sau {:.0}s", elapsed.as_secs_f32()));
                return 0;
            }
            Ok(s) => {
                let code = s.code().map(|c| format!("{c} (0x{:X})", c as u32)).unwrap_or("bị hệ điều hành chấm dứt".into());
                startup_log(&format!("{be}: thoát lỗi, mã {code}, sau {:.1}s", elapsed.as_secs_f32()));
                attempts.push(format!("{be}: mã {code}"));
                if elapsed > STARTUP_WINDOW {
                    return 0; // đã chạy được một lúc rồi mới lỗi: không mở lại để tránh bật cửa sổ liên tục
                }
            }
            Err(e) => {
                startup_log(&format!("{be}: không chạy được tiến trình con: {e}"));
                attempts.push(format!("{be}: {e}"));
            }
        }
    }
    report_fatal(&format!(
        "Không khởi động được giao diện với bất kỳ kiểu đồ họa nào:\n{}\n\n\
         Chi tiết trong hazegard_startup.log. Hãy cập nhật driver card màn hình, \
         hoặc dùng bản Python (hazegard_gui.py).",
        attempts.join("\n")
    ));
    1
}

fn main() {
    let is_child = std::env::var_os(CHILD_ENV).is_some();
    std::panic::set_hook(Box::new(move |info| {
        let msg = format!("Lỗi chương trình:\n{info}");
        if is_child {
            let _ = std::fs::write(app::app_dir().join("hazegard_crash.log"), &msg);
        } else {
            report_fatal(&msg);
        }
    }));
    let direct = std::env::args().any(|a| a == "--direct") || std::env::var_os("WGPU_BACKEND").is_some();
    let code = if is_child {
        run_gui(true)
    } else if direct {
        run_gui(false) // chạy thẳng trong tiến trình này (để gỡ lỗi)
    } else {
        launcher()
    };
    exit(code);
}
