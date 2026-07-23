use crate::ep::tile_cache::TileKey;
use crate::ep::EngineCommand;
use anyhow::{anyhow, Result};
use image::{DynamicImage, GenericImageView};
use std::fs::read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use zune_jpeg::JpegDecoder;

/// Unified tiled image decoder interface.
pub trait TiledImageDecoder: Send + Sync {
    /// Retrieve the original metadata dimensions without decoding full pixel buffers.
    fn dimensions(&self) -> (u32, u32);

    /// Decodes only the requested tiled bounding box.
    fn decode_tile(&self, x: u32, y: u32, w: u32, h: u32) -> Result<DynamicImage>;
}

#[derive(Debug, PartialEq)]
pub struct McuRegion {
    pub mcu_x: u32,
    pub mcu_y: u32,
    pub mcu_w: u32,
    pub mcu_h: u32,
}

pub struct McuInterceptor;

impl McuInterceptor {
    pub fn calculate_region(x: u32, y: u32, w: u32, h: u32, mcu_size: u32) -> McuRegion {
        let mcu_x = (x / mcu_size) * mcu_size;
        let mcu_y = (y / mcu_size) * mcu_size;
        let mcu_end_x = (x + w).div_ceil(mcu_size) * mcu_size;
        let mcu_end_y = (y + h).div_ceil(mcu_size) * mcu_size;

        McuRegion {
            mcu_x,
            mcu_y,
            mcu_w: mcu_end_x - mcu_x,
            mcu_h: mcu_end_y - mcu_y,
        }
    }
}

/// Jpeg specialized physical tile decoder using TurboJPEG ROI and Zune fallback.
pub struct JpegTiledDecoder {
    width: u32,
    height: u32,
    file_data: std::sync::Arc<Vec<u8>>,
    icc_profile: Option<Vec<u8>>,
}

impl JpegTiledDecoder {
    pub fn new(path: &Path) -> Result<Self> {
        let data = read(path)?;
        let decoder = libjpeg_turbo_rs::Decoder::new(&data)
            .map_err(|e| anyhow!("TurboJPEG header error: {:?}", e))?;
        let w = decoder.header().width as u32;
        let h = decoder.header().height as u32;

        let icc_profile = crate::utils::color_space::extract_icc_profile_from_bytes(&data);

        Ok(Self {
            width: w,
            height: h,
            file_data: std::sync::Arc::new(data),
            icc_profile,
        })
    }

    pub fn get_icc_profile(&self) -> Option<Vec<u8>> {
        self.icc_profile.clone()
    }

    pub fn decode_fast_preview(&self, target_w: u32, target_h: u32) -> Result<DynamicImage> {
        use libjpeg_turbo_rs::api::streaming::StreamingDecoder;
        use libjpeg_turbo_rs::{PixelFormat, ScalingFactor};

        let scale =
            TiledDecoder::determine_scale_factor(self.width, self.height, target_w, target_h);
        let mut decoder = StreamingDecoder::new(&self.file_data)
            .map_err(|e| anyhow!("StreamingDecoder creation failed: {:?}", e))?;

        decoder.set_scale(ScalingFactor::new(1, scale));
        decoder.set_output_format(PixelFormat::Rgb);

        let image = decoder
            .decode()
            .map_err(|e| anyhow!("StreamingDecoder decode failed: {:?}", e))?;

        let img_buf = image::ImageBuffer::<image::Rgb<u8>, _>::from_raw(
            image.width as u32,
            image.height as u32,
            image.data,
        )
        .ok_or_else(|| anyhow!("Failed to create ImageBuffer from streaming data"))?;

        Ok(DynamicImage::ImageRgb8(img_buf))
    }

