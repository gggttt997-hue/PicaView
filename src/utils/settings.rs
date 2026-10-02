use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub tile_size: u32,             // Default: 512
    pub fallback_size: u32,         // Default: 2048
    pub max_zune_dimension: u32,    // Default: 30000
    pub hud_dismiss_delay_ms: u64,  // Default: 800
    pub max_cache_size_mb: usize,   // Default: 512
    pub mem_limit_fraction: f32,    // Default: 0.125 (1/8)
    pub allow_multi_instance: bool, // Default: false
    // General Preferences & Modes
    pub scroll_mode: String,            // "zoom" | "navigate"
    pub default_view_mode: String,      // "single" | "waterfall" | "manga"
    pub language: String,               // "zh-CN" | "en-US"
    pub remember_window_geometry: bool, // Default: true
    // Window Geometry Memory
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub window_maximized: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            tile_size: 512,
            fallback_size: 2048,
            max_zune_dimension: 30000,
            hud_dismiss_delay_ms: 800,
            max_cache_size_mb: 512,
            mem_limit_fraction: 0.125,
            allow_multi_instance: false,
            scroll_mode: "zoom".to_string(),
            default_view_mode: "single".to_string(),
            language: "zh-CN".to_string(),
            remember_window_geometry: true,
            window_x: None,
            window_y: None,
            window_width: Some(1280),
            window_height: Some(800),
            window_maximized: false,
        }
    }
}

pub fn get_settings_path() -> PathBuf {
    // 1. Smart Portable Probe: Check current executable directory first
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let portable_settings = exe_dir.join("settings.json");
            let portable_flag = exe_dir.join("portable.flag");
            if portable_settings.exists() || portable_flag.exists() {
                return portable_settings;
            }
        }
    }

    // 2. Fallback to standard AppData directory (%APPDATA%\PicaView\settings.json)
    if let Ok(appdata) = std::env::var("APPDATA") {
        let mut path = PathBuf::from(appdata);
        path.push("PicaView");
        // Ensure folder exists
        let _ = fs::create_dir_all(&path);
        path.push("settings.json");
        path
    } else {
        PathBuf::from("settings.json")
    }
}

impl AppSettings {
    pub fn load() -> Self {
        let path = get_settings_path();
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(settings) = serde_json::from_str::<AppSettings>(&content) {
                    return settings;
                }
            }
        }
        // Save default if file doesn't exist or is corrupted
        let default_settings = Self::default();
        let _ = default_settings.save();
        default_settings
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = get_settings_path();
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)
    }
}

static SETTINGS_LOCK: OnceLock<RwLock<AppSettings>> = OnceLock::new();

pub fn get_settings() -> AppSettings {
    SETTINGS_LOCK
        .get_or_init(|| RwLock::new(AppSettings::load()))
        .read()
        .expect("Failed to acquire read lock on settings")
        .clone()
}

pub fn update_settings(new_settings: AppSettings) -> Result<(), std::io::Error> {
    // Save to disk first
    new_settings.save()?;

    // Update memory state
    if let Some(lock) = SETTINGS_LOCK.get() {
        if let Ok(mut writer) = lock.write() {
            *writer = new_settings;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let default_s = AppSettings::default();
        assert_eq!(default_s.tile_size, 512);
        assert_eq!(default_s.fallback_size, 2048);
        assert_eq!(default_s.max_zune_dimension, 30000);
        assert_eq!(default_s.hud_dismiss_delay_ms, 800);
        assert_eq!(default_s.max_cache_size_mb, 512);
        assert_eq!(default_s.mem_limit_fraction, 0.125);
        assert!(!default_s.allow_multi_instance);
        assert_eq!(default_s.scroll_mode, "zoom");
        assert_eq!(default_s.default_view_mode, "single");
        assert_eq!(default_s.language, "zh-CN");
        assert!(default_s.remember_window_geometry);
    }

    #[test]
    fn test_settings_path() {
        let path = get_settings_path();
        assert!(path.to_string_lossy().contains("settings.json"));
    }
}
