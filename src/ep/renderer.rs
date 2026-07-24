use crate::ep::physics::ContinuousImageInfo;
use crate::ep::tile_cache::{MappedTexture, TileCache, TileKey};
use crate::ep::{EngineCommand, ViewportUniform};
use image::DynamicImage;
use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use wgpu;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    tex_coords: [f32; 2],
}

const VERTICES: &[Vertex] = &[
    // Triangle 1
    Vertex {
        position: [-0.5, -0.5, 0.0],
        tex_coords: [0.0, 0.0],
    }, // Top-Left
    Vertex {
        position: [-0.5, 0.5, 0.0],
        tex_coords: [0.0, 1.0],
    }, // Bottom-Left
    Vertex {
        position: [0.5, -0.5, 0.0],
        tex_coords: [1.0, 0.0],
    }, // Top-Right
    // Triangle 2
    Vertex {
        position: [0.5, -0.5, 0.0],
        tex_coords: [1.0, 0.0],
    }, // Top-Right
    Vertex {
        position: [-0.5, 0.5, 0.0],
        tex_coords: [0.0, 1.0],
    }, // Bottom-Left
    Vertex {
        position: [0.5, 0.5, 0.0],
        tex_coords: [1.0, 1.0],
    }, // Bottom-Right
];

const SHADER_SOURCE: &str = "
struct Viewport {
    view_proj: mat4x4<f32>,
    mouse_pos: vec2<f32>,
};

struct Tile {
    offset: vec2<f32>,
    scale: vec2<f32>,
    alpha: f32,
    channel_mode: u32,
};

@group(0) @binding(0)
var<uniform> viewport: Viewport;

@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(1) @binding(1)
var s_diffuse: sampler;

@group(2) @binding(0)
var<uniform> tile: Tile;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
};

@vertex
fn vs_main(
    model: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = model.tex_coords;
    
    // Map [-0.5, 0.5] to normalized tile region [offset - 0.5, offset + scale - 0.5]
    let center = tile.offset + tile.scale * 0.5 - vec2<f32>(0.5, 0.5);
    let pos = model.position.xy * tile.scale + center;
    
    out.clip_position = viewport.view_proj * vec4<f32>(pos, model.position.z, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    
    var out_color: vec4<f32>;
    if (tile.channel_mode == 0u) {
        out_color = color;
    } else if (tile.channel_mode == 1u) {
        out_color = vec4<f32>(color.r, color.r, color.r, 1.0);
    } else if (tile.channel_mode == 2u) {
        out_color = vec4<f32>(color.g, color.g, color.g, 1.0);
    } else if (tile.channel_mode == 3u) {
        out_color = vec4<f32>(color.b, color.b, color.b, 1.0);
    } else if (tile.channel_mode == 4u) {
        out_color = vec4<f32>(color.a, color.a, color.a, 1.0);
    } else {
        out_color = color;
    }
    
    return vec4<f32>(out_color.rgb, out_color.a * tile.alpha);
}
";

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TileUniform {
    pub offset: [f32; 2],
    pub scale: [f32; 2],
    pub alpha: f32,
    pub channel_mode: u32,
    pub padding: [f32; 2],
}

pub struct WgpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    tile_cache: TileCache,
    pub base_texture: Option<MappedTexture>,
    pub continuous_base_textures: std::collections::HashMap<std::path::PathBuf, MappedTexture>,
    pub is_long_image: bool,
    pub continuous_images: Vec<ContinuousImageInfo>,
    pub current_zoom: f32,
    pub current_offset: [f32; 2],
    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    viewport_buffer: wgpu::Buffer,
    viewport_bind_group: wgpu::BindGroup,
    tile_bind_group_layout: wgpu::BindGroupLayout,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    cmd_receiver: Receiver<EngineCommand>,
    pending_tiles: HashSet<TileKey>,
    last_matrix: [[f32; 4]; 4],
    pub image_size: [f32; 2],
    pipeline_cache: Option<wgpu::PipelineCache>,
    current_path: Option<std::path::PathBuf>,
    pub channel_mode: u32,
}