    fn decode_jpeg_with_turbo(&self, x: u32, y: u32, w: u32, h: u32) -> Result<DynamicImage> {
        let mut decoder = libjpeg_turbo_rs::Decoder::new(&self.file_data)
            .map_err(|e| anyhow!("TurboJPEG error: {:?}", e))?;

        let header_w = decoder.header().width as u32;
        let header_h = decoder.header().height as u32;

        let max_h = decoder
            .header()
            .components
            .iter()
            .map(|c| c.horizontal_sampling)
            .max()
            .unwrap_or(1) as u32;
        let max_v = decoder
            .header()
            .components
            .iter()
            .map(|c| c.vertical_sampling)
            .max()
            .unwrap_or(1) as u32;
        let mcu_w = max_h * 8;
        let mcu_h = max_v * 8;

        let mcu_region = McuInterceptor::calculate_region(x, y, w, h, mcu_w.max(mcu_h));

        let crop_x = mcu_region.mcu_x.min(header_w - 1);
        let crop_y = mcu_region.mcu_y.min(header_h - 1);
        let crop_w = mcu_region.mcu_w.min(header_w - crop_x);
        let crop_h = mcu_region.mcu_h.min(header_h - crop_y);

        decoder.set_crop_region(
            crop_x as usize,
            crop_y as usize,
            crop_w as usize,
            crop_h as usize,
        );
        decoder.set_output_format(libjpeg_turbo_rs::PixelFormat::Rgb);
        decoder.set_dct_method(libjpeg_turbo_rs::DctMethod::IsFast);

        let image = decoder
            .decode_image()
            .map_err(|e| anyhow!("TurboJPEG decode error: {:?}", e))?;

        let out_w = image.width as u32;
        let out_h = image.height as u32;

        let img = image::ImageBuffer::<image::Rgb<u8>, _>::from_raw(out_w, out_h, image.data)
            .ok_or_else(|| anyhow!("Failed to create ImageBuffer from TurboJPEG data"))?;

        let dynamic_img = DynamicImage::ImageRgb8(img);

        let final_x = x.saturating_sub(crop_x);
        let final_y = y.saturating_sub(crop_y);
        let view_w = w.min(dynamic_img.width().saturating_sub(final_x));
        let view_h = h.min(dynamic_img.height().saturating_sub(final_y));

        let tile = dynamic_img
            .view(final_x, final_y, view_w, view_h)
            .to_image();
        Ok(DynamicImage::ImageRgba8(tile))
    }

    fn decode_jpeg_with_zune(&self, x: u32, y: u32, w: u32, h: u32) -> Result<DynamicImage> {
        let mut decoder = JpegDecoder::new(std::io::Cursor::new(&*self.file_data));

        let pixels = decoder
            .decode()
            .map_err(|e| anyhow!("Zune-JPEG error: {:?}", e))?;
        let (width, height) = decoder
            .dimensions()
            .ok_or_else(|| anyhow!("Failed to get JPEG dimensions"))?;

        let img =
            image::ImageBuffer::<image::Rgb<u8>, _>::from_raw(width as u32, height as u32, pixels)
                .ok_or_else(|| anyhow!("Failed to create ImageBuffer"))?;

        let dynamic_img = DynamicImage::ImageRgb8(img);
        let view_w = w.min(dynamic_img.width().saturating_sub(x));
        let view_h = h.min(dynamic_img.height().saturating_sub(y));
        let tile = dynamic_img.view(x, y, view_w, view_h).to_image();
        Ok(DynamicImage::ImageRgba8(tile))
    }
}

impl TiledImageDecoder for JpegTiledDecoder {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn decode_tile(&self, x: u32, y: u32, w: u32, h: u32) -> Result<DynamicImage> {
        match self.decode_jpeg_with_turbo(x, y, w, h) {
            Ok(img) => Ok(img),
            Err(e) => {
                tracing::warn!(error = %e, "TurboJPEG ROI decode failed, falling back to Zune full decode");
                self.decode_jpeg_with_zune(x, y, w, h)
            }
        }
    }
}

