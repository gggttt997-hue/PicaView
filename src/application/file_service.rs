use crate::core::file_manager::perform_directory_scan_blocking;
use crate::core::image_loader::load_image_info;
use crate::models::{ImageInfo, SortCriteria, SortOrder};
use crate::state::AppState;
use crate::utils::image_utils::is_archive_extension;
use crate::utils::path_utils::resolve_input_path;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info, info_span, instrument, warn};
use tracing_futures::Instrument;

#[derive(serde::Serialize, serde::Deserialize, Debug)]
#[serde(tag = "type", content = "payload")]
pub enum OpenTargetResponse {
    Viewer(ImageInfo),
    Gallery(String),
}

#[derive(Clone)]
pub struct FileService {
    state: Arc<AppState>,
}

impl FileService {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    /// Unified Entry Point (UEP): Opens the specified path and returns the intent (Viewer or Gallery)
    #[instrument(skip(self), fields(path = %path))]
    pub async fn open_target(
        &self,
        path: String,
    ) -> std::result::Result<OpenTargetResponse, String> {
        debug!("[Evidence] Location: FileService::open_target, Capture: path={}, Rationale: Received UEP request.", path);

        let input_path = resolve_input_path(&path).map_err(|e| e.to_accurate_frontend_message())?;

        if input_path.is_dir() || is_archive_extension(&input_path) {
            // If it is a directory or an archive, try to perform a scan to update the state
            if is_archive_extension(&input_path) {
                self.open_file(path.clone()).await?;
            } else {
                let _ = self.open_file(path.clone()).await.map_err(|e| {
                    info!("Gallery mode pre-scan hint: {} (This usually doesn't affect entering the gallery)", e);
                    e
                });
            }
            Ok(OpenTargetResponse::Gallery(path))
        } else {
            // If it is a file, execute open_file and return Viewer intent
            let info = self.open_file(path).await?;
            Ok(OpenTargetResponse::Viewer(info))
        }
    }

