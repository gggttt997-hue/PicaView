use crate::core::image_generator::generate_thumbnail;
use crate::models::ImageInfo;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::debug;

pub struct CacheManager;

impl CacheManager {
    /// Generate a BLAKE3 hash index based on file path, modification time, and size
    pub fn generate_cache_key(path: &Path, mtime: SystemTime, size: u32) -> String {
        let start = std::time::Instant::now();
        let mut hasher = blake3::Hasher::new();
        hasher.update(path.to_string_lossy().as_bytes());
        let nanos = mtime
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        hasher.update(&nanos.to_le_bytes());
        hasher.update(&size.to_le_bytes());
        let key = hasher.finalize().to_hex().to_string();

        let duration = start.elapsed();
        // [Evidence] Location: CacheManager::generate_cache_key
        // [Capture] key, hash_time_ms: duration.as_millis()
        // [Rationale] Verify BLAKE3 generation performance meets expectations.
        debug!("[Evidence] Location: CacheManager::generate_cache_key, Capture: key={}, hash_time_ms={}, Rationale: Verify hash generation performance.", key, duration.as_millis());

        key
    }

    /// Save thumbnail to disk cache
    pub fn save_to_disk(cache_dir: &Path, key: &str, info: &ImageInfo) -> std::io::Result<()> {
        if !cache_dir.exists() {
            std::fs::create_dir_all(cache_dir)?;
        }

        let cache_path = cache_dir.join(format!("{}.json", key));
        let tmp_path = cache_dir.join(format!("{}.json.tmp", key));

        let json = serde_json::to_string(info)?;
        std::fs::write(&tmp_path, json)?;
        std::fs::rename(tmp_path, cache_path)?;

        // [Evidence] Location: CacheManager::save_to_disk
        // [Capture] key
        // [Rationale] Confirm physical write success (atomic).
        debug!("[Evidence] Location: CacheManager::save_to_disk (Atomic), Capture: key={}, Rationale: Confirm physical write success.",
            key);

        Ok(())
    }

    /// Load thumbnail from disk cache
    pub fn load_from_disk(cache_dir: &Path, key: &str) -> Option<ImageInfo> {
        let cache_path = cache_dir.join(format!("{}.json", key));
        if !cache_path.exists() {
            return None;
        }

        let result = std::fs::read_to_string(cache_path)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok());

        // [Evidence] Location: CacheManager::load_from_disk
        // [Capture] key, hit: result.is_some()
        // [Rationale] Monitor cache hit rate.
        debug!("[Evidence] Location: CacheManager::load_from_disk, Capture: key={}, hit={}, Rationale: Monitor cache hit.", key, result.is_some());

        result
    }

    /// Execute thumbnail generation tasks in parallel
    pub fn process_batch(paths: Vec<PathBuf>, size: u32) -> Vec<ImageInfo> {
        // [Evidence] Location: CacheManager::process_batch
        // [Capture] batch_size: paths.len()
        // [Rationale] Verify triggering of parallel batch processing.
        debug!("[Evidence] Location: CacheManager::process_batch, Capture: batch_size={}, Rationale: Verify parallel batch processing trigger.", paths.len());

        paths
            .into_par_iter()
            .filter_map(|path| {
                let path_str = path.to_string_lossy().to_string();

                // 1. Get mtime (for hash verification, although currently only used for generation)
                let _mtime = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::now());

                // 2. Call generator (disk cache read/write not handled here yet, verifying parallel calls only)
                generate_thumbnail(&path_str, size).ok()
            })
            .collect()
    }

    /// Save directory image list snapshot to disk cache
    pub fn save_directory_snapshot(
        cache_dir: &Path,
        dir_path: &Path,
        mtime: SystemTime,
        infos: &[ImageInfo],
    ) -> std::io::Result<()> {
        let dir_str = dir_path.to_string_lossy().to_string();
        let mut hasher = blake3::Hasher::new();
        hasher.update(dir_str.as_bytes());
        let key = hasher.finalize().to_hex().to_string();

        let nanos = mtime
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        let snapshot = DirectorySnapshot {
            directory: dir_str,
            mtime_nanos: nanos,
            image_infos: infos.to_vec(),
        };

        if !cache_dir.exists() {
            std::fs::create_dir_all(cache_dir)?;
        }

        let cache_path = cache_dir.join(format!("snap_{}.json", key));
        let tmp_path = cache_dir.join(format!("snap_{}.json.tmp", key));

        let json = serde_json::to_string(&snapshot)?;
        std::fs::write(&tmp_path, json)?;
        std::fs::rename(tmp_path, cache_path)?;
        Ok(())
    }

    /// Load directory image list snapshot from disk cache
    pub fn load_directory_snapshot(
        cache_dir: &Path,
        dir_path: &Path,
        current_mtime: SystemTime,
    ) -> Option<Vec<ImageInfo>> {
        let dir_str = dir_path.to_string_lossy().to_string();
        let mut hasher = blake3::Hasher::new();
        hasher.update(dir_str.as_bytes());
        let key = hasher.finalize().to_hex().to_string();

        let cache_path = cache_dir.join(format!("snap_{}.json", key));
        if !cache_path.exists() {
            return None;
        }

        let json = std::fs::read_to_string(cache_path).ok()?;
        let snapshot: DirectorySnapshot = serde_json::from_str(&json).ok()?;

        let current_nanos = current_mtime
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        if snapshot.mtime_nanos == current_nanos {
            Some(snapshot.image_infos)
        } else {
            None
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct DirectorySnapshot {
    pub directory: String,
    pub mtime_nanos: u128,
    pub image_infos: Vec<ImageInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, RgbImage};
    use tempfile::TempDir;

    #[test]
    fn test_cache_key_consistency() {
        let path = Path::new("/test/image.jpg");
        let now = SystemTime::now();
        let key1 = CacheManager::generate_cache_key(path, now, 100);
        let key2 = CacheManager::generate_cache_key(path, now, 100);
        let key3 = CacheManager::generate_cache_key(path, now, 200);
        assert_eq!(key1, key2);
        assert_ne!(
            key1, key3,
            "Cache keys for different sizes should be inconsistent"
        );
    }

    #[test]
    fn test_cache_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let cache_dir = temp_dir.path();

        let path = "test_image.jpg".to_string();
        let key = "test_cache_key";
        let mut info = ImageInfo::new(path.clone());
        info.proxy_path = Some("proxy.jpg".to_string());

        // 1. Save to disk
        CacheManager::save_to_disk(cache_dir, key, &info).unwrap();

        // 2. Load from disk
        let loaded = CacheManager::load_from_disk(cache_dir, key);

        // Expect load to succeed
        assert!(loaded.is_some(), "Should be able to load cache from disk");
        let loaded_info = loaded.unwrap();
        assert_eq!(loaded_info.path, path);
        assert_eq!(loaded_info.proxy_path, Some("proxy.jpg".to_string()));
    }

    #[test]
    fn test_process_batch_parallelism() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        let mut paths = Vec::new();
        for i in 0..5 {
            let p = root.join(format!("{}.png", i));
            // Create real image files, otherwise generate_thumbnail will fail
            let img = DynamicImage::ImageRgb8(RgbImage::new(10, 10));
            img.save_with_format(&p, ImageFormat::Png).unwrap();
            paths.push(p);
        }

        let results = CacheManager::process_batch(paths, 100);

        // Expect result count to match input (5 successfully generated ImageInfos)
        assert_eq!(
            results.len(),
            5,
            "Batch processing result count must match input"
        );
        for res in results {
            assert!(res.pixel_buffer.is_some());
        }
    }
}
