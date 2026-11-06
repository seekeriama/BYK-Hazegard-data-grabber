//! Giao thức RS232 của BYK haze-gard plus (theo "Interface Description" trong sách hướng dẫn)
//!  - 9600 baud, 8N1 cố định
//!  - PC gửi 0x4F  -> máy trả 0x4F 0x21 + 10 byte trạng thái (vào online mode)
//!  - PC gửi 0x04  -> máy trả 0x04 0x21, đo xong gửi 0x66 ('f') rồi 3 giá trị,
//!                    mỗi giá trị = 4 ký tự ASCII + 0x00 (vd "95,4" 00)
//!  - 0x77 ('w') có thể chen giữa các giá trị (chuyển mẫu từ cổng haze sang cổng clarity)
//!  - PC gửi 0x50 ('P') để thoát online mode

use regex::Regex;
use std::sync::LazyLock;

pub const CMD_ONLINE: u8 = 0x4F;
pub const CMD_OPERATE: u8 = 0x04;
pub const CMD_END: u8 = 0x50;
pub const ACK: u8 = 0x21;
const READOUT: u8 = 0x66;
const SWITCH_PORT: u8 = 0x77;

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub mode: u8,
    pub continuous: bool,
    pub cal_valid: bool,
    pub firmware: String,
}

impl Status {
    pub fn has_transmission(&self) -> bool {
        matches!(self.mode, 0 | 3 | 4 | 6)
    }
    pub fn has_haze(&self) -> bool {
        matches!(self.mode, 1 | 3 | 5 | 6)
    }
    pub fn has_clarity(&self) -> bool {
        matches!(self.mode, 2 | 4 | 5 | 6)
    }
}

/// 10 byte trạng thái máy gửi sau lệnh 4Fh.
pub fn decode_status(st: &[u8]) -> Option<Status> {
    if st.len() < 10 {
        return None;
    }
    Some(Status {
        mode: st[1],
        continuous: st[4] != 0,
        cal_valid: st[5] != 0,
        firmware: format!("{:02X}{:02X}", st[9], st[8]),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseEvent {
    /// [T, H, C] theo thứ tự máy gửi, kèm chuỗi gốc từng giá trị
    Reading { values: Vec<Option<f64>>, raw: Vec<String> },
    SwitchPort,
}

pub fn to_number(token: &[u8]) -> Option<f64> {
    let s: String = token.iter().map(|&b| b as char).collect();
    s.trim().replace(',', ".").parse::<f64>().ok()
}

/// Bộ tách dữ liệu online: nhận luồng byte, trả về các kết quả đo hoàn chỉnh.
pub struct OnlineParser {
    n_values: usize,
    in_frame: bool,
    token: Vec<u8>,
    values: Vec<Option<f64>>,
    raw: Vec<String>,
}

impl OnlineParser {
    pub fn new(n_values: usize) -> Self {
        Self { n_values, in_frame: false, token: vec![], values: vec![], raw: vec![] }
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<ParseEvent> {
        let mut out = vec![];
        for &b in data {
            if !self.in_frame {
                if b == READOUT {
                    self.in_frame = true;
                    self.token.clear();
                    self.values.clear();
                    self.raw.clear();
                }
                continue; // bỏ qua ACK / echo lệnh
            }
            if b == SWITCH_PORT && self.token.is_empty() {
                out.push(ParseEvent::SwitchPort);
                continue;
            }
            if b == 0x00 {
                self.raw.push(self.token.iter().map(|&c| c as char).collect());
                self.values.push(to_number(&self.token));
                self.token.clear();
                if self.values.len() >= self.n_values {
                    out.push(ParseEvent::Reading {
                        values: std::mem::take(&mut self.values),
                        raw: std::mem::take(&mut self.raw),
                    });
                    self.in_frame = false;
                }
                continue;
            }
            self.token.push(b);
            if self.token.len() > 8 {
                self.in_frame = false; // lỗi đồng bộ -> huỷ bản tin
            }
        }
        out
    }
}

// ---------------------------------------------------------------------
//  Chế độ nghe thụ động (Print Online): tách T / H / C từ một dòng văn bản
// ---------------------------------------------------------------------
static NUM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[-+]?\d+(?:[.,]\d+)?").unwrap());
static DATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d{1,2}[./-]\d{1,2}[./-]\d{2,4}\b").unwrap());
static TIME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b\d{1,2}:\d{2}(?::\d{2})?\s*(?:[ap]\.?m\.?)?").unwrap());
static PAIR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z]+)\s*[:=]?\s*([-+]?\d+(?:[.,]\d+)?)").unwrap());
static LABEL_T: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?i:t|tt|trans\w*|transmission)$").unwrap());
static LABEL_H: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?i:h|haze)$").unwrap());
static LABEL_C: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?i:c|cl|clar\w*|clarity)$").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Parsed {
    pub t: Option<f64>,
    pub h: Option<f64>,
    pub c: Option<f64>,
}