impl WgpuRenderer {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        cmd_receiver: Receiver<EngineCommand>,
    ) -> Self {
        tracing::info!("WgpuRenderer: Captured GPU Context and Command Bus");

        let (viewport_buffer, viewport_bind_group, viewport_layout) =
            create_viewport_resources(&device);

        let (_, _, tile_bind_group_layout) = create_tile_resources(&device);

        let (render_pipeline, texture_bind_group_layout, sampler, pipeline_cache) =
            create_render_pipeline(&device, &viewport_layout, &tile_bind_group_layout);

        use wgpu::util::DeviceExt;
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            device,
            queue,
            tile_cache: TileCache::new(),
            base_texture: None,
            continuous_base_textures: std::collections::HashMap::new(),
            is_long_image: false,
            continuous_images: Vec::new(),
            current_zoom: 1.0,
            current_offset: [0.0, 0.0],
            render_pipeline,
            vertex_buffer,
            viewport_buffer,
            viewport_bind_group,
            tile_bind_group_layout,
            texture_bind_group_layout,
            sampler,
            cmd_receiver,
            pending_tiles: HashSet::new(),
            last_matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            image_size: [1.0, 1.0],
            pipeline_cache,
            current_path: None,
            channel_mode: 0,
        }
    }

    pub fn has_tile(&mut self, key: &TileKey) -> bool {
        self.tile_cache.get(key).is_some() || self.pending_tiles.contains(key)
    }

    pub fn mark_pending(&mut self, key: &TileKey) -> bool {
        self.pending_tiles.insert(*key)
    }

    pub fn update_matrix(&mut self, view_proj: [[f32; 4]; 4]) {
        self.last_matrix = view_proj;
        self.write_viewport_uniform(view_proj, [0.0, 0.0]);
    }

    pub fn execute_commands(&mut self) {
        let mut last_matrix = None;
        let mut last_viewport = None;

        while let Ok(cmd) = self.cmd_receiver.try_recv() {
            match cmd {
                EngineCommand::ViewportUpdate {
                    width,
                    height,
                    mx,
                    my,
                } => {
                    last_viewport = Some((width, height, mx, my));
                }
                EngineCommand::MatrixUpdate { view_proj } => {
                    last_matrix = Some(view_proj);
                }
                EngineCommand::RequestTile { key } => {
                    if self.tile_cache.get(&key).is_some() {
                        continue;
                    }
                    if self.pending_tiles.insert(key) {
                        tracing::info!(?key, "EngineCommand: RequestTile accepted");
                    }
                }
                EngineCommand::UploadTile { key, image, path } => {
                    if Some(&path) == self.current_path.as_ref() {
                        self.upload_tile(key, image);
                    }
                    self.pending_tiles.remove(&key);
                }
                EngineCommand::UploadBase {
                    image,
                    path,
                    physical_width,
                    physical_height,
                } => {
                    self.upload_base(image, Some(path), physical_width, physical_height);
                }
                EngineCommand::SetImagePath { path } => {
                    if self.is_long_image && self.current_path.as_ref() != Some(&path) {
                        self.tile_cache.clear();
                        self.pending_tiles.clear();
                    }
                    self.current_path = Some(path);
                }
                EngineCommand::SetChannelMode { mode } => {
                    self.channel_mode = mode;
                }
            }
        }

        if let Some((_width, _height, mx, my)) = last_viewport {
            self.update_viewport(mx, my);
        }
        if let Some(view_proj) = last_matrix {
            self.update_matrix(view_proj);
        }
    }

    pub fn update_viewport(&self, mx: f32, my: f32) {
        self.write_viewport_uniform(self.last_matrix, [mx, my]);
    }

    pub fn upload_base(
        &mut self,
        image: DynamicImage,
        path: Option<std::path::PathBuf>,
        physical_width: u32,
        physical_height: u32,
    ) {
        if self.is_long_image {
            if let Some(p) = path {
                let texture = self.upload_texture(image);
                self.continuous_base_textures.insert(p, texture);
            }
        } else {
            self.tile_cache.clear();
            self.pending_tiles.clear();
            self.image_size = [physical_width as f32, physical_height as f32];
            self.current_path = path;

            let texture = self.upload_texture(image);
            self.base_texture = Some(texture);
            tracing::info!("Base LOD 0 texture uploaded and cached.");
        }
    }

    pub fn has_continuous_base(&self, path: &std::path::Path) -> bool {
        self.continuous_base_textures.contains_key(path)
    }

    pub fn upload_tile(&mut self, key: TileKey, image: DynamicImage) {
        let texture = self.upload_texture(image);
        self.tile_cache.insert(key, texture);
        tracing::info!(?key, "Tile uploaded and cached.");
    }

    fn upload_texture(&self, image: DynamicImage) -> MappedTexture {
        let (width, height) = (image.width(), image.height());
        let rgba = image.into_rgba8();

        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            texture_size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        use wgpu::util::DeviceExt;
        let dummy_uniform = TileUniform {
            offset: [0.0, 0.0],
            scale: [1.0, 1.0],
            alpha: 1.0,
            channel_mode: 0,
            padding: [0.0; 2],
        };
        let tile_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("MappedTexture Uniform Buffer"),
                contents: bytemuck::cast_slice(&[dummy_uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        let tile_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MappedTexture Tile Bind Group"),
            layout: &self.tile_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: tile_buffer.as_entire_binding(),
            }],
        });

        MappedTexture {
            texture,
            view,
            uploaded_at: std::time::Instant::now(),
            tile_buffer,
            tile_bind_group,
        }
    }

    pub fn create_texture_bind_group(&self, view: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Texture Bind Group"),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    pub fn has_animating_tiles(&self) -> bool {
        for (_, tile_tex) in self.tile_cache.iter() {
            if tile_tex.uploaded_at.elapsed().as_millis() < 150 {
                return true;
            }
        }
        false
    }

    pub fn render(
        &mut self,
        target_texture: &wgpu::Texture,
        current_path: Option<&std::path::PathBuf>,
    ) {
        let view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.viewport_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));

            if self.is_long_image && !self.continuous_images.is_empty() {
                // Sliding window memory management
                let total_height = self.continuous_images.last().unwrap().y_offset
                    + self.continuous_images.last().unwrap().scaled_height;
                let zoom = self.current_zoom;
                let offset_y = self.current_offset[1];
                let vh = target_texture.height() as f32;

                // Determine visible and active range in unscaled coordinate system
                let y_visible_start = total_height * 0.5 - (offset_y + vh * 0.5) / zoom;
                let y_visible_end = total_height * 0.5 - (offset_y - vh * 0.5) / zoom;

                let safety_margin = vh / zoom;
                let padded_start = y_visible_start - safety_margin;
                let padded_end = y_visible_end + safety_margin;

                let mut active_paths = std::collections::HashSet::new();
                for img in &self.continuous_images {
                    let img_start = img.y_offset;
                    let img_end = img.y_offset + img.scaled_height;

                    let is_active = !(img_end < padded_start || img_start > padded_end);
                    if is_active {
                        active_paths.insert(img.path.clone());
                    }
                }

                // Evict out-of-bounds textures to free VRAM
                self.continuous_base_textures
                    .retain(|path, _| active_paths.contains(path));

                // Draw visible sub-images that are uploaded
                for img in &self.continuous_images {
                    if let Some(mapped) = self.continuous_base_textures.get(&img.path) {
                        let uniform = TileUniform {
                            // Map local waterfall Y offset into [-0.5, 0.5] master space
                            offset: [0.0, img.y_offset / total_height],
                            scale: [1.0, img.scaled_height / total_height],
                            alpha: 1.0,
                            channel_mode: self.channel_mode,
                            padding: [0.0; 2],
                        };
                        self.queue.write_buffer(
                            &mapped.tile_buffer,
                            0,
                            bytemuck::cast_slice(&[uniform]),
                        );

                        let bind_group = self.create_texture_bind_group(&mapped.view);
                        render_pass.set_bind_group(1, &bind_group, &[]);
                        render_pass.set_bind_group(2, &mapped.tile_bind_group, &[]);
                        render_pass.draw(0..6, 0..1);
                    }
                }

                // Draw high-res tiles on top of the continuous layout for the active image
                self.draw_tiles(&mut render_pass, current_path);
            } else {
                // 1. Render Base LOD if available
                self.draw_base_lod(&mut render_pass);

                // 2. Render Tiles on top
                self.draw_tiles(&mut render_pass, current_path);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
    }

    fn draw_base_lod<'a>(&self, render_pass: &mut wgpu::RenderPass<'a>) {
        if let Some(base) = &self.base_texture {
            let uniform = TileUniform {
                offset: [0.0, 0.0],
                scale: [1.0, 1.0],
                alpha: 1.0,
                channel_mode: self.channel_mode,
                padding: [0.0; 2],
            };
            self.queue
                .write_buffer(&base.tile_buffer, 0, bytemuck::cast_slice(&[uniform]));

            let bind_group = self.create_texture_bind_group(&base.view);
            render_pass.set_bind_group(1, &bind_group, &[]);
            render_pass.set_bind_group(2, &base.tile_bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }
    }

    fn draw_tiles<'a>(
        &self,
        render_pass: &mut wgpu::RenderPass<'a>,
        current_path: Option<&std::path::PathBuf>,
    ) {
        let mut y_offset_ratio = 0.0;
        let mut original_w = self.image_size[0];
        let mut original_h = self.image_size[1];
        let mut local_scale_y = 1.0;

        let mut total_height = self.image_size[1];
        if self.is_long_image && !self.continuous_images.is_empty() {
            total_height = self.continuous_images.last().unwrap().y_offset
                + self.continuous_images.last().unwrap().scaled_height;
        }

        if self.is_long_image {
            if let Some(cp) = current_path {
                for img in &self.continuous_images {
                    if &img.path == cp {
                        y_offset_ratio = img.y_offset / total_height;
                        original_w = img.original_size.x;
                        original_h = img.original_size.y;
                        local_scale_y = img.scaled_height / total_height;
                        break;
                    }
                }
            }
        }

        for (key, tile_tex) in self.tile_cache.iter() {
            let tw_base = crate::utils::settings::get_settings().tile_size as f32;
            let elapsed = tile_tex.uploaded_at.elapsed().as_millis() as f32;
            let alpha = (elapsed / 150.0).min(1.0);

            // Avoid division by zero if original size is missing
            let w_div = if original_w > 0.0 {
                original_w
            } else {
                self.image_size[0]
            };
            let h_div = if original_h > 0.0 {
                original_h
            } else {
                self.image_size[1]
            };

            let uniform = TileUniform {
                offset: [
                    key.x as f32 * tw_base / w_div,
                    y_offset_ratio + (key.y as f32 * tw_base / h_div * local_scale_y),
                ],
                scale: [
                    tile_tex.texture.width() as f32 / w_div,
                    tile_tex.texture.height() as f32 / h_div * local_scale_y,
                ],
                alpha,
                channel_mode: self.channel_mode,
                padding: [0.0; 2],
            };
            self.queue
                .write_buffer(&tile_tex.tile_buffer, 0, bytemuck::cast_slice(&[uniform]));

            let bind_group = self.create_texture_bind_group(&tile_tex.view);
            render_pass.set_bind_group(1, &bind_group, &[]);
            render_pass.set_bind_group(2, &tile_tex.tile_bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }
    }

    pub fn teardown(&mut self) {
        tracing::info!("WgpuRenderer: Releasing GPU resources and saving Pipeline Cache");
        self.save_pipeline_cache();
        self.tile_cache.clear();
        self.pending_tiles.clear();
        self.base_texture = None;
    }

    fn write_viewport_uniform(&self, view_proj: [[f32; 4]; 4], mouse_pos: [f32; 2]) {
        let uniform = ViewportUniform {
            view_proj,
            mouse_pos,
            padding: [0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.viewport_buffer, 0, bytemuck::cast_slice(&[uniform]));
    }

    fn save_pipeline_cache(&self) {
        let Some(ref cache) = self.pipeline_cache else {
            return;
        };
        let Some(data) = cache.get_data() else {
            return;
        };
        if data.is_empty() {
            return;
        }
        let cache_dir = crate::utils::path_utils::get_app_cache_dir();
        if !cache_dir.exists() {
            let _ = std::fs::create_dir_all(&cache_dir);
        }
        let cache_path = cache_dir.join("wgpu_pipeline.cache");
        match std::fs::write(&cache_path, &data) {
            Ok(_) => tracing::info!("WGPU Pipeline Cache successfully saved to {:?}", cache_path),
            Err(e) => tracing::error!("Failed to write WGPU Pipeline Cache: {}", e),
        }
    }
}

// Private file-level helper functions for renderer initialization

fn create_viewport_resources(
    device: &wgpu::Device,
) -> (wgpu::Buffer, wgpu::BindGroup, wgpu::BindGroupLayout) {
    use wgpu::util::DeviceExt;
    let viewport_uniform = ViewportUniform {
        view_proj: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        mouse_pos: [0.0, 0.0],
        padding: [0.0, 0.0],
    };

    let viewport_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Viewport Uniform Buffer"),
        contents: bytemuck::cast_slice(&[viewport_uniform]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let viewport_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Viewport Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

    let viewport_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Viewport Bind Group"),
        layout: &viewport_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: viewport_buffer.as_entire_binding(),
        }],
    });

    (
        viewport_buffer,
        viewport_bind_group,
        viewport_bind_group_layout,
    )
}

