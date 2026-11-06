//! Cấu hình lưu trong hazegard_config.json (cùng định dạng với bản Python).
//! Password đổi cài đặt: chỉ lưu mã băm SHA-256, không lưu chữ rõ.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// SHA-256 của password xác nhận khi lưu cài đặt.
pub const SETTINGS_PW_SHA256: &str = "f51413286fbac40dfc6e9c3757427acf784c86fd80b749f1ccb5d23de51414df";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub port: String,
    pub line: String,
    /// "" = thư mục chứa chương trình
    pub outdir: String,
    pub t_offset: f64,
    pub h_offset: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self { port: String::new(), line: "VGI3".into(), outdir: String::new(), t_offset: 0.0, h_offset: 0.0 }
    }
}

impl Config {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let s = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, s)
    }
    /// Thư mục lưu CSV thực tế; rỗng -> thư mục chương trình.
    pub fn outdir_or(&self, app_dir: &Path) -> PathBuf {
        let o = self.outdir.trim();
        if o.is_empty() { app_dir.to_path_buf() } else { PathBuf::from(o) }
    }
}

pub fn password_ok(input: &str) -> bool {
    let h = Sha256::digest(input.as_bytes());
    let hex: String = h.iter().map(|b| format!("{:02x}", b)).collect();
    hex == SETTINGS_PW_SHA256
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password() {
        assert!(password_ok("ZZCARROT"));
        assert!(!password_ok("zzcarrot") && !password_ok("") && !password_ok("sai"));
    }

    #[test]
    fn reads_python_config() {
        let c: Config = serde_json::from_str(r#"{"port":"COM3","line":"VGI1","outdir":"","t_offset":-0.6,"h_offset":0}"#).unwrap();
        assert_eq!((c.port.as_str(), c.t_offset, c.h_offset), ("COM3", -0.6, 0.0));
        let c2: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(c2.line, "VGI3");
    }
}
