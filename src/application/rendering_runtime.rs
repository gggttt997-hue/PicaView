use crate::ep::physics::ViewportPhysics;
use crate::ep::renderer::WgpuRenderer;
use crate::ep::EngineCommand;
use slint::{ComponentHandle, GraphicsAPI, RenderingState, Window};
use std::sync::{mpsc, Arc, Mutex};

pub struct RuntimeRenderingState {
    pub renderer: Option<WgpuRenderer>,
    pub renderer_compare: Option<WgpuRenderer>,
    pub physics: ViewportPhysics,
    pub physics_compare: ViewportPhysics,
    pub output_texture: Option<wgpu::Texture>,
    pub output_texture_compare: Option<wgpu::Texture>,
    pub last_physical_size: (u32, u32),
    pub image_dirty: bool,
    pub last_tick: std::time::Instant,
    pub current_file_size: String,
    pub current_path: Option<std::path::PathBuf>,
    pub current_path_compare: Option<std::path::PathBuf>,
    pub missing_continuous_paths: std::collections::HashSet<std::path::PathBuf>,
    pub loading_paths: std::collections::HashSet<std::path::PathBuf>,
    pub last_requested_center_index: Option<usize>,
}

impl RuntimeRenderingState {
    /// 执行物理引擎时间滴答，更新状态栏，计算视图矩阵。
    /// 返回 `(should_render, view_matrix)`
    fn tick_physics_and_status(
        &mut self,
        ui_weak: &slint::Weak<crate::MainWindow>,
        physical_width: u32,
        physical_height: u32,
        compare_mode_active: bool,
    ) -> (bool, glam::Mat4) {
        let now = std::time::Instant::now();
        let dt = {
            let elapsed = now.duration_since(self.last_tick).as_secs_f32();
            self.last_tick = now;
            elapsed.min(0.1)
        };

        let changed = self
            .physics
            .tick(dt, physical_width as f32, physical_height as f32);

        if compare_mode_active {
            self.physics_compare.zoom = self.physics.zoom;
            self.physics_compare.target_zoom = self.physics.target_zoom;
            self.physics_compare.offset = self.physics.offset;
            self.physics_compare.target_offset = self.physics.target_offset;
            self.physics_compare.rotation = self.physics.rotation;
            self.physics_compare.target_rotation = self.physics.target_rotation;
            self.physics_compare.flip_h = self.physics.flip_h;
            self.physics_compare.flip_v = self.physics.flip_v;
            self.physics_compare.image_size = self.physics.image_size;

            let _ = self
                .physics_compare
                .tick(dt, physical_width as f32, physical_height as f32);
        }

        let resized = self.last_physical_size.0 != physical_width
            || self.last_physical_size.1 != physical_height;

        let mut should_render = false;
        if resized || self.image_dirty {
            if self.physics.image_size.x > 0.0 {
                let iw = self.physics.image_size.x;
                let ih = self.physics.image_size.y;
                self.physics
                    .reset_with_size(iw, ih, physical_width as f32, physical_height as f32);
                if compare_mode_active {
                    self.physics_compare.reset_with_size(
                        iw,
                        ih,
                        physical_width as f32,
                        physical_height as f32,
                    );
                }
            }
            self.last_physical_size = (physical_width, physical_height);
            should_render = true;
        }

        if changed || self.image_dirty || should_render {
            should_render = true;
            self.image_dirty = false;

            let status = if self.physics.image_size.x > 1.0 {
                if self.current_file_size.is_empty() {
                    self.physics.format_status()
                } else {
                    format!(
                        "{} | {}",
                        self.physics.format_status(),
                        self.current_file_size
                    )
                }
            } else {
                "Ready".to_string()
            };

            let is_long = self.physics.is_long_image;
            let max_y = self
                .physics
                .calculate_bounds(physical_width as f32, physical_height as f32)
                .1;
            let progress = if max_y > 0.0 {
                // Corrected mapping: 0.0 is top (max_y), 1.0 is bottom (-max_y)
                ((max_y - self.physics.offset.y) / (2.0 * max_y)).clamp(0.0, 1.0)
            } else {
                0.0
            };

            let ui_weak_status = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak_status.upgrade() {
                    if ui.get_status_text() != status.as_str() {
                        ui.set_status_text(status.into());
                    }
                    if is_long {
                        ui.set_long_image_scroll_progress(progress);
                    }
                }
            });
        }

        let matrix = self
            .physics
            .get_view_proj(physical_width as f32, physical_height as f32);

        (should_render, matrix)
    }

    /// 如果视口足够大且图像为大图，计算在当前屏幕视口内的瓦片，并异步请求 Tile
    fn load_lod_tiles(
        &mut self,
        async_decoder: &Arc<crate::ep::decoder::AsyncDecoder>,
        matrix: &glam::Mat4,
        physical_width: u32,
        physical_height: u32,
    ) {
        if self.physics.is_long_image {
            return;
        }

        let current_path = match &self.current_path {
            Some(path) => path.clone(),
            None => return,
        };

        let iw = self.physics.image_size.x;
        let ih = self.physics.image_size.y;
        if iw <= 1024.0 && ih <= 1024.0 {
            return;
        }

        let inv_matrix = matrix.inverse();
        let tl = inv_matrix.project_point3(glam::vec3(0.0, 0.0, 0.0));
        let br = inv_matrix.project_point3(glam::vec3(
            physical_width as f32,
            physical_height as f32,
            0.0,
        ));

        let x_min = ((tl.x + 0.5) * iw).floor().max(0.0) as u32;
        let y_min = ((tl.y + 0.5) * ih).floor().max(0.0) as u32;
        let x_max = ((br.x + 0.5) * iw).ceil().min(iw) as u32;
        let y_max = ((br.y + 0.5) * ih).ceil().min(ih) as u32;
        let mut local_y_min = y_min;
        let mut local_y_max = y_max;
        let mut local_ih = ih as u32;
        let mut orig_w = iw;
        let mut orig_h = ih;

        if self.physics.is_long_image {
            for img in &self.physics.continuous_images {
                if img.path == current_path {
                    let y_offset = img.y_offset as u32;
                    let scaled_height = img.scaled_height as u32;

                    let clamped_y_min = y_min.max(y_offset).min(y_offset + scaled_height);
                    let clamped_y_max = y_max.max(y_offset).min(y_offset + scaled_height);

                    local_y_min = clamped_y_min - y_offset;
                    local_y_max = clamped_y_max - y_offset;
                    local_ih = scaled_height;

                    orig_w = img.original_size.x;
                    orig_h = img.original_size.y;
                    break;
                }
            }
        }

        let scale_x = if iw > 0.0 { orig_w / iw } else { 1.0 };
        let scale_y = if local_ih > 0 {
            orig_h / (local_ih as f32)
        } else {
            1.0
        };

        let orig_x_min = (x_min as f32 * scale_x) as u32;
        let orig_x_max = (x_max as f32 * scale_x).ceil() as u32;
        let orig_y_min = (local_y_min as f32 * scale_y) as u32;
        let orig_y_max = (local_y_max as f32 * scale_y).ceil() as u32;

        let tile_size = crate::utils::settings::get_settings().tile_size;
        let start_tx = orig_x_min / tile_size;
        let start_ty = orig_y_min / tile_size;
        let end_tx = orig_x_max.div_ceil(tile_size);
        let end_ty = orig_y_max.div_ceil(tile_size);

        for ty in start_ty..end_ty {
            for tx in start_tx..end_tx {
                let key = crate::ep::tile_cache::TileKey {
                    x: tx,
                    y: ty,
                    lod: 0,
                };
                if let Some(r) = self.renderer.as_mut() {
                    if !r.has_tile(&key) {
                        let x = tx * tile_size;
                        let y = ty * tile_size;
                        let w = tile_size.min(orig_w as u32 - x);
                        let h = tile_size.min(orig_h as u32 - y);

                        if w > 0 && h > 0 && r.mark_pending(&key) {
                            async_decoder.request_tile(key, current_path.clone(), x, y, w, h);
                        }
                    }
                }
            }
        }
    }

    fn load_compare_lod_tiles(
        &mut self,
        async_decoder: &Arc<crate::ep::decoder::AsyncDecoder>,
        matrix: &glam::Mat4,
        physical_width: u32,
        physical_height: u32,
    ) {
        if self.physics_compare.is_long_image {
            return;
        }
        if self.physics_compare.zoom <= 1.05 {
            return;
        }
        let current_path = match &self.current_path_compare {
            Some(path) => path.clone(),
            None => return,
        };

        let iw = self.physics_compare.image_size.x;
        let ih = self.physics_compare.image_size.y;
        if iw <= 1024.0 && ih <= 1024.0 {
            return;
        }

        let inv_matrix = matrix.inverse();
        let tl = inv_matrix.project_point3(glam::vec3(0.0, 0.0, 0.0));
        let br = inv_matrix.project_point3(glam::vec3(
            physical_width as f32,
            physical_height as f32,
            0.0,
        ));

        let x_min = ((tl.x + 0.5) * iw).floor().max(0.0) as u32;
        let y_min = ((tl.y + 0.5) * ih).floor().max(0.0) as u32;
        let x_max = ((br.x + 0.5) * iw).ceil().min(iw) as u32;
        let y_max = ((br.y + 0.5) * ih).ceil().min(ih) as u32;

        let tile_size = crate::utils::settings::get_settings().tile_size;
        let start_tx = x_min / tile_size;
        let start_ty = y_min / tile_size;
        let end_tx = x_max.div_ceil(tile_size);
        let end_ty = y_max.div_ceil(tile_size);

        for ty in start_ty..end_ty {
            for tx in start_tx..end_tx {
                let key = crate::ep::tile_cache::TileKey {
                    x: tx,
                    y: ty,
                    lod: 0,
                };

                if let Some(r) = self.renderer_compare.as_mut() {
                    if !r.has_tile(&key) {
                        let x = tx * tile_size;
                        let y = ty * tile_size;
                        let w = tile_size.min(iw as u32 - x);
                        let h = tile_size.min(ih as u32 - y);

                        if w > 0 && h > 0 && r.mark_pending(&key) {
                            async_decoder.request_tile(key, current_path.clone(), x, y, w, h);
                        }
                    }
                }
            }
        }
    }

    /// 确认 WGPU 离屏渲染纹理的大小适配。
    fn ensure_output_texture(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        compare_active: bool,
    ) {
        if self.renderer.is_none() {
            return;
        }

        let recreate = match &self.output_texture {
            Some(tex) => tex.width() != width || tex.height() != height,
            None => true,
        };

        if recreate {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Slint_WGPU_Output"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            self.output_texture = Some(tex);
        }

        if compare_active {
            let recreate_compare = match &self.output_texture_compare {
                Some(tex) => tex.width() != width || tex.height() != height,
                None => true,
            };

            if recreate_compare {
                let tex = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Slint_WGPU_Compare_Output"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                self.output_texture_compare = Some(tex);
            }
        }
    }

    /// 渲染并向 Slint 投递生成的图片句柄。
    fn render_and_export(
        &mut self,
        ui_weak: &slint::Weak<crate::MainWindow>,
        compare_active: bool,
    ) {
        let tex = match &self.output_texture {
            Some(t) => t,
            None => return,
        };

        if let Some(r) = self.renderer.as_mut() {
            r.is_long_image = self.physics.is_long_image;
            r.continuous_images = self.physics.continuous_images.clone();
            r.current_zoom = self.physics.zoom;
            r.current_offset = [self.physics.offset.x, self.physics.offset.y];
            r.render(tex, self.current_path.as_ref());

            let tex_clone = tex.clone();
            let ui_weak_inner = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak_inner.upgrade() {
                    if let Ok(slint_img) = slint::Image::try_from(tex_clone) {
                        ui.set_wgpu_image(slint_img);
                    }
                }
            });
        }

        if compare_active {
            if let Some(tex_comp) = &self.output_texture_compare {
                if let Some(r_comp) = self.renderer_compare.as_mut() {
                    r_comp.is_long_image = self.physics_compare.is_long_image;
                    r_comp.continuous_images = self.physics_compare.continuous_images.clone();
                    r_comp.current_zoom = self.physics_compare.zoom;
                    r_comp.current_offset =
                        [self.physics_compare.offset.x, self.physics_compare.offset.y];
                    r_comp.render(tex_comp, self.current_path_compare.as_ref());

                    let tex_comp_clone = tex_comp.clone();
                    let ui_weak_inner = ui_weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak_inner.upgrade() {
                            if let Ok(slint_img) = slint::Image::try_from(tex_comp_clone) {
                                ui.set_wgpu_image_compare(slint_img);
                            }
                        }
                    });
                }
            }
        }
    }
}