fn create_tile_resources(
    device: &wgpu::Device,
) -> (wgpu::Buffer, wgpu::BindGroup, wgpu::BindGroupLayout) {
    use wgpu::util::DeviceExt;
    let tile_uniform = TileUniform {
        offset: [0.0, 0.0],
        scale: [1.0, 1.0],
        alpha: 1.0,
        channel_mode: 0,
        padding: [0.0; 2],
    };

    let tile_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Tile Uniform Buffer"),
        contents: bytemuck::cast_slice(&[tile_uniform]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let tile_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Tile Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

    let tile_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Tile Bind Group"),
        layout: &tile_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: tile_buffer.as_entire_binding(),
        }],
    });

    (tile_buffer, tile_bind_group, tile_bind_group_layout)
}

fn create_render_pipeline(
    device: &wgpu::Device,
    viewport_layout: &wgpu::BindGroupLayout,
    tile_layout: &wgpu::BindGroupLayout,
) -> (
    wgpu::RenderPipeline,
    wgpu::BindGroupLayout,
    wgpu::Sampler,
    Option<wgpu::PipelineCache>,
) {
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Dual Layer Shader"),
        source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER_SOURCE)),
    });

    let texture_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Texture Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Render Pipeline Layout"),
        bind_group_layouts: &[viewport_layout, &texture_bind_group_layout, tile_layout],
        immediate_size: 0,
    });

    let cache_path = crate::utils::path_utils::get_app_cache_dir().join("wgpu_pipeline.cache");
    let cache_data = std::fs::read(&cache_path).ok();
    let pipeline_cache = if device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        tracing::info!(
            "WgpuRenderer: PIPELINE_CACHE feature is supported. Initializing Pipeline Cache."
        );
        unsafe {
            Some(
                device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                    label: Some("PicaView WGPU Pipeline Cache"),
                    data: cache_data.as_deref(),
                    fallback: true,
                }),
            )
        }
    } else {
        tracing::warn!("WgpuRenderer: PIPELINE_CACHE feature is NOT supported on this device. Disabling Pipeline Cache.");
        None
    };

    let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Render Pipeline"),
        layout: Some(&render_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3,
                    1 => Float32x2,
                ],
            }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: pipeline_cache.as_ref(),
    });

    (
        render_pipeline,
        texture_bind_group_layout,
        sampler,
        pipeline_cache,
    )
}

#[cfg(test)]
#[path = "renderer_tests.rs"]
mod tests;
