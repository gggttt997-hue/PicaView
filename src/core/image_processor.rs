use crate::error::{PicaViewError, Result};
use base64::{engine::general_purpose, Engine as _};
use image::{imageops, DynamicImage, ImageFormat, Rgb, RgbImage};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tracing::{debug, error, info};

/// Supported output formats for conversion
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    Jpeg,
    Png,
    WebP,
    Tiff,
    Bmp,
}

impl From<OutputFormat> for ImageFormat {
    fn from(format: OutputFormat) -> Self {
        match format {
            OutputFormat::Jpeg => ImageFormat::Jpeg,
            OutputFormat::Png => ImageFormat::Png,
            OutputFormat::WebP => ImageFormat::WebP,
            OutputFormat::Tiff => ImageFormat::Tiff,
            OutputFormat::Bmp => ImageFormat::Bmp,
        }
    }
}

impl OutputFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            OutputFormat::Jpeg => "jpg",
            OutputFormat::Png => "png",
            OutputFormat::WebP => "webp",
            OutputFormat::Tiff => "tiff",
            OutputFormat::Bmp => "bmp",
        }
    }
}

/// Direction for image stitching
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
pub enum StitchDirection {
    Horizontal,
    Vertical,
}

/// Configuration for image stitching
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StitchConfig {
    pub direction: StitchDirection,
    pub align: bool,
    pub gap: u32,
    pub background_color: [u8; 3], // RGB
}

/// Core processor for batch image operations
pub struct BatchProcessor;

impl Default for BatchProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchProcessor {
    pub fn new() -> Self {
        Self
    }

    /// Perform batch conversion of images
    pub fn convert<F>(
        &self,
        input_paths: Vec<String>,
        target_format: OutputFormat,
        output_dir: &str,
        on_progress: F,
    ) -> Result<Vec<String>>
    where
        F: Fn(usize, usize) + Send + Sync,
    {
        let mut created_files = Vec::new();
        let image_format: ImageFormat = target_format.into();
        let extension = target_format.extension();
        let total = input_paths.len();

        info!(
            location = "BatchProcessor::convert",
            task = "batch_convert",
            count = total,
            format = ?target_format,
            "Starting batch conversion"
        );

        for (i, path_str) in input_paths.into_iter().enumerate() {
            let input_path = Path::new(&path_str);
            let file_name = input_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("converted");

            let output_path =
                PathBuf::from(output_dir).join(format!("{}.{}", file_name, extension));

            match self.convert_single(input_path, &output_path, image_format) {
                Ok(_) => {
                    created_files.push(output_path.to_string_lossy().to_string());
                    on_progress(i + 1, total);
                    debug!(
                        path = %path_str,
                        output = %output_path.display(),
                        "Successfully converted image"
                    );
                }
                Err(e) => {
                    error!(
                        path = %path_str,
                        error = ?e,
                        "Conversion failed, triggering Fail-Fast cleanup"
                    );

                    // Zero-Residue cleanup
                    for created in created_files {
                        let _ = fs::remove_file(created);
                    }

                    return Err(e);
                }
            }
        }

        info!(
            count = created_files.len(),
            "Batch conversion completed successfully"
        );

        Ok(created_files)
    }

    fn convert_single(&self, input: &Path, output: &Path, format: ImageFormat) -> Result<()> {
        let img = decode_image(input)?;
        img.save_with_format(output, format)?;
        Ok(())
    }
}

/// Core processor for image stitching
pub struct ImageStitcher;

impl Default for ImageStitcher {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageStitcher {
    pub fn new() -> Self {
        Self
    }

