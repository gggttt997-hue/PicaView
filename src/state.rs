/// Application state management
///
/// Provides thread-safe application state management, including state sharing for the file manager.
use crate::core::file_manager::FileManager;
use crate::utils::history_manager::HistoryManager;
use lru::LruCache; // Import LruCache
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{debug, info};

// Define thumbnail cache structure
#[derive(Debug)]
pub struct ThumbnailCache {
    inner: LruCache<String, crate::models::ImageInfo>,
    current_weight: usize,
    max_weight: usize,
}

impl ThumbnailCache {
    pub fn new(max_weight: usize) -> Self {
        Self {
            inner: LruCache::unbounded(),
            current_weight: 0,
            max_weight,
        }
    }

    pub fn put(&mut self, key: String, value: crate::models::ImageInfo) {
        let weight = value.pixel_weight();

        // Handle key replacement
        if let Some(old_value) = self.inner.put(key.clone(), value) {
            let old_weight = old_value.pixel_weight();
            self.current_weight = self.current_weight.saturating_sub(old_weight);
        }

        self.current_weight += weight;

        // [Evidence] Location: ThumbnailCache::put
        // [Capture] key, weight, current_weight, max_weight
        // [Rationale] Verify weight-aware insertion logic
        debug!(
            "[Weight Cache] Put: {}, weight: {}, current: {}/{}",
            key, weight, self.current_weight, self.max_weight
        );

        // Eviction logic: If current weight exceeds max weight, pop the least recently used items
        while self.current_weight > self.max_weight && !self.inner.is_empty() {
            if let Some((evicted_key, evicted_value)) = self.inner.pop_lru() {
                let evicted_weight = evicted_value.pixel_weight();
                self.current_weight = self.current_weight.saturating_sub(evicted_weight);

                // [Evidence] Location: ThumbnailCache::put (Eviction)
                // [Capture] evicted_key, evicted_weight, current_weight
                // [Rationale] Monitor cache reclamation facts
                debug!(
                    "[Weight Cache] Evicted: {}, weight: {}, remaining: {}",
                    evicted_key, evicted_weight, self.current_weight
                );
            }
        }
    }

    pub fn get(&mut self, key: &str) -> Option<&crate::models::ImageInfo> {
        self.inner.get(key)
    }

    pub fn current_weight(&self) -> usize {
        self.current_weight
    }

    pub fn set_max_weight(&mut self, max_weight: usize) {
        self.max_weight = max_weight;
        while self.current_weight > self.max_weight && !self.inner.is_empty() {
            if let Some((evicted_key, evicted_value)) = self.inner.pop_lru() {
                let evicted_weight = evicted_value.pixel_weight();
                self.current_weight = self.current_weight.saturating_sub(evicted_weight);
                debug!(
                    "[Weight Cache] Dynamic Evicted: {}, weight: {}, remaining: {}",
                    evicted_key, evicted_weight, self.current_weight
                );
            }
        }
    }
}

// Define preload cache for full-size images
#[derive(Debug)]
pub struct PreloadCache {
    inner: LruCache<String, Arc<image::DynamicImage>>,
}

impl PreloadCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: LruCache::new(std::num::NonZeroUsize::new(capacity).unwrap()),
        }
    }

    pub fn put(&mut self, key: String, value: Arc<image::DynamicImage>) {
        self.inner.put(key, value);
    }

    pub fn get(&mut self, key: &str) -> Option<Arc<image::DynamicImage>> {
        self.inner.get(key).map(Arc::clone)
    }

    pub fn remove(&mut self, key: &str) {
        self.inner.pop(key);
    }
}

