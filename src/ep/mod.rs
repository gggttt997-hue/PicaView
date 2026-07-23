pub mod decoder;
pub mod physics;
pub mod renderer;
pub mod tile_cache;
#[cfg(target_os = "windows")]
pub mod wic_decoder;

use crate::ep::tile_cache::TileKey;
use image::DynamicImage;
pub use physics::ContinuousImageInfo;

#[derive(Debug)]
pub enum EngineCommand {
    ViewportUpdate {
        width: f32,
        height: f32,
        mx: f32,
        my: f32,
    },
    MatrixUpdate {
        view_proj: [[f32; 4]; 4],
    },
    RequestTile {
        key: TileKey,
    },
    UploadTile {
        key: TileKey,
        image: DynamicImage,
        path: std::path::PathBuf,
    },
    UploadBase {
        image: DynamicImage,
        path: std::path::PathBuf,
        physical_width: u32,
        physical_height: u32,
    },
    SetImagePath {
        path: std::path::PathBuf,
    },
    SetChannelMode {
        mode: u32,
    },
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ViewportUniform {
    pub view_proj: [[f32; 4]; 4],
    pub mouse_pos: [f32; 2],
    pub padding: [f32; 2],
}
