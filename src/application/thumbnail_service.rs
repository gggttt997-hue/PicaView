use crate::models::ImageInfo;
use crate::state::AppState;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::sync::Semaphore;
use tracing::{debug, info};

static THUMB_SEMAPHORE: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn get_semaphore() -> Arc<Semaphore> {
    THUMB_SEMAPHORE
        .get_or_init(|| Arc::new(Semaphore::new(8)))
        .clone()
}

#[derive(Clone)]
pub struct ThumbnailService {
    state: Arc<AppState>,
}

impl ThumbnailService {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    pub async fn get_all_image_paths(&self) -> Result<Vec<String>, String> {
        let file_manager_arc = Arc::clone(&self.state.file_manager);

        tokio::task::spawn_blocking(move || {
            let file_manager_guard = file_manager_arc.lock().map_err(|e| e.to_string())?;
            Ok(file_manager_guard.get_all_image_paths())
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn get_thumbnail_data(
        &self,
        path: String,
        size: u32,
        cache_dir: PathBuf,
    ) -> Result<ImageInfo, String> {
        let mtime = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::now());

        let cache_key = crate::core::cache_manager::CacheManager::generate_cache_key(
            std::path::Path::new(&path),
            mtime,
            size,
        );

        // 1. Memory cache check
        {
            let mut cache_guard = self
                .state
                .thumbnail_cache
                .lock()
                .map_err(|e| e.to_string())?;
            if let Some(thumbnail) = cache_guard.get(&cache_key) {
                return Ok(thumbnail.clone());
            }
        }

        // 2. Single-Flight Check: Is someone already generating this?
        let (tx, rx) = {
            let mut pending = self.state.pending_tasks.lock().map_err(|e| e.to_string())?;
            if let Some(receiver) = pending.get(&cache_key) {
                // Someone else is working on it.
                (None, Some(receiver.resubscribe()))
            } else {
                // We are the first.
                let (tx, rx) = tokio::sync::broadcast::channel(1);
                pending.insert(cache_key.clone(), rx);
                (Some(tx), None)
            }
        };

        if let Some(mut receiver) = rx {
            debug!("[Single-Flight] Waiting for existing task: {}", path);
            return receiver.recv().await.map_err(|e| e.to_string());
        }

        // We are the generator
        let tx = tx.expect("Logic error: tx must be Some here");

        // 3. Disk cache check
        let thumb_cache_dir = cache_dir.join("thumbnails");
        if let Some(mut cached_info) =
            crate::core::cache_manager::CacheManager::load_from_disk(&thumb_cache_dir, &cache_key)
        {
            debug!("[Cache Hit] Disk hit: {}", path);

            // If it's a disk hit but no pixel_buffer (as expected from disk), we need to load the pixel file
            if cached_info.pixel_buffer.is_none() {
                if let Some(proxy_path) = &cached_info.proxy_path {
                    let proxy_path = PathBuf::from(proxy_path);
                    if proxy_path.exists() {
                        // Load and decode the small proxy image from disk
                        let res = tokio::task::spawn_blocking(move || {
                            image::open(&proxy_path).map(|img| {
                                let rgba = img.to_rgba8();
                                slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                                    rgba.as_raw(),
                                    rgba.width(),
                                    rgba.height(),
                                )
                            })
                        })
                        .await
                        .map_err(|e| e.to_string())?;

                        if let Ok(buffer) = res {
                            cached_info.pixel_buffer = Some(buffer);
                        }
                    }
                }
            }

            if cached_info.pixel_buffer.is_some() {
                let mut cache_guard = self
                    .state
                    .thumbnail_cache
                    .lock()
                    .map_err(|e| e.to_string())?;
                cache_guard.put(cache_key.clone(), cached_info.clone());

                // Broadcast result to waiters
                let _ = tx.send(cached_info.clone());
                // Cleanup pending tasks
                let _ = self
                    .state
                    .pending_tasks
                    .lock()
                    .map(|mut p| p.remove(&cache_key));

                return Ok(cached_info);
            }
        }

        // 4. Generate
        let path_clone = path.clone();
        let cache_key_clone = cache_key.clone();
        let coordinator = Arc::clone(&self.state.task_coordinator);
        let semaphore = get_semaphore();

        let generated_thumbnail = tokio::spawn(async move {
            let _permit = semaphore.acquire().await.map_err(|e| e.to_string())?;
            debug!("[Thumbnail Gen] Semaphore acquired: {}", path_clone);

            let res = tokio::task::spawn_blocking(move || {
                crate::core::image_generator::generate_thumbnail(&path_clone, size)
            })
            .await
            .map_err(|e| e.to_string())?;

            coordinator.cleanup_task(&cache_key_clone).await;
            res.map_err(|e| e.to_string())
        });

        self.state
            .task_coordinator
            .register_task(cache_key.clone(), generated_thumbnail.abort_handle())
            .await;

        let result = match generated_thumbnail.await {
            Ok(Ok(res)) => res,
            Ok(Err(e)) => {
                let _ = self
                    .state
                    .pending_tasks
                    .lock()
                    .map(|mut p| p.remove(&cache_key));
                return Err(e);
            }
            Err(e) => {
                let _ = self
                    .state
                    .pending_tasks
                    .lock()
                    .map(|mut p| p.remove(&cache_key));
                return Err(e.to_string());
            }
        };

        // 5. Persistence & Cache update
        {
            let mut result_to_save = result.clone();
            let cache_dir_clone = thumb_cache_dir.clone();
            let cache_key_clone = cache_key.clone();

            // Save the proxy pixels to a file
            if let Some(buffer) = &result.pixel_buffer {
                let proxy_img_path = cache_dir_clone.join(format!("{}.jpg", cache_key_clone));
                result_to_save.proxy_path = Some(proxy_img_path.to_string_lossy().to_string());

                let buffer_clone = buffer.clone();
                tokio::spawn(async move {
                    let _ = tokio::task::spawn_blocking(move || {
                        let img = image::RgbaImage::from_raw(
                            buffer_clone.width(),
                            buffer_clone.height(),
                            buffer_clone.as_bytes().to_vec(),
                        );
                        if let Some(img) = img {
                            let _ = image::DynamicImage::ImageRgba8(img)
                                .save_with_format(&proxy_img_path, image::ImageFormat::Jpeg);
                        }
                    })
                    .await;

                    let _ = crate::core::cache_manager::CacheManager::save_to_disk(
                        &cache_dir_clone,
                        &cache_key_clone,
                        &result_to_save,
                    );
                });
            }

            let mut cache_guard = self
                .state
                .thumbnail_cache
                .lock()
                .map_err(|e| e.to_string())?;
            cache_guard.put(cache_key.clone(), result.clone());
        }

        // Broadcast to waiters
        let _ = tx.send(result.clone());
        // Cleanup
        let _ = self
            .state
            .pending_tasks
            .lock()
            .map(|mut p| p.remove(&cache_key));

        Ok(result)
    }

    pub async fn cancel_thumbnail(&self, path: String, size: u32) -> Result<bool, String> {
        let mtime = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::now());

        let cache_key = crate::core::cache_manager::CacheManager::generate_cache_key(
            std::path::Path::new(&path),
            mtime,
            size,
        );

        let success = self.state.task_coordinator.abort_task(&cache_key).await;
        if success {
            info!("ThumbnailService: Task cancelled for {}", path);
        }
        Ok(success)
    }
}