/// Helper function to transform various TIFF decoding buffers safely to RGBA8 format.
fn decoding_result_to_rgba(
    result: tiff::decoder::DecodingResult,
    color_type: tiff::ColorType,
) -> Result<Vec<u8>> {
    let raw_pixels: Vec<u8> = match result {
        tiff::decoder::DecodingResult::U8(v) => v,
        tiff::decoder::DecodingResult::U16(v) => v.iter().map(|&x| (x >> 8) as u8).collect(),
        tiff::decoder::DecodingResult::U32(v) => v.iter().map(|&x| (x >> 24) as u8).collect(),
        tiff::decoder::DecodingResult::U64(v) => v.iter().map(|&x| (x >> 56) as u8).collect(),
        tiff::decoder::DecodingResult::I8(v) => v.iter().map(|&x| (x as i16 + 128) as u8).collect(),
        tiff::decoder::DecodingResult::I16(v) => {
            v.iter().map(|&x| ((x as i32 + 32768) >> 8) as u8).collect()
        }
        _ => return Err(anyhow!("Unsupported TIFF pixel data representation")),
    };

    match color_type {
        tiff::ColorType::RGBA(_) => Ok(raw_pixels),
        tiff::ColorType::RGB(_) => {
            let mut rgba = Vec::with_capacity((raw_pixels.len() / 3) * 4);
            for chunk in raw_pixels.chunks_exact(3) {
                rgba.push(chunk[0]);
                rgba.push(chunk[1]);
                rgba.push(chunk[2]);
                rgba.push(255);
            }
            Ok(rgba)
        }
        tiff::ColorType::Gray(_) => {
            let mut rgba = Vec::with_capacity(raw_pixels.len() * 4);
            for &g in &raw_pixels {
                rgba.push(g);
                rgba.push(g);
                rgba.push(g);
                rgba.push(255);
            }
            Ok(rgba)
        }
        tiff::ColorType::GrayA(_) => {
            let mut rgba = Vec::with_capacity((raw_pixels.len() / 2) * 4);
            for chunk in raw_pixels.chunks_exact(2) {
                rgba.push(chunk[0]);
                rgba.push(chunk[0]);
                rgba.push(chunk[0]);
                rgba.push(chunk[1]);
            }
            Ok(rgba)
        }
        _ => Err(anyhow!("Unsupported TIFF color type: {:?}", color_type)),
    }
}

/// Native Tiled-TIFF Decoder supporting sub-tile random access on demand.
pub struct TiffTiledDecoder {
    path: PathBuf,
    width: u32,
    height: u32,
    icc_profile: Option<Vec<u8>>,
}

impl TiffTiledDecoder {
    pub fn new(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let mut decoder = tiff::decoder::Decoder::new(file)?;
        let (w, h) = decoder.dimensions()?;

        let icc_profile =
            crate::utils::color_space::extract_icc_profile(path.to_str().unwrap_or(""));

        Ok(Self {
            path: path.to_path_buf(),
            width: w,
            height: h,
            icc_profile,
        })
    }

    pub fn get_icc_profile(&self) -> Option<Vec<u8>> {
        self.icc_profile.clone()
    }
}

impl TiledImageDecoder for TiffTiledDecoder {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn decode_tile(&self, x: u32, y: u32, w: u32, h: u32) -> Result<DynamicImage> {
        let file = std::fs::File::open(&self.path)?;
        let mut decoder = tiff::decoder::Decoder::new(file)?;
        if decoder.get_chunk_type() == tiff::decoder::ChunkType::Tile {
            let (tile_w, tile_h) = decoder.chunk_dimensions();
            let tiles_across = self.width.div_ceil(tile_w);
            let col = x / tile_w;
            let row = y / tile_h;
            let tile_idx = row * tiles_across + col;

            let decoding_result = decoder.read_chunk(tile_idx)?;
            let color_type = decoder.colortype()?;
            let raw_pixels = decoding_result_to_rgba(decoding_result, color_type)?;

            let base_img = DynamicImage::ImageRgba8(
                image::ImageBuffer::from_raw(tile_w, tile_h, raw_pixels)
                    .ok_or_else(|| anyhow!("Failed to allocate tile buffer"))?,
            );
            let rel_x = x - col * tile_w;
            let rel_y = y - row * tile_h;
            let view_w = w.min(base_img.width().saturating_sub(rel_x));
            let view_h = h.min(base_img.height().saturating_sub(rel_y));
            let final_tile = base_img.view(rel_x, rel_y, view_w, view_h).to_image();
            Ok(DynamicImage::ImageRgba8(final_tile))
        } else {
            let mut limits = tiff::decoder::Limits::default();
            limits.decoding_buffer_size = 256 * 1024 * 1024;
            decoder = decoder.with_limits(limits);

            let full_img = decoder.read_image()?;
            let color_type = decoder.colortype()?;
            let raw_pixels = decoding_result_to_rgba(full_img, color_type)?;
            let base_img = DynamicImage::ImageRgba8(
                image::ImageBuffer::from_raw(self.width, self.height, raw_pixels)
                    .ok_or_else(|| anyhow!("Failed to allocate full image buffer"))?,
            );
            let view_w = w.min(base_img.width().saturating_sub(x));
            let view_h = h.min(base_img.height().saturating_sub(y));
            let cropped = base_img.view(x, y, view_w, view_h).to_image();
            Ok(DynamicImage::ImageRgba8(cropped))
        }
    }
}

