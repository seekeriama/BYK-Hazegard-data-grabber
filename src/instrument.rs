//! Luồng giao tiếp với máy qua cổng COM; gửi sự kiện về giao diện bằng mpsc channel.

use crate::protocol::{self, OnlineParser, ParseEvent, Status};
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum Event {
    Connected { online: bool, status: Option<Status> },
    Result { t: Option<f64>, h: Option<f64> },
    PortSwitch,
    Error(String),
}

enum Cmd {
    Operate,
}

pub struct Instrument {
    cmd_tx: Sender<Cmd>,
    stop: Arc<AtomicBool>,
    online: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

fn read_chunk(port: &mut dyn serialport::SerialPort, buf: &mut [u8]) -> std::io::Result<usize> {
    match port.read(buf) {
        Ok(n) => Ok(n),
        Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted) => Ok(0),
        Err(e) => Err(e),
    }
}

impl Instrument {
    pub fn start(
        port_name: String,
        events: Sender<Event>,
        raw_log: PathBuf,
        release_first: bool,
        repaint: impl Fn() + Send + 'static,
    ) -> Self {
        let (cmd_tx, cmd_rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let online = Arc::new(AtomicBool::new(false));
        let (s2, o2) = (stop.clone(), online.clone());
        let handle = std::thread::spawn(move || run(port_name, events, cmd_rx, s2, o2, raw_log, release_first, repaint));
        Self { cmd_tx, stop, online, handle: Some(handle) }
    }

    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    pub fn operate(&self) {
        let _ = self.cmd_tx.send(Cmd::Operate);
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Instrument {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(
    port_name: String,
    tx: Sender<Event>,
    rx: Receiver<Cmd>,
    stop: Arc<AtomicBool>,
    online_flag: Arc<AtomicBool>,
    raw_log: PathBuf,
    release_first: bool,
    repaint: impl Fn(),
) {
    let send = |ev: Event| {
        let _ = tx.send(ev);
        repaint();
    };
    let mut port = match serialport::new(&port_name, 9600)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .timeout(Duration::from_millis(200))
        .open()
    {
        Ok(p) => p,
        Err(e) => {
            send(Event::Error(format!("Không mở được {port_name}: {e}")));
            return;
        }
    };
    let _ = port.write_data_terminal_ready(true);
    let _ = port.write_request_to_send(true);
    let mut raw_f = std::fs::OpenOptions::new().create(true).append(true).open(&raw_log).ok();
    let mut log_raw = |b: &[u8]| {
        if let (Some(f), false) = (raw_f.as_mut(), b.is_empty()) {
            let _ = f.write_all(b);
            let _ = f.flush();
        }
    };

    // 0) giải phóng máy đang kẹt Online Mode (lệnh 50h) rồi mới handshake
    if release_first {
        let _ = port.clear(serialport::ClearBuffer::Input);
        let _ = port.write_all(&[protocol::CMD_END]);
        std::thread::sleep(Duration::from_millis(400));
    }
    // 1) thử vào online mode (lệnh 4Fh)
    let _ = port.clear(serialport::ClearBuffer::Input);
    let _ = port.write_all(&[protocol::CMD_ONLINE]);
    let mut resp: Vec<u8> = vec![];
    let t0 = Instant::now();
    let mut tmp = [0u8; 64];
    while t0.elapsed() < Duration::from_secs(2) && resp.len() < 12 && !stop.load(Ordering::Relaxed) {
        match read_chunk(&mut *port, &mut tmp) {
            Ok(n) => resp.extend_from_slice(&tmp[..n.min(12 - resp.len())]),
            Err(e) => {
                send(Event::Error(format!("Mất kết nối {port_name}: {e}")));
                return;
            }
        }
    }
    log_raw(&resp);
    let mut is_online = false;
    if resp.len() >= 12 && resp[0] == protocol::CMD_ONLINE && resp[1] == protocol::ACK {
        is_online = true;
        online_flag.store(true, Ordering::Relaxed);
        send(Event::Connected { online: true, status: protocol::decode_status(&resp[2..12]) });
    } else {
        // 2) không phản hồi -> nghe thụ động (máy ở chế độ Print Online)
        send(Event::Connected { online: false, status: None });
    }

    let mut parser = OnlineParser::new(3);
    // chế độ thụ động: byte nhận được trong lúc chờ 4Fh có thể đã là kết quả đo -> giữ lại
    let mut buf: Vec<u8> = if is_online { vec![] } else { resp };
    let mut last_rx = Instant::now();

    'main: while !stop.load(Ordering::Relaxed) {
        while let Ok(Cmd::Operate) = rx.try_recv() {
            if is_online && port.write_all(&[protocol::CMD_OPERATE]).is_err() {
                send(Event::Error(format!("Mất kết nối {port_name}")));
                break 'main;
            }
        }
        let n = match read_chunk(&mut *port, &mut tmp) {
            Ok(n) => n,
            Err(e) => {
                send(Event::Error(format!("Mất kết nối {port_name}: {e}")));
                break 'main;
            }
        };
        let chunk = &tmp[..n];
        if n > 0 {
            log_raw(chunk);
            last_rx = Instant::now();
        }
        if is_online {
            for ev in parser.feed(chunk) {
                match ev {
                    ParseEvent::SwitchPort => send(Event::PortSwitch),
                    ParseEvent::Reading { values, .. } => {
                        let t = values.first().copied().flatten();
                        let h = values.get(1).copied().flatten();
                        send(Event::Result { t, h });
                    }
                }
            }
            continue;
        }
        // thụ động: tách dòng theo CR/LF, hoặc sau 1 s im lặng thì coi là hết bản tin
        buf.extend_from_slice(chunk);
        let mut lines: Vec<Vec<u8>> = buf.split(|b| *b == b'\r' || *b == b'\n').map(|l| l.to_vec()).collect();
        let complete: Vec<Vec<u8>>;
        if !buf.is_empty() && last_rx.elapsed() > Duration::from_secs(1) {
            complete = std::mem::take(&mut lines);
            buf.clear();
        } else {
            buf = lines.pop().unwrap_or_default();
            complete = lines;
        }
        for l in complete {
            let text: String = l.iter().map(|&b| b as char).collect();
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            let p = protocol::parse_line(text);
            if p.t.is_some() || p.h.is_some() {
                send(Event::Result { t: p.t, h: p.h });
            }
        }
    }
    // thoát online mode, mở khoá bàn phím máy
    if is_online {
        let _ = port.write_all(&[protocol::CMD_END]);
        std::thread::sleep(Duration::from_millis(200));
    }
    online_flag.store(false, Ordering::Relaxed);
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serialport::{SerialPort, TTYPort};
    use std::io::Read;

    fn collect(rx: &Receiver<Event>, n: usize, secs: u64) -> Vec<Event> {
        let mut out = vec![];
        let t0 = Instant::now();
        while out.len() < n && t0.elapsed() < Duration::from_secs(secs) {
            if let Ok(e) = rx.recv_timeout(Duration::from_millis(100)) {
                out.push(e);
            }
        }
        out
    }

    #[test]
    fn online_mode_with_fake_instrument() {
        let seq = online_session(true);
        assert_eq!(seq, vec![0x50, 0x4F, 0x50], "bat dau phai gui 50h roi 4Fh, thoat gui 50h");
    }

    #[test]
    fn online_mode_without_release() {
        let seq = online_session(false);
        assert_eq!(seq, vec![0x4F, 0x50], "tat tuy chon: chi gui 4Fh khi bat dau");
    }

    /// Chạy 1 phiên online với máy giả, trả về chuỗi byte máy nhận được (trừ lệnh 04h).
    fn online_session(release_first: bool) -> Vec<u8> {
        let (mut master, slave) = TTYPort::pair().unwrap();
        let name = slave.name().unwrap();
        let fake = std::thread::spawn(move || {
            let mut b = [0u8; 16];
            let t0 = Instant::now();
            let mut seq: Vec<u8> = vec![];
            let mut got_end = false;
            while t0.elapsed() < Duration::from_secs(8) && !got_end {
                if let Ok(n) = master.read(&mut b) {
                    for &c in &b[..n] {
                        if c != 0x04 { seq.push(c); }
                        match c {
                            0x4F => { master.write_all(&[0x4F, 0x21, 1, 3, 1, 0, 0, 1, 1, 5, 0x12, 0x01]).unwrap(); }
                            0x04 => {
                                std::thread::sleep(Duration::from_millis(100));
                                master.write_all(b"\x04!f89,2\x000,05\x00").unwrap();
                                master.write_all(b"w----\x00").unwrap();
                            }
                            0x50 => got_end = seq.contains(&0x4F),
                            _ => {}
                        }
                    }
                }
            }
            seq
        });
        let (tx, rx) = channel();
        let log = std::env::temp_dir().join(format!("hz_raw_{}.log", std::process::id()));
        let mut inst = Instrument::start(name, tx, log.clone(), release_first, || {});
        let ev = collect(&rx, 1, 5);
        assert!(matches!(&ev[0], Event::Connected { online: true, status: Some(s) } if s.firmware == "0112" && s.has_haze()));
        assert!(inst.is_online());
        inst.operate();
        let ev = collect(&rx, 2, 5);
        assert!(matches!(ev[0], Event::PortSwitch), "{ev:?}");
        assert!(matches!(ev[1], Event::Result { t: Some(t), h: Some(h) } if (t - 89.2).abs() < 1e-9 && (h - 0.05).abs() < 1e-9), "{ev:?}");
        inst.stop();
        let seq = fake.join().unwrap();
        let _ = std::fs::remove_file(log);
        seq
    }

    #[test]
    fn passive_mode_when_no_reply() {
        let (mut master, slave) = TTYPort::pair().unwrap();
        let name = slave.name().unwrap();
        let (tx, rx) = channel();
        let log = std::env::temp_dir().join(format!("hz_raw2_{}.log", std::process::id()));
        let mut inst = Instrument::start(name, tx, log.clone(), true, || {});
        let ev = collect(&rx, 1, 6);
        assert!(matches!(ev[0], Event::Connected { online: false, .. }), "{ev:?}");
        assert!(!inst.is_online());
        master.write_all(b"11.05.26  01:51pm    100   0.00\r\n").unwrap();
        let ev = collect(&rx, 1, 4);
        assert!(matches!(ev[0], Event::Result { t: Some(t), h: Some(h) } if t == 100.0 && h == 0.0), "{ev:?}");
        // ban tin khong xuong dong -> het 1 s im lang van duoc tach
        master.write_all(b"11.05.26  01:52pm    96.7   2.70").unwrap();
        let ev = collect(&rx, 1, 4);
        assert!(matches!(ev[0], Event::Result { t: Some(t), h: Some(h) } if t == 96.7 && h == 2.7), "{ev:?}");
        inst.stop();
        let _ = std::fs::remove_file(log);
    }
}