    /// Opens the specified image file or folder path
    #[instrument(skip(self), fields(path = %path))]
    pub async fn open_file(&self, path: String) -> std::result::Result<ImageInfo, String> {
        info!("Received request to open file/folder: {}", path);

        let input_path =
            { resolve_input_path(&path).map_err(|e| e.to_accurate_frontend_message())? };

        let (file_count_from_scan, scanned_infos, target_index, scanned_directory) = {
            use tokio::task;

            let input_path_clone = input_path.clone();
            task::spawn_blocking(move || perform_directory_scan_blocking(&input_path_clone))
                .instrument(info_span!("scanning_directory_blocking"))
                .await
                .map_err(|e| format!("Directory scan task failed: {}", e))?
                .map_err(|e| e.to_accurate_frontend_message())?
        };

        if file_count_from_scan == 0 {
            let error_msg = if input_path.is_dir() {
                "No supported image files found in the selected folder".to_string()
            } else {
                "No supported image files found in the current directory".to_string()
            };
            warn!("{}", error_msg);
            return Err(error_msg);
        }

        self.state
            .with_file_manager_mut(|manager| {
                manager.update_state(scanned_infos, target_index, scanned_directory);
                debug!("[Evidence] Location: FileService::open_file, after update_state, Capture: criteria={:?}, current_index={}, Rationale: Verify that the backend list re-sorts automatically.", manager.sort_criteria, manager.current_index());
            })
            .map_err(|lock_error| {
                let error_msg = format!("Failed to acquire state lock: {}", lock_error);
                warn!("{}", error_msg);
                "System busy, please try again later".to_string()
            })?;

        // Start background metadata scraper to asynchronously load EXIF data
        let new_scan_id = self.state.next_scan_id();
        self.spawn_lazy_metadata_scraper(new_scan_id);

        let target_image_path = self.get_initial_image_path(&input_path)?;

        let image_info = load_image_info(&target_image_path)
            .instrument(info_span!("loading_image_info", path = %target_image_path.display()))
            .await
            .map_err(|e| {
                let error_msg = e.to_accurate_frontend_message();
                warn!(
                    "Failed to load image: {} - {}",
                    target_image_path.display(),
                    error_msg
                );
                error_msg
            })?;

        info!("Image loaded successfully: {}", image_info.path);

        // Anticipatory neighboring directory (chapter) pre-caching
        let state_clone = self.state.clone();
        tokio::spawn(async move {
            let next_dir = state_clone
                .with_file_manager(|manager| manager.find_neighboring_dir(true).ok().flatten())
                .ok()
                .flatten();

            if let Some(dir) = next_dir {
                debug!(
                    "Anticipatory pre-caching neighboring directory: {}",
                    dir.display()
                );
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = perform_directory_scan_blocking(&dir);
                })
                .await;
            }
        });

        Ok(image_info)
    }

    /// Determine the initial image path to load based on the input path and file manager state.
    fn get_initial_image_path(&self, input_path: &Path) -> Result<PathBuf, String> {
        if input_path.is_file() && !is_archive_extension(input_path) {
            Ok(input_path.to_path_buf())
        } else {
            let current_image = self.state.with_file_manager(|manager| manager.current());
            match current_image {
                Ok(Some(path)) => {
                    info!("Selecting first image from folder: {}", path.display());
                    Ok(path)
                }
                Ok(None) => {
                    let error_msg =
                        "Directory scan succeeded but could not get the first image".to_string();
                    warn!("{}", error_msg);
                    Err(error_msg)
                }
                Err(lock_error) => {
                    let error_msg = format!("Failed to acquire state lock: {}", lock_error);
                    warn!("{}", error_msg);
                    Err("System busy, please try again later".to_string())
                }
            }
        }
    }

    /// Delete the current image file and sync the state
    pub async fn delete_current_file(&self) -> std::result::Result<Option<ImageInfo>, String> {
        info!("Received request to delete current file");

        let current_path = self
            .state
            .with_file_manager(|manager| manager.current())
            .map_err(|e| format!("Failed to access file manager: {}", e))?
            .ok_or_else(|| "No image is currently being viewed".to_string())?;

        trash::delete(&current_path).map_err(|e| format!("Failed to delete file: {}", e))?;

        let filename = current_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| "Failed to get filename".to_string())?;

        self.state
            .with_file_manager_mut(|manager| {
                manager.remove_filename(filename);
            })
            .map_err(|e| format!("Failed to update in-memory state: {}", e))?;

        let next_path = self
            .state
            .with_file_manager(|manager| manager.current())
            .map_err(|e| format!("Failed to get next image path: {}", e))?;

        match next_path {
            Some(path) => {
                let info = load_image_info(&path)
                    .await
                    .map_err(|e| e.to_accurate_frontend_message())?;
                Ok(Some(info))
            }
            None => Ok(None),
        }
    }

    /// Set sorting configuration and perform re-sorting
    pub async fn set_sort_config(
        &self,
        criteria: SortCriteria,
        order: SortOrder,
    ) -> std::result::Result<Vec<String>, String> {
        debug!("[Evidence] Location: FileService::set_sort_config, Capture: criteria={:?}, order={:?}, Rationale: Received sorting request.", criteria, order);

        self.state.with_file_manager_mut(|manager| {
            manager.re_sort(criteria, order);
            let paths = manager.get_all_image_paths();
            debug!("[Evidence] Location: End of FileService::set_sort_config, Capture: count={}, Rationale: Ensure persistent state modified.", paths.len());
            paths
        }).map_err(|e| format!("Sorting operation failed: {}", e))
    }

    /// Spawn background task to scan and extract metadata incrementally
    fn spawn_lazy_metadata_scraper(&self, scan_id: u64) {
        let state = self.state.clone();
        tokio::spawn(async move {
            let paths: Vec<String> = {
                if let Ok(manager) = state.file_manager.lock() {
                    manager
                        .get_all_infos()
                        .iter()
                        .map(|info| info.path.clone())
                        .collect()
                } else {
                    return;
                }
            };

            debug!(
                "Background metadata scraper started (scan_id = {}, items = {})",
                scan_id,
                paths.len()
            );

            for path_str in paths {
                // If a new directory scan has been triggered, abort this task immediately
                if state.get_scan_id() != scan_id {
                    debug!(
                        "Background metadata scraper aborted (scan_id = {})",
                        scan_id
                    );
                    return;
                }

                let path_buf = std::path::PathBuf::from(&path_str);

                // 1. 在后台读取文件的基本元数据 (对滑动窗口外图片的补充)
                let mut size = None;
                let mut modified_ms = None;
                if let Ok(meta) = std::fs::metadata(&path_buf) {
                    size = Some(meta.len());
                    modified_ms = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as u64);
                }

                // 2. 提取 EXIF 并同步到主线程的 FileManager
                let exif = crate::core::image_loader::extract_exif(&path_buf);

                let state_clone = state.clone();
                let path_clone = path_str.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Ok(mut manager) = state_clone.file_manager.lock() {
                        let (camera, focal, aperture, iso) = match exif {
                            Some((c, f, a, i)) => (c, f, a, i),
                            None => (None, None, None, None),
                        };
                        manager.supplement_metadata_and_exif(
                            &path_clone,
                            size,
                            modified_ms,
                            camera,
                            focal,
                            aperture,
                            iso,
                        );
                    }
                });

                // Sleep to avoid high CPU and I/O load
                tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
            }

            debug!(
                "Background metadata scraper finished (scan_id = {})",
                scan_id
            );

            // ⚡ 当后台将大目录的所有图片完整采集完后，自动向磁盘写入一次包含完整 EXIF 倒排索引的高级快照
            let (dir_opt, infos_opt): (Option<PathBuf>, Option<Vec<ImageInfo>>) = {
                if let Ok(manager) = state.file_manager.lock() {
                    (
                        manager.current_directory().cloned(),
                        Some(manager.get_all_infos().to_vec()),
                    )
                } else {
                    (None, None)
                }
            };

            if let (Some(directory), Some(infos)) = (dir_opt, infos_opt) {
                let cache_dir = crate::utils::path_utils::get_app_cache_dir();
                let current_mtime = std::fs::metadata(&directory)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::now());
                let _ = crate::core::cache_manager::CacheManager::save_directory_snapshot(
                    &cache_dir,
                    &directory,
                    current_mtime,
                    &infos,
                );
                debug!("Full Directory Snapshot saved for: {}", directory.display());
            }
        });
    }
}
