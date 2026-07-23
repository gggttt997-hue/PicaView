use crate::application::asset_browser_service::AssetBrowserService;
use crate::application::file_service::FileService;
use crate::application::history_service::HistoryService;
use crate::application::navigation_service::NavigationService;
use crate::application::protocol::{AppCommand, AppEvent, AppResponse, EventEmitter};
use crate::application::slideshow_service::SlideshowService;
use crate::application::startup_service::StartupService;
use crate::application::thumbnail_service::ThumbnailService;
use crate::application::wallpaper_service::WallpaperService;
use crate::application::window_service::WindowService;
use crate::application::workshop_service::WorkshopService;
use crate::state::AppState;
use std::sync::Arc;

#[derive(Clone)]
pub struct Runtime {
    pub state: Arc<AppState>,
    pub file_service: FileService,
    pub navigation_service: NavigationService,
    pub history_service: HistoryService,
    pub startup_service: StartupService,
    pub thumbnail_service: ThumbnailService,
    pub workshop_service: WorkshopService,
    pub slideshow_service: SlideshowService,
    pub wallpaper_service: WallpaperService,
    pub asset_browser_service: AssetBrowserService,
    pub window_service: Arc<std::sync::RwLock<Option<Arc<dyn WindowService>>>>,
}

impl Runtime {
    pub fn init() -> Self {
        let state = Arc::new(AppState::new());
        Self::new(state)
    }

    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state: state.clone(),
            file_service: FileService::new(state.clone()),
            navigation_service: NavigationService::new(state.clone()),
            history_service: HistoryService::new(state.clone()),
            startup_service: StartupService::new(state.clone()),
            thumbnail_service: ThumbnailService::new(state.clone()),
            workshop_service: WorkshopService::new(state.clone()),
            slideshow_service: SlideshowService::new(state.clone()),
            wallpaper_service: WallpaperService::new(),
            asset_browser_service: AssetBrowserService::new(),
            window_service: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    pub fn set_window_service(&self, service: Arc<dyn WindowService>) {
        *self.window_service.write().unwrap() = Some(service);
    }

