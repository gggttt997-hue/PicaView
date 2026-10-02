use crate::error::{PicaViewError, Result};
/// File Manager
///
/// Responsible for managing the list of image files in the current directory and the navigation state.
use crate::models::{ImageInfo, SortCriteria, SortOrder};
use crate::utils::image_utils::{is_archive_extension, is_supported_by_extension};
use std::path::{Path, PathBuf};
use tracing::{debug, info, instrument, warn};
use walkdir::WalkDir;

/// File Manager
///
/// Manages the list of images in the current directory and the currently selected image index.
/// Provides image navigation functionality (previous, next).
#[derive(Debug, Clone)]
pub struct FileManager {
    /// List of image information in the current directory, including metadata
    image_infos: Vec<ImageInfo>,
    /// Index of the currently selected image
    current_index: usize,
    /// Current working directory
    base_directory: Option<PathBuf>,
    /// Current sorting criteria
    pub sort_criteria: SortCriteria,
    /// Current sorting order
    pub sort_order: SortOrder,
    /// Full unfiltered list of images
    all_image_infos: Vec<ImageInfo>,
    /// In-memory ISO inverted index
    pub iso_index: std::collections::BTreeMap<u32, std::collections::HashSet<String>>,
    /// In-memory Focal Length inverted index (focal_length * 1000 as u32)
    pub focal_length_index: std::collections::BTreeMap<u32, std::collections::HashSet<String>>,
    /// In-memory Aperture inverted index (aperture * 1000 as u32)
    pub aperture_index: std::collections::BTreeMap<u32, std::collections::HashSet<String>>,
    /// In-memory Camera Model index
    pub camera_index: std::collections::HashMap<String, std::collections::HashSet<String>>,
}

impl Default for FileManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Scans the directory containing the specified path to retrieve all supported image file information.
///
/// This is a blocking function intended to perform time-consuming filesystem operations in a separate thread.
///
/// # Returns
/// * `Ok((usize, Vec<ImageInfo>, usize, PathBuf))` - Returns a tuple on success:
///   - `usize`: Number of image files found
///   - `Vec<ImageInfo>`: List of scanned image information (including metadata)
///   - `usize`: If `path` is a file, its index in the list; otherwise 0
///   - `PathBuf`: The actual scanned directory path
fn collect_supported_entries(directory: &Path) -> (Vec<PathBuf>, usize) {
    let mut entries = Vec::new();
    let mut scan_errors = 0;
    for entry in WalkDir::new(directory).max_depth(1) {
        match entry {
            Ok(entry) => {
                let entry_path = entry.path();
                if entry_path.is_file() && is_supported_by_extension(entry_path) {
                    entries.push(entry_path.to_path_buf());
                }
            }
            Err(e) => {
                warn!("Error scanning file: {}", e);
                scan_errors += 1;
            }
        }
    }
    (entries, scan_errors)
}