/// Fallback decoder handling PNG, BMP, WebP with a defensive 256MB memory cap & LOD Downsampling.
pub struct FallbackTiledDecoder {
    path: PathBuf,
    width: u32,
    height: u32,
    scaled_image: std::sync::Mutex<Option<DynamicImage>>,
    icc_profile: Option<Vec<u8>>,
}

impl FallbackTiledDecoder {
    pub fn new(path: &Path) -> Result<Self> {
        let mut reader = image::ImageReader::open(path)?;
        reader.no_limits();
        let (w, h) = reader.into_dimensions()?;

        let icc_profile =
            crate::utils::color_space::extract_icc_profile(path.to_str().unwrap_or(""));

        Ok(Self {
            path: path.to_path_buf(),
            width: w,
            height: h,
            scaled_image: std::sync::Mutex::new(None),
            icc_profile,
        })
    }

    pub fn get_icc_profile(&self) -> Option<Vec<u8>> {
        self.icc_profile.clone()
    }

    fn get_safe_image(&self) -> Result<DynamicImage> {
        if let Some(cached) = &*self.scaled_image.lock().unwrap() {
            return Ok(cached.clone());
        }

        let orig_size = self.width as u64 * self.height as u64 * 4;
        let max_alloc = 256 * 1024 * 1024; // 256MB

        if orig_size <= max_alloc {
            let mut reader = image::ImageReader::open(&self.path)?;
            let mut limits = image::Limits::default();
            limits.max_alloc = Some(max_alloc);
            reader.limits(limits);
            let img = reader.decode()?;
            *self.scaled_image.lock().unwrap() = Some(img.clone());
            Ok(img)
        } else {
            tracing::warn!(
                path = ?self.path,
                width = self.width,
                height = self.height,
                "Image raw size exceeds 256MB. Activating fallback LOD downsampling."
            );

            let mut reader = image::ImageReader::open(&self.path)?;
            let mut limits = image::Limits::default();
            limits.max_alloc = Some(max_alloc);
            reader.limits(limits);

            match reader.decode() {
                Ok(img) => {
                    let scaled = img.thumbnail(4096, 4096);
                    *self.scaled_image.lock().unwrap() = Some(scaled.clone());
                    Ok(scaled)
                }
                Err(e) => {
                    tracing::error!(
                        path = ?self.path,
                        error = %e,
                        "Memory safety guard intercepted huge raw buffer allocation."
                    );
                    Err(anyhow!("Image safety allocation limit exceeded: {:?}", e))
                }
            }
        }
    }
}

impl TiledImageDecoder for FallbackTiledDecoder {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn decode_tile(&self, x: u32, y: u32, w: u32, h: u32) -> Result<DynamicImage> {
        let base_img = self.get_safe_image()?;
        let (bw, bh) = (base_img.width(), base_img.height());

        if bw == self.width && bh == self.height {
            let view_w = w.min(bw.saturating_sub(x));
            let view_h = h.min(bh.saturating_sub(y));
            let tile = base_img.view(x, y, view_w, view_h).to_image();
            Ok(DynamicImage::ImageRgba8(tile))
        } else {
            let sx = bw as f32 / self.width as f32;
            let sy = bh as f32 / self.height as f32;

            let mapped_x = ((x as f32 * sx).floor() as u32).min(bw - 1);
            let mapped_y = ((y as f32 * sy).floor() as u32).min(bh - 1);
            let mapped_w = (((w as f32 * sx).ceil() as u32).max(1)).min(bw - mapped_x);
            let mapped_h = (((h as f32 * sy).ceil() as u32).max(1)).min(bh - mapped_y);

            let tile = base_img
                .view(mapped_x, mapped_y, mapped_w, mapped_h)
                .to_image();
            let dynamic_tile = DynamicImage::ImageRgba8(tile);
            let resized_tile =
                dynamic_tile.resize_exact(w, h, image::imageops::FilterType::Triangle);
            Ok(resized_tile)
        }
    }
}

#[cfg(target_os = "windows")]
pub struct WicTiledDecoder {
    decoder: crate::ep::wic_decoder::WicDecoder,
    icc_profile: Option<Vec<u8>>,
}

#[cfg(not(target_os = "windows"))]
pub struct WicTiledDecoder {}

