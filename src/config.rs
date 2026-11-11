//! Cấu hình + nơi lưu.
//! - Windows: lưu trong Registry (HKEY_CURRENT_USER\Software\Pilkington\HazeGardLogger), KHÔNG tạo file json.
//!   Nếu Registry chưa có mà cạnh chương trình còn hazegard_config.json cũ -> đọc file đó và chuyển vào Registry.
//! - Hệ điều hành khác (Linux khi test): dùng file hazegard_config.json.
//! Password đổi cài đặt: chỉ lưu mã băm SHA-256, không lưu chữ rõ.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// SHA-256 của password xác nhận khi lưu cài đặt.
pub const SETTINGS_PW_SHA256: &str = "f51413286fbac40dfc6e9c3757427acf784c86fd80b749f1ccb5d23de51414df";

/// Định dạng tên file CSV: (khoá, tên hệ thống dùng trong tên file; "" = hazegarddata<yyyymmdd>.csv)
pub const FILE_FORMATS: [(&str, &str); 3] = [("default", ""), ("ERS", "ERS"), ("Sequoia", "Sequoia")];

#[cfg(windows)]
const REG_PATH: &str = "Software\\Pilkington\\HazeGardLogger";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub port: String,
    pub line: String,
    /// "" = thư mục chứa chương trình
    pub outdir: String,
    pub t_offset: f64,
    pub h_offset: f64,
    /// Gửi 50h trước khi handshake 4Fh để giải phóng máy đang kẹt Online Mode (máy plus).
    pub release_first: bool,
    /// "plus" (RS232) hoặc "i" (USB qua BYKCommunication.dll)
    pub model: String,
    /// Chỉ điền nếu máy i nối qua mạng LAN
    pub i_ip: String,
    /// Đường dẫn ảnh nền; "" = ảnh mặc định
    pub bg_image: String,
    /// 1-3 trong: default, ERS, Sequoia
    pub file_formats: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: String::new(),
            line: "VGI3".into(),
            outdir: String::new(),
            t_offset: 0.0,
            h_offset: 0.0,
            release_first: true,
            model: "plus".into(),
            i_ip: String::new(),
            bg_image: String::new(),
            file_formats: vec!["default".into()],
        }
    }
}

/// Loại giá trị lạ/trùng, giữ thứ tự cố định; rỗng -> ["default"].
pub fn normalize_formats(v: &[String]) -> Vec<String> {
    let out: Vec<String> =
        FILE_FORMATS.iter().filter(|(k, _)| v.iter().any(|x| x == k)).map(|(k, _)| k.to_string()).collect();
    if out.is_empty() { vec!["default".into()] } else { out }
}

impl Config {
    /// Đọc từ chuỗi JSON (cả file cấu hình bản cũ): bổ sung giá trị thiếu, đổi `file_system` cũ sang `file_formats`.
    pub fn from_json(s: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(s).ok()?;
        let mut c: Config = serde_json::from_value(v.clone()).ok()?;
        if v.get("file_formats").is_none() {
            if let Some(old) = v.get("file_system").and_then(|x| x.as_str()) {
                let old = old.trim().to_lowercase();
                if let Some((k, _)) = FILE_FORMATS.iter().find(|(k, _)| k.to_lowercase() == old && !old.is_empty()) {
                    c.file_formats = vec![k.to_string()];
                }
            }
        }
        c.normalize();
        Some(c)
    }

    pub fn normalize(&mut self) {
        self.file_formats = normalize_formats(&self.file_formats);
        if self.model != "i" {
            self.model = "plus".into();
        }
    }

    /// Các tên hệ thống tương ứng với lựa chọn định dạng file ("" = Default).
    pub fn systems(&self) -> Vec<String> {
        normalize_formats(&self.file_formats)
            .iter()
            .filter_map(|k| FILE_FORMATS.iter().find(|(x, _)| x == k).map(|(_, s)| s.to_string()))
            .collect()
    }

    /// Thư mục lưu CSV thực tế; rỗng -> thư mục chương trình.
    pub fn outdir_or(&self, app_dir: &Path) -> PathBuf {
        let o = self.outdir.trim();
        if o.is_empty() { app_dir.to_path_buf() } else { PathBuf::from(o) }
    }
}

/// Nơi lưu cấu hình.
#[derive(Debug, Clone)]
pub enum ConfigStore {
    #[cfg(windows)]
    Registry { legacy_json: PathBuf },
    Json(PathBuf),
}

impl ConfigStore {
    /// Windows -> Registry; nơi khác -> file json cạnh chương trình.
    pub fn default_for(app_dir: &Path) -> Self {
        let json = app_dir.join("hazegard_config.json");
        #[cfg(windows)]
        {
            Self::Registry { legacy_json: json }
        }
        #[cfg(not(windows))]
        {
            Self::Json(json)
        }
    }