pub struct RenderingRuntime {
    state: Arc<Mutex<RuntimeRenderingState>>,
    command_tx: mpsc::Sender<EngineCommand>,
    pub command_compare_tx: mpsc::Sender<EngineCommand>,
    ui_weak: slint::Weak<crate::MainWindow>,
    pub _async_decoder: Arc<crate::ep::decoder::AsyncDecoder>,
    _async_decoder_compare: Arc<crate::ep::decoder::AsyncDecoder>,
}

impl RenderingRuntime {
    pub fn new<F>(
        _window: &Window,
        _get_viewport: F,
        ui_weak: slint::Weak<crate::MainWindow>,
    ) -> Self
    where
        F: Fn() -> (f32, f32, f32) + 'static,
    {
        let (tx, rx) = mpsc::channel::<EngineCommand>();
        let command_tx = tx.clone();
        let rx = Arc::new(Mutex::new(Some(rx)));

        let (tx_compare, rx_compare) = mpsc::channel::<EngineCommand>();
        let command_compare_tx = tx_compare.clone();
        let rx_compare = Arc::new(Mutex::new(Some(rx_compare)));

        let state = Arc::new(Mutex::new(RuntimeRenderingState {
            renderer: None,
            renderer_compare: None,
            physics: ViewportPhysics::new(),
            physics_compare: ViewportPhysics::new(),
            output_texture: None,
            output_texture_compare: None,
            last_physical_size: (0, 0),
            image_dirty: false,
            last_tick: std::time::Instant::now(),
            current_file_size: String::new(),
            current_path: None,
            current_path_compare: None,
            missing_continuous_paths: std::collections::HashSet::new(),
            loading_paths: std::collections::HashSet::new(),
            last_requested_center_index: None,
        }));

        let async_decoder = Arc::new(crate::ep::decoder::AsyncDecoder::new(tx));
        let async_decoder_compare = Arc::new(crate::ep::decoder::AsyncDecoder::new(tx_compare));

        // Initialize Rendering Notifier
        {
            let runtime_state = state.clone();
            let command_tx_notifier = command_tx.clone();
            let command_compare_tx_notifier = command_compare_tx.clone();
            let ui_weak_notifier = ui_weak.clone();
            let async_decoder_notifier = async_decoder.clone();
            let async_decoder_compare_notifier = async_decoder_compare.clone();
            let rx_ref = rx.clone();
            let rx_compare_ref = rx_compare.clone();

            _window
                .set_rendering_notifier(move |rendering_state, graphics_api| {
                    match (rendering_state, graphics_api) {
                        (RenderingState::RenderingSetup, api)
                            if format!("{:?}", api).contains("WGPU") =>
                        {
                            if let GraphicsAPI::WGPU28 { device, queue, .. } = api {
                                let mut st = runtime_state.lock().unwrap();
                                if st.renderer.is_none() {
                                    if let Some(rx_owned) = rx_ref.lock().unwrap().take() {
                                        st.renderer = Some(WgpuRenderer::new(
                                            device.clone(),
                                            queue.clone(),
                                            rx_owned,
                                        ));
                                    }
                                }
                                if st.renderer_compare.is_none() {
                                    if let Some(rx_owned) = rx_compare_ref.lock().unwrap().take() {
                                        st.renderer_compare = Some(WgpuRenderer::new(
                                            device.clone(),
                                            queue.clone(),
                                            rx_owned,
                                        ));
                                    }
                                }
                            }
                        }
                        (RenderingState::BeforeRendering, api)
                            if format!("{:?}", api).contains("WGPU") =>
                        {
                            if let GraphicsAPI::WGPU28 { device, .. } = api {
                                Self::handle_before_rendering_static(
                                    &runtime_state,
                                    &command_tx_notifier,
                                    &command_compare_tx_notifier,
                                    &ui_weak_notifier,
                                    &async_decoder_notifier,
                                    &async_decoder_compare_notifier,
                                    device,
                                );
                            }
                        }
                        (RenderingState::RenderingTeardown, _) => {
                            let mut st = runtime_state.lock().unwrap();
                            if let Some(mut r) = st.renderer.take() {
                                r.teardown();
                            }
                            if let Some(mut r) = st.renderer_compare.take() {
                                r.teardown();
                            }
                            st.output_texture = None;
                            st.output_texture_compare = None;
                        }
                        _ => {}
                    }
                })
                .expect("Failed to set rendering notifier");
        }

        Self {
            state,
            command_tx,
            command_compare_tx,
            ui_weak,
            _async_decoder: async_decoder,
            _async_decoder_compare: async_decoder_compare,
        }
    }