#[instrument(skip(path), fields(dir = %path.display()))]
pub fn perform_directory_scan_blocking(
    path: &Path,
) -> Result<(usize, Vec<ImageInfo>, usize, PathBuf)> {
    let path_str = path.to_string_lossy().to_string();
    let is_archive_file = path.is_file() && is_archive_extension(path);

    if path_str.contains('|') || is_archive_file {
        let (archive_path_str, entry_name) = if path_str.contains('|') {
            let (a, b) = path_str.split_once('|').unwrap();
            (a.to_string(), b.to_string())
        } else {
            (path_str.clone(), "".to_string())
        };
        let entry_name_norm = entry_name.replace('\\', "/");
        let archive_path = Path::new(&archive_path_str);

        info!("Scanning archive: {}", archive_path.display());
        let shared_vfs = crate::core::archive_vfs::get_shared_vfs(archive_path).map_err(|e| {
            PicaViewError::directory_scan_error(format!("Failed to mount archive: {}", e))
        })?;

        let mut entries = Vec::new();
        let pass_store = crate::core::archive_vfs::archive_passwords()
            .read()
            .unwrap();
        let pass = pass_store
            .get(&archive_path_str)
            .map(|s: &String| s.as_str());

        let mut vfs = shared_vfs.lock().unwrap();
        let listed = vfs.list_entries(pass).map_err(|e| {
            if matches!(e, PicaViewError::PasswordRequired) {
                // If it's PasswordRequired, we return it directly instead of wrapping it in directory_scan_error
                e
            } else {
                PicaViewError::directory_scan_error(format!("Failed to list archive: {}", e))
            }
        })?;
        let mut i = 0;
        for (entry_name_str, size) in listed {
            let entry_path = std::path::Path::new(&entry_name_str);
            if is_supported_by_extension(entry_path) {
                entries.push((entry_name_str, i, size));
                i += 1;
            }
        }

        entries.sort_by(|a, b| crate::utils::path_utils::compare_natural(&a.0, &b.0));

        let mut new_infos = Vec::new();
        for (name, _index, size) in entries {
            let virtual_path = format!("{}|{}", archive_path_str, name);
            let mut info = ImageInfo::from_path(&virtual_path);
            info.size = Some(size);
            info.name = name;
            new_infos.push(info);
        }

        let file_count = new_infos.len();
        if file_count == 0 {
            return Err(PicaViewError::directory_scan_error(format!(
                "No supported image files found in archive: {}",
                archive_path.display()
            )));
        }

        let target_index = if !entry_name_norm.is_empty() {
            new_infos
                .iter()
                .position(|info| {
                    if let Some((_, name)) = info.path.split_once('|') {
                        name == entry_name_norm
                    } else {
                        false
                    }
                })
                .unwrap_or(0)
        } else {
            0
        };

        debug!(
            "Archive scan completed: {} image files, current index: {}",
            file_count, target_index
        );
        return Ok((
            file_count,
            new_infos,
            target_index,
            archive_path.to_path_buf(),
        ));
    }

    let directory = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .ok_or_else(|| PicaViewError::directory_scan_error(path.to_string_lossy()))?
            .to_path_buf()
    };

    info!("Starting directory scan: {}", directory.display());

    if !directory.exists() || !directory.is_dir() {
        return Err(PicaViewError::directory_scan_error(
            directory.to_string_lossy(),
        ));
    }

    let current_mtime = std::fs::metadata(&directory)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::now());
    let cache_dir = crate::utils::path_utils::get_app_cache_dir();

    if let Some(cached_infos) = crate::core::cache_manager::CacheManager::load_directory_snapshot(
        &cache_dir,
        &directory,
        current_mtime,
    ) {
        info!("Directory Snapshot HIT for: {}", directory.display());
        let file_count = cached_infos.len();
        let target_index = if path.is_file() {
            let target_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            cached_infos
                .iter()
                .position(|info| info.name == target_name)
                .unwrap_or(0)
        } else {
            0
        };
        return Ok((file_count, cached_infos, target_index, directory));
    }

    info!("Directory Snapshot MISS for: {}", directory.display());

    let (entries, scan_errors) = collect_supported_entries(&directory);
    if scan_errors > 0 {
        warn!("Directory scan completed with {} scan errors", scan_errors);
    }

    // 1. 轻量组装不消耗 I/O 的骨架列表并排序
    let mut new_infos: Vec<ImageInfo> = entries
        .iter()
        .map(|entry_path| {
            let normalized_path =
                crate::utils::path_utils::normalize_native_path(entry_path.to_path_buf());
            ImageInfo::from_path(normalized_path.to_string_lossy())
        })
        .collect();
    new_infos.sort_by(|a, b| crate::utils::path_utils::compare_natural(&a.name, &b.name));

    let file_count = new_infos.len();
    let target_index = if path.is_file() {
        let target_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        new_infos
            .iter()
            .position(|info| info.name == target_name)
            .unwrap_or(0)
    } else {
        0
    };

    // 2. 仅对以聚焦图为中心的前后 25 张 (共 50 张) 同步读取元数据闪显加载
    let window_size = 50;
    let start_idx = target_index.saturating_sub(window_size / 2);
    let end_idx = std::cmp::min(target_index + window_size / 2 + 1, file_count);

    for i in start_idx..end_idx {
        if let Some(info) = new_infos.get_mut(i) {
            let p = std::path::Path::new(&info.path);
            if let Ok(meta) = std::fs::metadata(p) {
                info.size = Some(meta.len());
                info.modified_ms = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64);
            }
        }
    }

    debug!(
        "Directory scan completed (Window Scan): {} image files, current index: {}",
        file_count, target_index
    );
    Ok((file_count, new_infos, target_index, directory))
}