    /// Đã từng lưu cấu hình chưa (lần đầu chạy -> luôn hỏi password khi bấm Lưu).
    pub fn exists(&self) -> bool {
        match self {
            #[cfg(windows)]
            Self::Registry { .. } => winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(REG_PATH).is_ok(),
            Self::Json(p) => p.exists(),
        }
    }

    pub fn load(&self) -> Config {
        match self {
            #[cfg(windows)]
            Self::Registry { legacy_json } => {
                if let Some(c) = reg_load() {
                    return c;
                }
                // lần đầu trên máy này: chuyển cấu hình json cũ (nếu có) vào Registry
                if let Some(c) = std::fs::read_to_string(legacy_json).ok().and_then(|s| Config::from_json(&s)) {
                    let _ = reg_save(&c);
                    return c;
                }
                Config::default()
            }
            Self::Json(p) => std::fs::read_to_string(p).ok().and_then(|s| Config::from_json(&s)).unwrap_or_default(),
        }
    }

    pub fn save(&self, cfg: &Config) -> std::io::Result<()> {
        match self {
            #[cfg(windows)]
            Self::Registry { .. } => reg_save(cfg),
            Self::Json(p) => {
                let s = serde_json::to_string_pretty(cfg).map_err(std::io::Error::other)?;
                std::fs::write(p, s)
            }
        }
    }
}

#[cfg(windows)]
fn reg_save(c: &Config) -> std::io::Result<()> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(REG_PATH)?;
    let set = |name: &str, v: String| key.set_value(name, &v);
    set("port", c.port.clone())?;
    set("line", c.line.clone())?;
    set("outdir", c.outdir.clone())?;
    set("t_offset", c.t_offset.to_string())?;
    set("h_offset", c.h_offset.to_string())?;
    set("model", c.model.clone())?;
    set("i_ip", c.i_ip.clone())?;
    set("bg_image", c.bg_image.clone())?;
    set("file_formats", c.file_formats.join(","))?;
    key.set_value("release_first", &(c.release_first as u32))?;
    Ok(())
}

#[cfg(windows)]
fn reg_load() -> Option<Config> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(REG_PATH).ok()?;
    let d = Config::default();
    let s = |name: &str, def: &str| key.get_value::<String, _>(name).unwrap_or_else(|_| def.to_string());
    let f = |name: &str, def: f64| key.get_value::<String, _>(name).ok().and_then(|x| x.trim().parse().ok()).unwrap_or(def);
    let mut c = Config {
        port: s("port", &d.port),
        line: s("line", &d.line),
        outdir: s("outdir", &d.outdir),
        t_offset: f("t_offset", d.t_offset),
        h_offset: f("h_offset", d.h_offset),
        release_first: key.get_value::<u32, _>("release_first").map(|v| v != 0).unwrap_or(d.release_first),
        model: s("model", &d.model),
        i_ip: s("i_ip", &d.i_ip),
        bg_image: s("bg_image", &d.bg_image),
        file_formats: s("file_formats", "default").split(',').map(|x| x.trim().to_string()).collect(),
    };
    c.normalize();
    Some(c)
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
        let c = Config::from_json(r#"{"port":"COM3","line":"VGI1","outdir":"","t_offset":-0.6,"h_offset":0}"#).unwrap();
        assert_eq!((c.port.as_str(), c.t_offset, c.h_offset), ("COM3", -0.6, 0.0));
        assert_eq!(c.file_formats, vec!["default"]);
        assert_eq!(c.model, "plus");
        let c2 = Config::from_json("{}").unwrap();
        assert_eq!(c2.line, "VGI3");
    }

    #[test]
    fn file_format_migration_and_normalize() {
        assert_eq!(Config::from_json(r#"{"file_system":"ers"}"#).unwrap().file_formats, vec!["ERS"]);
        assert_eq!(Config::from_json(r#"{"file_system":"khac"}"#).unwrap().file_formats, vec!["default"]);
        let c = Config::from_json(r#"{"file_formats":["Sequoia","default","x"]}"#).unwrap();
        assert_eq!(c.file_formats, vec!["default", "Sequoia"]);
        assert_eq!(Config::from_json(r#"{"file_formats":[]}"#).unwrap().file_formats, vec!["default"]);
        assert_eq!(c.systems(), vec!["", "Sequoia"]);
        assert_eq!(Config::from_json(r#"{"model":"abc"}"#).unwrap().model, "plus");
    }

    #[test]
    fn json_store_roundtrip() {
        let p = std::env::temp_dir().join(format!("hz_store_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let st = ConfigStore::Json(p.clone());
        assert!(!st.exists());
        let c = Config { model: "i".into(), file_formats: vec!["ERS".into(), "Sequoia".into()], t_offset: -0.6, ..Default::default() };
        st.save(&c).unwrap();
        assert!(st.exists());
        assert_eq!(st.load(), c);
        let _ = std::fs::remove_file(p);
    }
}
