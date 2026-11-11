//! Máy BYK haze-gard i (USB / mạng) qua BYKCommunication.dll (bản 32-bit).
//! Giao thức (lệnh XML, mã lệnh, mã sự kiện) lấy theo phần mềm gốc Client_HazeGard.exe.
//! Phần giao tiếp DLL nằm sau trait `ByKApi` để kiểm thử bằng máy giả.

use crate::instrument::{Device, Event};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub const INIT_NAME: &str = "BykGardnerInstrumentUnits";
pub const INIT_KEY: &str = "BYKUnits81!654";

pub const EV_ADD: i32 = 1;
pub const EV_REMOVE: i32 = 2;
pub const EV_STATUS: i32 = 4;

pub const ST_BUSY: i32 = -2147483237;
pub const ST_MEASURE_END: i32 = -2147483236; // đo xong -> đọc kết quả (lệnh 403)
pub const ST_ONLINE_END: i32 = -2147483235;
pub const ST_PORT_HAZE: i32 = -2147483234;
pub const ST_PORT_CLARITY: i32 = -2147483233;
pub const ST_NO_PORT: i32 = -2147483360;
pub const EVENT_IDS: [i32; 6] = [ST_BUSY, ST_MEASURE_END, ST_ONLINE_END, ST_PORT_HAZE, ST_PORT_CLARITY, ST_NO_PORT];

pub const CMD_DEVICE_INFO: i32 = 4;
pub const CMD_START_ONLINE: i32 = 400;
pub const CMD_STOP_ONLINE: i32 = 401;
pub const CMD_READ_ONLINE: i32 = 403;

pub const SCALE_TRANSMISSION_C: i64 = 500;
pub const SCALE_HAZE_C: i64 = 501;

const BUF: usize = 5 * 1024 * 1024;

/// Kết quả một lần chờ sự kiện
#[derive(Debug, Clone, Default)]
pub struct WaitResult {
    pub ev_type: i32,
    pub ident: String,
    pub code: i32,
}

pub trait ByKApi: Send + Sync {
    fn enable_tcpip(&self);
    fn set_ip_range(&self, ip: [u8; 4]);
    fn init(&self, name: &str, key: &str) -> i32;
    fn de_init(&self);
    fn register_event(&self, id: i32);
    fn list_all(&self) -> Result<String, i32>;
    fn init_connection(&self, ident: &str) -> i32;
    fn send_receive(&self, ident: &str, xml: &str) -> Result<String, i32>;
    /// Chặn cho tới khi có sự kiện (hoặc bị cancel_wait)
    fn wait_event(&self) -> WaitResult;
    fn cancel_wait(&self);
}

// ---------------------------------------------------------------------------------------------
//  XML
// ---------------------------------------------------------------------------------------------
pub fn command_xml(cmd: i32, params: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><Command Id=\"{cmd}\">{params}</Command>")
}

const START_ONLINE_PARAMS: &str = "<Parameters><ParameterBool Name=\"Transmission\" Value=\"true\" />\
<ParameterBool Name=\"Haze\" Value=\"true\" /><ParameterBool Name=\"Clarity\" Value=\"false\" />\
<ParameterBool Name=\"Continuous\" Value=\"false\" /><ParameterInt32 Name=\"Continuous_Delay\" Value=\"0\" /></Parameters>";

#[derive(Debug, PartialEq)]
pub struct Measure {
    pub t: Option<f64>,
    pub h: Option<f64>,
    pub timestamp: Option<String>,
}

