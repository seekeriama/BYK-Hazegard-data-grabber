//! Giao diện Haze Gard (egui)

use crate::config::{self, Config, ConfigStore, FILE_FORMATS};
use crate::csvout::{DailyCsv, RowInfo, fmt_offset};
use crate::instrument::{Device, Event, Instrument};
use crate::instrument_i::InstrumentI;
use chrono::{Local, NaiveDateTime};
use eframe::egui::{self, Color32, FontData, FontFamily, RichText, Vec2};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

const POSITIONS: [&str; 5] = ["LE", "LC", "CC", "RC", "RE"]; // thứ tự đo
const BUTTON_ORDER: [&str; 5] = ["RE", "RC", "CC", "LC", "LE"]; // thứ tự hiển thị từ trên xuống

const C_BG: Color32 = Color32::from_rgb(0xf3, 0xf3, 0xf3);
const C_OFF: Color32 = Color32::from_rgb(0xe9, 0xe9, 0xe9);
const C_ON: Color32 = Color32::from_rgb(0xcf, 0xe5, 0xff);
const C_NEXT: Color32 = Color32::from_rgb(0xff, 0xd9, 0x66);
const C_DONE: Color32 = Color32::from_rgb(0xc6, 0xef, 0xce);
const C_OK: Color32 = Color32::from_rgb(0x1e, 0x8c, 0x5a);
const C_WARN: Color32 = Color32::from_rgb(0xc5, 0x5a, 0x11);
const C_ERR: Color32 = Color32::from_rgb(0xc0, 0x00, 0x00);

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Normal,
    Warn,
    Err,
    Ok,
}

impl Kind {
    fn color(self, dark_text: Color32) -> Color32 {
        match self {
            Kind::Normal => dark_text,
            Kind::Warn => C_WARN,
            Kind::Err => C_ERR,
            Kind::Ok => C_OK,
        }
    }
}

struct SettingsState {
    port: String,
    line: String,
    outdir: String,
    t_offset: String,
    h_offset: String,
    release_first: bool,
    model: String,
    i_ip: String,
    bg_image: String,
    formats: [bool; 3],
    ports: Vec<String>,
    error: Option<String>,
}

struct PwState {
    input: String,
    error: Option<String>,
    pending: Config,
    focus: bool,
}

/// (file đích, thời điểm, T, H, dòng) – mỗi file đang bị khoá được thử ghi lại riêng
type PendingRow = (PathBuf, NaiveDateTime, Option<f64>, Option<f64>, RowInfo);

pub struct HazeGardApp {
    ctx: egui::Context,
    app_dir: PathBuf,
    cfg_store: ConfigStore,
    cfg: Config,
    store: DailyCsv,
    events_tx: Sender<Event>,
    events_rx: Receiver<Event>,
    inst: Option<Box<dyn Device>>,
    connected: bool,

    address: String,
    serial: String,
    device: String,
    chk: [bool; 4], // T, H, C, Continuous
    run_number: String,
    run_minute: String,
    focus_run_minute: bool,

    k: usize,
    seq: Vec<&'static str>,
    selected: [bool; 5],

    pending: Vec<PendingRow>,
    last_retry: Instant,
    log: Vec<(String, Kind)>,
    status: (String, Kind),
    texture: Option<egui::TextureHandle>,

    settings: Option<SettingsState>,
    pw: Option<PwState>,
    exit_confirm: bool,
    allow_close: bool,
}