/// Application state
///
/// Uses Arc<Mutex<T>> to implement thread-safe state sharing.
/// All Tauri commands can access and modify this state.
#[derive(Debug)]
pub struct AppState {
    /// File manager, uses Mutex for thread safety
    pub file_manager: Arc<Mutex<FileManager>>,
    /// File path at startup, used for handling command-line arguments
    pub startup_file: Arc<Mutex<Option<String>>>,
    /// Handle for the slideshow playback asynchronous task
    pub slideshow_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Interval for slideshow playback (seconds)
    pub slideshow_interval_secs: Arc<Mutex<u64>>,
    /// Thumbnail cache, uses Mutex for thread safety
    pub thumbnail_cache: Arc<Mutex<ThumbnailCache>>,
    /// Preload cache for full-size images
    pub preload_cache: Arc<Mutex<PreloadCache>>,
    /// History manager
    pub history_manager: Arc<Mutex<HistoryManager>>,
    /// Whether manga mode is enabled
    pub manga_mode_enabled: Arc<Mutex<bool>>,
    /// Task coordinator, manages cancellation of backend async tasks (e.g., thumbnail generation)
    pub task_coordinator: Arc<crate::core::task_coordinator::TaskCoordinator>,
    /// Single-flight pending tasks for thumbnail generation
    pub pending_tasks: Arc<Mutex<HashMap<String, broadcast::Receiver<crate::models::ImageInfo>>>>,
    /// Workshop cache hub for high-performance image processing
    pub workshop_cache: Arc<crate::utils::workshop_cache::WorkshopCacheHub>,
    /// Internationalization manager
    pub i18n: Arc<crate::utils::i18n::I18nManager>,
    /// User's original directory path before switching to rating directories via shortcut keys
    pub original_directory: Arc<Mutex<Option<std::path::PathBuf>>>,
    /// Last viewed image path for each folder: Map<folder_path, last_viewed_image_path>
    pub folder_last_viewed: Arc<Mutex<std::collections::HashMap<String, String>>>,
    /// Active image's ICC Profile cache: Option<(image_path, Option<icc_bytes>)>
    pub active_icc_profile: Arc<Mutex<Option<(String, Option<Vec<u8>>)>>>,
    /// Current directory scan session ID, used for task cancellation
    pub current_scan_id: Arc<std::sync::atomic::AtomicU64>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    /// Create a new application state instance
    pub fn new() -> Self {
        info!("Initializing application state");
        let history_manager_capacity = 50;
        // [Evidence] Location: AppState::new
        // [Capture] max_size: history_manager_capacity
        // [Rationale] Ensure initial configuration logic modification is correct
        info!(
            "Setting HistoryManager initial capacity to {}",
            history_manager_capacity
        );

        // Max cache size and initial preferences loaded from AppSettings
        let settings = crate::utils::settings::get_settings();
        let max_cache_weight = settings.max_cache_size_mb * 1024 * 1024;

        let i18n = Arc::new(crate::utils::i18n::I18nManager::new());
        i18n.set_language(&settings.language);

        let is_manga_initial = settings.default_view_mode == "manga";

        let state = Self {
            file_manager: Arc::new(Mutex::new(FileManager::new())),
            startup_file: Arc::new(Mutex::new(None)),
            slideshow_handle: Arc::new(Mutex::new(Option::None)),
            slideshow_interval_secs: Arc::new(Mutex::new(3)), // Default 3 seconds
            thumbnail_cache: Arc::new(Mutex::new(ThumbnailCache::new(max_cache_weight))),
            preload_cache: Arc::new(Mutex::new(PreloadCache::new(3))), // Buffer for 3 full-res images
            history_manager: Arc::new(Mutex::new(HistoryManager::new(history_manager_capacity))), // Default 50 records
            manga_mode_enabled: Arc::new(Mutex::new(is_manga_initial)),
            task_coordinator: Arc::new(crate::core::task_coordinator::TaskCoordinator::new()),
            pending_tasks: Arc::new(Mutex::new(HashMap::new())),
            workshop_cache: Arc::new(crate::utils::workshop_cache::WorkshopCacheHub::new()),
            i18n,
            original_directory: Arc::new(Mutex::new(None)),
            folder_last_viewed: Arc::new(Mutex::new(std::collections::HashMap::new())),
            active_icc_profile: Arc::new(Mutex::new(None)),
            current_scan_id: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        };

        state.load_folder_positions();

        // Set history path (Initialization of path only)
        let config_dir = crate::utils::path_utils::get_app_config_dir();
        let history_path = config_dir.join("history.json");
        state.set_history_path(history_path);

        state
    }

    /// Set history configuration path
    pub fn set_history_path(&self, path: std::path::PathBuf) {
        if let Ok(mut history) = self.history_manager.lock() {
            history.set_config_path(path);
        }
    }

    /// Set startup file path
    pub fn set_startup_file(&self, file_path: String) {
        match self.startup_file.lock() {
            Ok(mut startup_file) => {
                *startup_file = Some(file_path.clone());
                info!("Setting startup file path: {}", file_path);
            }
            Err(e) => {
                tracing::error!("Failed to set startup file path: {}", e);
            }
        }
    }

    /// Get and clear startup file path
    pub fn take_startup_file(&self) -> Option<String> {
        match self.startup_file.lock() {
            Ok(mut startup_file) => {
                let file_path = startup_file.take();
                if let Some(ref path) = file_path {
                    info!("Acquiring startup file path: {}", path);
                }
                file_path
            }
            Err(e) => {
                tracing::error!("Failed to acquire startup file path: {}", e);
                None
            }
        }
    }