impl FileManager {
    /// Create a new FileManager instance
    pub fn new() -> Self {
        let manager = Self {
            image_infos: Vec::new(),
            current_index: 0,
            base_directory: None,
            sort_criteria: SortCriteria::Name,
            sort_order: SortOrder::Ascending,
            all_image_infos: Vec::new(),
            iso_index: std::collections::BTreeMap::new(),
            focal_length_index: std::collections::BTreeMap::new(),
            aperture_index: std::collections::BTreeMap::new(),
            camera_index: std::collections::HashMap::new(),
        };

        // [Evidence] Location: FileManager struct definition, Capture: sort_criteria={:?}, sort_order={:?}, Rationale: Verify fields integration.
        debug!("[Evidence] Location: FileManager struct definition, Capture: sort_criteria={:?}, sort_order={:?}, Rationale: Verify fields integration.", manager.sort_criteria, manager.sort_order);

        manager
    }

    /// Update FileManager state
    ///
    /// # Arguments
    /// * `image_infos` - New image info list
    /// * `current_index` - Currently selected image index
    /// * `base_directory` - Current working directory
    pub fn update_state(
        &mut self,
        image_infos: Vec<ImageInfo>,
        current_index: usize,
        base_directory: PathBuf,
    ) {
        self.all_image_infos = image_infos.clone();
        self.image_infos = image_infos;
        self.current_index = current_index;
        self.base_directory = Some(base_directory);

        self.rebuild_exif_indices();

        // Automatically apply current sorting criteria
        self.apply_current_sort();

        // [Evidence] Location: End of FileManager::update_state, Capture: criteria={:?}, order={:?}, count={}, Rationale: Physical proof that sorting criteria are applied automatically after state update.
        debug!("[Evidence] Location: End of FileManager::update_state, Capture: criteria={:?}, order={:?}, count={}, Rationale: Verify auto-sort after update.", self.sort_criteria, self.sort_order, self.image_infos.len());
    }

    /// Supplement file metadata and EXIF for a single image dynamically.
    pub fn supplement_metadata_and_exif(
        &mut self,
        target_path: &str,
        size: Option<u64>,
        modified_ms: Option<u64>,
        camera_model: Option<String>,
        focal_length: Option<f32>,
        aperture: Option<f32>,
        iso: Option<u32>,
    ) {
        if let Some(info) = self
            .all_image_infos
            .iter_mut()
            .find(|info| info.path == target_path)
        {
            if info.size.is_none() {
                info.size = size;
            }
            if info.modified_ms.is_none() {
                info.modified_ms = modified_ms;
            }
        }
        if let Some(info) = self
            .image_infos
            .iter_mut()
            .find(|info| info.path == target_path)
        {
            if info.size.is_none() {
                info.size = size;
            }
            if info.modified_ms.is_none() {
                info.modified_ms = modified_ms;
            }
        }
        self.supplement_exif(target_path, camera_model, focal_length, aperture, iso);
    }

    /// Supplement EXIF metadata for a single image dynamically and incrementally update indices.
    pub fn supplement_exif(
        &mut self,
        target_path: &str,
        camera_model: Option<String>,
        focal_length: Option<f32>,
        aperture: Option<f32>,
        iso: Option<u32>,
    ) {
        let mut modified = false;

        if let Some(info) = self
            .all_image_infos
            .iter_mut()
            .find(|info| info.path == target_path)
        {
            info.camera_model = camera_model.clone();
            info.focal_length = focal_length;
            info.aperture = aperture;
            info.iso = iso;
            modified = true;
        }

        if let Some(info) = self
            .image_infos
            .iter_mut()
            .find(|info| info.path == target_path)
        {
            info.camera_model = camera_model.clone();
            info.focal_length = focal_length;
            info.aperture = aperture;
            info.iso = iso;
            modified = true;
        }

        if !modified {
            return;
        }

        let path_str = target_path.to_string();
        if let Some(iso_val) = iso {
            self.iso_index
                .entry(iso_val)
                .or_default()
                .insert(path_str.clone());
        }
        if let Some(focal) = focal_length {
            let key = (focal * 1000.0) as u32;
            self.focal_length_index
                .entry(key)
                .or_default()
                .insert(path_str.clone());
        }
        if let Some(ap) = aperture {
            let key = (ap * 1000.0) as u32;
            self.aperture_index
                .entry(key)
                .or_default()
                .insert(path_str.clone());
        }
        if let Some(ref cam) = camera_model {
            self.camera_index
                .entry(cam.clone())
                .or_default()
                .insert(path_str);
        }
    }

