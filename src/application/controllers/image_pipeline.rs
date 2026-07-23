use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;

use image::DynamicImage;
use slint::Model;

use super::gallery::GalleryController;
use crate::application::protocol::{AppCommand, AppEvent, AppResponse, EventEmitter};
use crate::application::rendering_runtime::RenderingRuntime;
use crate::application::runtime::Runtime;
use crate::ep::EngineCommand;
use crate::utils::image_utils::get_formatted_file_size;
use crate::MainWindow;

/// Direction for sequential image navigation.
enum NavigationDirection {
    Forward,
    Backward,
}

pub struct ImagePipelineController {
    pub ui: slint::Weak<MainWindow>,
    runtime: Arc<Runtime>,
    rendering_runtime: Arc<RenderingRuntime>,
    gallery: Arc<GalleryController>,
    current_load_id: Arc<std::sync::atomic::AtomicU64>,
    current_load_id_compare: Arc<std::sync::atomic::AtomicU64>,
}

impl ImagePipelineController {
    pub fn new(
        ui: slint::Weak<MainWindow>,
        runtime: Arc<Runtime>,
        rendering_runtime: Arc<RenderingRuntime>,
        gallery: Arc<GalleryController>,
    ) -> Self {
        Self {
            ui,
            runtime,
            rendering_runtime,
            gallery,
            current_load_id: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            current_load_id_compare: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    // ── Navigation ───────────────────────────────────────────────

    pub async fn display_image_from_path(&self, path: String) {
        let ui_weak = self.ui.clone();
        let tx = self.rendering_runtime.get_command_sender();
        let rr = self.rendering_runtime.clone();
        let gallery = self.gallery.clone();

        rr.update_file_size(get_formatted_file_size(&path));
        self.runtime.state.record_last_viewed(Path::new(&path));
        Self::set_current_path_and_gallery(ui_weak.clone(), path.clone(), gallery.clone());

        // Auto-rebuild layout if the user navigated to a different directory in continuous long image mode
        let is_long_mode = if let Some(ui) = ui_weak.upgrade() {
            ui.get_long_image_mode_active()
        } else {
            false
        };
        if is_long_mode {
            let rr_clone = rr.clone();
            let runtime_clone = self.runtime.clone();
            let path_clone = path.clone();
            tokio::spawn(async move {
                let parent = Path::new(&path_clone).parent();
                let should_scan = {
                    if rr_clone.is_continuous_images_empty() {
                        true
                    } else if let Some(first_path) = rr_clone.first_continuous_image_parent() {
                        Some(first_path.as_path()) != parent
                    } else {
                        true
                    }
                };

                if should_scan {
                    let paths = if let Ok(fm) = runtime_clone.state.file_manager.lock() {
                        fm.get_all_image_paths().clone()
                    } else {
                        Vec::new()
                    };

                    let mut images = Vec::new();
                    for path_buf in paths {
                        let path_str = path_buf.clone();
                        let size =
                            if crate::core::image_loader::parse_archive_path(&path_str).is_some() {
                                glam::Vec2::new(1000.0, 1000.0)
                            } else {
                                image::image_dimensions(std::path::Path::new(&path_buf))
                                    .map(|d| glam::Vec2::new(d.0 as f32, d.1 as f32))
                                    .unwrap_or(glam::Vec2::new(1000.0, 1000.0))
                            };

                        images.push(crate::ep::physics::ContinuousImageInfo {
                            path: std::path::PathBuf::from(path_buf),
                            original_size: size,
                            y_offset: 0.0,
                            scaled_height: 0.0,
                        });
                    }
                    rr_clone.set_continuous_images(images, Some(path_clone));
                }
            });
        }

        let load_id = self
            .current_load_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        Self::spawn_load_and_display(
            path,
            tx,
            ui_weak.clone(),
            rr,
            Some(gallery),
            self.runtime.state.clone(),
            load_id,
            self.current_load_id.clone(),
        );

        let compare_active = if let Some(ui) = ui_weak.upgrade() {
            ui.get_compare_mode_active()
        } else {
            false
        };

        if compare_active {
            let compare_path_opt = if let Ok(fm) = self.runtime.state.file_manager.lock() {
                let current_idx = fm.current_index();
                let total = fm.total_count();
                if total > 1 {
                    let compare_idx = (current_idx + 1) % total;
                    fm.get_all_image_paths().get(compare_idx).cloned()
                } else {
                    fm.current().map(|p| p.to_string_lossy().to_string())
                }
            } else {
                None
            };
            if let Some(compare_path) = compare_path_opt {
                self.display_compare_image(compare_path);
            }
        }
    }

    pub fn display_compare_image(&self, path: String) {
        let ui_weak = self.ui.clone();
        let tx = self.rendering_runtime.command_compare_tx.clone();
        let rr = self.rendering_runtime.clone();
        let load_id = self
            .current_load_id_compare
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;

        Self::spawn_load_compare_and_display(
            path,
            tx,
            ui_weak,
            rr,
            self.runtime.state.clone(),
            load_id,
            self.current_load_id_compare.clone(),
        );
    }

    pub async fn next_image(&self) {
        self.navigate_image(NavigationDirection::Forward).await;
    }

    pub async fn prev_image(&self) {
        self.navigate_image(NavigationDirection::Backward).await;
    }

    /// Unified navigation: loads the next or previous image depending on `direction`.
    async fn navigate_image(&self, direction: NavigationDirection) {
        let ui_weak = self.ui.clone();
        let nav = self.runtime.navigation_service.clone();
        let tx = self.rendering_runtime.get_command_sender();
        let rr = self.rendering_runtime.clone();
        let gallery = self.gallery.clone();

        let _start_total = std::time::Instant::now();

        let nav_result = match direction {
            NavigationDirection::Forward => nav.get_next_image().await,
            NavigationDirection::Backward => nav.get_prev_image().await,
        };
        let label = match direction {
            NavigationDirection::Forward => "next",
            NavigationDirection::Backward => "prev",
        };

        match nav_result {
            Ok(Some(res)) => {
                let path = res.path.clone();
                rr.update_file_size(get_formatted_file_size(&path));
                self.runtime.state.record_last_viewed(Path::new(&path));
                Self::set_current_path_and_gallery(ui_weak.clone(), path.clone(), gallery.clone());
                let load_id = self
                    .current_load_id
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                Self::spawn_load_and_display(
                    path,
                    tx,
                    ui_weak,
                    rr,
                    Some(gallery),
                    self.runtime.state.clone(),
                    load_id,
                    self.current_load_id.clone(),
                );
            }
            Ok(None) => {
                tracing::info!("No {} image found", label);
            }
            Err(e) => {
                // --- Manga Mode Interception and Integration ---
                if e.starts_with("Entering next folder:")
                    || e.starts_with("Entering previous folder:")
                {
                    let is_forward = e.starts_with("Entering next folder:");

                    // Extract path: handles Windows/Unix backslashes and double quotes
                    let raw_path = if is_forward {
                        e.trim_start_matches("Entering next folder:").trim()
                    } else {
                        e.trim_start_matches("Entering previous folder:").trim()
                    };
                    let target_path_str = raw_path.trim_matches('"').to_string();

                    // 1. Show UI HUD Panel
                    if let Some(ui) = ui_weak.upgrade() {
                        let msg = if is_forward {
                            "正在进入下一话..."
                        } else {
                            "正在返回上一话..."
                        };
                        ui.set_status_text(msg.into());
                        ui.set_manga_hud_visible(true);

                        // Asynchronously hide HUD after dynamic settings delay
                        let ui_clone = ui_weak.clone();
                        tokio::spawn(async move {
                            let delay = crate::utils::settings::get_settings().hud_dismiss_delay_ms;
                            tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui_inst) = ui_clone.upgrade() {
                                    ui_inst.set_manga_hud_visible(false);
                                }
                            });
                        });
                    }

                    // 2. Open sibling target folder asynchronously on a background task
                    let runtime_clone = self.runtime.clone();
                    let rr_clone = self.rendering_runtime.clone();
                    let tx_clone = self.rendering_runtime.get_command_sender();
                    let gallery_clone = self.gallery.clone();
                    let ui_weak_clone = ui_weak.clone();
                    let current_load_id_clone = self.current_load_id.clone();

                    tokio::spawn(async move {
                        tracing::info!(
                            "Manga Mode: Transitioning directory to: {}",
                            target_path_str
                        );

                        // Async directory scanner call via FileService
                        let res = runtime_clone
                            .file_service
                            .open_target(target_path_str)
                            .await;
                        match res {
                            Ok(response) => {
                                use crate::application::file_service::OpenTargetResponse;
                                match response {
                                    OpenTargetResponse::Gallery(dir_path) => {
                                        // The backend has completed pre-scanning the directory during open_file.
                                        // Acquire the current focused image path Buf.
                                        let mut final_path_opt = runtime_clone
                                            .state
                                            .with_file_manager(|manager| manager.current())
                                            .unwrap_or(None);

                                        // 3. Pointer Correction: For backward navigation, redirect to directory end
                                        if !is_forward {
                                            let relocation_successful = {
                                                let mut check = false;
                                                if runtime_clone
                                                    .state
                                                    .with_file_manager_mut(|manager| {
                                                        let total = manager.total_count();
                                                        if total > 0 {
                                                            manager.update_state(
                                                                manager.get_all_infos().clone(),
                                                                total - 1, // Focus cursor on last image
                                                                manager
                                                                    .current_directory()
                                                                    .cloned()
                                                                    .unwrap(),
                                                            );
                                                            check = true;
                                                        }
                                                    })
                                                    .is_ok()
                                                {
                                                    check
                                                } else {
                                                    false
                                                }
                                            };

                                            if relocation_successful {
                                                final_path_opt = runtime_clone
                                                    .state
                                                    .with_file_manager(|manager| manager.current())
                                                    .unwrap_or(None);
                                            }
                                        }

                                        if let Some(path_buf) = final_path_opt {
                                            let final_path_str =
                                                path_buf.to_string_lossy().to_string();

                                            // Synchronize rendering UI state
                                            rr_clone.set_current_path(path_buf.clone());
                                            rr_clone.set_file_size(get_formatted_file_size(
                                                &final_path_str,
                                            ));
                                            Self::set_ui_image_path(
                                                ui_weak_clone.clone(),
                                                final_path_str.clone(),
                                            );

                                            let load_id = current_load_id_clone
                                                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                                                + 1;

                                            // Trigger WGPU tiling asset upload
                                            Self::spawn_load_and_display(
                                                final_path_str,
                                                tx_clone,
                                                ui_weak_clone.clone(),
                                                rr_clone.clone(),
                                                Some(gallery_clone.clone()),
                                                runtime_clone.state.clone(),
                                                load_id,
                                                current_load_id_clone.clone(),
                                            );
                                        }

                                        // 4. Synchronize Asset Browser Thumbnail Gallery List
                                        let mut all_assets = Vec::new();
                                        let gallery_inner = gallery_clone.clone();
                                        let _ = runtime_clone
                                            .asset_browser_service
                                            .trigger_recursive_scan(
                                                dir_path,
                                                Some(1),
                                                move |batch| {
                                                    all_assets.extend(batch);
                                                    gallery_inner.update_thumbnails_model(
                                                        all_assets.clone(),
                                                    );
                                                },
                                                || {},
                                            )
                                            .await;
                                    }
                                    OpenTargetResponse::Viewer(info) => {
                                        // Handled primarily by normal single-file click pathways.
                                        // For safety, synchronizing Viewer response as fallback:
                                        rr_clone
                                            .set_current_path(std::path::PathBuf::from(&info.path));
                                        rr_clone.set_file_size(get_formatted_file_size(&info.path));
                                        Self::set_ui_image_path(
                                            ui_weak_clone.clone(),
                                            info.path.clone(),
                                        );

                                        let load_id = current_load_id_clone
                                            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                                            + 1;

                                        Self::spawn_load_and_display(
                                            info.path,
                                            tx_clone,
                                            ui_weak_clone.clone(),
                                            rr_clone.clone(),
                                            Some(gallery_clone.clone()),
                                            runtime_clone.state.clone(),
                                            load_id,
                                            current_load_id_clone.clone(),
                                        );
                                    }
                                }
                            }
                            Err(err) => {
                                tracing::error!("Manga Mode target switch failed: {}", err);
                            }
                        }
                    });
                } else {
                    tracing::error!("Failed to get {} image: {}", label, e);
                }
            }
        }
    }

    // ── Open Target ──────────────────────────────────────────────

    pub async fn open_target(&self, path_str: String, emitter: Arc<dyn EventEmitter>) {
        let target_path = self.resolve_target_path(path_str, &emitter).await;
        if target_path.is_empty() {
            tracing::info!("Target path is empty, aborting open.");
            return;
        }

        tracing::info!("Calling open_target for: '{}'", target_path);
        let res = self
            .runtime
            .file_service
            .open_target(target_path.clone())
            .await;
        match res {
            Ok(response) => self.dispatch_open_response(response).await,
            Err(e) => {
                if e.contains("Password required") {
                    let _ = emitter.emit(AppEvent::PasswordRequired { path: target_path });
                } else {
                    tracing::error!("open_target failed: {}", e);
                }
            }
        }
    }

    /// Resolve the actual filesystem path from user intent.
    /// Handles empty string (file dialog), "folder" (folder dialog), or passthrough.
    async fn resolve_target_path(
        &self,
        path_str: String,
        emitter: &Arc<dyn EventEmitter>,
    ) -> String {
        if path_str.is_empty() {
            tracing::info!("Requesting OpenFileDialog...");
            match self
                .runtime
                .dispatch(
                    AppCommand::OpenFileDialog { multiple: false },
                    Some(emitter.clone()),
                )
                .await
            {
                AppResponse::Json(val) => val.as_str().map(|s| s.to_string()).unwrap_or_default(),
                res => {
                    tracing::warn!("OpenFileDialog returned unexpected response: {:?}", res);
                    String::new()
                }
            }
        } else if path_str == "folder" {
            tracing::info!("Requesting OpenFolderDialog...");
            match self
                .runtime
                .dispatch(AppCommand::OpenFolderDialog, Some(emitter.clone()))
                .await
            {
                AppResponse::StringOption(Some(p)) => p,
                res => {
                    tracing::info!(
                        "OpenFolderDialog cancelled or returned unexpected: {:?}",
                        res
                    );
                    String::new()
                }
            }
        } else {
            path_str
        }
    }

    /// Route the open-target response to the correct mode handler.
    async fn dispatch_open_response(
        &self,
        response: crate::application::file_service::OpenTargetResponse,
    ) {
        use crate::application::file_service::OpenTargetResponse;
        match response {
            OpenTargetResponse::Viewer(info) => {
                self.open_in_viewer_mode(&info).await;
            }
            OpenTargetResponse::Gallery(dir) => {
                self.open_in_gallery_mode(dir).await;
            }
        }
    }

    /// Handle Viewer mode: load the image, sync state, and scan gallery if needed.
    async fn open_in_viewer_mode(&self, info: &crate::models::ImageInfo) {
        let rr = self.rendering_runtime.clone();
        let tx = self.rendering_runtime.get_command_sender();
        let ui_weak = self.ui.clone();
        let gallery = self.gallery.clone();

        // Sync path, file size, and UI path label
        rr.set_current_path(std::path::PathBuf::from(&info.path));
        rr.set_file_size(get_formatted_file_size(&info.path));
        self.runtime.state.record_last_viewed(Path::new(&info.path));
        Self::set_ui_image_path(ui_weak.clone(), info.path.clone());

        // Async image load → GPU upload
        let path = info.path.clone();
        rr.update_file_size(get_formatted_file_size(&path));
        let load_id = self
            .current_load_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        Self::spawn_load_and_display(
            path.clone(),
            tx,
            ui_weak.clone(),
            rr,
            Some(gallery.clone()),
            self.runtime.state.clone(),
            load_id,
            self.current_load_id.clone(),
        );

        // Trigger gallery scan for parent directory if it changed
        self.scan_gallery_if_directory_changed(&info.path, gallery)
            .await;
    }

    /// Handle Gallery mode: recursive scan with first-image auto-load.
    async fn open_in_gallery_mode(&self, folder_path: String) {
        tracing::info!("Gallery mode requested for: {}", folder_path);

        let ui_weak = self.ui.clone();
        let tx = self.rendering_runtime.get_command_sender();
        let rr = self.rendering_runtime.clone();
        let gallery = self.gallery.clone();
        let runtime = self.runtime.clone();
        let state = runtime.state.clone();

        // 统一寻找上次查看的历史位置
        let last_viewed = {
            let map = runtime.state.folder_last_viewed.lock().unwrap();
            let norm_folder = crate::utils::path_utils::normalize_native_path(
                std::path::PathBuf::from(&folder_path),
            )
            .to_string_lossy()
            .to_string();
            map.get(&norm_folder).cloned()
        };

        let last_viewed_exists = last_viewed
            .as_ref()
            .map(|p| std::path::Path::new(p).exists())
            .unwrap_or(false);

        // 如果上次查看位置有效，我们直接提前并强制触发其大图加载以实现极致响应
        let target_image_to_load = if last_viewed_exists {
            if let Some(ref path_str) = last_viewed {
                let path = path_str.clone();
                let ui_c = ui_weak.clone();
                let tx_c = tx.clone();
                let rr_c = rr.clone();
                let state_clone = state.clone();
                let load_id = self
                    .current_load_id
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                let current_load_id_clone = self.current_load_id.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    Self::set_ui_image_path(ui_c.clone(), path.clone());
                    rr_c.update_file_size(get_formatted_file_size(&path));
                    Self::spawn_load_and_display(
                        path,
                        tx_c,
                        ui_c,
                        rr_c,
                        None,
                        state_clone,
                        load_id,
                        current_load_id_clone,
                    );
                });
            }
            last_viewed
        } else {
            None
        };

        let current_load_id_spawn = self.current_load_id.clone();
        tokio::spawn(async move {
            let mut all_assets = Vec::new();
            let gallery_inner = gallery.clone();
            let state_clone = state.clone();
            let tx_clone = tx.clone();
            let rr_clone = rr.clone();
            let ui_weak_clone = ui_weak.clone();
            let current_load_id_clone = current_load_id_spawn.clone();

            // 是否已完成此目录的大图强装
            let mut has_loaded_image = target_image_to_load.is_some();

            let _ = runtime
                .asset_browser_service
                .trigger_recursive_scan(
                    folder_path,
                    Some(1),
                    move |batch| {
                        all_assets.extend(batch);
                        let current_assets = all_assets.clone();
                        let gallery_batch = gallery_inner.clone();

                        if !has_loaded_image && !current_assets.is_empty() {
                            // 无历史记录时，在第一批资产到达时强制加载首张图片
                            let first_path = current_assets[0].path.clone();
                            let ui_c = ui_weak_clone.clone();
                            let tx_c = tx_clone.clone();
                            let rr_c = rr_clone.clone();
                            let state_clone_inner = state_clone.clone();
                            let current_load_id_inner = current_load_id_clone.clone();
                            let load_id = current_load_id_inner
                                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                                + 1;
                            let _ = slint::invoke_from_event_loop(move || {
                                Self::set_ui_image_path(ui_c.clone(), first_path.clone());
                                rr_c.update_file_size(get_formatted_file_size(&first_path));
                                Self::spawn_load_and_display(
                                    first_path,
                                    tx_c,
                                    ui_c,
                                    rr_c,
                                    None,
                                    state_clone_inner,
                                    load_id,
                                    current_load_id_inner,
                                );
                            });
                            has_loaded_image = true;
                        }

                        gallery_batch.update_thumbnails_model(current_assets);
                    },
                    || {},
                )
                .await;
        });
    }

    // ── Delete ───────────────────────────────────────────────────

    pub async fn delete_current(&self, emitter: Arc<dyn EventEmitter>) {
        let ui_weak = self.ui.clone();
        let tx = self.rendering_runtime.get_command_sender();
        let rr = self.rendering_runtime.clone();

        match self
            .runtime
            .dispatch(AppCommand::DeleteCurrentFile, Some(emitter))
            .await
        {
            AppResponse::ImageInfoOption(Some(info)) => {
                let path = info.path.clone();
                rr.update_file_size(get_formatted_file_size(&path));
                self.runtime.state.record_last_viewed(Path::new(&path));

                let load_id = self
                    .current_load_id
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                Self::spawn_load_and_display(
                    path.clone(),
                    tx,
                    ui_weak.clone(),
                    rr,
                    None,
                    self.runtime.state.clone(),
                    load_id,
                    self.current_load_id.clone(),
                );
                Self::set_ui_image_path(ui_weak, path);
            }
            AppResponse::ImageInfoOption(None) => {
                Self::set_ui_image_path(ui_weak, String::new());
            }
            _ => {}
        }
    }

    // ── Shared Helpers (Private) ─────────────────────────────────

    /// Try to load an image from the preload cache, falling back to disk.
    #[allow(dead_code)]
    fn resolve_from_cache_or_open(
        &self,
        path: &str,
    ) -> Result<(DynamicImage, u32, u32), image::ImageError> {
        let cached = self
            .runtime
            .state
            .preload_cache
            .lock()
            .ok()
            .and_then(|mut cache| cache.get(path));

        if let Some(img) = cached {
            tracing::info!("Preload Cache HIT for: {}", path);
            let w = img.width();
            let h = img.height();
            Ok(((*img).clone(), w, h))
        } else {
            tracing::info!("Preload Cache MISS for: {}", path);

            // Try Windows WIC Shell Cache
            if let Some((pixels, w, h)) = crate::core::wic_cache::extract_wic_thumbnail(path, 1024)
            {
                let dims = image::image_dimensions(path).unwrap_or((w, h));
                if let Some(buf) = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(w, h, pixels)
                {
                    return Ok((DynamicImage::ImageRgba8(buf), dims.0, dims.1));
                }
            }

            // Try libjpeg-turbo 1/8 Scale-on-Decode
            let is_jpeg =
                path.to_lowercase().ends_with(".jpg") || path.to_lowercase().ends_with(".jpeg");
            if is_jpeg {
                if let Ok(decoder) = crate::ep::decoder::TiledDecoder::new(Path::new(path)) {
                    let (w, h) = decoder.dimensions();
                    if let Ok(preview_img) = decoder.decode_fast_preview(1024, 1024) {
                        return Ok((preview_img, w, h));
                    }
                }
            }

            // Fallback to image::open
            match image::open(path) {
                Ok(mut img) => {
                    let w = img.width();
                    let h = img.height();
                    if w > 2048 || h > 2048 {
                        img = img.thumbnail(2048, 2048);
                    }
                    Ok((img, w, h))
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to open image '{}' directly, trying fast safe proxy. Error: {:?}",
                        path,
                        e
                    );
                    match crate::core::image_generator::generate_thumbnail(
                        path,
                        crate::utils::settings::get_settings().fallback_size,
                    ) {
                        Ok(info) => {
                            if let Some(buf) = info.pixel_buffer {
                                let orig_w = info.original_width.unwrap_or(buf.width());
                                let orig_h = info.original_height.unwrap_or(buf.height());
                                let rgba_img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
                                    buf.width(),
                                    buf.height(),
                                    buf.as_bytes().to_vec(),
                                )
                                .ok_or_else(|| {
                                    image::ImageError::Limits(image::error::LimitError::from_kind(
                                        image::error::LimitErrorKind::DimensionError,
                                    ))
                                })?;
                                Ok((DynamicImage::ImageRgba8(rgba_img), orig_w, orig_h))
                            } else {
                                Err(e)
                            }
                        }
                        Err(_) => Err(e),
                    }
                }
            }
        }
    }
    /// Spawn a background task that opens an image, pushes RGBA to the UI, and
    /// uploads the full image to the GPU via the engine command channel.
    #[allow(clippy::too_many_arguments)]
    fn spawn_load_and_display(
        path: String,
        tx: mpsc::Sender<EngineCommand>,
        ui_weak: slint::Weak<MainWindow>,
        rr: Arc<RenderingRuntime>,
        gallery: Option<Arc<GalleryController>>,
        state: Arc<crate::state::AppState>,
        load_id: u64,
        current_load_id: Arc<std::sync::atomic::AtomicU64>,
    ) {
        Self::spawn_load_and_display_impl(
            path,
            tx,
            ui_weak,
            rr,
            gallery,
            state,
            load_id,
            current_load_id,
            false,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_load_compare_and_display(
        path: String,
        tx: mpsc::Sender<EngineCommand>,
        ui_weak: slint::Weak<MainWindow>,
        rr: Arc<RenderingRuntime>,
        state: Arc<crate::state::AppState>,
        load_id: u64,
        current_load_id: Arc<std::sync::atomic::AtomicU64>,
    ) {
        Self::spawn_load_and_display_impl(
            path,
            tx,
            ui_weak,
            rr,
            None,
            state,
            load_id,
            current_load_id,
            true,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_load_and_display_impl(
        path: String,
        tx: mpsc::Sender<EngineCommand>,
        ui_weak: slint::Weak<MainWindow>,
        rr: Arc<RenderingRuntime>,
        gallery: Option<Arc<GalleryController>>,
        state: Arc<crate::state::AppState>,
        load_id: u64,
        current_load_id: Arc<std::sync::atomic::AtomicU64>,
        is_compare: bool,
    ) {
        tokio::spawn(async move {
            let check_canceled =
                || current_load_id.load(std::sync::atomic::Ordering::SeqCst) != load_id;

            if check_canceled() {
                return;
            }

            let start_time = std::time::Instant::now();
            let is_archive = crate::core::image_loader::parse_archive_path(&path).is_some();
            let mut orig_w = 0u32;
            let mut orig_h = 0u32;
            let mut final_base_img: Option<image::DynamicImage> = None;

            // ── 优先从 Preload Cache 命中加载 ──
            {
                let cached = state
                    .preload_cache
                    .lock()
                    .ok()
                    .and_then(|mut cache| cache.get(&path).map(|img| (*img).clone()));
                if let Some(img) = cached {
                    orig_w = img.width();
                    orig_h = img.height();
                    final_base_img = Some(img);
                    tracing::info!("Preload Cache HIT (Active Load) for: {}", path);
                }
            }

            if !is_archive {
                #[cfg(target_os = "windows")]
                {
                    // ---------- PHASE 1: WIC 流式按需架构 (Base Texture 秒开) ----------
                    let path_obj = std::path::Path::new(&path);
                    match crate::ep::wic_decoder::WicDecoder::new(path_obj) {
                        Ok(decoder) => {
                            let (w, h) = decoder.dimensions();
                            orig_w = w;
                            orig_h = h;

                            let (viewport_w, viewport_h) = rr.get_viewport_size();
                            let fallback_size = viewport_w.max(viewport_h).max(2048);

                            if let Ok((scaled_w, scaled_h, buffer)) =
                                decoder.get_scaled_preview(fallback_size)
                            {
                                if check_canceled() {
                                    return;
                                }
                                if let Some(rgba_buf) =
                                    image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
                                        scaled_w, scaled_h, buffer,
                                    )
                                {
                                    let mut preview_img = image::DynamicImage::ImageRgba8(rgba_buf);
                                    crate::utils::color_space::correct_image_colors(
                                        &path,
                                        &mut preview_img,
                                        &state,
                                    );
                                    final_base_img = Some(preview_img);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "WIC Decoder fail for {}: {}, fallback to image crate",
                                path,
                                e
                            );
                        }
                    }
                }
            }

            // ---------- 降级兼容：压缩包或 WIC 失败时的传统全像素加载 ----------
            if final_base_img.is_none() {
                if check_canceled() {
                    return;
                }
                let path_clone = path.clone();
                let current_load_id_blocking = current_load_id.clone();
                let state_clone = state.clone();

                let decode_res = tokio::task::spawn_blocking(move || {
                    if current_load_id_blocking.load(std::sync::atomic::Ordering::SeqCst) != load_id
                    {
                        return Err(anyhow::anyhow!("Canceled"));
                    }

                    let max_alloc = crate::utils::image_utils::get_dynamic_decode_limit();

                    if let Some((archive_path, entry_name)) =
                        crate::core::image_loader::parse_archive_path(&path_clone)
                    {
                        if let Ok(bytes) = crate::core::image_loader::load_archive_image_bytes(
                            archive_path,
                            entry_name,
                        ) {
                            let mut img = image::load_from_memory(&bytes)?;
                            crate::utils::color_space::correct_image_colors(
                                &path_clone,
                                &mut img,
                                &state_clone,
                            );
                            let w = img.width();
                            let h = img.height();
                            return Ok::<_, anyhow::Error>((img, w, h));
                        }
                    } else {
                        let mut reader = image::ImageReader::open(&path_clone)?;
                        let mut limits = image::Limits::default();
                        limits.max_alloc = Some(max_alloc);
                        reader.limits(limits);
                        let mut img = reader.decode()?;
                        crate::utils::color_space::correct_image_colors(
                            &path_clone,
                            &mut img,
                            &state_clone,
                        );
                        let w = img.width();
                        let h = img.height();
                        return Ok::<_, anyhow::Error>((img, w, h));
                    }
                    Err(anyhow::anyhow!("Fallback decode failed"))
                })
                .await;

                if check_canceled() {
                    return;
                }

                if let Ok(Ok((img, w, h))) = decode_res {
                    orig_w = w;
                    orig_h = h;
                    final_base_img = Some(img);
                } else {
                    tracing::error!("Failed to decode image '{}'", path);
                    return;
                }
            }

            // ---------- 同步到 UI 与 GPU ----------
            if let Some(img) = final_base_img {
                if check_canceled() {
                    return;
                }

                let rgba = img.to_rgba8();
                let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                );

                let exif_str = if !is_compare {
                    crate::core::image_loader::extract_exif_formatted(std::path::Path::new(&path))
                        .unwrap_or_else(|| "No EXIF Data".to_string())
                } else {
                    String::new()
                };

                let ui_weak_update = ui_weak.clone();
                let path_update = path.clone();
                let gallery_update = gallery.clone();
                if check_canceled() {
                    return;
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak_update.upgrade() {
                        if is_compare {
                            ui.set_current_image_compare(slint::Image::from_rgba8(buffer));
                        } else {
                            ui.set_current_image(slint::Image::from_rgba8(buffer));
                            ui.set_exif_hud_text(exif_str.into());
                            if let Some(g) = gallery_update.as_ref() {
                                g.update_active_item(path_update);
                            }
                        }
                    }
                });

                if check_canceled() {
                    return;
                }
                if is_compare {
                    rr.set_current_compare_path(std::path::PathBuf::from(&path));
                    rr.update_compare_image_size(orig_w as f32, orig_h as f32);
                } else {
                    rr.set_current_path(std::path::PathBuf::from(&path));
                    rr.update_image_size(orig_w as f32, orig_h as f32);
                }
                let _ = tx.send(EngineCommand::UploadBase {
                    image: img,
                    path: std::path::PathBuf::from(&path),
                    physical_width: orig_w,
                    physical_height: orig_h,
                });

                tracing::info!(
                    event = "performance_metrics",
                    path = ?path,
                    total_ms = start_time.elapsed().as_secs_f64() * 1000.0,
                    "Image Base Texture loaded (WIC Stream Architecture)"
                );

                // ---------- PHASE 2: VRAM Streaming (Zero-Loss 144Hz Panning) ----------
                // Pre-fetch all tiles directly into GPU VRAM if within capacity (<= 512 tiles / 2GB VRAM).
                // Use a single sequential background decode instead of random WIC extraction to avoid JPEG sequential scan bottlenecks.
                if !is_archive && orig_w > 0 && orig_h > 0 {
                    let settings = crate::utils::settings::get_settings();
                    let tile_size = settings.tile_size;
                    let end_tx = orig_w.div_ceil(tile_size);
                    let end_ty = orig_h.div_ceil(tile_size);
                    let total_tiles = end_tx * end_ty;

                    if total_tiles <= 512 {
                        let path_for_bg = path.clone();
                        let tx_for_bg = tx.clone();
                        let state_for_bg = state.clone();
                        let current_load_id_bg = current_load_id.clone();

                        tokio::task::spawn_blocking(move || {
                            tracing::info!("VRAM Streaming Activated: Silently pre-fetching {} tiles into VRAM via sequential decode", total_tiles);

                            // 1. Single sequential decode (Fast for JPEGs)
                            let mut reader = match image::ImageReader::open(&path_for_bg) {
                                Ok(r) => r,
                                Err(_) => return,
                            };
                            let mut limits = image::Limits::default();
                            limits.max_alloc =
                                Some(crate::utils::image_utils::get_dynamic_decode_limit());
                            reader.limits(limits);

                            let mut img = match reader.decode() {
                                Ok(i) => i,
                                Err(_) => return,
                            };
                            crate::utils::color_space::correct_image_colors(
                                &path_for_bg,
                                &mut img,
                                &state_for_bg,
                            );
                            let mut img_rgba = img.to_rgba8();

                            // 2. Slice into tiles and upload
                            for ty in 0..end_ty {
                                for tx in 0..end_tx {
                                    // Check cancellation continuously
                                    if current_load_id_bg.load(std::sync::atomic::Ordering::SeqCst)
                                        != load_id
                                    {
                                        return;
                                    }

                                    let x_min = tx * tile_size;
                                    let y_min = ty * tile_size;
                                    let w = tile_size.min(orig_w.saturating_sub(x_min));
                                    let h = tile_size.min(orig_h.saturating_sub(y_min));

                                    if w == 0 || h == 0 {
                                        continue;
                                    }

                                    let sub_img =
                                        image::imageops::crop(&mut img_rgba, x_min, y_min, w, h)
                                            .to_image();
                                    let dyn_sub = image::DynamicImage::ImageRgba8(sub_img);

                                    let key = crate::ep::tile_cache::TileKey {
                                        lod: 0,
                                        x: tx,
                                        y: ty,
                                    };

                                    let _ = tx_for_bg.send(EngineCommand::UploadTile {
                                        key,
                                        image: dyn_sub,
                                        path: std::path::PathBuf::from(&path_for_bg),
                                    });
                                }
                            }
                            tracing::info!(
                                "VRAM Streaming: Successfully pushed {} tiles to VRAM",
                                total_tiles
                            );
                        });
                    }
                }
            }
        });
    }

    /// Set the current image path on the UI thread.
    fn set_ui_image_path(ui_weak: slint::Weak<MainWindow>, path: String) {
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_current_image_path(path.into());
            }
        });
    }

    /// Update current image path and gallery active item on the UI thread.
    fn set_current_path_and_gallery(
        ui_weak: slint::Weak<MainWindow>,
        path: String,
        gallery: Arc<GalleryController>,
    ) {
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_current_image_path(path.clone().into());
                gallery.update_active_item(path);
            }
        });
    }

    /// Check if the gallery already has thumbnails from the same directory.
    /// If not, trigger a background scan.
    async fn scan_gallery_if_directory_changed(
        &self,
        image_path: &str,
        gallery: Arc<GalleryController>,
    ) {
        let parent_dir = if let Some((_archive_path, _)) =
            crate::core::image_loader::parse_archive_path(image_path)
        {
            image_path.to_string()
        } else {
            Path::new(image_path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string())
        };

        let already_loaded = self.check_gallery_directory(&parent_dir).await;
        if already_loaded {
            gallery.update_active_item(image_path.to_string());
            return;
        }

        let gallery_scan = gallery.clone();
        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            let mut all_assets = Vec::new();
            let gallery_inner = gallery_scan.clone();
            let _ = runtime
                .asset_browser_service
                .trigger_recursive_scan(
                    parent_dir,
                    Some(1),
                    move |batch| {
                        all_assets.extend(batch);
                        gallery_inner.update_thumbnails_model(all_assets.clone());
                    },
                    || {},
                )
                .await;
        });
    }

    /// Query the UI thread to determine if the gallery's first thumbnail
    /// belongs to the given parent directory.
    async fn check_gallery_directory(&self, parent_dir: &str) -> bool {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let ui_weak = self.ui.clone();
        let parent_dir = parent_dir.to_string();

        let _ = slint::invoke_from_event_loop(move || {
            let result = ui_weak
                .upgrade()
                .and_then(|ui| ui.get_all_thumbnails().row_data(0))
                .and_then(|first| {
                    Path::new(&first.path.to_string())
                        .parent()
                        .map(|p| p.to_string_lossy() == parent_dir)
                })
                .unwrap_or(false);
            let _ = tx.blocking_send(result);
        });

        rx.recv().await.unwrap_or(false)
    }

    /// 评分并归类当前图片，画面保持不变，仅在后台异步复制
    pub async fn rate_image(&self, rating: i32) {
        let current_path = match self.runtime.state.with_file_manager(|m| m.current()) {
            Ok(Some(path)) => path,
            _ => {
                tracing::warn!("No current image viewed, ignoring rate_image");
                return;
            }
        };

        let parent_dir = match current_path.parent() {
            Some(p) => p.to_path_buf(),
            None => {
                tracing::warn!("Failed to get parent directory of current image");
                return;
            }
        };

        // 统一使用 Star_{rating} 命名方案
        let folder_name = format!("Star_{}", rating);
        let target_dir = parent_dir.join(&folder_name);

        let ui_weak = self.ui.clone();

        tokio::spawn(async move {
            if !target_dir.exists() {
                if let Err(e) = std::fs::create_dir_all(&target_dir) {
                    tracing::error!("Failed to create rating directory: {}", e);
                    return;
                }
            }

            if let Some(filename) = current_path.file_name() {
                let target_path = target_dir.join(filename);
                match std::fs::copy(&current_path, &target_path) {
                    Ok(_) => {
                        tracing::info!("Successfully rated and copied to {:?}", target_path);

                        // 主线程提示 HUD 反馈，画面保持当前图不变
                        let folder_name_clone = folder_name.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_status_text(
                                    format!("图片已归类至 {}", folder_name_clone).into(),
                                );
                                ui.set_manga_hud_visible(true);

                                // 定时自动关闭 HUD
                                let ui_clone = ui_weak.clone();
                                tokio::spawn(async move {
                                    let delay =
                                        crate::utils::settings::get_settings().hud_dismiss_delay_ms;
                                    tokio::time::sleep(tokio::time::Duration::from_millis(delay))
                                        .await;
                                    let _ = slint::invoke_from_event_loop(move || {
                                        if let Some(ui_inst) = ui_clone.upgrade() {
                                            ui_inst.set_manga_hud_visible(false);
                                        }
                                    });
                                });
                            }
                        });
                    }
                    Err(e) => {
                        tracing::error!("Failed to copy file: {}", e);
                    }
                }
            }
        });
    }

    /// 切换到对应的评分目录
    pub async fn switch_to_rating_dir(&self, rating: i32) {
        let current_dir = match self
            .runtime
            .state
            .with_file_manager(|m| m.current_directory().cloned())
        {
            Ok(Some(dir)) => dir,
            _ => {
                tracing::warn!("No current directory loaded, ignoring switch_to_rating_dir");
                return;
            }
        };

        let folder_name = format!("Star_{}", rating);

        // 区分当前是否已经在评分目录中
        let is_currently_in_rating = current_dir
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.starts_with("Star_"))
            .unwrap_or(false);

        let target_dir = if is_currently_in_rating {
            let orig_dir_lock = self.runtime.state.original_directory.lock().unwrap();
            let parent_base = match &*orig_dir_lock {
                Some(p) => p.clone(),
                None => match current_dir.parent() {
                    Some(p) => p.to_path_buf(),
                    None => {
                        tracing::warn!("No parent base available for rating navigation");
                        return;
                    }
                },
            };
            parent_base.join(&folder_name)
        } else {
            // 保存原始目录
            let mut orig_dir_lock = self.runtime.state.original_directory.lock().unwrap();
            if orig_dir_lock.is_none() {
                *orig_dir_lock = Some(current_dir.clone());
            }
            current_dir.join(&folder_name)
        };

        if !target_dir.exists() {
            // 评分目录不存在，给出 HUD 提示并不跳转
            let ui_weak = self.ui.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_status_text(format!("评分目录 {} 暂无图片", folder_name).into());
                    ui.set_manga_hud_visible(true);

                    let ui_clone = ui_weak.clone();
                    tokio::spawn(async move {
                        let delay = crate::utils::settings::get_settings().hud_dismiss_delay_ms;
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui_inst) = ui_clone.upgrade() {
                                ui_inst.set_manga_hud_visible(false);
                            }
                        });
                    });
                }
            });
            return;
        }

        // 调用 open_target 重新装载目录
        let emitter = Arc::new(crate::adapters::native_adapter::SlintEventEmitter {
            ui: self.ui.clone(),
        });
        self.open_target(target_dir.to_string_lossy().to_string(), emitter)
            .await;
    }

    /// 返回原始目录
    pub async fn go_to_original_dir(&self) {
        let orig_dir = {
            let mut orig_dir_lock = self.runtime.state.original_directory.lock().unwrap();
            orig_dir_lock.take()
        };

        if let Some(path) = orig_dir {
            let emitter = Arc::new(crate::adapters::native_adapter::SlintEventEmitter {
                ui: self.ui.clone(),
            });
            self.open_target(path.to_string_lossy().to_string(), emitter)
                .await;
        } else {
            // 已经在原始目录，提示
            let ui_weak = self.ui.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_status_text("当前已在原始目录".into());
                    ui.set_manga_hud_visible(true);

                    let ui_clone = ui_weak.clone();
                    tokio::spawn(async move {
                        let delay = crate::utils::settings::get_settings().hud_dismiss_delay_ms;
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui_inst) = ui_clone.upgrade() {
                                ui_inst.set_manga_hud_visible(false);
                            }
                        });
                    });
                }
            });
        }
    }
}