/// Kết quả lệnh 403; Err nếu máy báo lỗi hoặc XML hỏng.
pub fn parse_measure(xml: &str) -> Result<Measure, String> {
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}').trim()).map_err(|e| format!("XML lỗi: {e}"))?;
    if let Some(err) = doc.descendants().find(|n| n.has_tag_name("Error")) {
        let t = err.text().unwrap_or("").trim();
        return Err(format!("Máy báo lỗi: {}", if t.is_empty() { err.attribute("Text").unwrap_or("?") } else { t }));
    }
    let (mut t, mut h) = (None, None);
    for sv in doc.descendants().filter(|n| n.has_tag_name("ScaleValue")) {
        let (Some(id), Some(v)) = (sv.attribute("Id"), sv.attribute("Value")) else { continue };
        let (Ok(id), Ok(v)) = (id.trim().parse::<i64>(), v.trim().replace(',', ".").parse::<f64>()) else { continue };
        match id {
            SCALE_TRANSMISSION_C => t = Some(v),
            SCALE_HAZE_C => h = Some(v),
            _ => {}
        }
    }
    let timestamp = doc.descendants().find(|n| n.has_tag_name("MeasureValue")).and_then(|n| n.attribute("TimeStamp")).map(String::from);
    Ok(Measure { t, h, timestamp })
}

pub fn parse_serial(xml: &str) -> String {
    roxmltree::Document::parse(xml.trim_start_matches('\u{feff}').trim())
        .ok()
        .and_then(|d| d.descendants().find(|n| n.attribute("Name") == Some("SerialNumber")).and_then(|n| n.attribute("Value").map(String::from)))
        .unwrap_or_default()
}

