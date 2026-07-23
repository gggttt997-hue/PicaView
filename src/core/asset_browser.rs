use crate::utils::image_utils::{get_file_metadata, is_supported_by_extension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::debug;
use walkdir::WalkDir;

/// Asset information for display in the asset browser
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetInfo {
    /// Full path of the asset
    pub path: String,
    /// Filename of the asset
    pub name: String,
    /// Size of the asset in bytes
    pub size: Option<u64>,
    /// Last modified timestamp of the asset in milliseconds
    pub modified_ms: Option<u64>,
}

impl AssetInfo {
    /// Create new AssetInfo from path
    pub fn new(path: String) -> Self {
        let name = Path::new(&path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        Self {
            path,
            name,
            size: None,
            modified_ms: None,
        }
    }
}

/// Asset browser state
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AssetBrowserState {
    /// List of currently discovered assets
    pub assets: Vec<AssetInfo>,
}

impl AssetBrowserState {
    /// Create a new AssetBrowserState instance
    pub fn new() -> Self {
        // [Evidence] Location: src-tauri/src/core/asset_browser.rs
        // [Capture] AssetBrowserState struct definition
        // [Rationale] Ensure absolute path storage and state isolation.
        debug!("[Evidence] Location: src-tauri/src/core/asset_browser.rs, Capture: AssetBrowserState struct definition, Rationale: Ensure absolute path storage and state isolation.");

        Self { assets: Vec::new() }
    }
}

/// Task ID wrapper for identifying asynchronous scanning sessions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskId(pub String);

/// Asynchronous asset stream scanner responsible for managing stream-based asset acquisition
pub struct AsyncAssetStreamer {
    /// Session unique identifier
    pub id: TaskId,
}

impl AsyncAssetStreamer {
    /// Create a new AsyncAssetStreamer instance
    pub fn new(session_id: String) -> Self {
        Self {
            id: TaskId(session_id),
        }
    }
}

/// Recursive scanning engine
pub struct RecursiveScanner;

impl RecursiveScanner {
    /// Asynchronously scan a directory and return results via channel
    ///
    /// # Arguments
    /// * `path` - Directory to start scanning
    /// * `max_depth` - Maximum recursion depth
    /// * `batch_size` - Size for batching results
    /// * `tx` - Asynchronous channel to send results
    pub async fn scan_stream(
        path: PathBuf,
        max_depth: usize,
        batch_size: usize,
        tx: tokio::sync::mpsc::Sender<Vec<AssetInfo>>,
    ) {
        let mut batch = Vec::with_capacity(batch_size);

        for entry in WalkDir::new(&path)
            .max_depth(max_depth)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() && is_supported_by_extension(entry.path()) {
                let current_file = entry.path().to_string_lossy().to_string();
                let mut asset = AssetInfo::new(current_file.clone());

                // Fetch metadata
                let (size, modified_ms) = get_file_metadata(entry.path());
                asset.size = size;
                asset.modified_ms = modified_ms;

                batch.push(asset);

                if batch.len() >= batch_size {
                    // [Evidence] Location: RecursiveScanner::scan_stream
                    // [Capture] batch_size: batch.len(), current_file: current_file
                    // [Rationale] Monitor scan throughput and progress, ensuring batched reporting.
                    debug!("[Evidence] Location: RecursiveScanner::scan_stream, Capture: batch_size={}, current_file={}, Rationale: Monitor scan throughput and progress.", batch.len(), current_file);

                    if tx
                        .send(std::mem::replace(
                            &mut batch,
                            Vec::with_capacity(batch_size),
                        ))
                        .await
                        .is_err()
                    {
                        // Stop scanning if receiver is closed
                        return;
                    }
                }
            }
        }

        // Send final batch
        if !batch.is_empty() {
            let _ = tx.send(batch).await;
        }
    }

    /// Recursively scan a directory
    ///
    /// # Arguments
    /// * `path` - Directory to start scanning
    /// * `max_depth` - Maximum recursion depth
    /// * `batch_size` - Size for batching results
    /// * `on_batch` - Callback triggered when batch_size assets are collected
    pub async fn scan<F>(path: PathBuf, max_depth: usize, batch_size: usize, mut on_batch: F)
    where
        F: FnMut(Vec<AssetInfo>),
    {
        let mut batch = Vec::with_capacity(batch_size);
        let mut total_found = 0;

        for entry in WalkDir::new(&path)
            .max_depth(max_depth)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() && is_supported_by_extension(entry.path()) {
                let mut asset = AssetInfo::new(entry.path().to_string_lossy().to_string());

                // Fetch metadata
                let (size, modified_ms) = get_file_metadata(entry.path());
                asset.size = size;
                asset.modified_ms = modified_ms;

                batch.push(asset);
                total_found += 1;

                if batch.len() >= batch_size {
                    // [Evidence] Location: RecursiveScanner::scan
                    // [Capture] batch_size: batch.len(), found_count: total_found
                    // [Rationale] Verify batched push logic effectiveness.
                    debug!("[Evidence] Location: RecursiveScanner::scan, Capture: batch_size={}, found_count={}, Rationale: Verify batched push logic effectiveness.", batch.len(), total_found);

                    on_batch(std::mem::replace(
                        &mut batch,
                        Vec::with_capacity(batch_size),
                    ));
                }
            }
        }

        // Send final batch
        if !batch.is_empty() {
            debug!("[Evidence] Location: RecursiveScanner::scan (final), Capture: batch_size={}, found_count={}, Rationale: Sending final batch of assets.", batch.len(), total_found);
            on_batch(batch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_asset_info_creation() {
        let mut info = AssetInfo::new("/test/path/img.jpg".into());
        assert_eq!(info.path, "/test/path/img.jpg");
        assert_eq!(info.name, "img.jpg");

        // Verify metadata fields
        info.size = Some(1234);
        info.modified_ms = Some(1672531200000);
        assert_eq!(info.size, Some(1234));
        assert_eq!(info.modified_ms, Some(1672531200000));
    }

    #[test]
    fn test_asset_browser_state_initialization() {
        let state = AssetBrowserState::new();
        assert!(state.assets.is_empty());
    }

    #[test]
    fn test_async_asset_streamer_initialization() {
        let session_id = "test-session-123".to_string();
        let streamer = AsyncAssetStreamer::new(session_id.clone());
        assert_eq!(streamer.id.0, session_id);
    }

    #[tokio::test]
    async fn test_recursive_scanner_metadata() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();
        fs::write(root.join("a.jpg"), "test content").unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        RecursiveScanner::scan_stream(root.to_path_buf(), 2, 1, tx).await;

        if let Some(batch) = rx.recv().await {
            let asset = &batch[0];
            assert!(asset.size.is_some(), "Asset size should be populated");
            assert!(
                asset.modified_ms.is_some(),
                "Asset modified_ms should be populated"
            );
        } else {
            panic!("No assets found");
        }
    }

    #[tokio::test]
    async fn test_recursive_scanner_streaming() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();
        fs::write(root.join("a.jpg"), "").unwrap();
        fs::write(root.join("b.png"), "").unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        // Expect: RecursiveScanner::scan_stream sends assets asynchronously
        RecursiveScanner::scan_stream(root.to_path_buf(), 2, 1, tx).await;

        let mut count = 0;
        while let Some(batch) = rx.recv().await {
            count += batch.len();
        }
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_recursive_scanner_batching() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        // Create nested structure
        // root/a.jpg (depth 1)
        // root/sub/b.png (depth 2)
        // root/sub/deep/c.webp (depth 3)
        fs::write(root.join("a.jpg"), "").unwrap();
        let sub = root.join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("b.png"), "").unwrap();
        let deep = sub.join("deep");
        fs::create_dir(&deep).unwrap();
        fs::write(deep.join("c.webp"), "").unwrap();

        let mut found_assets = Vec::new();
        let batch_size = 2;

        // Expect: Only a.jpg (1) found at depth 1
        RecursiveScanner::scan(root.to_path_buf(), 1, batch_size, |batch| {
            found_assets.extend(batch);
        })
        .await;
        assert_eq!(found_assets.len(), 1);

        // Expect: a.jpg (1) and b.png (2) found at depth 2
        found_assets.clear();
        RecursiveScanner::scan(root.to_path_buf(), 2, batch_size, |batch| {
            found_assets.extend(batch);
        })
        .await;
        assert_eq!(found_assets.len(), 2);

        // Expect: a, b, c found at depth 3
        found_assets.clear();
        RecursiveScanner::scan(root.to_path_buf(), 3, batch_size, |batch| {
            found_assets.extend(batch);
        })
        .await;
        assert_eq!(found_assets.len(), 3);
    }
}
