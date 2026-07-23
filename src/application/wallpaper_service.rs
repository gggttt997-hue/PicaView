use tracing::{debug, info};

#[derive(Clone)]
pub struct WallpaperService;

impl Default for WallpaperService {
    fn default() -> Self {
        Self::new()
    }
}

impl WallpaperService {
    pub fn new() -> Self {
        Self
    }

    pub async fn set_wallpaper(&self, path: String, mode: String) -> Result<(), String> {
        debug!(
            "[Evidence] WallpaperService::set_wallpaper, Capture: path={}, mode={}",
            path, mode
        );

        if mode == "desktop" {
            wallpaper::set_from_path(&path)
                .map_err(|e| format!("Failed to set desktop wallpaper: {}", e))?;
        } else if mode == "lockscreen" {
            #[cfg(target_os = "linux")]
            {
                let desktop = std::env::var("XDG_CURRENT_DESKTOP")
                    .unwrap_or_default()
                    .to_lowercase();
                if desktop.contains("gnome") || desktop.contains("unity") {
                    let file_uri = format!("file://{}", path);
                    let status = std::process::Command::new("gsettings")
                        .args([
                            "set",
                            "org.gnome.desktop.screensaver",
                            "picture-uri",
                            &file_uri,
                        ])
                        .status()
                        .map_err(|e| format!("Failed to call gsettings: {}", e))?;

                    if !status.success() {
                        return Err("gsettings command returned a non-zero exit code".into());
                    }
                } else {
                    return Err(format!(
                        "Current Linux desktop environment ({}) does not support setting lockscreen wallpaper yet",
                        desktop
                    ));
                }
            }
            #[cfg(target_os = "windows")]
            {
                return Err("Windows lockscreen wallpaper setting is not yet implemented".into());
            }
            #[cfg(not(any(target_os = "linux", target_os = "windows")))]
            {
                return Err("The current operating system does not support setting lockscreen wallpaper yet".into());
            }
        } else {
            return Err(format!("Unsupported wallpaper mode: {}", mode));
        }

        info!("Wallpaper set successfully: {} ({})", path, mode);
        Ok(())
    }
}