pub fn app_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let load = |fonts: &mut egui::FontDefinitions, name: &str, family: FontFamily, paths: &[&str]| {
        for p in paths {
            if let Ok(bytes) = std::fs::read(p) {
                fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
                fonts.families.entry(family).or_default().insert(0, name.to_owned());
                return;
            }
        }
    };
    // font hệ thống có đủ dấu tiếng Việt
    load(&mut fonts, "sys_prop", FontFamily::Proportional, &[
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "C:\\Windows\\Fonts\\tahoma.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]);
    load(&mut fonts, "sys_mono", FontFamily::Monospace, &[
        "C:\\Windows\\Fonts\\consola.ttf",
        "C:\\Windows\\Fonts\\cour.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    ]);
    ctx.set_fonts(fonts);
}

fn load_picture(ctx: &egui::Context, dir: &std::path::Path, custom: &str) -> (Option<egui::TextureHandle>, Option<String>) {
    let try_open = |path: &std::path::Path| -> Result<egui::TextureHandle, String> {
        let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let rgba = img.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let ci = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        Ok(ctx.load_texture("hazegard_pic", ci, Default::default()))
    };
    let mut err = None;
    if !custom.trim().is_empty() {
        match try_open(std::path::Path::new(custom.trim())) {
            Ok(t) => return (Some(t), None),
            Err(e) => err = Some(e),
        }
    }
    for name in ["hazegard.png", "hazegard.jpg", "hazegard.jpeg"] {
        if let Ok(t) = try_open(&dir.join(name)) {
            return (Some(t), err);
        }
    }
    (None, err)
}

impl HazeGardApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::with_context(cc.egui_ctx.clone())
    }

    pub fn with_context(ctx: egui::Context) -> Self {
        setup_fonts(&ctx);
        ctx.set_visuals(egui::Visuals::light());
        ctx.all_styles_mut(|s| {
            s.visuals.panel_fill = C_BG;
            s.text_styles.insert(egui::TextStyle::Body, egui::FontId::new(14.0, FontFamily::Proportional));
            s.text_styles.insert(egui::TextStyle::Button, egui::FontId::new(14.0, FontFamily::Proportional));
            s.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::new(13.0, FontFamily::Monospace));
        });
        let app_dir = app_dir();
        let cfg_store = ConfigStore::default_for(&app_dir);
        let cfg = cfg_store.load();
        let (events_tx, events_rx) = channel();
        let (texture, pic_err) = load_picture(&ctx, &app_dir, &cfg.bg_image);
        let mut app = Self {
            ctx,
            store: DailyCsv { outdir: app_dir.clone(), systems: vec![String::new()] },
            app_dir,
            cfg_store,
            cfg,
            events_tx,
            events_rx,
            inst: None,
            connected: false,
            address: String::new(),
            serial: String::new(),
            device: String::new(),
            chk: [true, true, false, false],
            run_number: String::new(),
            run_minute: String::new(),
            focus_run_minute: false,
            k: 0,
            seq: vec![],
            selected: [true; 5],
            pending: vec![],
            last_retry: Instant::now(),
            log: vec![],
            status: (String::new(), Kind::Ok),
            texture,
            settings: None,
            pw: None,
            exit_confirm: false,
            allow_close: false,
        };
        if let Some(e) = pic_err {
            app.write_log(&format!("Không đọc được ảnh nền: {e}\n"), Kind::Warn);
        }
        app.store = app.make_store();
        app.start_connection();
        app
    }

    // ---------- lưu trữ ----------
    /// Thư mục lưu CSV; không ghi được (vd đường dẫn của máy khác) -> dùng thư mục chương trình.
    fn make_store(&mut self) -> DailyCsv {
        let want = self.cfg.outdir_or(&self.app_dir);
        if DailyCsv::writable(&want) {
            return DailyCsv { outdir: want, systems: self.cfg.systems() };
        }
        self.write_log(
            &format!(
                "Không dùng được thư mục lưu:\n  {}\n→ Tạm lưu CSV vào thư mục chương trình.\nBấm F12 để chọn lại thư mục.\n\n",
                want.display()
            ),
            Kind::Warn,
        );
        DailyCsv { outdir: self.app_dir.clone(), systems: self.cfg.systems() }
    }

    fn save_row(&mut self, now: NaiveDateTime, t: Option<f64>, h: Option<f64>, row: RowInfo) {
        let mut failed = false;
        for p in self.store.paths_for(&now) {
            if self.store.write_to(&p, &now, t, h, &row).is_err() {
                self.pending.push((p, now, t, h, row.clone()));
                failed = true;
            }
        }
        if failed {
            self.write_log(
                "!! Không ghi được file CSV (đang mở trong Excel?). Sẽ tự ghi lại khi đóng file.\n",
                Kind::Err,
            );
        }
    }

    fn retry_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let items = std::mem::take(&mut self.pending);
        let n = items.len();
        for it in items {
            if self.store.write_to(&it.0, &it.1, it.2, it.3, &it.4).is_err() {
                self.pending.push(it);
            }
        }
        if self.pending.is_empty() {
            self.write_log(&format!("Đã ghi bù {n} kết quả vào file CSV.\n"), Kind::Normal);
        }
    }

    // ---------- kết nối ----------
    fn start_connection(&mut self) {
        if let Some(mut i) = self.inst.take() {
            i.stop();
        }
        self.connected = false;
        self.address.clear();
        self.serial.clear();
        self.device.clear();
        let ctx = self.ctx.clone();
        if self.cfg.model == "i" {
            self.set_status("đang kết nối máy i ...", Kind::Warn);
            self.inst = InstrumentI::start_dll(
                &self.app_dir,
                self.events_tx.clone(),
                self.app_dir.join("hazegard_raw.log"),
                self.cfg.i_ip.clone(),
                move || ctx.request_repaint(),
            )
            .map(|i| Box::new(i) as Box<dyn Device>);
            return;
        }
        let port = self.cfg.port.trim().to_string();
        if port.is_empty() {
            self.set_status("chưa chọn cổng COM", Kind::Err);
            self.open_settings();
            return;
        }
        self.set_status("đang kết nối ...", Kind::Warn);
        self.inst = Some(Box::new(Instrument::start(
            port,
            self.events_tx.clone(),
            self.app_dir.join("hazegard_raw.log"),
            self.cfg.release_first,
            move || ctx.request_repaint(),
        )));
    }

    fn on_connected_i(&mut self, serial: String, identifier: String) {
        self.connected = true;
        self.address = identifier;
        self.serial = serial;
        self.device = "haze-gard i".into();
        self.chk = [true, true, false, false];
        self.write_log("Kết nối máy i. Bấm đo trực tiếp trên máy, kết quả tự ghi vào CSV.\n\n", Kind::Normal);
        self.set_status("port haze", Kind::Ok);
    }

    fn on_connected(&mut self, online: bool, status: Option<crate::protocol::Status>) {
        self.connected = true;
        self.address = self.cfg.port.clone();
        match (online, status) {
            (true, Some(st)) => {
                self.chk = [st.has_transmission(), st.has_haze(), st.has_clarity(), st.continuous];
                self.device = format!("haze-gard plus  FW {}", st.firmware);
                if !st.cal_valid {
                    self.write_log("Cảnh báo: máy báo calibration KHÔNG hợp lệ.\n", Kind::Warn);
                }
                if st.continuous {
                    self.write_log("Cảnh báo: máy đang ở Continuous mode.\n", Kind::Warn);
                }
                self.write_log("Kết nối online (4Fh). F5 = đo từ máy tính.\n\n", Kind::Normal);
            }
            _ => self.write_log(
                "Máy không trả lời lệnh 4Fh → nghe thụ động. Bấm operate trên máy để đo.\n\n",
                Kind::Warn,
            ),
        }
        self.set_status("port haze", Kind::Ok);
    }

    // ---------- vị trí đo ----------
    fn current_seq(&self) -> Vec<&'static str> {
        if self.k > 0 {
            self.seq.clone()
        } else {
            POSITIONS.iter().copied().enumerate().filter(|(i, _)| self.selected[*i]).map(|(_, p)| p).collect()
        }
    }

    fn toggle(&mut self, p: &str) {
        if self.k > 0 {
            self.write_log("Đang trong lượt đo – bấm Clear để đổi vị trí.\n", Kind::Warn);
            return;
        }
        if let Some(i) = POSITIONS.iter().position(|x| *x == p) {
            self.selected[i] = !self.selected[i];
        }
    }

    fn check_ready(&self) -> Option<&'static str> {
        if self.current_seq().is_empty() {
            Some("Chưa chọn vị trí đo")
        } else if self.run_number.trim().is_empty() {
            Some("Chưa nhập RunNumber")
        } else if self.run_minute.trim().is_empty() {
            Some("Chưa nhập RunMinute")
        } else {
            None
        }
    }

    pub fn trigger_measure(&mut self) {
        if self.cfg.model == "i" {
            self.set_status("máy i: bấm đo trên máy", Kind::Warn);
            return;
        }
        let online = self.inst.as_ref().is_some_and(|i| i.is_online());
        if !online {
            self.set_status("bấm operate trên máy", Kind::Warn);
            return;
        }
        if let Some(p) = self.check_ready() {
            self.set_status(p, Kind::Err);
            return;
        }
        if let Some(i) = &self.inst {
            i.operate();
        }
        self.set_status("đang đo ...", Kind::Warn);
    }

    fn on_result(&mut self, t: Option<f64>, h: Option<f64>) {
        if let Some(p) = self.check_ready() {
            self.write_log(
                &format!("Nhận T={} H={} nhưng KHÔNG LƯU: {p}\n\n", opt(t), opt(h)),
                Kind::Err,
            );
            self.set_status(p, Kind::Err);
            return;
        }
        if self.k == 0 {
            self.seq = self.current_seq();
        }
        let pos = self.seq[self.k];
        let (toff, hoff) = (self.cfg.t_offset, self.cfg.h_offset);
        let tc = t.map(|v| v + toff);
        let hc = h.map(|v| v + hoff);
        let row = RowInfo {
            line: self.cfg.line.clone(),
            run_number: self.run_number.trim().to_string(),
            run_minute: self.run_minute.trim().to_string(),
            position: pos.to_string(),
            t_offset: fmt_offset(toff),
            h_offset: fmt_offset(hoff),
        };
        let minute_label = row.run_minute.clone();
        self.save_row(Local::now().naive_local(), tc, hc, row);
        let ts = tc.map(|v| format!("{v:.1}")).unwrap_or("-".into());
        let hs = hc.map(|v| format!("{v:.2}")).unwrap_or("-".into());
        let rm = minute_label; // RunMinute do người vận hành nhập (không còn số 04 cố định)
        self.write_log(&format!("{rm}: {pos}- %T: {ts}\n{rm}: {pos}- %H: {hs}\n\n"), Kind::Normal);
        self.k += 1;
        if self.k >= self.seq.len() {
            // xong 1 lượt: RunNumber giữ nguyên, RunMinute xoá để nhập số mới
            self.k = 0;
            self.seq.clear();
            self.run_minute.clear();
            self.focus_run_minute = true;
        }
        self.set_status("port haze", Kind::Ok);
    }

    fn clear(&mut self) {
        self.log.clear();
        self.k = 0;
        self.seq.clear();
        if self.connected {
            self.set_status("port haze", Kind::Ok);
        }
    }

    fn write_log(&mut self, text: &str, kind: Kind) {
        self.log.push((text.to_string(), kind));
    }

    fn set_status(&mut self, text: &str, kind: Kind) {
        self.status = (text.to_string(), kind);
    }

    fn poll_events(&mut self) {
        while let Ok(ev) = self.events_rx.try_recv() {
            match ev {
                Event::Connected { online, status } => self.on_connected(online, status),
                Event::ConnectedI { serial, identifier } => self.on_connected_i(serial, identifier),
                Event::Result { t, h } => self.on_result(t, h),
                Event::PortSwitch => self.set_status("port clarity", Kind::Warn),
                Event::Error(e) => {
                    self.connected = false;
                    self.write_log(&format!("{e}\n"), Kind::Err);
                    self.set_status("lỗi kết nối", Kind::Err);
                }
            }
        }
    }

    // ---------- cài đặt + password ----------
    fn open_settings(&mut self) {
        if self.settings.is_some() {
            return;
        }
        let ports = serialport::available_ports().map(|v| v.into_iter().map(|p| p.port_name).collect()).unwrap_or_default();
        self.settings = Some(SettingsState {
            port: self.cfg.port.clone(),
            line: self.cfg.line.clone(),
            outdir: if self.cfg.outdir.is_empty() { self.app_dir.display().to_string() } else { self.cfg.outdir.clone() },
            t_offset: fmt_offset(self.cfg.t_offset),
            h_offset: fmt_offset(self.cfg.h_offset),
            release_first: self.cfg.release_first,
            model: self.cfg.model.clone(),
            i_ip: self.cfg.i_ip.clone(),
            bg_image: self.cfg.bg_image.clone(),
            formats: {
                let sel = config::normalize_formats(&self.cfg.file_formats);
                let mut f = [false; 3];
                for (i, (k, _)) in FILE_FORMATS.iter().enumerate() {
                    f[i] = sel.iter().any(|x| x == k);
                }
                f
            },
            ports,
            error: None,
        });
    }

    /// Bấm "Lưu & kết nối": kiểm tra giá trị, nếu có thay đổi thì hỏi password.
    fn try_save_settings(&mut self) {
        let Some(s) = self.settings.as_mut() else { return };
        let (t, h) = match (s.t_offset.trim().replace(',', ".").parse::<f64>(), s.h_offset.trim().replace(',', ".").parse::<f64>()) {
            (Ok(t), Ok(h)) => (t, h),
            _ => {
                s.error = Some("Offset phải là số, vd -0.6".into());
                return;
            }
        };
        if !s.formats.iter().any(|&x| x) {
            s.error = Some("Phải chọn ít nhất 1 định dạng tên file.".into());
            return;
        }
        let outdir = if s.outdir.trim().is_empty() { self.app_dir.clone() } else { PathBuf::from(s.outdir.trim()) };
        if let Err(e) = std::fs::create_dir_all(&outdir) {
            s.error = Some(format!("Không tạo được thư mục: {e}"));
            return;
        }
        let same_as_app = outdir == self.app_dir;
        let new = Config {
            port: s.port.trim().to_string(),
            line: s.line.trim().to_string(),
            outdir: if same_as_app { String::new() } else { outdir.display().to_string() },
            t_offset: t,
            h_offset: h,
            release_first: s.release_first,
            model: if s.model == "i" { "i".into() } else { "plus".into() },
            i_ip: s.i_ip.trim().to_string(),
            bg_image: s.bg_image.trim().to_string(),
            file_formats: FILE_FORMATS.iter().enumerate().filter(|(i, _)| s.formats[*i]).map(|(_, (k, _))| k.to_string()).collect(),
        };
        s.error = None;
        let changed = new != self.cfg || !self.cfg_store.exists();
        if changed {
            self.pw = Some(PwState { input: String::new(), error: None, pending: new, focus: true });
        } else {
            self.settings = None;
            self.start_connection(); // không đổi gì -> kết nối lại cổng COM, không cần password
        }
    }

    /// Bấm OK trong hộp password: đúng -> lưu cài đặt; sai -> báo lỗi, KHÔNG lưu.
    fn submit_password(&mut self) {
        let ok = self.pw.as_ref().is_some_and(|p| config::password_ok(&p.input));
        if ok {
            let new = self.pw.as_ref().unwrap().pending.clone();
            self.apply_config(new);
        } else if let Some(p) = self.pw.as_mut() {
            p.error = Some("Sai password. Cài đặt CHƯA được lưu.".into());
            p.input.clear();
            p.focus = true;
        }
    }

    fn apply_config(&mut self, new: Config) {
        self.cfg = new;
        if let Err(e) = self.cfg_store.save(&self.cfg) {
            self.write_log(&format!("Không lưu được cấu hình: {e}\n"), Kind::Err);
        }
        let (tex, err) = load_picture(&self.ctx, &self.app_dir, &self.cfg.bg_image);
        self.texture = tex;
        if let Some(e) = err {
            self.write_log(&format!("Không đọc được ảnh nền: {e}\n"), Kind::Warn);
        }
        self.store = self.make_store();
        self.settings = None;
        self.pw = None;
        self.start_connection();
    }

    // ---------- vẽ ----------
    fn draw_picture(&self, ui: &mut egui::Ui) {
        let w = ui.available_width().clamp(300.0, 560.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, w * 0.4), egui::Sense::hover());
        let sc = (w / 560.0).min(rect.height() / 280.0); // hình minh hoạ vẽ ở khung 560x280 rồi co vừa khung
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, Color32::from_rgb(0xee, 0xf3, 0xf8));
        if let Some(tex) = &self.texture {
            let sz = tex.size_vec2();
            let scale = (rect.width() / sz.x).min(rect.height() / sz.y);
            let r = egui::Rect::from_center_size(rect.center(), sz * scale);
            p.image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            return;
        }
        let o = rect.min.to_vec2() + Vec2::new((rect.width() - 560.0 * sc) / 2.0, (rect.height() - 280.0 * sc) / 2.0);
        let r = |x0: f32, y0: f32, x1: f32, y1: f32| egui::Rect::from_min_max(egui::pos2(x0 * sc, y0 * sc) + o, egui::pos2(x1 * sc, y1 * sc) + o);
        let c = |x: f32, y: f32| egui::pos2(x * sc, y * sc) + o;
        let (blue, dark) = (Color32::from_rgb(0x2f, 0x86, 0xd1), Color32::from_rgb(0x1f, 0x5f, 0x99));
        p.rect_filled(r(0.0, 240.0, 560.0, 280.0), 0.0, Color32::from_rgb(0xd0, 0xd6, 0xdc));
        p.rect_filled(r(60.0, 222.0, 500.0, 240.0), 0.0, Color32::from_rgb(0x8a, 0x90, 0x99));
        p.rect_filled(r(30.0, 50.0, 230.0, 245.0), 0.0, blue);
        p.rect_filled(r(55.0, 75.0, 165.0, 160.0), 0.0, Color32::from_rgb(0x4a, 0x4f, 0x57));
        p.circle_filled(c(195.0, 120.0), 15.0 * sc, Color32::from_rgb(0xe8, 0xe8, 0xe8));
        p.rect_filled(r(222.0, 95.0, 262.0, 175.0), 16.0 * sc, dark);
        p.rect_filled(r(360.0, 50.0, 530.0, 245.0), 0.0, blue);
        p.rect_filled(r(330.0, 80.0, 380.0, 190.0), 25.0 * sc, dark);
        p.rect_filled(r(296.0, 120.0, 304.0, 222.0), 0.0, Color32::from_rgb(0x22, 0x22, 0x22));
        p.rect_stroke(r(282.0, 100.0, 318.0, 180.0), 18.0 * sc, egui::Stroke::new(4.0 * sc, Color32::from_rgb(0x22, 0x22, 0x22)), egui::StrokeKind::Inside);
        p.rect_filled(r(278.0, 205.0, 322.0, 222.0), 0.0, Color32::from_rgb(0x33, 0x33, 0x33));
    }

    fn draw_main(&mut self, ui: &mut egui::Ui) {
        // dòng "Connected"
        let conn_text = if self.connected {
            RichText::new(format!("Connected: {}", self.cfg.line)).color(Color32::BLACK)
        } else {
            RichText::new("Chưa kết nối  (nhấp đúp vào đây để cài đặt)").color(C_ERR)
        };
        egui::Frame::new().fill(Color32::WHITE).inner_margin(6.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let r = ui.add(egui::Label::new(conn_text).sense(egui::Sense::click()));
            if r.double_clicked() {
                self.open_settings();
            }
        });
        ui.add_space(4.0);
        ui.vertical_centered(|ui| self.draw_picture(ui));
        ui.add_space(6.0);

        let locked = self.k > 0;
        ui.horizontal_top(|ui| {
            // --- cột trái ---
            ui.vertical(|ui| {
                ui.set_width(225.0);
                egui::Grid::new("fields").num_columns(2).spacing([8.0, 5.0]).show(ui, |ui| {
                    let ro = |ui: &mut egui::Ui, label: &str, text: &str, w: f32| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(label));
                        ui.add(egui::TextEdit::singleline(&mut text.to_string().as_str()).desired_width(w).align(egui::Align2::CENTER_CENTER));
                        ui.end_row();
                    };
                    ro(ui, "Address", &self.address, 130.0);
                    ro(ui, "Serial #", &self.serial, 130.0);
                    ro(ui, "Device ID", &self.device, 125.0);

                    ui.label("");
                    ui.horizontal(|ui| {
                        for (i, name) in ["Transmission", "Clarity"].iter().enumerate() {
                            let mut v = self.chk[if i == 0 { 0 } else { 2 }];
                            ui.add_enabled(false, egui::Checkbox::new(&mut v, *name));
                        }
                    });
                    ui.end_row();
                    ui.label("");
                    ui.horizontal(|ui| {
                        let mut h = self.chk[1];
                        ui.add_enabled(false, egui::Checkbox::new(&mut h, "Haze"));
                        let mut c = self.chk[3];
                        ui.add_enabled(false, egui::Checkbox::new(&mut c, "Continuous"));
                    });
                    ui.end_row();

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label("RunNumber"));
                    ui.add_enabled(!locked, egui::TextEdit::singleline(&mut self.run_number).desired_width(115.0));
                    ui.end_row();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label("RunMinute"));
                    let r = ui.add_enabled(!locked, egui::TextEdit::singleline(&mut self.run_minute).desired_width(115.0));
                    if self.focus_run_minute {
                        r.request_focus();
                        self.focus_run_minute = false;
                    }
                    ui.end_row();

                    ro(ui, "Delay (s)", "0", 50.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label("Reading"));
                    ui.horizontal(|ui| {
                        let (k, n) = (self.k.to_string(), self.current_seq().len().to_string());
                        ui.add(egui::TextEdit::singleline(&mut k.as_str()).desired_width(40.0).align(egui::Align2::CENTER_CENTER));
                        ui.label("of");
                        ui.add(egui::TextEdit::singleline(&mut n.as_str()).desired_width(40.0).align(egui::Align2::CENTER_CENTER));
                    });
                    ui.end_row();

                    let corr = |v: f64, nd: &str| if v != 0.0 { format!("True: {}", fmt_offset(v)) } else { format!("False: {nd}") };
                    for (label, v) in [("%T Corrected", self.cfg.t_offset), ("%H Corrected", self.cfg.h_offset)] {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(label));
                        let r = ui.add(egui::TextEdit::singleline(&mut corr(v, "0").as_str()).desired_width(115.0).align(egui::Align2::CENTER_CENTER));
                        if r.double_clicked() {
                            self.open_settings();
                        }
                        ui.end_row();
                    }
                });
            });

            // --- cột giữa: nhật ký ---
            let dark_text = ui.visuals().text_color();
            let log_w = (ui.available_width() - 88.0).max(120.0);
            let log_h = (ui.available_height() - 50.0).max(200.0);
            egui::Frame::new().fill(Color32::WHITE).stroke(egui::Stroke::new(1.0, Color32::GRAY)).inner_margin(4.0).show(ui, |ui| {
                ui.set_width(log_w - 10.0);
                ui.set_height(log_h);
                egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                    // ScrollArea thừa hưởng layout ngang của hàng cha -> ép xếp dọc
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        for (text, kind) in &self.log {
                            ui.add(egui::Label::new(RichText::new(text.trim_end_matches('\n')).monospace().color(kind.color(dark_text))).wrap());
                        }
                    });
                });
            });

            // --- cột phải: 5 vị trí ---
            ui.add_space(6.0);
            let mut clicked: Option<&str> = None;
            let seq = self.current_seq();
            let nxt = seq.get(self.k).copied();
            let done: Vec<&str> = seq.iter().take(self.k).copied().collect();
            egui::Frame::new().stroke(egui::Stroke::new(1.0, Color32::GRAY)).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                ui.vertical(|ui| {
                    for p in BUTTON_ORDER {
                        let idx = POSITIONS.iter().position(|x| *x == p).unwrap();
                        let (bg, bold) = if Some(p) == nxt {
                            (C_NEXT, true)
                        } else if done.contains(&p) {
                            (C_DONE, false)
                        } else if self.selected[idx] {
                            (C_ON, true)
                        } else {
                            (C_OFF, false)
                        };
                        let mut t = RichText::new(p).color(Color32::BLACK);
                        if bold {
                            t = t.strong();
                        }
                        if ui.add(egui::Button::new(t).fill(bg).min_size(Vec2::new(46.0, 30.0))).clicked() {
                            clicked = Some(p);
                        }
                        ui.add_space(4.0);
                    }
                });
            });
            if let Some(p) = clicked {
                self.toggle(p);
            }
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(40.0);
            if ui.add(egui::Button::new("Exit").min_size(Vec2::new(80.0, 26.0))).clicked() {
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.add_space(10.0);
            let (txt, kind) = self.status.clone();
            ui.add_sized([150.0, 26.0], egui::Label::new(RichText::new(txt).strong().size(16.0).color(kind.color(Color32::BLACK))));
            if ui.add(egui::Button::new("Clear").min_size(Vec2::new(90.0, 26.0))).clicked() {
                self.clear();
            }
        });
    }

    fn draw_dialogs(&mut self, ctx: &egui::Context) {
        let mut do_save = false;
        let mut cancel = false;
        let pw_open = self.pw.is_some();
        if let Some(s) = self.settings.as_mut() {
            egui::Window::new("Cài đặt").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).enabled(!pw_open).show(ctx, |ui| {
                egui::Grid::new("settings").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                    ui.label("Đời máy");
                    egui::ComboBox::from_id_salt("model")
                        .selected_text(if s.model == "i" { "haze-gard i (USB)" } else { "haze-gard plus (RS232)" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut s.model, "plus".to_string(), "haze-gard plus (RS232)");
                            ui.selectable_value(&mut s.model, "i".to_string(), "haze-gard i (USB)");
                        });
                    ui.end_row();
                    if s.model == "i" {
                        ui.label("IP máy i (nếu qua LAN)");
                        ui.add(egui::TextEdit::singleline(&mut s.i_ip).desired_width(260.0));
                        ui.end_row();
                    } else {
                        ui.label("Cổng COM");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut s.port).desired_width(120.0));
                            let ports = s.ports.clone();
                            egui::ComboBox::from_id_salt("ports").selected_text("chọn").show_ui(ui, |ui| {
                                if ports.is_empty() {
                                    ui.label("(không thấy cổng COM)");
                                }
                                for p in ports {
                                    ui.selectable_value(&mut s.port, p.clone(), p);
                                }
                            });
                        });
                        ui.end_row();
                    }
                    ui.label("Line");
                    ui.add(egui::TextEdit::singleline(&mut s.line).desired_width(260.0));
                    ui.end_row();
                    ui.label("Thư mục lưu CSV");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut s.outdir).desired_width(225.0));
                        #[cfg(windows)]
                        if ui.button("...").clicked() {
                            if let Some(d) = rfd::FileDialog::new().set_directory(&s.outdir).pick_folder() {
                                s.outdir = d.display().to_string();
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("Ảnh nền");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut s.bg_image).desired_width(150.0));
                        #[cfg(windows)]
                        if ui.button("...").clicked() {
                            if let Some(f) = rfd::FileDialog::new().add_filter("Ảnh", &["png", "jpg", "jpeg", "bmp", "gif"]).pick_file() {
                                s.bg_image = f.display().to_string();
                            }
                        }
                        if ui.button("Mặc định").clicked() {
                            s.bg_image.clear();
                        }
                    });
                    ui.end_row();
                    ui.label("%T offset");
                    ui.add(egui::TextEdit::singleline(&mut s.t_offset).desired_width(260.0));
                    ui.end_row();
                    ui.label("%H offset");
                    ui.add(egui::TextEdit::singleline(&mut s.h_offset).desired_width(260.0));
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.group(|ui| {
                    ui.label("Định dạng tên file CSV (chọn 1-3):");
                    ui.horizontal(|ui| {
                        let labels = ["Default (hazegarddata…)", "ERS", "Sequoia"];
                        for (i, l) in labels.iter().enumerate() {
                            ui.checkbox(&mut s.formats[i], *l);
                        }
                    });
                });
                ui.checkbox(&mut s.release_first, "Giải phóng máy đang kẹt Online Mode khi khởi động (gửi 50h)");
                ui.label(RichText::new("Kết quả ghi vào CSV = giá trị máy đo + offset").color(Color32::GRAY));
                if let Some(e) = &s.error {
                    ui.colored_label(C_ERR, e);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new("Lưu & kết nối").min_size(Vec2::new(110.0, 26.0))).clicked() {
                        do_save = true;
                    }
                    if ui.add(egui::Button::new("Hủy").min_size(Vec2::new(70.0, 26.0))).clicked() {
                        cancel = true;
                    }
                });
            });
        }
        if cancel {
            self.settings = None;
        }
        if do_save {
            self.try_save_settings();
        }

        // hộp password
        let mut pw_action: Option<bool> = None; // Some(true)=OK, Some(false)=Cancel
        if let Some(pw) = self.pw.as_mut() {
            egui::Window::new("Xác nhận thay đổi").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 130.0]).show(ctx, |ui| {
                ui.label("Nhập password để lưu cài đặt:");
                let r = ui.add(egui::TextEdit::singleline(&mut pw.input).password(true).desired_width(240.0));
                if pw.focus {
                    r.request_focus();
                    pw.focus = false;
                }
                if let Some(e) = &pw.error {
                    ui.colored_label(C_ERR, e);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.add(egui::Button::new("OK").min_size(Vec2::new(80.0, 26.0))).clicked() || enter {
                        pw_action = Some(true);
                    }
                    if ui.add(egui::Button::new("Cancel").min_size(Vec2::new(80.0, 26.0))).clicked() {
                        pw_action = Some(false);
                    }
                });
            });
        }
        match pw_action {
            Some(true) => self.submit_password(),
            Some(false) => self.pw = None,
            None => {}
        }

        // xác nhận thoát khi còn dữ liệu chưa ghi
        if self.exit_confirm {
            egui::Window::new("Haze Gard").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(format!("Còn {} kết quả chưa ghi được vào CSV (file đang mở?). Vẫn thoát?", self.pending.len()));
                ui.horizontal(|ui| {
                    if ui.button("Thoát").clicked() {
                        self.allow_close = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Ở lại").clicked() {
                        self.exit_confirm = false;
                    }
                });
            });
        }
    }
}