pub fn parse_identifiers(xml: &str) -> Vec<String> {
    roxmltree::Document::parse(xml.trim_start_matches('\u{feff}').trim())
        .map(|d| d.descendants().filter_map(|n| n.attribute("Identifier").map(String::from)).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
//  Luồng máy i
// ---------------------------------------------------------------------------------------------
pub struct InstrumentI {
    stop: Arc<AtomicBool>,
    online: Arc<AtomicBool>,
    api: Arc<dyn ByKApi>,
    handle: Option<JoinHandle<()>>,
}

impl InstrumentI {
    pub fn start(
        api: Arc<dyn ByKApi>,
        events: Sender<Event>,
        raw_log: PathBuf,
        ip: String,
        repaint: impl Fn() + Send + 'static,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let online = Arc::new(AtomicBool::new(false));
        let (s2, o2, a2) = (stop.clone(), online.clone(), api.clone());
        let handle = std::thread::spawn(move || {
            let mut sess = Session { api: a2, tx: events, stop: s2, online: o2, raw: Mutex::new(std::fs::OpenOptions::new().create(true).append(true).open(raw_log).ok()), ident: None, repaint: Box::new(repaint) };
            sess.run(&ip);
        });
        Self { stop, online, api, handle: Some(handle) }
    }

    /// Nạp BYKCommunication.dll thật cạnh chương trình rồi chạy. Lỗi -> gửi Event::Error.
    pub fn start_dll(app_dir: &Path, events: Sender<Event>, raw_log: PathBuf, ip: String, repaint: impl Fn() + Send + 'static) -> Option<Self> {
        match dll::DllApi::load(app_dir) {
            Ok(api) => Some(Self::start(Arc::new(api), events, raw_log, ip, repaint)),
            Err(e) => {
                let _ = events.send(Event::Error(e));
                repaint();
                None
            }
        }
    }
}

impl Device for InstrumentI {
    fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }
    fn operate(&self) {}
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.api.cancel_wait();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for InstrumentI {
    fn drop(&mut self) {
        Device::stop(self);
    }
}

struct Session {
    api: Arc<dyn ByKApi>,
    tx: Sender<Event>,
    stop: Arc<AtomicBool>,
    online: Arc<AtomicBool>,
    raw: Mutex<Option<std::fs::File>>,
    ident: Option<String>,
    repaint: Box<dyn Fn() + Send>,
}

impl Session {
    fn log(&self, s: &str) {
        use std::io::Write;
        if let Ok(mut g) = self.raw.lock() {
            if let Some(f) = g.as_mut() {
                let _ = writeln!(f, "{s}");
            }
        }
    }

    fn send(&self, ev: Event) {
        let _ = self.tx.send(ev);
        (self.repaint)();
    }

    fn command(&self, ident: &str, cmd: i32, params: &str) -> Result<String, String> {
        let xml = command_xml(cmd, params);
        self.log(&format!(">> {xml}"));
        match self.api.send_receive(ident, &xml) {
            Ok(r) => {
                self.log(&format!("<< {r}"));
                Ok(r)
            }
            Err(rc) => Err(format!("BykCommunicationDLL Error: {rc}")),
        }
    }

    fn run(&mut self, ip: &str) {
        self.api.enable_tcpip();
        let ip = ip.trim();
        if !ip.is_empty() {
            let parts: Vec<u8> = ip.split('.').filter_map(|x| x.parse().ok()).collect();
            if parts.len() == 4 {
                self.api.set_ip_range([parts[0], parts[1], parts[2], parts[3]]);
            }
        }
        if self.api.init(INIT_NAME, INIT_KEY) != 0 {
            self.send(Event::Error("BykCommunication.dll đang được chương trình khác dùng (đóng Client_HazeGard.exe rồi thử lại).".into()));
            return;
        }
        for id in EVENT_IDS {
            self.api.register_event(id);
        }
        match self.api.list_all() {
            Ok(text) => {
                self.log(&format!("listAllInstruments: {text}"));
                for id in parse_identifiers(&text) {
                    self.add(&id);
                }
            }
            Err(rc) => self.log(&format!("listAllInstruments lỗi {rc}")),
        }
        while !self.stop.load(Ordering::Relaxed) {
            let w = self.api.wait_event();
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            match w.ev_type {
                EV_ADD => self.add(&w.ident),
                EV_REMOVE => {
                    if self.ident.as_deref() == Some(w.ident.as_str()) {
                        self.online.store(false, Ordering::Relaxed);
                        self.ident = None;
                        self.send(Event::Error("Máy i bị rút / mất kết nối.".into()));
                    }
                }
                EV_STATUS if self.ident.as_deref() == Some(w.ident.as_str()) => self.status(w.code),
                _ => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
        // thoát: tắt online mode, giải phóng DLL
        if let (Some(id), true) = (self.ident.clone(), self.online.load(Ordering::Relaxed)) {
            let _ = self.command(&id, CMD_STOP_ONLINE, "");
        }
        self.api.de_init();
    }

    fn add(&mut self, ident: &str) {
        if self.ident.is_some() {
            return; // chỉ dùng máy đầu tiên
        }
        self.api.init_connection(ident);
        let serial = self.command(ident, CMD_DEVICE_INFO, "").map(|x| parse_serial(&x)).unwrap_or_default();
        self.ident = Some(ident.to_string());
        if let Err(e) = self.command(ident, CMD_START_ONLINE, START_ONLINE_PARAMS) {
            self.send(Event::Error(e));
            return;
        }
        self.online.store(true, Ordering::Relaxed);
        self.send(Event::ConnectedI { serial, identifier: ident.to_string() });
    }

    fn status(&mut self, code: i32) {
        match code {
            ST_MEASURE_END => {
                let id = self.ident.clone().unwrap_or_default();
                match self.command(&id, CMD_READ_ONLINE, "").and_then(|x| parse_measure(&x)) {
                    Ok(m) => self.send(Event::Result { t: m.t, h: m.h }),
                    Err(e) => self.send(Event::Error(format!("Đọc kết quả lỗi: {e}"))),
                }
            }
            ST_ONLINE_END => self.online.store(false, Ordering::Relaxed),
            ST_PORT_CLARITY => self.send(Event::PortSwitch),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------------------------
//  DLL thật (libloading). Hàm export dạng stdcall decorated: _ten@N
// ---------------------------------------------------------------------------------------------
pub mod dll {
    use super::*;
    use libloading::Library;
    use std::ffi::{CString, c_char};

    type FInit = unsafe extern "system" fn(*const c_char, *const c_char) -> i32;
    type FVoid = unsafe extern "system" fn();
    type FDeInit = unsafe extern "system" fn() -> i32;
    type FRegister = unsafe extern "system" fn(i32) -> i32;
    type FList = unsafe extern "system" fn(*mut u8, u32, *mut i32) -> i32;
    type FConn = unsafe extern "system" fn(*const c_char) -> i32;
    type FSendRecv = unsafe extern "system" fn(*const c_char, *const c_char, u32, *mut u8, u32, *mut i32) -> i32;
    type FWait = unsafe extern "system" fn(i32, *mut i32, *mut u8, u32, *mut i32, *mut u8, u32, *mut i32) -> i32;
    type FIpRange = unsafe extern "system" fn(*const u8);

    pub struct DllApi {
        _lib: Library,
        init: FInit,
        de_init: FDeInit,
        enable_tcpip: FVoid,
        set_ip: FIpRange,
        register: FRegister,
        list: FList,
        init_conn: FConn,
        send_recv: FSendRecv,
        wait: FWait,
        cancel: FVoid,
    }

    // con trỏ hàm + Library đều dùng được giữa các luồng; DLL tự khoá nội bộ (phần mềm gốc cũng gọi từ nhiều luồng)
    unsafe impl Send for DllApi {}
    unsafe impl Sync for DllApi {}

    impl DllApi {
        #[allow(unreachable_code)]
        pub fn load(app_dir: &Path) -> Result<Self, String> {
            #[cfg(all(windows, target_pointer_width = "64"))]
            return Err("Máy i cần bản chương trình 32-bit (BYKCommunication.dll là bản 32-bit). Build với target i686-pc-windows-msvc.".into());
            #[allow(unreachable_code)]
            {
                // ưu tiên file cạnh exe; không có -> dùng bản nhúng trong exe (nếu build với --features embed_dll)
                let beside = [app_dir.join("BYKCommunication.dll"), PathBuf::from("BYKCommunication.dll")].into_iter().find(|p| p.exists());
                let path = match beside {
                    Some(p) => p,
                    None => extract_embedded()?,
                };
                Self::load_path(&path, true)
            }
        }

        /// `decorated` = tên export stdcall `_ten@N` (DLL thật); false = tên trơn (chỉ để test bằng thư viện giả trên Linux)
        pub fn load_path(path: &Path, decorated: bool) -> Result<Self, String> {
            unsafe {
                let lib = Library::new(path).map_err(|e| format!("Không nạp được {}: {e}", path.display()))?;
                macro_rules! sym {
                    ($base:expr, $n:expr, $t:ty) => {{
                        let name = if decorated { format!("_{}@{}", $base, $n) } else { $base.to_string() };
                        *lib.get::<$t>(name.as_bytes()).map_err(|e| format!("Thiếu hàm {name} trong DLL: {e}"))?
                    }};
                }
                Ok(Self {
                    init: sym!("init", 8, FInit),
                    de_init: sym!("deInit", 0, FDeInit),
                    enable_tcpip: sym!("enableTCPIPConnections", 0, FVoid),
                    set_ip: sym!("setIPAddressRange", 4, FIpRange),
                    register: sym!("registerEventMessage", 4, FRegister),
                    list: sym!("listAllInstruments", 12, FList),
                    init_conn: sym!("initConnection", 4, FConn),
                    send_recv: sym!("SendReceive", 24, FSendRecv),
                    wait: sym!("waitForEventDataEx", 32, FWait),
                    cancel: sym!("cancelWaitForEvent", 0, FVoid),
                    _lib: lib,
                })
            }
        }
    }

    #[cfg(feature = "embed_dll")]
    static EMBEDDED: &[u8] = include_bytes!("../assets/BYKCommunication.dll");

    /// Giải nén DLL nhúng ra %TEMP%\\HazeGardLogger\\<mã băm>\\ rồi trả đường dẫn.
    fn extract_embedded() -> Result<PathBuf, String> {
        #[cfg(feature = "embed_dll")]
        {
            use sha2::{Digest, Sha256};
            let h: String = Sha256::digest(EMBEDDED).iter().take(6).map(|b| format!("{b:02x}")).collect();
            let dir = std::env::temp_dir().join("HazeGardLogger").join(h);
            std::fs::create_dir_all(&dir).map_err(|e| format!("Không tạo được thư mục tạm: {e}"))?;
            let p = dir.join("BYKCommunication.dll");
            if std::fs::metadata(&p).map(|m| m.len() as usize).ok() != Some(EMBEDDED.len()) {
                std::fs::write(&p, EMBEDDED).map_err(|e| format!("Không giải nén được DLL nhúng: {e}"))?;
            }
            Ok(p)
        }
        #[cfg(not(feature = "embed_dll"))]
        Err("Không thấy BYKCommunication.dll cạnh chương trình.".into())
    }

    fn cstr(s: &str) -> CString {
        CString::new(s.replace('\0', "")).unwrap_or_default()
    }

    fn cut(buf: &[u8], n: i32) -> String {
        let n = (n.max(0) as usize).min(buf.len());
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    impl ByKApi for DllApi {
        fn enable_tcpip(&self) {
            unsafe { (self.enable_tcpip)() }
        }
        fn set_ip_range(&self, ip: [u8; 4]) {
            unsafe { (self.set_ip)(ip.as_ptr()) }
        }
        fn init(&self, name: &str, key: &str) -> i32 {
            let (n, k) = (cstr(name), cstr(key));
            unsafe { (self.init)(n.as_ptr(), k.as_ptr()) }
        }
        fn de_init(&self) {
            unsafe {
                (self.de_init)();
            }
        }
        fn register_event(&self, id: i32) {
            unsafe {
                (self.register)(id);
            }
        }
        fn list_all(&self) -> Result<String, i32> {
            let mut buf = vec![0u8; BUF];
            let mut n: i32 = 0;
            let rc = unsafe { (self.list)(buf.as_mut_ptr(), BUF as u32, &mut n) };
            let _ = rc; // phần mềm gốc bỏ qua mã trả về của hàm này
            Ok(cut(&buf, n))
        }
        fn init_connection(&self, ident: &str) -> i32 {
            let i = cstr(ident);
            unsafe { (self.init_conn)(i.as_ptr()) }
        }
        fn send_receive(&self, ident: &str, xml: &str) -> Result<String, i32> {
            let (i, x) = (cstr(ident), cstr(xml));
            let mut buf = vec![0u8; BUF];
            let mut n: i32 = 0;
            let rc = unsafe { (self.send_recv)(i.as_ptr(), x.as_ptr(), xml.len() as u32, buf.as_mut_ptr(), BUF as u32, &mut n) };
            if rc != 0 { Err(rc) } else { Ok(cut(&buf, n)) }
        }
        fn wait_event(&self) -> WaitResult {
            let (mut ev, mut code, mut dlen) = (0i32, 0i32, 0i32);
            let mut ident = [0u8; 60];
            let mut data = vec![0u8; 5125];
            unsafe { (self.wait)(7, &mut ev, ident.as_mut_ptr(), 60, &mut code, data.as_mut_ptr(), 5125, &mut dlen) };
            let end = ident.iter().position(|&b| b == 0).unwrap_or(ident.len());
            WaitResult { ev_type: ev, ident: String::from_utf8_lossy(&ident[..end]).into_owned(), code }
        }
        fn cancel_wait(&self) {
            unsafe { (self.cancel)() }
        }
    }
}

// ---------------------------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    const MEAS: &str = r#"<CommandResult><Data><MeasureValue Guid="x" TimeStamp="2026-11-11T10:00:00"><ScaleValue Id="500" Value="85.0"/><ScaleValue Id="501" Value="1.57"/><ScaleValue Id="506" Value="99.1"/></MeasureValue></Data></CommandResult>"#;
    const INFO: &str = r#"<CommandResult><Data><DataValueCollection><DataValue Name="Foo" Value="1"/><DataValue Name="SerialNumber" Value="2088061"/></DataValueCollection></Data></CommandResult>"#;

    #[test]
    fn xml_parsing() {
        let m = parse_measure(MEAS).unwrap();
        assert_eq!((m.t, m.h, m.timestamp.as_deref()), (Some(85.0), Some(1.57), Some("2026-11-11T10:00:00")));
        assert!(parse_measure(r#"<CommandResult><Error>No sample</Error></CommandResult>"#).unwrap_err().contains("No sample"));
        assert_eq!(parse_serial(INFO), "2088061");
        assert_eq!(parse_identifiers(r#"<Instruments><Instrument Identifier="USB0"/><Instrument Identifier="USB1"/></Instruments>"#), vec!["USB0", "USB1"]);
        assert_eq!(command_xml(401, ""), "<?xml version=\"1.0\" encoding=\"utf-8\"?><Command Id=\"401\"></Command>");
    }

    /// Máy giả: ghi lại lệnh, bắn 1 sự kiện "đo xong", rồi chờ cancel.
    struct Fake {
        sent: Mutex<Vec<String>>,
        fired: Mutex<bool>,
        cancel: (Mutex<bool>, Condvar),
    }
    impl Fake {
        fn new() -> Arc<Self> {
            Arc::new(Self { sent: Mutex::new(vec![]), fired: Mutex::new(false), cancel: (Mutex::new(false), Condvar::new()) })
        }
    }
    impl ByKApi for Fake {
        fn enable_tcpip(&self) {}
        fn set_ip_range(&self, _ip: [u8; 4]) {}
        fn init(&self, name: &str, key: &str) -> i32 {
            self.sent.lock().unwrap().push(format!("init {name} {key}"));
            0
        }
        fn de_init(&self) {
            self.sent.lock().unwrap().push("deinit".into());
        }
        fn register_event(&self, _id: i32) {}
        fn list_all(&self) -> Result<String, i32> {
            Ok(r#"<Instruments><Instrument Identifier="USB0"/></Instruments>"#.into())
        }
        fn init_connection(&self, _i: &str) -> i32 {
            0
        }
        fn send_receive(&self, _i: &str, xml: &str) -> Result<String, i32> {
            self.sent.lock().unwrap().push(xml.to_string());
            let id = xml.split("Id=\"").nth(1).unwrap().split('"').next().unwrap();
            Ok(match id {
                "4" => INFO,
                "403" => MEAS,
                _ => "<CommandResult/>",
            }
            .to_string())
        }
        fn wait_event(&self) -> WaitResult {
            let mut f = self.fired.lock().unwrap();
            if !*f {
                *f = true;
                std::thread::sleep(Duration::from_millis(100));
                return WaitResult { ev_type: EV_STATUS, ident: "USB0".into(), code: ST_MEASURE_END };
            }
            drop(f);
            let (m, c) = &self.cancel;
            let mut g = m.lock().unwrap();
            while !*g {
                g = c.wait_timeout(g, Duration::from_secs(5)).unwrap().0;
            }
            WaitResult::default()
        }
        fn cancel_wait(&self) {
            *self.cancel.0.lock().unwrap() = true;
            self.cancel.1.notify_all();
        }
    }

    #[test]
    fn full_session_with_fake_dll() {
        let fake = Fake::new();
        let (tx, rx) = channel();
        let log = std::env::temp_dir().join(format!("hz_i_{}.log", std::process::id()));
        let mut inst = InstrumentI::start(fake.clone(), tx, log.clone(), String::new(), || {});
        let a = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(&a, Event::ConnectedI { serial, identifier } if serial == "2088061" && identifier == "USB0"), "{a:?}");
        let b = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(b, Event::Result { t: Some(t), h: Some(h) } if t == 85.0 && h == 1.57), "{b:?}");
        assert!(inst.is_online());
        Device::stop(&mut inst);
        let sent = fake.sent.lock().unwrap().clone();
        let ids: Vec<&str> = sent.iter().filter(|x| x.starts_with("<?xml")).map(|x| x.split("Id=\"").nth(1).unwrap().split('"').next().unwrap()).collect();
        assert_eq!(ids, vec!["4", "400", "403", "401"]);
        assert!(sent[0].starts_with("init BykGardnerInstrumentUnits"));
        assert!(sent.iter().any(|x| x.contains("Name=\"Transmission\" Value=\"true\"") && x.contains("Name=\"Clarity\" Value=\"false\"")));
        assert_eq!(sent.last().unwrap(), "deinit");
        let _ = std::fs::remove_file(log);
    }

    /// Thử marshalling của DllApi thật bằng thư viện giả biên dịch bằng gcc (chỉ Linux, cần gcc).
    #[cfg(target_os = "linux")]
    #[test]
    fn real_dllapi_against_fake_shared_lib() {
        let dir = std::env::temp_dir().join(format!("hz_fakelib_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let c = dir.join("fake.c");
        std::fs::write(&c, FAKE_C).unwrap();
        let so = dir.join("BYKCommunication.dll");
        let ok = std::process::Command::new("gcc").args(["-shared", "-fPIC", "-o"]).arg(&so).arg(&c).status().map(|s| s.success()).unwrap_or(false);
        if !ok {
            panic!("gcc không biên dịch được thư viện giả");
            return;
        }
        let api = dll::DllApi::load_path(&so, false).unwrap();
        api.enable_tcpip();
        assert_eq!(api.init("BykGardnerInstrumentUnits", "BYKUnits81!654"), 0);
        assert_eq!(api.init("sai", "sai"), 7);
        assert!(api.list_all().unwrap().contains("Identifier=\"USB0\""));
        let r = api.send_receive("USB0", &command_xml(4, "")).unwrap();
        assert_eq!(parse_serial(&r), "2088061");
        assert_eq!(api.send_receive("USB0", "bad").unwrap_err(), 5);
        let w = api.wait_event();
        assert_eq!((w.ev_type, w.ident.as_str(), w.code), (EV_STATUS, "USB0", ST_MEASURE_END));
        api.de_init();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(target_os = "linux")]
    const FAKE_C: &str = r#"
#include <string.h>
#include <stdio.h>
int init(const char*n,const char*k);
int init(const char*n,const char*k){ return (strcmp(n,"BykGardnerInstrumentUnits")==0 && strcmp(k,"BYKUnits81!654")==0)?0:7; }
int deInit(void);
int deInit(void){ return 0; }
void enableTCPIPConnections(void);
void enableTCPIPConnections(void){}
void setIPAddressRange(const unsigned char*b);
void setIPAddressRange(const unsigned char*b){}
int registerEventMessage(int e);
int registerEventMessage(int e){ return 0; }
int listAllInstruments(unsigned char*buf, unsigned int size, int*n);
int listAllInstruments(unsigned char*buf, unsigned int size, int*n){ const char*s="<Instruments><Instrument Identifier=\"USB0\"/></Instruments>"; strcpy((char*)buf,s); *n=(int)strlen(s); return 0; }
int initConnection(const char*i);
int initConnection(const char*i){ return 0; }
int SendReceive(const char*i,const char*x,unsigned int len,unsigned char*buf,unsigned int size,int*n);
int SendReceive(const char*i,const char*x,unsigned int len,unsigned char*buf,unsigned int size,int*n){
  if(strncmp(x,"<?xml",5)!=0 || len!=strlen(x)) return 5;
  const char*s="<CommandResult><Data><DataValueCollection><DataValue Name=\"SerialNumber\" Value=\"2088061\"/></DataValueCollection></Data></CommandResult>";
  strcpy((char*)buf,s); *n=(int)strlen(s); return 0; }
int waitForEventDataEx(int mask,int*ev,unsigned char*ident,unsigned int isz,int*code,unsigned char*data,unsigned int dsz,int*dlen);
int waitForEventDataEx(int mask,int*ev,unsigned char*ident,unsigned int isz,int*code,unsigned char*data,unsigned int dsz,int*dlen){
  if(mask!=7||isz!=60||dsz!=5125) return 9; *ev=4; strcpy((char*)ident,"USB0"); *code=-2147483236; *dlen=0; return 0; }
void cancelWaitForEvent(void);
void cancelWaitForEvent(void){}
"#;
}