    /// Get the path of the next image
    ///
    /// Implements cyclic navigation: if current is the last, returns the first
    pub fn next_image(&mut self) -> Option<PathBuf> {
        if self.image_infos.is_empty() {
            debug!("Image list is empty, cannot navigate next");
            return None;
        }

        let old_index = self.current_index;
        self.current_index = (self.current_index + 1) % self.image_infos.len();

        // [Evidence] Location: FileManager::next_image
        // [Capture] self.image_infos[self.current_index]: self.image_infos[self.current_index].path
        // [Rationale] Verify navigation logic.
        debug!("[Evidence] Location: FileManager::next_image, Capture: path={}, Rationale: Verify navigation logic.", self.image_infos[self.current_index].path);

        debug!(
            "Navigating next: {} -> {} (total {} files)",
            old_index,
            self.current_index,
            self.image_infos.len()
        );

        self.current()
    }

    /// Get the path of the previous image
    ///
    /// Implements cyclic navigation: if current is the first, returns the last
    pub fn prev(&mut self) -> Option<PathBuf> {
        if self.image_infos.is_empty() {
            debug!("Image list is empty, cannot navigate previous");
            return None;
        }

        let old_index = self.current_index;
        self.current_index = if self.current_index == 0 {
            self.image_infos.len() - 1
        } else {
            self.current_index - 1
        };

        debug!(
            "Navigating previous: {} -> {} (total {} files)",
            old_index,
            self.current_index,
            self.image_infos.len()
        );

        self.current()
    }

    /// Get current image path
    pub fn current(&self) -> Option<PathBuf> {
        self.image_infos
            .get(self.current_index)
            .map(|info| PathBuf::from(&info.path))
    }

    /// Get total number of image files
    pub fn total_count(&self) -> usize {
        self.image_infos.len()
    }

    /// Get current index (0-based)
    pub fn current_index(&self) -> usize {
        self.current_index
    }

    /// Set current index (0-based)
    pub fn set_current_index(&mut self, index: usize) {
        if index < self.image_infos.len() {
            self.current_index = index;
        }
    }

    /// Get current working directory
    pub fn current_directory(&self) -> Option<&PathBuf> {
        self.base_directory.as_ref()
    }

    /// Scan the specified directory and update state
    pub fn scan_directory(&mut self, path: &Path) -> Result<()> {
        let (_count, infos, index, dir) = perform_directory_scan_blocking(path)?;
        self.update_state(infos, index, dir);
        Ok(())
    }

    /// Check if there are any image files
    pub fn has_images(&self) -> bool {
        !self.image_infos.is_empty()
    }

    /// Get a list of paths for all image files
    pub fn get_all_image_paths(&self) -> Vec<String> {
        self.image_infos
            .iter()
            .map(|info| info.path.clone())
            .collect()
    }

    /// Get all image information objects
    pub fn get_all_infos(&self) -> &Vec<ImageInfo> {
        &self.image_infos
    }

    /// Apply currently stored sorting criteria and order, keeping the current selection
    fn apply_current_sort(&mut self) {
        // [Evidence] Location: FileManager::apply_current_sort entry, Capture: count={}, criteria={:?}, order={:?}, Rationale: Ensure sorting logic correctly applies current metadata.
        debug!("[Evidence] Location: FileManager::apply_current_sort entry, Capture: count={}, criteria={:?}, order={:?}, Rationale: Verify sorting logic application.", self.image_infos.len(), self.sort_criteria, self.sort_order);

        if self.image_infos.is_empty() {
            return;
        }

        let current_path = self.current();

        match self.sort_criteria {
            SortCriteria::Name => {
                self.image_infos
                    .sort_by(|a, b| crate::utils::path_utils::compare_natural(&a.name, &b.name));
            }
            SortCriteria::Size => {
                self.image_infos.sort_by_key(|a| a.size.unwrap_or(0));
            }
            SortCriteria::Date => {
                self.image_infos.sort_by_key(|a| a.modified_ms.unwrap_or(0));
            }
        }

        if self.sort_order == SortOrder::Descending {
            self.image_infos.reverse();
        }

        // Index preservation: relocate current_index
        if let Some(path) = current_path {
            if let Some(new_pos) = self
                .image_infos
                .iter()
                .position(|info| Path::new(&info.path) == path)
            {
                self.current_index = new_pos;
            }
        }
    }

