use lru::LruCache;
use std::num::NonZeroUsize;
use wgpu;

#[derive(Hash, Eq, PartialEq, Clone, Copy, Debug)]
pub struct TileKey {
    pub x: u32,
    pub y: u32,
    pub lod: u32,
}

pub struct MappedTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub uploaded_at: std::time::Instant,
    pub tile_buffer: wgpu::Buffer,
    pub tile_bind_group: wgpu::BindGroup,
}

pub struct TileCache {
    tiles: LruCache<TileKey, MappedTexture>,
}

impl Default for TileCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Maximum number of tiles to keep in the LRU cache.
/// 512 tiles * 4MB (1024x1024 RGBA) = 2048 MB VRAM.
/// This allows an entire 180MP image (e.g. 16464x10976, which is ~180 tiles) to comfortably sit entirely in VRAM without eviction!
pub const MAX_TILES: usize = 512;

impl TileCache {
    pub fn new() -> Self {
        tracing::info!(
            "TileCache initialized with bounded LRU capacity ({} tiles).",
            MAX_TILES
        );
        Self {
            tiles: LruCache::new(NonZeroUsize::new(MAX_TILES).unwrap()),
        }
    }

    pub fn insert(&mut self, key: TileKey, texture: MappedTexture) {
        tracing::debug!(?key, "Inserting tile into cache");
        self.tiles.put(key, texture);
    }

    pub fn get(&mut self, key: &TileKey) -> Option<&MappedTexture> {
        self.tiles.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&TileKey, &MappedTexture)> {
        self.tiles.iter()
    }

    pub fn clear(&mut self) {
        let count = self.tiles.len();
        self.tiles.clear();
        tracing::info!(count, "TileCache cleared, all textures released.");
    }
}

impl Drop for TileCache {
    fn drop(&mut self) {
        self.clear();
    }
}