/// Hàm hỗ trợ chạy thử tự động (examples/gui_demo.rs); không dùng khi chạy thật.
impl HazeGardApp {
    #[doc(hidden)]
    pub fn test_set_run(&mut self, number: &str, minute: &str) {
        self.run_number = number.into();
        self.run_minute = minute.into();
    }
    #[doc(hidden)]
    pub fn test_prompt_password(&mut self) {
        self.open_settings();
        if let Some(s) = self.settings.as_mut() {
            s.h_offset = "0.1".into();
        }
        self.try_save_settings();
    }
}

fn opt(v: Option<f64>) -> String {
    v.map(|x| x.to_string()).unwrap_or("-".into())
}

impl eframe::App for HazeGardApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(100));
        self.poll_events();
        if self.last_retry.elapsed() > Duration::from_secs(5) {
            self.last_retry = Instant::now();
            self.retry_pending();
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && !self.pending.is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.exit_confirm = true;
        }
        let dialog_open = self.settings.is_some() || self.pw.is_some() || self.exit_confirm;
        if !dialog_open {
            if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
                self.trigger_measure();
            }
            if ctx.input(|i| i.key_pressed(egui::Key::F12)) {
                self.open_settings();
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(C_BG).inner_margin(12.0)).show(ui, |ui| {
            let enabled = self.settings.is_none() && self.pw.is_none() && !self.exit_confirm;
            ui.add_enabled_ui(enabled, |ui| self.draw_main(ui));
        });
        self.draw_dialogs(&ctx);
    }

    fn on_exit(&mut self) {
        if let Some(mut i) = self.inst.take() {
            i.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Luồng đổi cài đặt: sai password -> không lưu; đúng -> lưu vào file; không đổi gì -> không hỏi.
    #[test]
    fn settings_need_password() {
        let mut app = HazeGardApp::with_context(egui::Context::default());
        let cfg_path = std::env::temp_dir().join(format!("hz_cfg_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&cfg_path);
        app.cfg_store = ConfigStore::Json(cfg_path.clone());
        app.app_dir = std::env::temp_dir();
        app.cfg = Config::default();
        app.settings = None;

        app.open_settings();
        let s = app.settings.as_mut().unwrap();
        s.port = "COMX".into();
        s.t_offset = "-0.6".into();
        s.outdir = std::env::temp_dir().display().to_string();
        app.try_save_settings();
        assert!(app.pw.is_some(), "co thay doi -> phai hoi password");
        assert_eq!(app.cfg.t_offset, 0.0);

        // sai password
        app.pw.as_mut().unwrap().input = "sai".into();
        app.submit_password();
        assert!(app.pw.as_ref().unwrap().error.is_some());
        assert_eq!(app.cfg.t_offset, 0.0, "sai password khong duoc luu");
        assert!(!cfg_path.exists());

        // dung password
        app.pw.as_mut().unwrap().input = "ZZCARROT".into();
        app.submit_password();
        assert!(app.pw.is_none() && app.settings.is_none());
        assert_eq!((app.cfg.port.as_str(), app.cfg.t_offset), ("COMX", -0.6));
        let saved = app.cfg_store.load();
        assert_eq!((saved.port.as_str(), saved.t_offset), ("COMX", -0.6));

        // mo lai, khong doi gi -> khong hoi password
        app.open_settings();
        app.try_save_settings();
        assert!(app.pw.is_none() && app.settings.is_none());
        let _ = std::fs::remove_file(&cfg_path);
    }
}
