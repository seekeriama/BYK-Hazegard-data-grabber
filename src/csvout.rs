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

#[derive(Debug, Clone)]
pub struct DailyCsv {
    pub outdir: PathBuf,
}

impl DailyCsv {
    pub fn new(outdir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let outdir = outdir.into();
        std::fs::create_dir_all(&outdir)?;
        Ok(Self { outdir })
    }

    pub fn path_for(&self, dt: &NaiveDateTime) -> PathBuf {
        self.outdir.join(format!("hazegarddata{}.csv", dt.format("%Y%m%d")))
    }

    pub fn write(&self, dt: &NaiveDateTime, t: Option<f64>, h: Option<f64>, e: &RowInfo) -> std::io::Result<PathBuf> {
        let path = self.path_for(dt);
        let is_new = std::fs::metadata(&path).map(|m| m.len() == 0).unwrap_or(true);
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
        let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut buf = String::new();
        if is_new {
            buf.push_str(CSV_HEADER);
            buf.push_str("\r\n");
        }
        buf.push_str(&cells.join(","));
        buf.push_str("\r\n");
        f.write_all(buf.as_bytes())?;
        Ok(path)
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
        let s2 = std::fs::read_to_string(&p2).unwrap();
        assert!(s2.contains("\"\",06/11/2026 00:00:05,\"\",\"\",\"\",100.0,0.00,\"\",\"\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn offset_format() {
        assert_eq!((fmt_offset(-0.6), fmt_offset(0.0), fmt_offset(-10.0)), ("-0.6".into(), "0".into(), "-10".into()));
    }
}