    /// Remove the specified filename from the managed list and adjust index
    ///
    /// # Arguments
    /// * `filename` - Filename to remove (not full path)
    pub fn remove_filename(&mut self, filename: &str) {
        if let Some(pos) = self.image_infos.iter().position(|x| x.name == filename) {
            self.image_infos.remove(pos);

            // Synchronize removal to the backup full list as well
            if let Some(all_pos) = self.all_image_infos.iter().position(|x| x.name == filename) {
                self.all_image_infos.remove(all_pos);
            }

            // Adjust index: ensure index is within bounds
            if !self.image_infos.is_empty() {
                if pos < self.current_index || self.current_index >= self.image_infos.len() {
                    // If removing an item before current, or removing the last item while current was pointing to it
                    if self.current_index > 0 {
                        self.current_index -= 1;
                    }
                }
            } else {
                self.current_index = 0;
            }

            // [Evidence] Location: FileManager::remove_filename
            // [Capture] self.image_infos.len(): self.image_infos.len()
            // [Rationale] Ensure memory list items are correctly removed
            debug!("[Evidence] Location: FileManager::remove_filename, Capture: {} items remaining, Rationale: Filename '{}' removed from memory list", self.image_infos.len(), filename);
        }
    }

    /// Re-sort and preserve the current selection
    pub fn re_sort(
        &mut self,
        criteria: crate::models::SortCriteria,
        order: crate::models::SortOrder,
    ) {
        self.sort_criteria = criteria;
        self.sort_order = order;

        self.apply_current_sort();

        // [Evidence] Location: End of FileManager::re_sort, Capture: criteria={:?}, order={:?}, current_index={}, Rationale: Verify re-sort persistence.
        debug!("[Evidence] Location: End of FileManager::re_sort, Capture: criteria={:?}, order={:?}, current_index={}, Rationale: Verify re-sort persistence.", self.sort_criteria, self.sort_order, self.current_index);
    }