#[cfg(target_os = "windows")]
impl WicTiledDecoder {
    pub fn new(path: &Path) -> Result<Self> {
        let decoder = crate::ep::wic_decoder::WicDecoder::new(path)?;

        let data = std::fs::read(path).unwrap_or_default();
        let icc_profile = crate::utils::color_space::extract_icc_profile_from_bytes(&data);

        Ok(Self {
            decoder,
            icc_profile,
        })
    }

    pub fn get_icc_profile(&self) -> Option<Vec<u8>> {
        self.icc_profile.clone()
    }
}

#[cfg(not(target_os = "windows"))]
impl WicTiledDecoder {
    pub fn new(_path: &Path) -> Result<Self> {
        Err(anyhow::anyhow!("WIC is not supported on this platform"))
    }
    pub fn get_icc_profile(&self) -> Option<Vec<u8>> {
        None
    }
}

#[cfg(target_os = "windows")]
impl TiledImageDecoder for WicTiledDecoder {
    fn dimensions(&self) -> (u32, u32) {
        self.decoder.dimensions()
    }

    fn decode_tile(&self, x: u32, y: u32, w: u32, h: u32) -> Result<DynamicImage> {
        let (width, height) = self.decoder.dimensions();
        let safe_w = if x + w > width {
            width.saturating_sub(x)
        } else {
            w
        };
        let safe_h = if y + h > height {
            height.saturating_sub(y)
        } else {
            h
        };

        if safe_w == 0 || safe_h == 0 {
            return Ok(DynamicImage::ImageRgba8(image::RgbaImage::new(1, 1)));
        }

        let buffer = self.decoder.extract_tile(x, y, safe_w, safe_h)?;
        let img_buf = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(safe_w, safe_h, buffer)
            .ok_or_else(|| anyhow!("Failed to create ImageBuffer from WIC tile buffer"))?;

        Ok(DynamicImage::ImageRgba8(img_buf))
    }
}

#[cfg(not(target_os = "windows"))]
impl TiledImageDecoder for WicTiledDecoder {
    fn dimensions(&self) -> (u32, u32) {
        (0, 0)
    }
    fn decode_tile(&self, _x: u32, _y: u32, _w: u32, _h: u32) -> Result<DynamicImage> {
        Err(anyhow::anyhow!("WIC is not supported on this platform"))
    }
}

/// High-performance unified Dispatcher enum.
pub enum TiledDecoder {
    Wic(WicTiledDecoder),
    Jpeg(JpegTiledDecoder),
    Tiff(TiffTiledDecoder),
    Fallback(FallbackTiledDecoder),
}

