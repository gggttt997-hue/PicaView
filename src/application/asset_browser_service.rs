use crate::core::asset_browser::{AssetInfo, RecursiveScanner};
use std::path::PathBuf;
use tracing::{debug, info};

#[derive(Clone)]
pub struct AssetBrowserService;

impl Default for AssetBrowserService {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetBrowserService {
    pub fn new() -> Self {
        Self
    }

    pub async fn trigger_recursive_scan<F, C>(
        &self,
        path: String,
        depth: Option<usize>,
        mut on_batch: F,
        on_complete: C,
    ) -> Result<(), String>
    where
        F: FnMut(Vec<AssetInfo>) + Send + Sync + 'static,
        C: Fn() + Send + Sync + 'static,
    {
        // --- 🚀 新增：针对 ZIP/CBZ 的虚拟资产扫描器 ---
        let is_archive_file =
            crate::utils::image_utils::is_archive_extension(std::path::Path::new(&path));
        let archive_info = if is_archive_file {
            Some((path.clone(), ""))
        } else {
            crate::core::image_loader::parse_archive_path(&path).map(|(a, e)| (a.to_string(), e))
        };

        if let Some((archive_path, _)) = archive_info {
            let archive_path_buf = PathBuf::from(&archive_path);
            if !archive_path_buf.exists() {
                return Err("Archive physical path does not exist".into());
            }

            let archive_path_str = archive_path;
            tokio::spawn(async move {
                if let Ok(mut vfs) = crate::core::archive_vfs::mount_archive(&archive_path_buf) {
                    if let Ok(entries) = vfs.list_entries(None) {
                        let mut batch = Vec::new();
                        for (name, size) in entries {
                            if crate::utils::image_utils::is_supported_by_extension(
                                std::path::Path::new(&name),
                            ) {
                                let virtual_path = format!("{}|{}", archive_path_str, name);
                                let mut info = AssetInfo::new(virtual_path);
                                info.size = Some(size);
                                batch.push(info);
                            }
                        }
                        if !batch.is_empty() {
                            on_batch(batch);
                        }
                    }
                }
                on_complete();
            });
            return Ok(());
        }

        let scan_depth = depth.unwrap_or(100);
        debug!(
            "AssetBrowserService: Triggering recursive scan: {}, depth: {}",
            path, scan_depth
        );

        let path_buf = PathBuf::from(path);
        if !path_buf.exists() {
            return Err("Path does not exist".into());
        }

        let resolved_path = self.resolve_scan_path(path_buf);
        info!(
            "AssetBrowserService: Resolved scan path: {}",
            resolved_path.display()
        );

        let batch_size = 20;

        tokio::spawn(async move {
            RecursiveScanner::scan(resolved_path, scan_depth, batch_size, move |batch| {
                on_batch(batch)
            })
            .await;
            on_complete();
        });

        Ok(())
    }

    fn resolve_scan_path(&self, path: PathBuf) -> PathBuf {
        if path.is_file() {
            if let Some(parent) = path.parent() {
                return parent.to_path_buf();
            }
        }
        path
    }
}
