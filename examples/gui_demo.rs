//! Chạy thử giao diện với máy giả lập (Linux) và chụp màn hình: cargo run --example gui_demo -- <thư mục ảnh>
use eframe::egui;
use hazegard_gui::app::{self, HazeGardApp};
use serialport::{SerialPort, TTYPort};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

struct Wrap {
    app: HazeGardApp,
    t0: Instant,
    step: usize,
    out: std::path::PathBuf,
    pending_name: Option<&'static str>,
}

impl Wrap {
    fn shot(&mut self, ctx: &egui::Context) {
        let path = self.out.join(format!("{}.png", self.pending_name.take().unwrap()));
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            let _ = image::save_buffer(path, &rgba, img.width() as u32, img.height() as u32, image::ColorType::Rgba8);
        })));
    }
}

impl eframe::App for Wrap {
    fn logic(&mut self, ctx: &egui::Context, f: &mut eframe::Frame) {
        self.app.logic(ctx, f);
        let t = self.t0.elapsed().as_secs_f32();
        let steps: [(f32, u8); 12] = [(1.5, 0), (2.0, 1), (2.6, 1), (3.2, 1), (3.8, 1), (4.4, 1), (5.8, 2), (6.2, 3), (7.0, 4), (7.6, 5), (8.4, 6), (9.5, 7)];
        if self.step < steps.len() && t >= steps[self.step].0 {
            match steps[self.step].1 {
                0 => self.app.test_set_run("5497", "120"),
                1 => self.app.trigger_measure(),
                2 => { self.pending_name = Some("1_after_5_readings"); self.shot(ctx); }
                3 => self.app.trigger_measure(), // RunMinute trống -> báo lỗi, không đo
                4 => { self.pending_name = Some("2_missing_runminute"); self.shot(ctx); }
                5 => self.app.test_prompt_password(),
                6 => { self.pending_name = Some("3_password_prompt"); self.shot(ctx); }
                _ => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
            self.step += 1;
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, f: &mut eframe::Frame) {
        self.app.ui(ui, f);
    }
    fn on_exit(&mut self) {
        self.app.on_exit();
    }
}

fn main() -> eframe::Result {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or(".".into()));
    let (mut master, slave) = TTYPort::pair().unwrap();
    let name = slave.name().unwrap();
    std::thread::spawn(move || {
        let vals = [(80.7, 0.19), (80.8, 0.07), (80.7, 0.29), (80.5, 0.26), (80.0, 0.28)];
        let mut i = 0;
        let mut b = [0u8; 16];
        loop {
            let Ok(n) = master.read(&mut b) else { continue };
            for &c in &b[..n] {
                if c == 0x4F { let _ = master.write_all(&[0x4F, 0x21, 1, 3, 1, 0, 0, 1, 1, 5, 0x12, 0x01]); }
                if c == 0x04 {
                    std::thread::sleep(Duration::from_millis(150));
                    let (t, h) = vals[i % 5]; i += 1;
                    let _ = master.write_all(format!("\x04!f{:4.1}\0{:4.2}\0----\0", t, h).replace('.', ",").as_bytes());
                }
            }
        }
    });
    let dir = app::app_dir();
    let outdir = out.join("csv");
    std::fs::write(dir.join("hazegard_config.json"), format!(r#"{{"port":"{name}","line":"VGI3","outdir":{:?},"t_offset":-0.6,"h_offset":0}}"#, outdir.display().to_string())).unwrap();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([560.0, 640.0]).with_min_inner_size([470.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native("Haze Gard", options, Box::new(move |cc| Ok(Box::new(Wrap { app: HazeGardApp::new(cc), t0: Instant::now(), step: 0, out, pending_name: None }))))
}