impl TiledDecoder {
    pub fn new(path: &Path) -> Result<Self> {
        // 对于非压缩包内的常规文件，优先尝试 WIC
        if crate::core::image_loader::parse_archive_path(path.to_string_lossy().as_ref())
            .is_none()
        {
            if let Ok(wic_dec) = WicTiledDecoder::new(path) {
                return Ok(TiledDecoder::Wic(wic_dec));
            }
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        match ext.as_str() {
            "jpg" | "jpeg" => Ok(TiledDecoder::Jpeg(JpegTiledDecoder::new(path)?)),
            "tif" | "tiff" => Ok(TiledDecoder::Tiff(TiffTiledDecoder::new(path)?)),
            _ => Ok(TiledDecoder::Fallback(FallbackTiledDecoder::new(path)?)),
        }
    }

    pub fn determine_scale_factor(orig_w: u32, orig_h: u32, target_w: u32, target_h: u32) -> u32 {
        if target_w == 0 || target_h == 0 {
            return 8;
        }
        for &scale in &[8, 4, 2] {
            if (orig_w / scale) >= target_w && (orig_h / scale) >= target_h {
                return scale;
            }
        }
        1
    }

    pub fn get_icc_profile(&self) -> Option<Vec<u8>> {
        match self {
            TiledDecoder::Wic(d) => d.get_icc_profile(),
            TiledDecoder::Jpeg(d) => d.get_icc_profile(),
            TiledDecoder::Tiff(d) => d.get_icc_profile(),
            TiledDecoder::Fallback(d) => d.get_icc_profile(),
        }
    }

    pub fn decode_fast_preview(&self, target_w: u32, target_h: u32) -> Result<DynamicImage> {
        let mut img = match self {
            TiledDecoder::Wic(d) => {
                let (w, h) = d.dimensions();
                d.decode_tile(0, 0, w, h)
                    .map(|img| img.thumbnail(target_w, target_h))
            }
            TiledDecoder::Jpeg(d) => d.decode_fast_preview(target_w, target_h),
            TiledDecoder::Tiff(d) => {
                let (w, h) = d.dimensions();
                d.decode_tile(0, 0, w, h)
                    .map(|img| img.thumbnail(target_w, target_h))
            }
            TiledDecoder::Fallback(d) => {
                let base_img = d.get_safe_image()?;
                Ok(base_img.thumbnail(target_w, target_h))
            }
        }?;

        if let Some(icc) = self.get_icc_profile() {
            let _ = crate::utils::color_space::apply_icc_color_correction(&mut img, &icc);
        }
        Ok(img)
    }

    pub fn decode_tile(&self, x: u32, y: u32, w: u32, h: u32) -> Result<DynamicImage> {
        let mut img = match self {
            TiledDecoder::Wic(d) => d.decode_tile(x, y, w, h),
            TiledDecoder::Jpeg(d) => d.decode_tile(x, y, w, h),
            TiledDecoder::Tiff(d) => d.decode_tile(x, y, w, h),
            TiledDecoder::Fallback(d) => d.decode_tile(x, y, w, h),
        }?;

        if let Some(icc) = self.get_icc_profile() {
            let _ = crate::utils::color_space::apply_icc_color_correction(&mut img, &icc);
        }
        Ok(img)
    }

    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            TiledDecoder::Wic(d) => d.dimensions(),
            TiledDecoder::Jpeg(d) => d.dimensions(),
            TiledDecoder::Tiff(d) => d.dimensions(),
            TiledDecoder::Fallback(d) => d.dimensions(),
        }
    }
}

/// Safe backpressured AsyncDecoder limiting concurrent threads and dropping stale tasks before/after locks.
#[allow(clippy::type_complexity)]
pub struct AsyncDecoder {
    tx: Sender<EngineCommand>,
    semaphore: std::sync::Arc<tokio::sync::Semaphore>,
    active_path: std::sync::Arc<std::sync::Mutex<Option<PathBuf>>>,
    active_decoder:
        std::sync::Arc<std::sync::Mutex<Option<(PathBuf, std::sync::Arc<TiledDecoder>)>>>,
}

impl AsyncDecoder {
    pub fn new(tx: Sender<EngineCommand>) -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Self {
            tx,
            semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(cpus)),
            active_path: std::sync::Arc::new(std::sync::Mutex::new(None)),
            active_decoder: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub fn set_active_path(&self, path: Option<PathBuf>) {
        *self.active_path.lock().unwrap() = path.clone();

        // Evict older decoder to free RAM resources immediately when path changes
        let mut active_dec = self.active_decoder.lock().unwrap();
        if let Some((ref cached_path, _)) = *active_dec {
            if path.as_ref() != Some(cached_path) {
                *active_dec = None;
            }
        }
    }