    fn handle_before_rendering_static(
        state_arc: &Arc<Mutex<RuntimeRenderingState>>,
        _command_tx: &mpsc::Sender<EngineCommand>,
        _command_compare_tx: &mpsc::Sender<EngineCommand>,
        ui_weak: &slint::Weak<crate::MainWindow>,
        async_decoder: &Arc<crate::ep::decoder::AsyncDecoder>,
        async_decoder_compare: &Arc<crate::ep::decoder::AsyncDecoder>,
        device: &wgpu::Device,
    ) {
        let mut state = state_arc.lock().unwrap();
        if let Some(r) = state.renderer.as_mut() {
            r.execute_commands();
        }
        if let Some(r) = state.renderer_compare.as_mut() {
            r.execute_commands();
        }

        let physical_size = if let Some(ui) = ui_weak.upgrade() {
            ui.window().size()
        } else {
            slint::PhysicalSize::new(0, 0)
        };
        let physical_width = physical_size.width;
        let physical_height = physical_size.height;
        if physical_width == 0 || physical_height == 0 {
            return;
        }

        let (compare_active, compare_vertical, compare_swipe) = if let Some(ui) = ui_weak.upgrade()
        {
            (
                ui.get_compare_mode_active(),
                ui.get_compare_vertical(),
                ui.get_compare_swipe(),
            )
        } else {
            (false, false, false)
        };

        let (view_width, view_height) = if compare_active && !compare_swipe {
            if compare_vertical {
                (physical_width, physical_height / 2)
            } else {
                (physical_width / 2, physical_height)
            }
        } else {
            (physical_width, physical_height)
        };

        let (mut should_render, matrix) =
            state.tick_physics_and_status(ui_weak, view_width, view_height, compare_active);

        let animating = if let Some(r) = state.renderer.as_ref() {
            r.has_animating_tiles()
        } else {
            false
        } || if compare_active {
            if let Some(r) = state.renderer_compare.as_ref() {
                r.has_animating_tiles()
            } else {
                false
            }
        } else {
            false
        };

        if animating {
            should_render = true;
        }

        if !should_render {
            return;
        }

        if let Some(r) = state.renderer.as_mut() {
            r.update_matrix(matrix.to_cols_array_2d());
        }

        if compare_active {
            if let Some(r) = state.renderer_compare.as_mut() {
                r.update_matrix(matrix.to_cols_array_2d());
            }
        }

        let mut missing_paths = std::collections::HashSet::new();
        let mut new_center_index = None;
        if state.physics.is_long_image && !state.physics.continuous_images.is_empty() {
            let total_height = state.physics.image_size.y;
            let zoom = state.physics.zoom;
            let offset_y = state.physics.offset.y;
            let vh = view_height as f32;

            let y_visible_start = total_height * 0.5 - (offset_y + vh * 0.5) / zoom;
            let y_visible_end = total_height * 0.5 - (offset_y - vh * 0.5) / zoom;
            let center_y = (y_visible_start + y_visible_end) * 0.5;

            for (i, img) in state.physics.continuous_images.iter().enumerate() {
                let img_start = img.y_offset;
                let img_end = img.y_offset + img.scaled_height;

                if center_y >= img_start && center_y <= img_end {
                    let mut should_request = false;
                    if let Some(current) = &state.current_path {
                        if current != &img.path && state.last_requested_center_index != Some(i) {
                            should_request = true;
                        }
                    } else if state.last_requested_center_index != Some(i) {
                        should_request = true;
                    }

                    if should_request {
                        new_center_index = Some(i);
                    }
                }

                let is_visible = !(img_end < y_visible_start || img_start > y_visible_end);
                if is_visible {
                    if let Some(r) = &state.renderer {
                        if !r.has_continuous_base(&img.path) {
                            missing_paths.insert(img.path.clone());
                        }
                    }
                }
            }
        }
        state.missing_continuous_paths = missing_paths;

        if let Some(index) = new_center_index {
            state.last_requested_center_index = Some(index);
            let ui_weak_clone = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak_clone.upgrade() {
                    ui.invoke_long_image_scrolled_to(index as i32);
                }
            });
        }

        state.load_lod_tiles(async_decoder, &matrix, view_width, view_height);
        if compare_active {
            state.load_compare_lod_tiles(async_decoder_compare, &matrix, view_width, view_height);
        }

        state.ensure_output_texture(device, view_width, view_height, compare_active);
        state.render_and_export(ui_weak, compare_active);

        if animating {
            if let Some(ui) = ui_weak.upgrade() {
                ui.window().request_redraw();
            }
        }

        // Asynchronously load missing sub-images for the continuous waterfall view and update layouts dynamically.
        let mut paths_to_load = Vec::new();
        {
            let missing = state.missing_continuous_paths.clone();
            for path in missing {
                if !state.loading_paths.contains(&path) {
                    state.loading_paths.insert(path.clone());
                    paths_to_load.push(path);
                }
            }
        }

        for path in paths_to_load {
            let state_clone = state_arc.clone();
            let command_tx_clone = _command_tx.clone();
            let path_clone = path.clone();
            tokio::spawn(async move {
                let img_res: Option<(image::DynamicImage, f32, f32)> =
                    if let Some((archive_path, entry_name)) =
                        crate::core::image_loader::parse_archive_path(&path_clone.to_string_lossy())
                    {
                        if let Ok(bytes) = crate::core::image_loader::load_archive_image_bytes(
                            archive_path,
                            entry_name,
                        ) {
                            tokio::task::spawn_blocking(move || {
                                if let Ok(img) = image::load_from_memory(&bytes) {
                                    let w = img.width() as f32;
                                    let h = img.height() as f32;
                                    Ok((img, w, h))
                                } else {
                                    Err(())
                                }
                            })
                            .await
                            .ok()
                            .and_then(|r| r.ok())
                        } else {
                            None
                        }
                    } else {
                        let path_for_io = path_clone.clone();
                        tokio::task::spawn_blocking(move || match image::open(&path_for_io) {
                            Ok(img) => {
                                let true_w = img.width() as f32;
                                let true_h = img.height() as f32;
                                Ok((img, true_w, true_h))
                            }
                            Err(_) => Err(()),
                        })
                        .await
                        .ok()
                        .and_then(|r| r.ok())
                    };

                if let Some((img, width, height)) = img_res {
                    {
                        let mut state = state_clone.lock().unwrap();
                        for item in state.physics.continuous_images.iter_mut() {
                            if item.path == path_clone {
                                item.original_size = glam::Vec2::new(width, height);
                            }
                        }
                    }

                    let state_layout = state_clone.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        let base_width = {
                            let state = state_layout.lock().unwrap();
                            state.physics.image_size.x
                        };
                        {
                            let mut state = state_layout.lock().unwrap();
                            let mut current_y = 0.0;
                            let spacing = 20.0;
                            for img_item in state.physics.continuous_images.iter_mut() {
                                img_item.y_offset = current_y;
                                if img_item.original_size.x > 0.0 && img_item.original_size.y > 0.0
                                {
                                    let aspect =
                                        img_item.original_size.y / img_item.original_size.x;
                                    img_item.scaled_height = base_width * aspect;
                                } else {
                                    img_item.scaled_height = 0.0;
                                }
                                current_y += img_item.scaled_height + spacing;
                            }
                            let total_height = if current_y > spacing {
                                current_y - spacing
                            } else {
                                current_y
                            };
                            state.physics.image_size.y = total_height;
                            state.physics.manual_dirty = true;
                        }
                    });

                    let _ = command_tx_clone.send(EngineCommand::UploadBase {
                        image: img,
                        path: path_clone.clone(),
                        physical_width: width as u32,
                        physical_height: height as u32,
                    });
                }

                {
                    let mut state = state_clone.lock().unwrap();
                    state.loading_paths.remove(&path_clone);
                }
            });
        }
    }

    pub fn update_file_size(&self, size: String) {
        self.state.lock().unwrap().current_file_size = size;
    }

    pub fn get_physics(&self) -> Arc<Mutex<ViewportPhysics>> {
        let p = self.state.lock().unwrap().physics.clone();
        Arc::new(Mutex::new(p))
    }

    pub fn handle_mouse_move(&self, _width: f32, _height: f32, mx: f32, my: f32) {
        let _ = self.command_tx.send(EngineCommand::ViewportUpdate {
            width: 0.0,
            height: 0.0,
            mx,
            my,
        });
        self.request_redraw();
    }

    pub fn rotate(&self, delta_deg: f32) {
        let mut state = self.state.lock().unwrap();
        state.physics.target_rotation += delta_deg.to_radians();
        state.physics.manual_dirty = true;
        self.request_redraw();
    }

    pub fn flip(&self, horizontal: bool) {
        let mut state = self.state.lock().unwrap();
        state.physics.flip(horizontal);
        state.physics.manual_dirty = true;
        self.request_redraw();
    }

    pub fn pan(&self, dx: f32, dy: f32) {
        let scale_factor = self.get_scale_factor();
        let mut state = self.state.lock().unwrap();
        let pdx = dx * scale_factor;
        let pdy = dy * scale_factor;

        state.physics.offset.x += pdx;
        state.physics.offset.y += pdy;
        state.physics.target_offset.x += pdx;
        state.physics.target_offset.y += pdy;

        state.physics.velocity = glam::Vec2::ZERO;
        state.physics.manual_dirty = true;

        self.request_redraw();
    }

    pub fn apply_inertia(&self, vx: f32, vy: f32) {
        let scale_factor = self.get_scale_factor();
        let mut state = self.state.lock().unwrap();
        state.physics.velocity =
            glam::Vec2::new(vx * scale_factor * 30.0, vy * scale_factor * 30.0);
        state.physics.manual_dirty = true;

        self.request_redraw();
    }

    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.physics.target_zoom = 1.0;
        state.physics.target_offset = glam::Vec2::ZERO;
        state.physics.target_rotation = 0.0;
        state.physics.flip_h = false;
        state.physics.flip_v = false;
        state.physics.manual_dirty = true;

        self.request_redraw();
    }

    pub fn zoom(&self, delta: f32, mx: f32, my: f32, has_control: bool, has_shift: bool) {
        let scale_factor = self.get_scale_factor();
        let mut state = self.state.lock().unwrap();
        let (viewport_w, viewport_h) = {
            let size = state.last_physical_size;
            (size.0 as f32, size.1 as f32)
        };

        if state.physics.is_long_image && !has_control {
            // Long image mode without Ctrl: translate wheel scroll to panning.
            let scroll_amount = delta * 1.5 * scale_factor;
            if has_shift {
                state.physics.target_offset.x += scroll_amount;
            } else {
                state.physics.target_offset.y += scroll_amount;
            }
            state.physics.manual_dirty = true;
            self.request_redraw();
            return;
        }

        let physical_mx = mx * scale_factor;
        let physical_my = my * scale_factor;

        state
            .physics
            .zoom_at(delta, physical_mx, physical_my, viewport_w, viewport_h);
        state.physics.manual_dirty = true;

        self.request_redraw();
    }

    pub fn toggle_auto_scroll(&self) {
        let mut state = self.state.lock().unwrap();
        if state.physics.auto_scroll_speed == 0.0 {
            state.physics.auto_scroll_speed = 300.0; // Start with 300 pixels/sec
        } else {
            state.physics.auto_scroll_speed = 0.0;
        }
        self.request_redraw();
    }

    pub fn adjust_auto_scroll_speed(&self, delta: f32) {
        let mut state = self.state.lock().unwrap();
        if state.physics.auto_scroll_speed != 0.0 {
            state.physics.auto_scroll_speed =
                (state.physics.auto_scroll_speed + delta).clamp(10.0, 5000.0);
            self.request_redraw();
        }
    }

    pub fn get_command_sender(&self) -> mpsc::Sender<EngineCommand> {
        self.command_tx.clone()
    }

    pub fn set_current_path(&self, path: std::path::PathBuf) {
        {
            let mut state = self.state.lock().unwrap();
            state.current_path = Some(path.clone());
            if !state.physics.is_long_image {
                state.image_dirty = true;
            }
        }
        self._async_decoder.set_active_path(Some(path.clone()));
        let _ = self.command_tx.send(EngineCommand::SetImagePath { path });
    }

    pub fn set_current_compare_path(&self, path: std::path::PathBuf) {
        {
            let mut state = self.state.lock().unwrap();
            state.current_path_compare = Some(path.clone());
            state.image_dirty = true;
        }
        self._async_decoder_compare
            .set_active_path(Some(path.clone()));
        let _ = self
            .command_compare_tx
            .send(EngineCommand::SetImagePath { path });
    }

    pub fn set_file_size(&self, size: String) {
        self.state.lock().unwrap().current_file_size = size;
    }

    pub fn update_image_size(&self, width: f32, height: f32) {
        let mut state = self.state.lock().unwrap();

        // In long image mode, the image size is determined by the continuous layout.
        // We do not want to reset the layout and zoom just because a new tile finished loading.
        if state.physics.is_long_image && !state.physics.continuous_images.is_empty() {
            return;
        }

        state.physics.image_size = glam::Vec2::new(width, height);
        state.image_dirty = true;
    }

    pub fn update_compare_image_size(&self, width: f32, height: f32) {
        let mut state = self.state.lock().unwrap();
        state.physics_compare.image_size = glam::Vec2::new(width, height);
        state.image_dirty = true;
    }

    pub fn reset_with_size(
        &self,
        width: f32,
        height: f32,
        viewport_w: f32,
        viewport_h: f32,
        scale_factor: f32,
    ) {
        let mut state = self.state.lock().unwrap();
        if state.physics.is_long_image && !state.physics.continuous_images.is_empty() {
            return;
        }
        state.physics.reset_with_size(
            width,
            height,
            viewport_w * scale_factor,
            viewport_h * scale_factor,
        );
    }

    pub fn get_viewport_size(&self) -> (u32, u32) {
        self.state.lock().unwrap().last_physical_size
    }

    pub fn set_long_image_mode(&self, enabled: bool) {
        {
            let mut state = self.state.lock().unwrap();
            state.physics.is_long_image = enabled;
            state.image_dirty = true;
        }
        self.request_redraw();
    }

    pub fn set_continuous_images(
        &self,
        images: Vec<crate::ep::physics::ContinuousImageInfo>,
        current_path: Option<String>,
    ) {
        {
            let mut state = self.state.lock().unwrap();
            state.physics.continuous_images = images;
        }
        self.rebuild_continuous_layout(current_path);
    }

    pub fn rebuild_continuous_layout(&self, current_path: Option<String>) {
        let mut state = self.state.lock().unwrap();
        let mut base_width = state.physics.image_size.x;
        if base_width <= 0.0 {
            base_width = state
                .physics
                .continuous_images
                .first()
                .map(|i| i.original_size.x)
                .unwrap_or(1000.0);
            if base_width <= 0.0 {
                base_width = 1000.0;
            }
            state.physics.image_size.x = base_width;
        }
        let spacing = state.physics.spacing;

        let mut current_y = 0.0;
        for img in state.physics.continuous_images.iter_mut() {
            img.y_offset = current_y;
            if img.original_size.x > 0.0 {
                let aspect = img.original_size.y / img.original_size.x;
                img.scaled_height = base_width * aspect;
            } else {
                img.scaled_height = 0.0;
            }
            current_y += img.scaled_height + spacing;
        }
        let total_height = if current_y > spacing {
            current_y - spacing
        } else {
            current_y
        };
        state.physics.image_size.y = total_height;

        let (viewport_w, viewport_h) = {
            let size = state.last_physical_size;
            (size.0 as f32, size.1 as f32)
        };
        let zoom = state.physics.target_zoom;
        let max_y = state.physics.calculate_bounds(viewport_w, viewport_h).1;

        if let Some(target_path) = current_path {
            let target_path_buf = std::path::PathBuf::from(&target_path);
            if let Some(img) = state
                .physics
                .continuous_images
                .iter()
                .find(|i| i.path == target_path_buf)
            {
                // max_y is 0.0 offset. Scrolling down means target_offset.y decreases.
                state.physics.target_offset.y = max_y - img.y_offset * zoom;
                state.physics.offset.y = state.physics.target_offset.y;
            }
        }

        state.image_dirty = true;
        self.request_redraw();
    }

    pub fn get_missing_paths(&self) -> std::collections::HashSet<std::path::PathBuf> {
        let state = self.state.lock().unwrap();
        state.missing_continuous_paths.clone()
    }

    pub fn is_continuous_images_empty(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.physics.continuous_images.is_empty()
    }

    pub fn first_continuous_image_parent(&self) -> Option<std::path::PathBuf> {
        let state = self.state.lock().unwrap();
        state
            .physics
            .continuous_images
            .first()
            .and_then(|img| img.path.parent().map(|p| p.to_path_buf()))
    }

    pub fn scroll_to_percent(&self, percent: f32) {
        let mut state = self.state.lock().unwrap();
        let (viewport_w, viewport_h) = {
            let size = state.last_physical_size;
            (size.0 as f32, size.1 as f32)
        };
        let max_y = state.physics.calculate_bounds(viewport_w, viewport_h).1;
        if max_y > 0.0 {
            // Corrected mapping: 0.0 is top (max_y), 1.0 is bottom (-max_y)
            let target_y = max_y - percent * 2.0 * max_y;
            state.physics.target_offset.y = target_y;
            state.physics.manual_dirty = true;
            self.request_redraw();
        }
    }

    pub fn adjust_width_percent(&self, percent: f32) {
        let mut state = self.state.lock().unwrap();
        state.physics.long_image_width_ratio = percent;
        state.image_dirty = true;
        self.request_redraw();
    }

    /// 请求重新绘制 UI 窗口
    pub fn request_redraw(&self) {
        if let Some(ui) = self.ui_weak.upgrade() {
            ui.window().request_redraw();
        }
    }

    /// 获取当前物理缩放因子
    fn get_scale_factor(&self) -> f32 {
        self.ui_weak
            .upgrade()
            .map(|ui| ui.window().scale_factor())
            .unwrap_or(1.0)
    }

    pub fn toggle_viewport_lock(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        state.physics.lock_viewport = !state.physics.lock_viewport;
        state.physics.lock_viewport
    }
}