    /// Get read-only access to the file manager
    ///
    /// This method provides a convenient way to perform read-only operations.
    ///
    /// # Returns
    /// * `Ok(T)` - Returns the result on success
    /// * `Err(String)` - Returns an error message if the lock acquisition fails
    pub fn with_file_manager<T, F>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&FileManager) -> T,
    {
        match self.file_manager.lock() {
            Ok(manager) => {
                debug!("Successfully acquired read-only lock for file manager");
                Ok(f(&manager))
            }
            Err(e) => {
                let error_msg = format!("Failed to acquire file manager lock: {}", e);
                tracing::error!("{}", error_msg);
                Err(error_msg)
            }
        }
    }

    /// Get write access to the file manager
    ///
    /// This method provides a convenient way to perform modification operations.
    ///
    /// # Returns
    /// * `Ok(T)` - Returns the result on success
    /// * `Err(String)` - Returns an error message if the lock acquisition fails
    pub fn with_file_manager_mut<T, F>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&mut FileManager) -> T,
    {
        match self.file_manager.lock() {
            Ok(mut manager) => {
                debug!("Successfully acquired write lock for file manager");
                Ok(f(&mut manager))
            }
            Err(e) => {
                let error_msg = format!("Failed to acquire file manager lock: {}", e);
                tracing::error!("{}", error_msg);
                Err(error_msg)
            }
        }
    }

    /// Get summary information of the current state
    ///
    /// Used for debugging and monitoring purposes
    pub fn get_status_summary(&self) -> Result<AppStatusSummary, String> {
        self.with_file_manager(|manager| AppStatusSummary {
            total_images: manager.total_count(),
            current_index: manager.current_index(),
            has_images: manager.has_images(),
            current_directory: manager
                .current_directory()
                .map(|p| p.to_string_lossy().to_string()),
        })
    }

    /// 记录文件夹的最后浏览位置，并保存到磁盘
    pub fn record_last_viewed(&self, image_path: &std::path::Path) {
        if let Some(parent) = image_path.parent() {
            let parent_str = parent.to_string_lossy().to_string();
            let image_str = image_path.to_string_lossy().to_string();

            // 规范化路径
            let parent_normalized = crate::utils::path_utils::normalize_native_path(
                std::path::PathBuf::from(parent_str),
            )
            .to_string_lossy()
            .to_string();
            let image_normalized = crate::utils::path_utils::normalize_native_path(
                std::path::PathBuf::from(image_str),
            )
            .to_string_lossy()
            .to_string();

            let mut map = self.folder_last_viewed.lock().unwrap();
            map.insert(parent_normalized, image_normalized);

            // 异步持久化到磁盘以避免卡顿
            let map_clone = map.clone();
            tokio::spawn(async move {
                let config_dir = crate::utils::path_utils::get_app_config_dir();
                let path = config_dir.join("folder_positions.json");
                if let Some(p_parent) = path.parent() {
                    let _ = std::fs::create_dir_all(p_parent);
                }
                if let Ok(content) = serde_json::to_string_pretty(&map_clone) {
                    let _ = std::fs::write(path, content);
                }
            });
        }
    }

    /// 从磁盘加载持久化的文件夹位置数据
    pub fn load_folder_positions(&self) {
        let config_dir = crate::utils::path_utils::get_app_config_dir();
        let path = config_dir.join("folder_positions.json");
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Ok(map) =
                    serde_json::from_str::<std::collections::HashMap<String, String>>(&content)
                {
                    let mut current_map = self.folder_last_viewed.lock().unwrap();
                    *current_map = map;
                }
            }
        }
    }

    /// Retrieve the ICC Profile for a path if it matches the active cached path
    pub fn get_active_icc(&self, path: &str) -> Option<Option<Vec<u8>>> {
        if let Ok(guard) = self.active_icc_profile.lock() {
            if let Some((ref cached_path, ref icc_opt)) = *guard {
                if cached_path == path {
                    return Some(icc_opt.clone());
                }
            }
        }
        None
    }

    /// Set the active cached ICC Profile for a path
    pub fn set_active_icc(&self, path: String, icc: Option<Vec<u8>>) {
        if let Ok(mut guard) = self.active_icc_profile.lock() {
            *guard = Some((path, icc));
        }
    }

    /// Increment and get the next scan session ID for directory metadata extraction tasks
    pub fn next_scan_id(&self) -> u64 {
        self.current_scan_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
    }

    /// Retrieve the current scan session ID
    pub fn get_scan_id(&self) -> u64 {
        self.current_scan_id
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Application status summary
///
/// Contains key information about the current state of the application for debugging and monitoring.
#[derive(Debug, Clone)]
pub struct AppStatusSummary {
    /// Total number of images
    pub total_images: usize,
    /// Current image index
    pub current_index: usize,
    /// Whether there are any images
    pub has_images: bool,
    /// Current directory path
    pub current_directory: Option<String>,
}

impl fmt::Display for AppStatusSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Status Summary: {} images, current index: {}, directory: {}",
            self.total_images,
            self.current_index,
            self.current_directory.as_deref().unwrap_or("Not set")
        )
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
