use crate::application::protocol::{AppCommand, AppResponse, EventEmitter};
use crate::application::rendering_runtime::RenderingRuntime;
use crate::application::runtime::Runtime;
use crate::MainWindow;
use i_slint_backend_winit::WinitWindowAccessor;
use slint::{ComponentHandle, Model};
use std::sync::Arc;

use crate::application::controllers::gallery::GalleryController;
use crate::application::controllers::image_pipeline::ImagePipelineController;
use crate::application::controllers::l10n::L10nManager;
use crate::application::controllers::workshop::WorkshopController;

pub struct AppController {
    ui: slint::Weak<MainWindow>,
    runtime: Arc<Runtime>,
    rendering_runtime: Arc<RenderingRuntime>,
    last_scroll_nav_time: std::sync::Mutex<std::time::Instant>,
    pub channel_mode: std::sync::atomic::AtomicU32,

    // Sub-controllers
    pub gallery: Arc<GalleryController>,
    pub image_pipeline: Arc<ImagePipelineController>,
    pub workshop: Arc<WorkshopController>,
    pub l10n: Arc<L10nManager>,
}

impl AppController {
    pub fn new(
        ui: slint::Weak<MainWindow>,
        runtime: Arc<Runtime>,
        rendering_runtime: Arc<RenderingRuntime>,
    ) -> Arc<Self> {
        let gallery = Arc::new(GalleryController::new(ui.clone(), runtime.clone()));
        let image_pipeline = Arc::new(ImagePipelineController::new(
            ui.clone(),
            runtime.clone(),
            rendering_runtime.clone(),
            gallery.clone(),
        ));
        let workshop = Arc::new(WorkshopController::new(ui.clone(), runtime.clone()));
        let l10n = Arc::new(L10nManager::new(ui.clone(), runtime.clone()));

        Arc::new(Self {
            ui,
            runtime,
            rendering_runtime,
            channel_mode: std::sync::atomic::AtomicU32::new(0),
            last_scroll_nav_time: std::sync::Mutex::new(
                std::time::Instant::now() - std::time::Duration::from_secs(1),
            ),
            gallery,
            image_pipeline,
            workshop,
            l10n,
        })
    }

    // --- Sub-controllers forwarding ---
    pub fn update_active_item(&self, current_path: String) {
        self.gallery.update_active_item(current_path);
    }

    pub fn update_thumbnails_model(&self, assets: Vec<crate::core::asset_browser::AssetInfo>) {
        self.gallery.update_thumbnails_model(assets);
    }

    pub async fn handle_next_image(&self) {
        self.image_pipeline.next_image().await;
    }

    pub async fn handle_prev_image(&self) {
        self.image_pipeline.prev_image().await;
    }

    pub async fn handle_open_target(&self, path_str: String, emitter: Arc<dyn EventEmitter>) {
        self.image_pipeline.open_target(path_str, emitter).await;
    }

    pub async fn handle_delete_current(&self, emitter: Arc<dyn EventEmitter>) {
        self.image_pipeline.delete_current(emitter).await;
    }

    pub fn handle_workshop_batch_convert(&self, format_str: String) {
        self.workshop.batch_convert(format_str);
    }

    pub fn handle_workshop_stitch(&self, dir_str: String, gap: i32) {
        self.workshop.stitch(dir_str, gap);
    }

    pub fn handle_switch_language(&self) {
        self.l10n.switch_language();
    }

    pub fn handle_apply_initial_language(&self) {
        self.l10n.apply_initial_language();
    }

    pub fn runtime(&self) -> &Arc<Runtime> {
        &self.runtime
    }

    pub fn rendering_runtime(&self) -> &Arc<RenderingRuntime> {
        &self.rendering_runtime
    }