    /// Generate a low-resolution preview of stitched images in memory (Base64)
    /// Prioritizes L2 cache reuse for near-zero IO synthesis
    pub fn generate_preview(
        &self,
        input_paths: Vec<String>,
        config: StitchConfig,
        cache_dir: Option<PathBuf>,
    ) -> Result<String> {
        if input_paths.is_empty() {
            return Err(PicaViewError::general("No images provided for preview"));
        }

        debug!(
            location = "ImageStitcher::generate_preview",
            count = input_paths.len(),
            "Starting zero-disk preview generation"
        );

        let mut images: Vec<DynamicImage> = input_paths
            .par_iter()
            .map(|path| self.load_preview_asset(path, cache_dir.as_ref()))
            .collect::<Result<Vec<_>>>()?;

        if config.align {
            align_images(
                &mut images,
                config.direction,
                imageops::FilterType::Triangle,
            );
        }

        let (total_w, total_h) = calculate_canvas_dimensions(&images, &config);

        // Safety limit for preview (prevent OOM)
        if total_w > 8192 || total_h > 8192 {
            return Err(PicaViewError::general("Preview canvas exceeds 8K limit"));
        }

        let canvas = draw_images_to_canvas(images, total_w, total_h, &config);

        let mut buffer = Cursor::new(Vec::new());
        canvas
            .write_to(&mut buffer, ImageFormat::WebP)
            .map_err(|e| PicaViewError::general(format!("Preview encoding failed: {}", e)))?;

        let encoded = general_purpose::STANDARD.encode(buffer.into_inner());
        Ok(format!("data:image/webp;base64,{}", encoded))
    }

    /// Load preview asset favoring L2 disk cache
    fn load_preview_asset(&self, path: &str, cache_dir: Option<&PathBuf>) -> Result<DynamicImage> {
        // Step A: Attempt L2 Cache Hit
        if let Some(dir) = cache_dir {
            if let Some(img) = self.try_load_from_l2_cache(path, dir) {
                return Ok(img);
            }
        }

        // Step B: Fast-Decode Fallback for JPEG
        if let Some(img) = self.try_fast_decode_jpeg(path) {
            return Ok(img);
        }

        // Step C: Standard Load (Slow Path)
        let img = decode_image(path)?;
        Ok(img.thumbnail(256, 256))
    }

    fn try_load_from_l2_cache(&self, path: &str, cache_dir: &Path) -> Option<DynamicImage> {
        let mtime = fs::metadata(path).and_then(|m| m.modified()).ok()?;
        let cache_key = crate::core::cache_manager::CacheManager::generate_cache_key(
            Path::new(path),
            mtime,
            256,
        );
        let info = crate::core::cache_manager::CacheManager::load_from_disk(cache_dir, &cache_key)?;
        let proxy_path = info.proxy_path?;
        image::open(proxy_path).ok()
    }

    fn try_fast_decode_jpeg(&self, path: &str) -> Option<DynamicImage> {
        let ext = Path::new(path).extension()?.to_str()?.to_lowercase();
        if ext != "jpg" && ext != "jpeg" {
            return None;
        }
        let data = fs::read(path).ok()?;
        let options = zune_core::options::DecoderOptions::default();
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(Cursor::new(&data), options);
        let pixels = decoder.decode().ok()?;
        let info = decoder.info()?;
        let img = RgbImage::from_raw(info.width as u32, info.height as u32, pixels)?;
        Some(DynamicImage::ImageRgb8(img).thumbnail(256, 256))
    }