    /// Unified Dispatcher: Routes AppCommand to the appropriate service
    /// Unified Dispatcher: Routes AppCommand to the appropriate service
    pub async fn dispatch(
        &self,
        command: AppCommand,
        emitter: Option<Arc<dyn EventEmitter>>,
    ) -> AppResponse {
        match command {
            AppCommand::OpenTarget { path } => match self.file_service.open_target(path).await {
                Ok(res) => AppResponse::OpenTarget(res),
                Err(e) => AppResponse::Error(e),
            },
            AppCommand::OpenFile { path } => match self.file_service.open_file(path).await {
                Ok(info) => AppResponse::ImageInfo(info),
                Err(e) => AppResponse::Error(e),
            },
            AppCommand::DeleteCurrentFile => match self.file_service.delete_current_file().await {
                Ok(info) => AppResponse::ImageInfoOption(info),
                Err(e) => AppResponse::Error(e),
            },
            AppCommand::GetNextImage => match self.navigation_service.get_next_image().await {
                Ok(info) => AppResponse::ImageInfoOption(info),
                Err(e) => AppResponse::Error(e),
            },
            AppCommand::GetPrevImage => match self.navigation_service.get_prev_image().await {
                Ok(info) => AppResponse::ImageInfoOption(info),
                Err(e) => AppResponse::Error(e),
            },
            AppCommand::GetHistory => match self.history_service.get_history().await {
                Ok(list) => AppResponse::StringList(list),
                Err(e) => AppResponse::Error(e),
            },
            AppCommand::AddToHistory { path } => {
                match self.history_service.add_to_history(path).await {
                    Ok(_) => AppResponse::Success,
                    Err(e) => AppResponse::Error(e),
                }
            }
            AppCommand::OpenFileDialog { multiple } => {
                tracing::info!("Triggering OpenFileDialog (multiple={})", multiple);
                let dialog = rfd::FileDialog::new().add_filter(
                    "Supported Files",
                    &[
                        "jpg", "jpeg", "png", "bmp", "gif", "webp", "tif", "tiff", "zip", "rar",
                        "7z", "cbr", "cbz", "cb7",
                    ],
                );

                let res = tokio::task::spawn_blocking(move || {
                    if multiple {
                        let files = dialog.pick_files();
                        tracing::info!("OpenFileDialog returned: {:?}", files);
                        match files {
                            Some(paths) => {
                                let paths_str: Vec<String> = paths
                                    .iter()
                                    .map(|p| p.to_string_lossy().to_string())
                                    .collect();
                                AppResponse::Json(serde_json::to_value(paths_str).unwrap())
                            }
                            None => AppResponse::Json(serde_json::Value::Null),
                        }
                    } else {
                        let file = dialog.pick_file();
                        tracing::info!("OpenFileDialog returned: {:?}", file);
                        match file {
                            Some(path) => AppResponse::Json(serde_json::Value::String(
                                path.to_string_lossy().to_string(),
                            )),
                            None => AppResponse::Json(serde_json::Value::Null),
                        }
                    }
                })
                .await
                .unwrap_or(AppResponse::Error("Dialog panicked".to_string()));
                res
            }
            AppCommand::OpenFolderDialog => {
                tracing::info!("Triggering OpenFolderDialog");
                let res = tokio::task::spawn_blocking(move || {
                    let folder = rfd::FileDialog::new().pick_folder();
                    tracing::info!("OpenFolderDialog returned: {:?}", folder);
                    match folder {
                        Some(path) => {
                            AppResponse::StringOption(Some(path.to_string_lossy().to_string()))
                        }
                        None => AppResponse::StringOption(None),
                    }
                })
                .await
                .unwrap_or(AppResponse::Error("Dialog panicked".to_string()));
                res
            }
            AppCommand::SaveFileDialog { title } => {
                let res = tokio::task::spawn_blocking(move || {
                    let mut dialog = rfd::FileDialog::new();
                    if let Some(t) = title {
                        dialog = dialog.set_title(t);
                    }
                    let file = dialog.save_file();
                    match file {
                        Some(path) => {
                            AppResponse::StringOption(Some(path.to_string_lossy().to_string()))
                        }
                        None => AppResponse::StringOption(None),
                    }
                })
                .await
                .unwrap_or(AppResponse::Error("Dialog panicked".to_string()));
                res
            }
            AppCommand::CloseWindow => {
                cleanup_temp_extracts();
                if let Some(ws) = self.window_service.read().unwrap().as_ref() {
                    ws.close();
                }
                AppResponse::Success
            }
            AppCommand::MinimizeWindow => {
                if let Some(ws) = self.window_service.read().unwrap().as_ref() {
                    ws.minimize();
                }
                AppResponse::Success
            }
            AppCommand::ToggleMaximize => {
                if let Some(ws) = self.window_service.read().unwrap().as_ref() {
                    ws.toggle_maximize();
                    AppResponse::Boolean(ws.get_state().is_maximized)
                } else {
                    AppResponse::Boolean(false)
                }
            }
            AppCommand::GetWindowState => {
                if let Some(ws) = self.window_service.read().unwrap().as_ref() {
                    AppResponse::WindowState(ws.get_state())
                } else {
                    AppResponse::WindowState(crate::models::WindowState::default())
                }
            }
            AppCommand::RevealInExplorer { path } => {
                match crate::utils::path_utils::reveal_in_explorer_native(&path) {
                    Ok(_) => AppResponse::Success,
                    Err(e) => AppResponse::Error(e),
                }
            }
            AppCommand::CopyImageToClipboard { path } => {
                match crate::utils::path_utils::copy_image_to_clipboard_native(&path) {
                    Ok(_) => AppResponse::Success,
                    Err(e) => AppResponse::Error(e),
                }
            }
            AppCommand::CopyFileToClipboard { path } => {
                let res =
                    tokio::task::spawn_blocking(move || match extract_to_temp_if_virtual(&path) {
                        Ok(physical_path) => {
                            match crate::utils::path_utils::copy_file_to_clipboard_native(
                                &physical_path,
                            ) {
                                Ok(_) => AppResponse::Success,
                                Err(e) => AppResponse::Error(e),
                            }
                        }
                        Err(e) => AppResponse::Error(e),
                    })
                    .await
                    .unwrap_or(AppResponse::Error("Task panicked".to_string()));
                res
            }
            AppCommand::SaveImageAs { path } => {
                let res =
                    tokio::task::spawn_blocking(move || match extract_to_temp_if_virtual(&path) {
                        Ok(_temp_path) => {
                            let mut dialog = rfd::FileDialog::new();
                            let ext = std::path::Path::new(&_temp_path)
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("jpg");
                            dialog = dialog.add_filter("Image", &[ext]);
                            if let Some(save_path) = dialog.save_file() {
                                match std::fs::copy(&_temp_path, &save_path) {
                                    Ok(_) => AppResponse::Success,
                                    Err(e) => AppResponse::Error(e.to_string()),
                                }
                            } else {
                                AppResponse::Success
                            }
                        }
                        Err(e) => AppResponse::Error(e),
                    })
                    .await
                    .unwrap_or(AppResponse::Error("Task panicked".to_string()));
                res
            }
            AppCommand::OpenInExternalEditor { path } => {
                let res =
                    tokio::task::spawn_blocking(move || match extract_to_temp_if_virtual(&path) {
                        Ok(_temp_path) => {
                            #[cfg(target_os = "windows")]
                            {
                                match std::process::Command::new("cmd")
                                    .args(["/c", "start", "", &_temp_path])
                                    .spawn()
                                {
                                    Ok(_) => AppResponse::Success,
                                    Err(e) => AppResponse::Error(e.to_string()),
                                }
                            }
                            #[cfg(not(target_os = "windows"))]
                            {
                                AppResponse::Error("Not supported on this OS".into())
                            }
                        }
                        Err(e) => AppResponse::Error(e),
                    })
                    .await
                    .unwrap_or(AppResponse::Error("Task panicked".to_string()));
                res
            }
            AppCommand::WriteToClipboard { text } => {
                use arboard::Clipboard;
                match Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
                    Ok(_) => AppResponse::Success,
                    Err(e) => AppResponse::Error(e.to_string()),
                }
            }
            AppCommand::SetMangaMode { enabled } => {
                match self.navigation_service.set_manga_mode(enabled) {
                    Ok(_) => AppResponse::Success,
                    Err(e) => AppResponse::Error(e),
                }
            }
            AppCommand::SetSortConfig { criteria, order } => {
                use crate::models::{SortCriteria, SortOrder};
                let crit = match criteria.as_str() {
                    "Name" => SortCriteria::Name,
                    "Size" => SortCriteria::Size,
                    "Date" => SortCriteria::Date,
                    _ => SortCriteria::Name,
                };
                let ord = match order.as_str() {
                    "Ascending" => SortOrder::Ascending,
                    "Descending" => SortOrder::Descending,
                    _ => SortOrder::Ascending,
                };
                match self.file_service.set_sort_config(crit, ord).await {
                    Ok(paths) => AppResponse::StringList(paths),
                    Err(e) => AppResponse::Error(e),
                }
            }
            AppCommand::RemoveFromHistory { path } => {
                match self.history_service.remove_from_history(path).await {
                    Ok(_) => AppResponse::Success,
                    Err(e) => AppResponse::Error(e),
                }
            }
            AppCommand::ClearHistory => match self.history_service.clear_history().await {
                Ok(_) => AppResponse::Success,
                Err(e) => AppResponse::Error(e),
            },
            AppCommand::SetWallpaper { path, mode: _mode } => {
                let temp_path;
                let target_path = if let Some((archive_path, entry_name)) =
                    crate::core::image_loader::parse_archive_path(&path)
                {
                    match crate::core::image_loader::load_archive_image_bytes(
                        archive_path,
                        entry_name,
                    ) {
                        Ok(bytes) => {
                            let temp_dir = std::env::temp_dir();
                            let ext = std::path::Path::new(entry_name)
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("jpg");
                            let filename = format!("picaview_wallpaper.{}", ext);
                            let path_buf = temp_dir.join(filename);
                            match std::fs::write(&path_buf, bytes) {
                                Ok(_) => {
                                    temp_path = path_buf.to_string_lossy().to_string();
                                    &temp_path
                                }
                                Err(e) => {
                                    return AppResponse::Error(format!(
                                        "Failed to write temp wallpaper file: {}",
                                        e
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            return AppResponse::Error(format!(
                                "Failed to extract wallpaper from archive: {}",
                                e
                            ));
                        }
                    }
                } else {
                    &path
                };

                match wallpaper::set_from_path(target_path) {
                    Ok(_) => AppResponse::Success,
                    Err(e) => AppResponse::Error(e.to_string()),
                }
            }
            AppCommand::ApplyShellIntegration => {
                let executable_path = match std::env::current_exe() {
                    Ok(path) => path,
                    Err(e) => {
                        return AppResponse::Error(format!(
                            "Failed to get current executable path: {}",
                            e
                        ))
                    }
                };

                let (reg, desktop) =
                    match crate::utils::shell_utils::generate_shell_integration_scripts(
                        executable_path,
                    ) {
                        Ok(res) => res,
                        Err(e) => return AppResponse::Error(e),
                    };

                let config_dir = crate::utils::path_utils::get_app_config_dir();
                if !config_dir.exists() {
                    if let Err(e) = std::fs::create_dir_all(&config_dir) {
                        return AppResponse::Error(format!(
                            "Failed to create config directory: {}",
                            e
                        ));
                    }
                }

                let reg_path = config_dir.join("picaview_context_menu.reg");
                let desktop_path = config_dir.join("picaview.desktop");

                // Windows .reg files must be encoded in UTF-16 LE with a BOM header to avoid garbled text
                let reg_content_utf16 = {
                    let utf16: Vec<u16> = reg.encode_utf16().collect();
                    let mut bytes = vec![0xFF, 0xFE]; // BOM header
                    for &u in &utf16 {
                        bytes.extend_from_slice(&u.to_le_bytes());
                    }
                    bytes
                };

                if let Err(e) = std::fs::write(&reg_path, reg_content_utf16) {
                    return AppResponse::Error(format!("Failed to write .reg file: {}", e));
                }

                if let Err(e) = std::fs::write(&desktop_path, &desktop) {
                    return AppResponse::Error(format!("Failed to write .desktop file: {}", e));
                }

                #[cfg(target_os = "windows")]
                {
                    use std::process::Command;
                    tracing::info!("Applying registry script: {}", reg_path.display());
                    let output = match Command::new("reg").arg("import").arg(&reg_path).output() {
                        Ok(out) => out,
                        Err(e) => {
                            return AppResponse::Error(format!(
                                "Failed to execute reg import: {}",
                                e
                            ))
                        }
                    };

                    if !output.status.success() {
                        let err = String::from_utf8_lossy(&output.stderr);
                        return AppResponse::Error(format!("Registry import failed: {}", err));
                    }
                    tracing::info!("Registry script applied successfully");
                }

                AppResponse::Success
            }
            AppCommand::ToggleSlideshow { interval_secs } => {
                let emitter_clone = emitter.clone();
                match self
                    .slideshow_service
                    .toggle_slideshow(interval_secs, move || {
                        if let Some(e) = &emitter_clone {
                            let _ = e.emit(AppEvent::SlideshowNextImage);
                        }
                    })
                    .await
                {
                    Ok(playing) => AppResponse::Boolean(playing),
                    Err(e) => AppResponse::Error(e),
                }
            }
            AppCommand::GetAllImagePaths => {
                let paths = self
                    .state
                    .with_file_manager(|manager| manager.get_all_image_paths())
                    .unwrap_or_default();
                AppResponse::StringList(paths)
            }
            AppCommand::TriggerRecursiveScan { path, depth } => {
                let depth_opt = depth.map(|d| d as usize);
                let _ = self
                    .asset_browser_service
                    .trigger_recursive_scan(
                        path,
                        depth_opt,
                        |_| {}, // on_batch
                        || {},  // on_complete
                    )
                    .await;
                AppResponse::Success
            }
            AppCommand::GetThumbnailData { path, size } => {
                let cache_dir = crate::utils::path_utils::get_app_cache_dir();
                match self
                    .thumbnail_service
                    .get_thumbnail_data(path, size, cache_dir)
                    .await
                {
                    Ok(info) => AppResponse::ImageInfo(info),
                    Err(e) => AppResponse::Error(e),
                }
            }
            AppCommand::CancelThumbnail { path, size } => {
                match self.thumbnail_service.cancel_thumbnail(path, size).await {
                    Ok(res) => AppResponse::Boolean(res),
                    Err(e) => AppResponse::Error(e),
                }
            }
            AppCommand::GetStartupMetadata => {
                let args = self.startup_service.get_args();
                let meta = self.startup_service.get_metadata(args);
                AppResponse::StartupMetadata(meta)
            }
            AppCommand::GetStartupArgs => {
                let args = self.startup_service.get_args();
                AppResponse::StringList(args)
            }
            AppCommand::PrepareWorkshopAsset { path } => {
                match self.workshop_service.prepare_workshop_asset(path).await {
                    Ok(meta) => AppResponse::Json(serde_json::to_value(meta).unwrap()),
                    Err(e) => AppResponse::Error(e),
                }
            }
            AppCommand::ClearWorkshopCache => {
                match self.workshop_service.clear_workshop_cache().await {
                    Ok(_) => AppResponse::Success,
                    Err(e) => AppResponse::Error(e),
                }
            }
            AppCommand::BatchConvert {
                input_paths,
                target_format,
                output_dir,
            } => {
                use crate::core::image_processor::OutputFormat;
                let format = match target_format.to_lowercase().as_str() {
                    "jpg" | "jpeg" => OutputFormat::Jpeg,
                    "png" => OutputFormat::Png,
                    "webp" => OutputFormat::WebP,
                    "tiff" => OutputFormat::Tiff,
                    "bmp" => OutputFormat::Bmp,
                    _ => {
                        return AppResponse::Error(format!("Unsupported format: {}", target_format))
                    }
                };

                let on_progress = |_current, _total| {};

                match self
                    .workshop_service
                    .batch_convert(input_paths, format, output_dir, on_progress)
                    .await
                {
                    Ok(paths) => AppResponse::StringList(paths),
                    Err(e) => AppResponse::Error(e),
                }
            }
            AppCommand::SynthesizeImages {
                input_paths,
                config,
                output_path,
            } => {
                use crate::core::image_processor::StitchConfig;
                let stitch_config: StitchConfig = match serde_json::from_value(config) {
                    Ok(c) => c,
                    Err(e) => return AppResponse::Error(format!("Invalid stitch config: {}", e)),
                };

                match self
                    .workshop_service
                    .synthesize_images(input_paths, stitch_config, output_path)
                    .await
                {
                    Ok(path) => AppResponse::Json(serde_json::Value::String(path)),
                    Err(e) => AppResponse::Error(e),
                }
            }
        }
    }
}

fn extract_to_temp_if_virtual(path: &str) -> Result<String, String> {
    if let Some((archive_path, entry_name)) = crate::core::image_loader::parse_archive_path(path) {
        let bytes = crate::core::image_loader::load_archive_image_bytes(archive_path, entry_name)
            .map_err(|e| format!("Failed to extract from archive: {}", e))?;

        let temp_dir = std::env::temp_dir().join("PicaView_Extracts");
        if !temp_dir.exists() {
            let _ = std::fs::create_dir_all(&temp_dir);
        }

        let safe_name = entry_name.replace(['/', '\\'], "_");
        let pid = std::process::id();
        let filename = format!("{}_{}", pid, safe_name);
        let path_buf = temp_dir.join(&filename);

        std::fs::write(&path_buf, bytes)
            .map_err(|e| format!("Failed to write temp file: {}", e))?;
        Ok(path_buf.to_string_lossy().to_string())
    } else {
        Ok(path.to_string())
    }
}

fn cleanup_temp_extracts() {
    let temp_dir = std::env::temp_dir().join("PicaView_Extracts");
    if temp_dir.exists() {
        let pid_prefix = format!("{}_", std::process::id());
        if let Ok(entries) = std::fs::read_dir(temp_dir) {
            for entry in entries.flatten() {
                if let Ok(name) = entry.file_name().into_string() {
                    if name.starts_with(&pid_prefix) {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
}
