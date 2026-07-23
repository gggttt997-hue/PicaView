use lru::LruCache;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use tracing::{error, info};

#[derive(Serialize, Deserialize)]
struct HistoryData {
    items: Vec<String>,
}

pub struct HistoryManager {
    cache: LruCache<String, ()>,
    _max_size: usize,
    config_path: Option<PathBuf>,
}

impl fmt::Debug for HistoryManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryManager")
            .field("items_count", &self.cache.len())
            .field("config_path", &self.config_path)
            .finish()
    }
}

impl HistoryManager {
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: LruCache::new(NonZeroUsize::new(max_size).unwrap()),
            _max_size: max_size,
            config_path: None,
        }
    }

    pub fn set_config_path(&mut self, path: PathBuf) {
        self.config_path = Some(path);
    }

    pub fn add(&mut self, path: String) {
        // [Evidence] Location: HistoryManager::add
        // [Capture] path, current_cache_len: self.cache.len()
        // [Rationale] Record new history path for auditing LRU state changes

        // Force path normalization
        let normalized = crate::utils::path_utils::normalize_native_path(PathBuf::from(path))
            .to_string_lossy()
            .to_string();

        self.cache.put(normalized, ());

        if let Err(e) = self.save() {
            error!("Failed to save history: {}", e);
        }
    }

    pub fn remove(&mut self, path: String) {
        // Force path normalization before removal
        let normalized = crate::utils::path_utils::normalize_native_path(PathBuf::from(path))
            .to_string_lossy()
            .to_string();

        let removed = self.cache.pop(&normalized).is_some();
        // [Evidence] Location: HistoryManager::remove
        // [Capture] path, removed_success: removed
        // [Rationale] Monitor if the deletion physically reaches the cache
        info!(
            "Removed history record [success: {}]: {}",
            removed, normalized
        );

        if removed {
            if let Err(e) = self.save() {
                error!("Failed to save history after removal: {}", e);
            }
        }
    }

    pub fn get_all(&self) -> Vec<String> {
        self.cache.iter().map(|(k, _)| k.clone()).collect()
    }

    pub fn capacity(&self) -> usize {
        self._max_size
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        let _ = self.save();
    }

    pub fn load(&mut self) -> Result<(), String> {
        let path = self.config_path.as_ref().ok_or("Config path not set")?;
        if !path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;

        // [Reality Check] Handle illegal JSON files
        let data: HistoryData = serde_json::from_str(&content).map_err(|e| {
            error!("History file corrupted, resetting: {}", e);
            "Corrupted".to_string()
        })?;

        self.cache.clear();
        for item in data.items.into_iter().rev() {
            self.cache.put(item, ());
        }

        // [Evidence] Location: HistoryManager::load
        // [Capture] loaded_count: self.cache.len(), file_path: path.display()
        // [Rationale] Ensure that existing history is correctly loaded from disk
        info!(
            "Loaded {} history items from {}",
            self.cache.len(),
            path.display()
        );
        Ok(())
    }

    fn save(&self) -> Result<(), String> {
        let path = match &self.config_path {
            Some(p) => p,
            None => return Ok(()),
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let data = HistoryData {
            items: self.get_all(),
        };

        let content = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
        fs::write(path, content).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tempfile::tempdir;

    #[test]
    fn test_history_removal() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("history_remove.json");

        {
            let mut hm = HistoryManager::new(5);
            hm.set_config_path(file_path.clone());
            hm.add("/path/1".to_string());
            hm.add("/path/2".to_string());
            hm.remove("/path/1".to_string());
        }

        // Verify file content after persistence
        {
            let mut hm = HistoryManager::new(5);
            hm.set_config_path(file_path);
            hm.load().unwrap();
            let history = hm.get_all();
            assert_eq!(history.len(), 1);
            let expected =
                crate::utils::path_utils::normalize_native_path(PathBuf::from("/path/2"))
                    .to_string_lossy()
                    .to_string();
            assert_eq!(history[0], expected);
        }
    }

    #[test]
    fn test_history_lru_order() {
        let mut hm = HistoryManager::new(3);
        hm.add("/path/1".to_string());
        hm.add("/path/2".to_string());
        hm.add("/path/3".to_string());
        hm.add("/path/1".to_string());

        let history = hm.get_all();
        let expected = crate::utils::path_utils::normalize_native_path(PathBuf::from("/path/1"))
            .to_string_lossy()
            .to_string();
        assert_eq!(history[0], expected);
        assert_eq!(history.len(), 3);
    }

    #[test]
    fn test_persistence() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("history.json");

        {
            let mut hm = HistoryManager::new(5);
            hm.set_config_path(file_path.clone());
            hm.add("/path/a".to_string());
            hm.add("/path/b".to_string());
        }

        {
            let mut hm = HistoryManager::new(5);
            hm.set_config_path(file_path);
            hm.load().unwrap();
            let history = hm.get_all();
            let expected_b =
                crate::utils::path_utils::normalize_native_path(PathBuf::from("/path/b"))
                    .to_string_lossy()
                    .to_string();
            let expected_a =
                crate::utils::path_utils::normalize_native_path(PathBuf::from("/path/a"))
                    .to_string_lossy()
                    .to_string();
            assert_eq!(history[0], expected_b);
            assert_eq!(history[1], expected_a);
        }
    }

    #[test]
    fn test_corrupted_file_recovery() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("corrupted.json");
        fs::write(&file_path, "invalid json content{").unwrap();

        let mut hm = HistoryManager::new(5);
        hm.set_config_path(file_path);

        // Verify that it doesn't crash on load failure, but returns an error or triggers reset logic
        let result = hm.load();
        assert!(result.is_err());
        assert_eq!(hm.get_all().len(), 0);
    }

    #[tokio::test]
    async fn test_async_validation_performance() {
        // [Reality Check] Simulate async validation performance for 10 physical paths
        let dir = tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..10 {
            let p = dir.path().join(format!("test_dir_{}", i));
            fs::create_dir(&p).unwrap();
            paths.push(p.to_string_lossy().to_string());
        }

        let start = Instant::now();

        use futures::future::join_all;
        use tokio::fs as tfs;

        let checks = paths.iter().map(|p| {
            let path = p.clone();
            async move { tfs::metadata(path).await.is_ok() }
        });

        let results = join_all(checks).await;
        let duration = start.elapsed();

        // [Evidence] Location: Reality Check Phase
        // [Capture] latency_ms: duration.as_millis(), results_count: results.len()
        // [Rationale] Verify if async parallel validation meets the < 20ms SLA
        println!(
            "Checked {} paths in {}ms",
            results.len(),
            duration.as_millis()
        );

        assert_eq!(results.iter().filter(|&&r| r).count(), 10);
        // SLA check: 10 parallel IOs should be far below 20ms on modern systems
        assert!(
            duration.as_millis() < 20,
            "SLA Violation: History validation too slow"
        );
    }
}
