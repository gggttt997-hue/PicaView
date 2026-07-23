use crate::core::image_processor::{BatchProcessor, ImageStitcher, OutputFormat, StitchConfig};
use crate::state::AppState;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, info, warn};

#[derive(Serialize, Clone)]
pub struct WorkshopProgress {
    pub current: usize,
    pub total: usize,
}

#[derive(Serialize)]
pub struct AssetMetadata {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
pub struct WorkshopService {
    state: Arc<AppState>,
}

impl WorkshopService {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    /// Prepares an asset for workshop by loading it into the high-performance cache
    pub async fn prepare_workshop_asset(&self, path: String) -> Result<AssetMetadata, String> {
        let cache = self.state.workshop_cache.clone();
        let coordinator = self.state.task_coordinator.clone();
        let task_id = format!("workshop-load-{}", path);

        debug!("[Evidence] Preparing workshop asset: {}", path);

        let path_clone = path.clone();

        let handle = tokio::spawn(async move {
            let task = tokio::task::spawn_blocking(move || {
                if let Some(img) = cache.get(&path_clone) {
                    debug!("[Evidence] ROI Cache Hit, skipping load: {}", path_clone);
                    return Ok(AssetMetadata {
                        width: img.width(),
                        height: img.height(),
                    });
                }

                info!(
                    "[Evidence] ROI Cache Miss, loading from disk: {}",
                    path_clone
                );
                match image::open(&path_clone) {
                    Ok(img) => {
                        let meta = AssetMetadata {
                            width: img.width(),
                            height: img.height(),
                        };
                        cache.load(&path_clone, img);
                        Ok(meta)
                    }
                    Err(e) => {
                        warn!("[ROI_ERR] Failed to load image {}: {}", path_clone, e);
                        Err(format!("Image load failed: {}", e))
                    }
                }
            });

            match task.await {
                Ok(res) => res,
                Err(e) => Err(format!("Blocking task failed: {}", e)),
            }
        });

        coordinator
            .register_task(task_id.clone(), handle.abort_handle())
            .await;

        match handle.await {
            Ok(result) => {
                coordinator.cleanup_task(&task_id).await;
                result
            }
            Err(e) => {
                if e.is_cancelled() {
                    debug!("[Evidence] Aborted previous request: {}", task_id);
                    Err("Request aborted by a newer one".to_string())
                } else {
                    Err(format!("Task failed: {}", e))
                }
            }
        }
    }

    /// Clears the workshop cache to release memory
    pub async fn clear_workshop_cache(&self) -> Result<(), String> {
        self.state.workshop_cache.clear();
        Ok(())
    }

    /// Perform batch image conversion
    pub async fn batch_convert<F>(
        &self,
        input_paths: Vec<String>,
        target_format: OutputFormat,
        output_dir: String,
        on_progress: F,
    ) -> Result<Vec<String>, String>
    where
        F: Fn(usize, usize) + Send + Sync + 'static,
    {
        let processor = BatchProcessor::new();

        tokio::task::spawn_blocking(move || {
            processor
                .convert(input_paths, target_format, &output_dir, on_progress)
                .map_err(|e| e.to_accurate_frontend_message())
        })
        .await
        .map_err(|e| format!("Task joined with error: {}", e))?
    }

    /// Synthesize multiple images into one
    pub async fn synthesize_images(
        &self,
        input_paths: Vec<String>,
        config: StitchConfig,
        output_path: String,
    ) -> Result<String, String> {
        let stitcher = ImageStitcher::new();

        tokio::task::spawn_blocking(move || {
            stitcher
                .stitch(input_paths, config, &output_path)
                .map_err(|e| e.to_accurate_frontend_message())
        })
        .await
        .map_err(|e| format!("Task joined with error: {}", e))?
    }

    /// Generate a fast preview of synthesized images
    pub async fn generate_preview(
        &self,
        input_paths: Vec<String>,
        config: StitchConfig,
        cache_dir: Option<PathBuf>,
    ) -> Result<String, String> {
        let stitcher = ImageStitcher::new();

        tokio::task::spawn_blocking(move || {
            stitcher
                .generate_preview(input_paths, config, cache_dir)
                .map_err(|e| e.to_accurate_frontend_message())
        })
        .await
        .map_err(|e| format!("Task joined with error: {}", e))?
    }
}