fn num(s: &str) -> Option<f64> {
    s.replace(',', ".").parse().ok()
}

pub fn parse_line(line: &str) -> Parsed {
    let mut r = Parsed::default();
    // 1) cặp "nhãn số": "T 91.2", "Haze: 0.45", "C=99.8"
    for m in PAIR_RE.captures_iter(line) {
        let (label, v) = (&m[1], num(&m[2]));
        if r.t.is_none() && LABEL_T.is_match(label) {
            r.t = v;
        } else if r.h.is_none() && LABEL_H.is_match(label) {
            r.h = v;
        } else if r.c.is_none() && LABEL_C.is_match(label) {
            r.c = v;
        }
    }
    // 2) không có nhãn: bỏ ngày/giờ máy in kèm, rồi lấy các số theo thứ tự T, H, C
    if r.t.is_none() && r.h.is_none() && r.c.is_none() {
        let rest = DATE_RE.replace_all(line, " ");
        let rest = TIME_RE.replace_all(&rest, " ");
        let mut it = NUM_RE.find_iter(&rest).filter_map(|m| num(m.as_str()));
        r.t = it.next();
        r.h = it.next();
        r.c = it.next();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_matches_python() {
        let p = parse_line("11.05.26  12:03pm    101   0.00");
        assert_eq!((p.t, p.h, p.c), (Some(101.0), Some(0.0), None));
        let p = parse_line("11.05.26 12:13pm  91.2  0.45  99.8");
        assert_eq!((p.t, p.h, p.c), (Some(91.2), Some(0.45), Some(99.8)));
        let p = parse_line("05.11.2026 14:03 90.5 1.20");
        assert_eq!((p.t, p.h, p.c), (Some(90.5), Some(1.2), None));
        let p = parse_line("T 91.2  H 0.45  C 99.8");
        assert_eq!((p.t, p.h, p.c), (Some(91.2), Some(0.45), Some(99.8)));
        let p = parse_line("Haze: 1,23 Clarity=98.7 Transmission 90.1");
        assert_eq!((p.t, p.h, p.c), (Some(90.1), Some(1.23), Some(98.7)));
        let p = parse_line("  91.5   0.38   99.9");
        assert_eq!((p.t, p.h, p.c), (Some(91.5), Some(0.38), Some(99.9)));
    }

    #[test]
    fn online_frame_split_across_chunks() {
        let stream: Vec<u8> = [&[0x04, 0x21][..], b"f", b"95,4\0", b" 0,4\0", &[0x77], b"99,8\0"].concat();
        let mut p = OnlineParser::new(3);
        let mut got = vec![];
        for chunk in stream.chunks(3) {
            got.extend(p.feed(chunk));
        }
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], ParseEvent::SwitchPort);
        match &got[1] {
            ParseEvent::Reading { values, .. } => {
                assert_eq!(values, &vec![Some(95.4), Some(0.4), Some(99.8)])
            }
            _ => panic!(),
        }
    }

    #[test]
    fn status_decoding() {
        let s = decode_status(&[1, 3, 1, 0, 0, 1, 1, 5, 0x12, 0x01]).unwrap();
        assert!(s.has_transmission() && s.has_haze() && !s.has_clarity());
        assert_eq!(s.firmware, "0112");
        assert!(s.cal_valid && !s.continuous);
    }
}
