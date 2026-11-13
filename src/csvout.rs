//! Ghi kết quả: mỗi ngày 1 file hazegarddata<yyyymmdd>.csv, chỉ chứa dữ liệu của ngày đó.
//! Cột: Line, Measurement time, RunNumber, RunMinute, Position, CTransmission, CHaze,
//!      TOffsetApplied, HOffsetApplied

use chrono::NaiveDateTime;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const CSV_HEADER: &str = "Line,Measurement time,RunNumber,RunMinute,Position,CTransmission,CHaze,TOffsetApplied,HOffsetApplied";

#[derive(Debug, Clone, Default)]
pub struct RowInfo {
    pub line: String,
    pub run_number: String,
    pub run_minute: String,
    pub position: String,
    pub t_offset: String,
    pub h_offset: String,
}

fn dedup_systems(v: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for s in v {
        let c = clean_system_name(&s);
        if !out.contains(&c) {
            out.push(c);
        }
    }
    if out.is_empty() { vec![String::new()] } else { out }
}

/// Giống `%g` của Python: -0.6 -> "-0.6", 0 -> "0", -10 -> "-10"
pub fn fmt_offset(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

fn fmt_num(v: Option<f64>, nd: usize) -> String {
    v.map(|x| format!("{:.*}", nd, x)).unwrap_or_default()
}

/// Ô trống ghi là "" để Excel/LibreOffice không gộp các dấu phẩy liền nhau.
fn cell(s: &str) -> String {
    if s.is_empty() { "\"\"".to_string() } else { s.to_string() }
}

/// Tên hệ thống dùng trong tên file: bỏ ký tự không hợp lệ, khoảng trắng -> `_`.
pub fn clean_system_name(name: &str) -> String {
    let t: String = name.chars().filter(|c| !matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()).collect();
    t.split_whitespace().collect::<Vec<_>>().join("_")
}

#[derive(Debug, Clone)]
pub struct DailyCsv {
    pub outdir: PathBuf,
    /// "" -> hazegarddata<yyyymmdd>.csv ; "ERS" -> "Hazegard Data ERS <yyyymmdd>.csv" (có dấu cách).
    /// Nhiều phần tử -> mỗi lần đo ghi vào TẤT CẢ các file.
    pub systems: Vec<String>,
}

impl DailyCsv {
    pub fn new(outdir: impl Into<PathBuf>) -> std::io::Result<Self> {
        Self::with_systems(outdir, vec![String::new()])
    }

    pub fn with_systems(outdir: impl Into<PathBuf>, systems: Vec<String>) -> std::io::Result<Self> {
        let outdir = outdir.into();
        std::fs::create_dir_all(&outdir)?;
        Ok(Self { outdir, systems: dedup_systems(systems) })
    }

    pub fn paths_for(&self, dt: &NaiveDateTime) -> Vec<PathBuf> {
        self.systems
            .iter()
            .map(|s| {
                if s.is_empty() {
                    self.outdir.join(format!("hazegarddata{}.csv", dt.format("%Y%m%d")))
                } else {
                    self.outdir.join(format!("Hazegard Data {} {}.csv", s, dt.format("%Y%m%d")))
                }
            })
            .collect()
    }

    pub fn path_for(&self, dt: &NaiveDateTime) -> PathBuf {
        self.paths_for(dt).remove(0)
    }

    /// Ghi 1 dòng vào 1 file cụ thể (tạo file + tiêu đề nếu chưa có).
    pub fn write_to(&self, path: &Path, dt: &NaiveDateTime, t: Option<f64>, h: Option<f64>, e: &RowInfo) -> std::io::Result<()> {
        let is_new = std::fs::metadata(path).map(|m| m.len() == 0).unwrap_or(true);
        let cells = [
            cell(&e.line),
            dt.format("%d/%m/%Y %H:%M:%S").to_string(),
            cell(&e.run_number),
            cell(&e.run_minute), // ghi dạng số, vd 540
            cell(&e.position),
            cell(&fmt_num(t, 1)),
            cell(&fmt_num(h, 2)),
            cell(&e.t_offset),
            cell(&e.h_offset),
        ];
        let mut f = OpenOptions::new().create(true).append(true).open(path)?;
        let mut buf = String::new();
        if is_new {
            buf.push_str(CSV_HEADER);
            buf.push_str("\r\n");
        }
        buf.push_str(&cells.join(","));
        buf.push_str("\r\n");
        f.write_all(buf.as_bytes())
    }

    /// Ghi vào tất cả các file đã chọn; file lỗi (đang mở trong Excel) không chặn file khác, lỗi đầu tiên được trả về.
    pub fn write(&self, dt: &NaiveDateTime, t: Option<f64>, h: Option<f64>, e: &RowInfo) -> std::io::Result<PathBuf> {
        let mut first_err = None;
        for p in self.paths_for(dt) {
            if let Err(er) = self.write_to(&p, dt, t, h, e) {
                first_err.get_or_insert(er);
            }
        }
        match first_err {
            Some(er) => Err(er),
            None => Ok(self.path_for(dt)),
        }
    }

    /// Kiểm tra thư mục ghi được (tạo rồi xoá file thử).
    pub fn writable(dir: &Path) -> bool {
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
        let test = dir.join(".hazegard_write_test");
        let ok = std::fs::File::create(&test).is_ok();
        let _ = std::fs::remove_file(&test);
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn rows_match_python_output() {
        let dir = std::env::temp_dir().join(format!("hz_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = DailyCsv::new(&dir).unwrap();
        let d = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let info = RowInfo {
            line: "VGI1".into(), run_number: "1852".into(), run_minute: "100".into(),
            position: "LE".into(), t_offset: fmt_offset(-0.6), h_offset: fmt_offset(0.0),
        };
        let p = store.write(&d.and_hms_opt(14, 33, 32).unwrap(), Some(88.6), Some(0.05), &info).unwrap();
        // qua nửa đêm -> file mới của ngày mới
        let p2 = store.write(&d.succ_opt().unwrap().and_hms_opt(0, 0, 5).unwrap(), Some(100.0), Some(0.0), &RowInfo::default()).unwrap();
        let s = std::fs::read_to_string(&p).unwrap();
        assert_eq!(s, format!("{}\r\nVGI1,05/11/2026 14:33:32,1852,100,LE,88.6,0.05,-0.6,0\r\n", CSV_HEADER));
        assert!(p.ends_with("hazegarddata20261105.csv") && p2.ends_with("hazegarddata20261106.csv"));
        // Tên file ERS / Sequoia dùng dấu cách như hệ thống thật
        let all = DailyCsv::with_systems(std::env::temp_dir(), vec!["".into(), "ERS".into(), "Sequoia".into()]).unwrap();
        let names: Vec<String> = all.paths_for(&d.and_hms_opt(14, 33, 32).unwrap()).iter().map(|x| x.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["hazegarddata20261105.csv", "Hazegard Data ERS 20261105.csv", "Hazegard Data Sequoia 20261105.csv"]);
        let s2 = std::fs::read_to_string(&p2).unwrap();
        assert!(s2.contains("\"\",06/11/2026 00:00:05,\"\",\"\",\"\",100.0,0.00,\"\",\"\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn offset_format() {
        assert_eq!((fmt_offset(-0.6), fmt_offset(0.0), fmt_offset(-10.0)), ("-0.6".into(), "0".into(), "-10".into()));
    }
}