    /// Find neighboring directories of the current directory (previous/next chapter)
    ///
    /// # Arguments
    /// * `forward` - Find next if true, previous if false
    pub fn find_neighboring_dir(&self, forward: bool) -> Result<Option<PathBuf>> {
        let current_dir = self
            .base_directory
            .as_ref()
            .ok_or_else(|| PicaViewError::general("No directory currently opened"))?;

        let parent = current_dir
            .parent()
            .ok_or_else(|| PicaViewError::general("Root directory has no neighbors"))?;

        // 1. List all subdirectories under the parent directory
        let mut sibling_dirs = Vec::new();
        for entry in std::fs::read_dir(parent)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                sibling_dirs.push(path);
            }
        }

        // 2. Natural sort
        sibling_dirs.sort_by(|a, b| {
            let a_name = a.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let b_name = b.file_name().and_then(|n| n.to_str()).unwrap_or("");
            crate::utils::path_utils::compare_natural(a_name, b_name)
        });

        // 3. Locate current directory index
        let current_pos = sibling_dirs
            .iter()
            .position(|p| p == current_dir)
            .ok_or_else(|| PicaViewError::general("Could not find current directory in parent"))?;

        // 4. Get target index
        let target_dir = if forward {
            if current_pos + 1 < sibling_dirs.len() {
                Some(sibling_dirs[current_pos + 1].clone())
            } else {
                None
            }
        } else if current_pos > 0 {
            Some(sibling_dirs[current_pos - 1].clone())
        } else {
            None
        };

        // [Evidence] Location: FileManager::find_neighboring_dir
        // [Capture] forward, current_pos, target_dir
        // [Rationale] Record cascading detection facts.
        debug!(
            "[Evidence] Location: FileManager::find_neighboring_dir, Capture: forward={}, current_pos={}, target={:?}, Rationale: Cascading detection",
            forward, current_pos, target_dir
        );

        Ok(target_dir)
    }

    pub fn rebuild_exif_indices(&mut self) {
        let mut iso_map = std::collections::BTreeMap::new();
        let mut focal_map = std::collections::BTreeMap::new();
        let mut aperture_map = std::collections::BTreeMap::new();
        let mut camera_map = std::collections::HashMap::new();

        for info in &self.all_image_infos {
            // Optimization: Skip non-EXIF images to avoid 100k redundant heap allocations and lock contention
            if info.iso.is_none()
                && info.focal_length.is_none()
                && info.aperture.is_none()
                && info.camera_model.is_none()
            {
                continue;
            }
            let path = info.path.clone();
            if let Some(iso) = info.iso {
                iso_map
                    .entry(iso)
                    .or_insert_with(std::collections::HashSet::new)
                    .insert(path.clone());
            }
            if let Some(focal) = info.focal_length {
                let key = (focal * 1000.0) as u32;
                focal_map
                    .entry(key)
                    .or_insert_with(std::collections::HashSet::new)
                    .insert(path.clone());
            }
            if let Some(ap) = info.aperture {
                let key = (ap * 1000.0) as u32;
                aperture_map
                    .entry(key)
                    .or_insert_with(std::collections::HashSet::new)
                    .insert(path.clone());
            }
            if let Some(ref cam) = info.camera_model {
                camera_map
                    .entry(cam.clone())
                    .or_insert_with(std::collections::HashSet::new)
                    .insert(path.clone());
            }
        }
        self.iso_index = iso_map;
        self.focal_length_index = focal_map;
        self.aperture_index = aperture_map;
        self.camera_index = camera_map;
    }

    /// 执行内存索引快速联合过滤
    pub fn filter_assets(
        &mut self,
        min_focal: Option<f32>,
        max_focal: Option<f32>,
        min_aperture: Option<f32>,
        max_aperture: Option<f32>,
        min_iso: Option<u32>,
        max_iso: Option<u32>,
        camera: Option<String>,
    ) {
        // 如果没有任何过滤条件，则恢复全量列表
        if min_focal.is_none()
            && max_focal.is_none()
            && min_aperture.is_none()
            && max_aperture.is_none()
            && min_iso.is_none()
            && max_iso.is_none()
            && camera.is_none()
        {
            self.image_infos = self.all_image_infos.clone();
            self.apply_current_sort();
            return;
        }

        // 初始化候选集
        let mut candidates: Option<std::collections::HashSet<String>> = None;

        // 1. ISO 过滤
        if min_iso.is_some() || max_iso.is_some() {
            let start = min_iso.unwrap_or(0);
            let end = max_iso.unwrap_or(u32::MAX);
            let mut iso_set = std::collections::HashSet::new();
            for (_iso, paths) in self.iso_index.range(start..=end) {
                iso_set.extend(paths.clone());
            }
            candidates = Some(match candidates {
                Some(existing) => existing.intersection(&iso_set).cloned().collect(),
                None => iso_set,
            });
        }

        // 2. 焦距过滤
        if min_focal.is_some() || max_focal.is_some() {
            let start = (min_focal.unwrap_or(0.0) * 1000.0) as u32;
            let end = (max_focal.unwrap_or(99999.0) * 1000.0) as u32;
            let mut focal_set = std::collections::HashSet::new();
            for (_focal, paths) in self.focal_length_index.range(start..=end) {
                focal_set.extend(paths.clone());
            }
            candidates = Some(match candidates {
                Some(existing) => existing.intersection(&focal_set).cloned().collect(),
                None => focal_set,
            });
        }

        // 3. 光圈过滤
        if min_aperture.is_some() || max_aperture.is_some() {
            let start = (min_aperture.unwrap_or(0.0) * 1000.0) as u32;
            let end = (max_aperture.unwrap_or(999.0) * 1000.0) as u32;
            let mut ap_set = std::collections::HashSet::new();
            for (_ap, paths) in self.aperture_index.range(start..=end) {
                ap_set.extend(paths.clone());
            }
            candidates = Some(match candidates {
                Some(existing) => existing.intersection(&ap_set).cloned().collect(),
                None => ap_set,
            });
        }

        // 4. 相机型号过滤 (模糊匹配)
        if let Some(target_cam) = camera {
            if !target_cam.is_empty() {
                let mut cam_set = std::collections::HashSet::new();
                let lower_target = target_cam.to_lowercase();
                for (model, paths) in &self.camera_index {
                    if model.to_lowercase().contains(&lower_target) {
                        cam_set.extend(paths.clone());
                    }
                }
                candidates = Some(match candidates {
                    Some(existing) => existing.intersection(&cam_set).cloned().collect(),
                    None => cam_set,
                });
            }
        }

        // 5. 将匹配到的路径子集恢复为 ImageInfo 列表并应用排序
        if let Some(matching_paths) = candidates {
            let current_sel = self.current();
            self.image_infos = self
                .all_image_infos
                .iter()
                .filter(|info| matching_paths.contains(&info.path))
                .cloned()
                .collect();

            // 重新定位当前索引
            if let Some(sel_path) = current_sel {
                if let Some(pos) = self
                    .image_infos
                    .iter()
                    .position(|info| info.path == sel_path.to_string_lossy())
                {
                    self.current_index = pos;
                } else {
                    self.current_index = 0;
                }
            } else {
                self.current_index = 0;
            }
        } else {
            self.image_infos = Vec::new();
            self.current_index = 0;
        }

        self.apply_current_sort();
    }
}

#[cfg(test)]
#[path = "file_manager_tests.rs"]
mod tests;