    pub fn request_tile(&self, key: TileKey, path: PathBuf, x: u32, y: u32, w: u32, h: u32) {
        // Fast-path pre-check
        {
            let active = self.active_path.lock().unwrap();
            if active.as_ref() != Some(&path) {
                return;
            }
        }

        let tx = self.tx.clone();
        let semaphore = self.semaphore.clone();
        let active_path = self.active_path.clone();
        let active_decoder = self.active_decoder.clone();
        let path_clone = path.clone();

        tokio::task::spawn(async move {
            let _permit = match semaphore.acquire().await {
                Ok(p) => p,
                Err(_) => return,
            };

            // Double check after queue wait
            {
                let active = active_path.lock().unwrap();
                if active.as_ref() != Some(&path_clone) {
                    return;
                }
            }

            // Retrieve or instantiate active TiledDecoder without holding locks across yield points
            let decoder_res = {
                let mut active_dec = active_decoder.lock().unwrap();
                if let Some((ref cached_path, ref dec)) = *active_dec {
                    if cached_path == &path_clone {
                        Ok(dec.clone())
                    } else {
                        match TiledDecoder::new(&path_clone) {
                            Ok(new_dec) => {
                                let shared_dec = std::sync::Arc::new(new_dec);
                                *active_dec = Some((path_clone.clone(), shared_dec.clone()));
                                Ok(shared_dec)
                            }
                            Err(e) => Err(e),
                        }
                    }
                } else {
                    match TiledDecoder::new(&path_clone) {
                        Ok(new_dec) => {
                            let shared_dec = std::sync::Arc::new(new_dec);
                            *active_dec = Some((path_clone.clone(), shared_dec.clone()));
                            Ok(shared_dec)
                        }
                        Err(e) => Err(e),
                    }
                }
            };

            let decoder = match decoder_res {
                Ok(dec) => dec,
                Err(e) => {
                    tracing::error!(?key, error = %e, "Failed to initialize decoder for request");
                    return;
                }
            };

            let decode_res =
                tokio::task::spawn_blocking(move || decoder.decode_tile(x, y, w, h)).await;

            match decode_res {
                Ok(Ok(image)) => {
                    let _ = tx.send(EngineCommand::UploadTile {
                        key,
                        image,
                        path: path_clone,
                    });
                    tracing::info!(?key, "Async decoding complete");
                }
                Ok(Err(e)) => {
                    tracing::error!(?key, error = %e, "Async decoding failed");
                }
                Err(e) => {
                    tracing::error!(?key, error = %e, "Blocking join task failed");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use tokio::runtime::Runtime;

    #[test]
    fn test_mcu_interception_calculation() {
        let region = McuInterceptor::calculate_region(10, 20, 256, 256, 16);
        assert_eq!(region.mcu_x, 0);
        assert_eq!(region.mcu_y, 16);
        assert_eq!(region.mcu_w, 272);
        assert_eq!(region.mcu_h, 272);
    }

    #[test]
    fn test_zune_jpeg_integration_check() {
        let img = image::ImageBuffer::<image::Rgb<u8>, _>::new(16, 16);
        let dynamic_img = image::DynamicImage::ImageRgb8(img);
        let tmp_file = tempfile::Builder::new().suffix(".jpg").tempfile().unwrap();
        dynamic_img
            .save_with_format(tmp_file.path(), image::ImageFormat::Jpeg)
            .unwrap();

        let decoder = TiledDecoder::new(tmp_file.path()).unwrap();
        let result = decoder.decode_tile(0, 0, 8, 8);
        assert!(result.is_ok());
        let decoded = result.unwrap();
        assert_eq!(decoded.width(), 8);
        assert_eq!(decoded.height(), 8);
    }

    #[test]
    fn test_async_decoding() {
        let rt = Runtime::new().unwrap();
        let (tx, rx) = mpsc::channel::<EngineCommand>();
        let decoder = AsyncDecoder::new(tx);

        let img = image::ImageBuffer::<image::Rgb<u8>, _>::new(16, 16);
        let dynamic_img = image::DynamicImage::ImageRgb8(img);
        let tmp_file = tempfile::Builder::new().suffix(".jpg").tempfile().unwrap();
        dynamic_img
            .save_with_format(tmp_file.path(), image::ImageFormat::Jpeg)
            .unwrap();

        let key = TileKey { x: 0, y: 0, lod: 0 };
        let path = tmp_file.path().to_path_buf();

        decoder.set_active_path(Some(path.clone()));

        rt.block_on(async {
            decoder.request_tile(key, path, 0, 0, 8, 8);
            let timeout = std::time::Duration::from_secs(5);
            let start = std::time::Instant::now();
            loop {
                if let Ok(EngineCommand::UploadTile { key: k, .. }) = rx.try_recv() {
                    assert_eq!(k, key);
                    break;
                }
                if start.elapsed() > timeout {
                    panic!("Timed out");
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        });
    }

    #[test]
    fn test_turbo_roi_math() {
        let img = image::ImageBuffer::<image::Rgb<u8>, _>::new(100, 100);
        let dynamic_img = image::DynamicImage::ImageRgb8(img);
        let tmp_file = tempfile::Builder::new().suffix(".jpg").tempfile().unwrap();
        dynamic_img
            .save_with_format(tmp_file.path(), image::ImageFormat::Jpeg)
            .unwrap();

        let decoder = TiledDecoder::new(tmp_file.path()).unwrap();
        let result = decoder.decode_tile(10, 20, 30, 40);
        assert!(result.is_ok());
        let decoded = result.unwrap();
        assert_eq!(decoded.width(), 30);
        assert_eq!(decoded.height(), 40);
    }
}