    pub fn handle_toggle_manga_mode(&self) {
        let new_state = {
            let mut mode = self
                .runtime
                .state
                .manga_mode_enabled
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *mode = !*mode;
            *mode
        };

        // Persist view mode in settings
        let mut settings = crate::utils::settings::get_settings();
        settings.default_view_mode = if new_state {
            "manga".to_string()
        } else {
            "single".to_string()
        };
        let _ = crate::utils::settings::update_settings(settings);

        // Sync active state back to UI property purely via event loop
        let ui_weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_manga_mode_active(new_state);

                // Render highly professional HUD status animation feedback
                ui.set_status_text(
                    if new_state {
                        "漫画模式已开启"
                    } else {
                        "漫画模式已关闭"
                    }
                    .into(),
                );
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

    pub fn handle_asset_search(&self, text: String) {
        self.gallery.search_assets(text);
    }

    pub fn handle_asset_filter(&self, format: String) {
        self.gallery.filter_assets(format);
    }

    pub fn handle_exif_filter(
        &self,
        min_focal: Option<f32>,
        max_focal: Option<f32>,
        min_ap: Option<f32>,
        max_ap: Option<f32>,
        min_iso: Option<u32>,
        max_iso: Option<u32>,
        camera: Option<String>,
    ) {
        let filtered_paths = if let Ok(mut fm) = self.runtime.state.file_manager.lock() {
            fm.filter_assets(
                min_focal, max_focal, min_ap, max_ap, min_iso, max_iso, camera,
            );
            let paths: std::collections::HashSet<String> =
                fm.get_all_image_paths().into_iter().collect();

            if !paths.is_empty() {
                let current_path = fm.current().map(|p| p.to_string_lossy().to_string());
                if let Some(ref path) = current_path {
                    if !paths.contains(path) {
                        if let Some(first_path) = fm.get_all_image_paths().first() {
                            let first_path_clone = first_path.clone();
                            let pipeline = self.image_pipeline.clone();
                            tokio::spawn(async move {
                                pipeline.display_image_from_path(first_path_clone).await;
                            });
                        }
                    }
                }
            }
            paths
        } else {
            return;
        };

        self.gallery.apply_exif_filter_to_thumbnails(filtered_paths);
    }

    pub fn show_hud_message(&self, message: &str) {
        let ui_weak = self.ui.clone();
        let msg = message.to_string();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_status_text(msg.clone().into());
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

    pub fn handle_set_channel_mode(&self, mode: u32) {
        self.channel_mode
            .store(mode, std::sync::atomic::Ordering::SeqCst);
        let sender = self.rendering_runtime.get_command_sender();
        let _ = sender.send(crate::ep::EngineCommand::SetChannelMode { mode });
        self.rendering_runtime.request_redraw();

        let msg = match mode {
            1 => "色彩通道：红色 (R)",
            2 => "色彩通道：绿色 (G)",
            3 => "色彩通道：蓝色 (B)",
            4 => "色彩通道：透明度 (Alpha)",
            _ => "色彩通道重置：正常 (RGB)",
        };
        self.show_hud_message(msg);
    }

    pub fn handle_toggle_compare_mode(&self) {
        let runtime = self.runtime.clone();
        let pipeline = self.image_pipeline.clone();
        let rendering_runtime = self.rendering_runtime.clone();
        let ui_weak = self.ui.clone();

        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let active = !ui.get_compare_mode_active();
                ui.set_compare_mode_active(active);

                if active {
                    let compare_path_opt = if let Ok(fm) = runtime.state.file_manager.lock() {
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
                        let pipeline_clone = pipeline.clone();
                        tokio::spawn(async move {
                            pipeline_clone.display_compare_image(compare_path);
                        });
                    }

                    ui.set_status_text("同步对比模式已开启".into());
                    ui.set_manga_hud_visible(true);

                    let ui_weak_hud = ui_weak.clone();
                    tokio::spawn(async move {
                        let delay = crate::utils::settings::get_settings().hud_dismiss_delay_ms;
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui_inst) = ui_weak_hud.upgrade() {
                                ui_inst.set_manga_hud_visible(false);
                            }
                        });
                    });
                } else {
                    ui.set_status_text("已退出同步对比模式".into());
                    ui.set_manga_hud_visible(true);

                    let ui_weak_hud = ui_weak.clone();
                    tokio::spawn(async move {
                        let delay = crate::utils::settings::get_settings().hud_dismiss_delay_ms;
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui_inst) = ui_weak_hud.upgrade() {
                                ui_inst.set_manga_hud_visible(false);
                            }
                        });
                    });
                }
                rendering_runtime.request_redraw();
            }
        });
    }

    pub fn handle_toggle_compare_orientation(&self) {
        let rendering_runtime = self.rendering_runtime.clone();
        let ui_weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let vertical = ui.get_compare_vertical();
                let swipe = ui.get_compare_swipe();
                if !vertical && !swipe {
                    // Horizontal -> Vertical
                    ui.set_compare_vertical(true);
                    ui.set_compare_swipe(false);
                } else if vertical && !swipe {
                    // Vertical -> Swipe
                    ui.set_compare_vertical(false);
                    ui.set_compare_swipe(true);
                } else {
                    // Swipe -> Horizontal
                    ui.set_compare_vertical(false);
                    ui.set_compare_swipe(false);
                }
                rendering_runtime.request_redraw();
            }
        });
    }

    pub fn handle_compare_with_thumbnail(&self, index: i32) {
        let runtime = self.runtime.clone();
        let pipeline = self.image_pipeline.clone();
        let rendering_runtime = self.rendering_runtime.clone();
        let ui_weak = self.ui.clone();

        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let compare_path_opt = if let Ok(fm) = runtime.state.file_manager.lock() {
                    fm.get_all_image_paths().get(index as usize).cloned()
                } else {
                    None
                };

                if let Some(compare_path) = compare_path_opt {
                    let pipeline_clone = pipeline.clone();
                    tokio::spawn(async move {
                        pipeline_clone.display_compare_image(compare_path);
                    });

                    ui.set_compare_mode_active(true);
                    ui.set_status_text("同步对比模式已开启 (自选图片)".into());
                    ui.set_manga_hud_visible(true);

                    let ui_weak_hud = ui_weak.clone();
                    tokio::spawn(async move {
                        let delay = crate::utils::settings::get_settings().hud_dismiss_delay_ms;
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui_inst) = ui_weak_hud.upgrade() {
                                ui_inst.set_manga_hud_visible(false);
                            }
                        });
                    });
                }
                rendering_runtime.request_redraw();
            }
        });
    }

    pub fn handle_asset_toggle_selection(&self, index: i32) {
        self.gallery.toggle_selection(index);
    }

    pub fn handle_asset_select_all(&self, select: bool) {
        self.gallery.select_all(select);
    }

    pub async fn handle_rate_image(&self, rating: i32) {
        self.image_pipeline.rate_image(rating).await;
    }

    pub async fn handle_switch_to_rating_dir(&self, rating: i32) {
        self.image_pipeline.switch_to_rating_dir(rating).await;
    }

    pub async fn handle_go_to_original_dir(&self) {
        self.image_pipeline.go_to_original_dir().await;
    }

    // --- Transits / Event forwards ---
    pub fn handle_thumbnail_clicked(&self, index: i32) {
        let ui_weak = self.ui.clone();
        let image_pipeline = self.image_pipeline.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let thumbnails = ui.get_thumbnails();
                if let Some(thumb) = thumbnails.row_data(index as usize) {
                    let path_str = thumb.path.to_string();
                    let ui_weak_inner = ui_weak.clone();
                    let image_pipeline_inner = image_pipeline.clone();
                    tokio::spawn(async move {
                        let emitter =
                            Arc::new(crate::adapters::native_adapter::SlintEventEmitter {
                                ui: ui_weak_inner.clone(),
                            });
                        image_pipeline_inner.open_target(path_str, emitter).await;
                    });
                }
            }
        });
    }

    pub fn handle_long_image_scrolled_to(&self, index: i32) {
        let path_buf_opt = self
            .runtime
            .state
            .with_file_manager_mut(|manager| {
                manager.set_current_index(index as usize);
                manager.current()
            })
            .ok()
            .flatten();

        let ui_weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_active_thumbnail_index(index);
            }
        });

        if let Some(path_buf) = path_buf_opt {
            self.rendering_runtime.set_current_path(path_buf);
        }
    }

    pub fn handle_drag_drop(&self, data_str: String) {
        let image_pipeline = self.image_pipeline.clone();
        let ui_weak = self.ui.clone();
        tracing::info!("Files dropped in controller: {}", data_str);

        tokio::spawn(async move {
            for line in data_str.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let path = if line.starts_with("file://") {
                    #[allow(unused_mut)]
                    let mut p = line.trim_start_matches("file://");
                    #[cfg(target_os = "windows")]
                    if p.starts_with('/') {
                        p = p.trim_start_matches('/');
                    }
                    urlencoding::decode(p)
                        .map(|s| s.into_owned())
                        .unwrap_or_else(|_| p.to_string())
                } else {
                    line.to_string()
                };

                let path = path.trim_matches('"').to_string(); // Strip quotes just in case

                tracing::info!("Controller parsed drop path: {}", path);

                // Show HUD notification for drag and drop
                let ui_weak_hud = ui_weak.clone();
                let path_hud = path.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak_hud.upgrade() {
                        ui.set_status_text(format!("Dropped: {}", path_hud).into());
                        ui.set_manga_hud_visible(true);

                        let ui_clone = ui_weak_hud.clone();
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

                let emitter = Arc::new(crate::adapters::native_adapter::SlintEventEmitter {
                    ui: ui_weak.clone(),
                });
                image_pipeline.open_target(path, emitter).await;
                break; // Just open first dropped file
            }
        });
    }

    pub fn handle_startup_args(&self) {
        let args = self.runtime.startup_service.get_args();
        tracing::info!("Startup args: {:?}", args);
        if let Some(path) = args.get(1) {
            let path_str = path.clone().trim_matches('"').to_string(); // Strip quotes
            tracing::info!("Opening startup arg path: {}", path_str);
            let image_pipeline = self.image_pipeline.clone();
            tokio::spawn(async move {
                let emitter = Arc::new(crate::adapters::native_adapter::SlintEventEmitter {
                    ui: image_pipeline.ui.clone(),
                });
                image_pipeline.open_target(path_str, emitter).await;
            });
        }
    }

    // --- Direct UI Window interactions ---
    pub fn handle_window_drag(&self, has_control: bool) {
        let is_ctrl_down = has_control;

        let is_empty = if let Ok(fm) = self.runtime.state.file_manager.lock() {
            fm.current().is_none()
        } else {
            true
        };

        if is_ctrl_down || is_empty {
            let ui_weak = self.ui.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    let _ = ui.window().with_winit_window(
                        |winit_win: &i_slint_backend_winit::winit::window::Window| {
                            let _ = winit_win.drag_window();
                        },
                    );
                }
            });
        }
    }

    pub fn handle_drag(&self, dx: f32, dy: f32) {
        let ui_weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let pos = ui.window().position();
                ui.window().set_position(slint::LogicalPosition::new(
                    pos.x as f32 + dx,
                    pos.y as f32 + dy,
                ));
            }
        });
    }

    pub fn handle_zoom(&self, delta: f32, mx: f32, my: f32, has_control: bool, has_shift: bool) {
        self.rendering_runtime
            .zoom(delta, mx, my, has_control, has_shift);
    }

    pub fn handle_pan(&self, dx: f32, dy: f32) {
        self.rendering_runtime.pan(dx, dy);
    }

    pub fn handle_toggle_auto_scroll(&self) {
        self.rendering_runtime.toggle_auto_scroll();
    }

    pub fn handle_adjust_auto_scroll_speed(&self, delta: f32) {
        self.rendering_runtime.adjust_auto_scroll_speed(delta);
    }

    pub fn handle_inertia(&self, vx: f32, vy: f32) {
        self.rendering_runtime.apply_inertia(vx, vy);
    }

    pub fn handle_rotate(&self, delta: f32) {
        self.rendering_runtime.rotate(delta);
    }

    pub fn handle_flip(&self, axis: String) {
        self.rendering_runtime.flip(axis == "h");
    }

    pub fn handle_reset_transform(&self) {
        self.rendering_runtime.reset();
    }

    pub fn handle_toggle_slideshow(&self) {
        let ui_weak = self.ui.clone();
        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            let interval = if let Some(ui) = ui_weak.upgrade() {
                ui.get_slideshow_interval() as u64
            } else {
                3
            };

            let ui_weak_tick = ui_weak.clone();
            match runtime
                .slideshow_service
                .toggle_slideshow(interval, move || {
                    let ui_weak = ui_weak_tick.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.invoke_slideshow_tick();
                        }
                    });
                })
                .await
            {
                Ok(playing) => {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_slideshow_playing(playing);
                        }
                    });
                }
                Err(e) => tracing::error!("Failed to toggle slideshow: {}", e),
            }
        });
    }

    pub async fn handle_dispatch_event(&self, command: AppCommand, emitter: Arc<dyn EventEmitter>) {
        let _ = self.runtime.dispatch(command, Some(emitter)).await;
    }

    pub fn handle_mouse_move(&self, mx: f32, my: f32) {
        if let Some(ui) = self.ui.upgrade() {
            self.rendering_runtime.handle_mouse_move(
                ui.get_viewport_width(),
                ui.get_viewport_height(),
                mx,
                my,
            );
        }
    }

    pub fn handle_current_image_command(
        self: &Arc<Self>,
        command_gen: impl Fn(String) -> crate::application::protocol::AppCommand + Send + Sync + 'static,
        emitter: Arc<dyn EventEmitter>,
    ) {
        let self_clone = self.clone();
        let ui_weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let path = ui.get_current_image_path().to_string();
                if !path.is_empty() {
                    let self_inner = self_clone.clone();
                    let emitter_inner = emitter.clone();
                    tokio::spawn(async move {
                        self_inner
                            .handle_dispatch_event(command_gen(path), emitter_inner)
                            .await;
                    });
                }
            }
        });
    }

    pub async fn handle_update_shell(&self, emitter: Arc<dyn EventEmitter>) {
        let ui_weak = self.ui.clone();
        let i18n = self.runtime.state.i18n.clone();

        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_status_text(i18n.t("updating-shell").into());
            }
        });

        let response = self
            .runtime
            .dispatch(AppCommand::ApplyShellIntegration, Some(emitter))
            .await;

        let ui_weak = self.ui.clone();
        let i18n = self.runtime.state.i18n.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                match response {
                    AppResponse::Success => {
                        ui.set_status_text(i18n.t("shell-applied-success").into());
                    }
                    AppResponse::Error(err) => {
                        ui.set_status_text(
                            format!("{}: {}", i18n.t("shell-applied-failed"), err).into(),
                        );
                    }
                    _ => {
                        ui.set_status_text(i18n.t("shell-applied-failed").into());
                    }
                }
            }
        });
    }

    pub fn handle_toggle_scroll_mode(self: &Arc<Self>) {
        let ui_weak = self.ui.clone();
        let i18n = self.runtime.state.i18n.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let new_mode = !ui.get_scroll_mode_zoom();
                ui.set_scroll_mode_zoom(new_mode);

                // Persist scroll_mode setting
                let mut settings = crate::utils::settings::get_settings();
                settings.scroll_mode = if new_mode {
                    "zoom".to_string()
                } else {
                    "navigate".to_string()
                };
                let _ = crate::utils::settings::update_settings(settings);

                let msg = if new_mode {
                    i18n.t("scroll-zoom-mode")
                } else {
                    i18n.t("scroll-navigate-mode")
                };
                ui.set_status_text(msg.into());
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

    pub fn handle_scroll_navigate(self: &Arc<Self>, delta: f32) {
        let mut last_time = self.last_scroll_nav_time.lock().unwrap();
        let now = std::time::Instant::now();
        if now.duration_since(*last_time) >= std::time::Duration::from_millis(250) {
            *last_time = now;
            let c = self.clone();
            tokio::spawn(async move {
                if delta > 0.0 {
                    c.handle_prev_image().await;
                } else if delta < 0.0 {
                    c.handle_next_image().await;
                }
            });
        }
    }

    pub fn handle_toggle_long_image_mode(self: &Arc<Self>) {
        let ui_weak = self.ui.clone();
        let i18n = self.runtime.state.i18n.clone();
        let rendering_runtime = self.rendering_runtime.clone();
        let self_clone = Arc::clone(self);

        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let new_mode = !ui.get_long_image_mode_active();
                ui.set_long_image_mode_active(new_mode);

                // Persist view mode setting
                let mut settings = crate::utils::settings::get_settings();
                settings.default_view_mode = if new_mode {
                    "waterfall".to_string()
                } else {
                    "single".to_string()
                };
                let _ = crate::utils::settings::update_settings(settings);

                rendering_runtime.set_long_image_mode(new_mode);

                if new_mode {
                    let c = self_clone.clone();
                    let current_path = ui.get_current_image_path().to_string();
                    tokio::spawn(async move {
                        c.scan_and_setup_continuous_images(Some(current_path)).await;
                    });
                }

                let msg = if new_mode {
                    i18n.t("long-image-mode-enabled")
                } else {
                    i18n.t("long-image-mode-disabled")
                };
                ui.set_status_text(msg.into());
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

    pub async fn scan_and_setup_continuous_images(&self, current_path: Option<String>) {
        let paths = if let Ok(fm) = self.runtime.state.file_manager.lock() {
            fm.get_all_image_paths().clone()
        } else {
            Vec::new()
        };

        let mut images = Vec::new();
        for path_buf in paths {
            let path_str = path_buf.clone();
            // Fast dimension probe. Fallback for archives to avoid heavy IO.
            let size = if crate::core::image_loader::parse_archive_path(&path_str).is_some() {
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

        self.rendering_runtime
            .set_continuous_images(images, current_path);
    }

    pub fn handle_scroll_to_percent(&self, percent: f32) {
        self.rendering_runtime.scroll_to_percent(percent);
    }

    pub fn handle_adjust_width_percent(&self, percent: f32) {
        self.rendering_runtime.adjust_width_percent(percent);
    }

    pub fn handle_toggle_viewport_lock(&self) {
        let is_locked = self.rendering_runtime.toggle_viewport_lock();
        let ui_weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_viewport_locked(is_locked);
                // Currently just english but ideally using L10n.
                let text = if is_locked {
                    "Viewport Locked"
                } else {
                    "Viewport Unlocked"
                };
                ui.set_status_text(text.into());
            }
        });
    }

    pub fn handle_toggle_exif_hud(&self) {
        let ui_weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let current = ui.get_exif_hud_visible();
                ui.set_exif_hud_visible(!current);
            }
        });
    }
}