    /// Stitch multiple images into one
    pub fn stitch(
        &self,
        input_paths: Vec<String>,
        config: StitchConfig,
        output_path: &str,
    ) -> Result<String> {
        if input_paths.is_empty() {
            return Err(PicaViewError::general("No images provided for stitching"));
        }

        info!(
            location = "ImageStitcher::stitch",
            task = "image_stitch",
            count = input_paths.len(),
            direction = ?config.direction,
            "Starting image stitching"
        );

        let mut images = Vec::new();
        for path in &input_paths {
            let img = decode_image(path)?;
            images.push(img);
        }

        if config.align {
            align_images(
                &mut images,
                config.direction,
                imageops::FilterType::Lanczos3,
            );
        }

        let (total_w, total_h) = calculate_canvas_dimensions(&images, &config);

        // Memory Guard: check if canvas exceeds safety threshold (e.g., 2GB)
        let estimated_bytes = total_w as u64 * total_h as u64 * 3;
        if estimated_bytes > 2 * 1024 * 1024 * 1024 {
            error!(
                width = total_w,
                height = total_h,
                bytes = estimated_bytes,
                "Canvas size exceeds safety threshold (2GB)"
            );
            return Err(PicaViewError::general(
                "Canvas size too large, synthesis aborted for memory safety",
            ));
        }

        let canvas = draw_images_to_canvas(images, total_w, total_h, &config);

        canvas
            .save(output_path)
            .map_err(|e| PicaViewError::decode_error(output_path, e.to_string()))?;

        info!(
            width = total_w,
            height = total_h,
            "Stitching completed successfully"
        );

        Ok(output_path.to_string())
    }
}

// Private helper functions to align images, calculate bounds, composite, and decode.

fn decode_image<P: AsRef<Path>>(path: P) -> Result<DynamicImage> {
    let p = path.as_ref();
    image::ImageReader::open(p)?
        .with_guessed_format()
        .map_err(|e| PicaViewError::decode_error(p.to_string_lossy(), e.to_string()))?
        .decode()
        .map_err(|e| PicaViewError::decode_error(p.to_string_lossy(), e.to_string()))
}

fn align_images(
    images: &mut [DynamicImage],
    direction: StitchDirection,
    filter: imageops::FilterType,
) {
    if images.len() <= 1 {
        return;
    }
    let target_w = images[0].width();
    let target_h = images[0].height();

    images
        .par_iter_mut()
        .skip(1)
        .for_each(|img| match direction {
            StitchDirection::Horizontal => {
                if img.height() != target_h {
                    let new_w =
                        (img.width() as f32 * (target_h as f32 / img.height() as f32)) as u32;
                    *img = img.resize(new_w, target_h, filter);
                }
            }
            StitchDirection::Vertical => {
                if img.width() != target_w {
                    let new_h =
                        (img.height() as f32 * (target_w as f32 / img.width() as f32)) as u32;
                    *img = img.resize(target_w, new_h, filter);
                }
            }
        });
}

fn calculate_canvas_dimensions(images: &[DynamicImage], config: &StitchConfig) -> (u32, u32) {
    let mut total_w = 0;
    let mut total_h = 0;
    let gap = config.gap;

    match config.direction {
        StitchDirection::Horizontal => {
            for (i, img) in images.iter().enumerate() {
                total_w += img.width();
                if i < images.len() - 1 {
                    total_w += gap;
                }
                total_h = total_h.max(img.height());
            }
        }
        StitchDirection::Vertical => {
            for (i, img) in images.iter().enumerate() {
                total_h += img.height();
                if i < images.len() - 1 {
                    total_h += gap;
                }
                total_w = total_w.max(img.width());
            }
        }
    }
    (total_w, total_h)
}

fn draw_images_to_canvas(
    images: Vec<DynamicImage>,
    total_w: u32,
    total_h: u32,
    config: &StitchConfig,
) -> RgbImage {
    let mut canvas = RgbImage::from_pixel(total_w, total_h, Rgb(config.background_color));
    let mut current_pos = 0;
    let gap = config.gap;

    for img in images {
        let rgb_img = img.to_rgb8();
        match config.direction {
            StitchDirection::Horizontal => {
                imageops::replace(&mut canvas, &rgb_img, current_pos as i64, 0);
                current_pos += rgb_img.width() + gap;
            }
            StitchDirection::Vertical => {
                imageops::replace(&mut canvas, &rgb_img, 0, current_pos as i64);
                current_pos += rgb_img.height() + gap;
            }
        }
    }
    canvas
}

#[cfg(test)]
#[path = "image_processor_tests.rs"]
mod tests;
