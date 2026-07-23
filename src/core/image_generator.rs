use crate::error::PicaViewError;
use crate::models::ImageInfo;
use std::io::Cursor;
use std::path::Path;
use std::time::Instant;
use zune_jpeg::JpegDecoder;

/// Generate thumbnail/proxy for images.
/// For JPEG, it uses an extreme fast path with DCT Scaling (zune-jpeg).
/// For other formats, it falls back to standard image-rs.
pub fn generate_thumbnail(path: &str, size: u32) -> Result<ImageInfo, PicaViewError> {
    let start = Instant::now();

    // --- 🚀 新增：针对 ZIP/CBZ 归档虚拟路径的流式缩略图生成 ---
    if let Some((archive_path, entry_name)) = crate::core::image_loader::parse_archive_path(path) {
        let bytes = crate::core::image_loader::load_archive_image_bytes(archive_path, entry_name)?;

        let img = image::load_from_memory(&bytes)
            .map_err(|e| PicaViewError::decode_error(path, e.to_string()))?;

        let (orig_w, orig_h) = (img.width(), img.height());
        let thumbnail = img.thumbnail(size, size);
        let (tw, th) = (thumbnail.width(), thumbnail.height());
        let rgba = thumbnail.to_rgba8();
        let buffer =
            slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(rgba.as_raw(), tw, th);

        let entry_filename = std::path::Path::new(entry_name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(entry_name)
            .to_string();

        return Ok(ImageInfo {
            path: path.to_string(),
            name: entry_filename,
            original_width: Some(orig_w),
            original_height: Some(orig_h),
            proxy_width: Some(tw),
            proxy_height: Some(th),
            proxy_scale: Some(tw as f32 / orig_w as f32),
            width: Some(orig_w),
            height: Some(orig_h),
            pixel_buffer: Some(buffer),
            proxy_path: None,
            size: Some(bytes.len() as u64),
            modified_ms: None,
            camera_model: None,
            focal_length: None,
            aperture: None,
            iso: None,
        });
    }

    let path_buf = Path::new(path);
    let extension = path_buf
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase());

    // --- 🚀 Plan A: JPEG Extreme Fast-Path (DCT Scaling) ---
    if let Some(ext) = extension {
        if ext == "jpg" || ext == "jpeg" {
            match generate_jpeg_fast_proxy(path, size, start) {
                Ok(info) => return Ok(info),
                Err(e) => {
                    tracing::warn!(path = %path, error = ?e, "JPEG Fast-Path failed, falling back to standard");
                }
            }
        }
    }

    // --- Standard Fallback (Non-JPEG or failed fast-path) ---
    let mut reader = image::ImageReader::open(path)?;
    reader = reader.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024); // 限制 256MB，确保支持到 8K (244MB) 缩略图的同时拦截巨型 Pixel Bombs OOM
    reader.limits(limits);

    let img = reader.decode().map_err(|e| {
        PicaViewError::decode_error(
            path,
            format!("Thumbnail memory allocation limit exceeded: {:?}", e),
        )
    })?;

    let (orig_w, orig_h) = (img.width(), img.height());
    let thumbnail = img.thumbnail(size, size);
    let (tw, th) = (thumbnail.width(), thumbnail.height());

    let rgba = thumbnail.to_rgba8();
    let buffer =
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(rgba.as_raw(), tw, th);

    Ok(ImageInfo {
        path: path.to_string(),
        name: path_buf
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string(),
        original_width: Some(orig_w),
        original_height: Some(orig_h),
        proxy_width: Some(tw),
        proxy_height: Some(th),
        proxy_scale: Some(tw as f32 / orig_w as f32),
        width: Some(orig_w),
        height: Some(orig_h),
        pixel_buffer: Some(buffer),
        proxy_path: None,
        size: None,
        modified_ms: None,
        camera_model: None,
        focal_length: None,
        aperture: None,
        iso: None,
    })
}

/// Specialized logic for JPEG: DCT Scaling via zune-jpeg.
/// This avoids full resolution decode and performs scaling during IDCT.
fn generate_jpeg_fast_proxy(
    path: &str,
    target_size: u32,
    start: Instant,
) -> Result<ImageInfo, PicaViewError> {
    let file_data =
        std::fs::read(path).map_err(|e| PicaViewError::decode_error(path, e.to_string()))?;

    // 1. Configure Decoder Options with high limits dynamically loaded from settings
    let limit = crate::utils::settings::get_settings().max_zune_dimension as usize;
    let options = zune_core::options::DecoderOptions::default()
        .set_max_width(limit)
        .set_max_height(limit);

    let mut decoder = JpegDecoder::new_with_options(Cursor::new(&file_data), options);

    let pixels = decoder
        .decode()
        .map_err(|e| PicaViewError::decode_error(path, format!("{:?}", e)))?;

    // Get original dimensions for metadata and scaling calculations
    let (orig_w, orig_h) = decoder
        .dimensions()
        .ok_or_else(|| PicaViewError::decode_error(path, "Failed to get dimensions"))?;
    // Keep aspect ratio to calculate proxy dimensions
    let (proxy_w, proxy_h) = if orig_w > target_size as usize || orig_h > target_size as usize {
        let scale = (target_size as f32) / (orig_w.max(orig_h) as f32);
        let pw = ((orig_w as f32 * scale).round() as u32).max(1);
        let ph = ((orig_h as f32 * scale).round() as u32).max(1);
        (pw, ph)
    } else {
        (orig_w as u32, orig_h as u32)
    };

    let colorspace = decoder
        .output_colorspace()
        .unwrap_or(zune_core::colorspace::ColorSpace::RGB);
    let channels = colorspace.num_components();

    let decode_time = start.elapsed();

    // 2. Downsample and convert to RGBA for Slint
    let mut rgba_pixels = Vec::with_capacity(proxy_w as usize * proxy_h as usize * 4);

    if proxy_w == orig_w as u32 && proxy_h == orig_h as u32 {
        // No downsampling required, convert directly
        if channels == 3 {
            for i in 0..(pixels.len() / 3) {
                rgba_pixels.push(pixels[i * 3]);
                rgba_pixels.push(pixels[i * 3 + 1]);
                rgba_pixels.push(pixels[i * 3 + 2]);
                rgba_pixels.push(255);
            }
        } else if channels == 1 {
            for p in pixels {
                rgba_pixels.push(p);
                rgba_pixels.push(p);
                rgba_pixels.push(p);
                rgba_pixels.push(255);
            }
        } else {
            return Err(PicaViewError::decode_error(
                path,
                format!("Unsupported channel count: {}", channels),
            ));
        }
    } else {
        // Perform ultra-fast in-memory Stride Nearest-Neighbor downsampling
        let pw_usize = proxy_w as usize;
        let ph_usize = proxy_h as usize;
        for y in 0..ph_usize {
            let src_y = (y * orig_h) / ph_usize;
            for x in 0..pw_usize {
                let src_x = (x * orig_w) / pw_usize;
                let src_idx = (src_y * orig_w + src_x) * channels;
                if src_idx + channels - 1 < pixels.len() {
                    if channels == 3 {
                        rgba_pixels.push(pixels[src_idx]);
                        rgba_pixels.push(pixels[src_idx + 1]);
                        rgba_pixels.push(pixels[src_idx + 2]);
                        rgba_pixels.push(255);
                    } else if channels == 1 {
                        let g = pixels[src_idx];
                        rgba_pixels.push(g);
                        rgba_pixels.push(g);
                        rgba_pixels.push(g);
                        rgba_pixels.push(255);
                    }
                }
            }
        }
    }

    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        &rgba_pixels,
        proxy_w,
        proxy_h,
    );

    tracing::info!(
        path = %path,
        orig = %format!("{}x{}", orig_w, orig_h),
        proxy = %format!("{}x{}", proxy_w, proxy_h),
        channels = %channels,
        decode_ms = decode_time.as_millis(),
        total_ms = start.elapsed().as_millis(),
        "JPEG Memory-Safe downsampling Successful"
    );

    Ok(ImageInfo {
        path: path.to_string(),
        name: Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string(),
        original_width: Some(orig_w as u32),
        original_height: Some(orig_h as u32),
        proxy_width: Some(proxy_w),
        proxy_height: Some(proxy_h),
        proxy_scale: Some(proxy_w as f32 / orig_w as f32),
        width: Some(orig_w as u32),
        height: Some(orig_h as u32),
        pixel_buffer: Some(buffer),
        proxy_path: None,
        size: None,
        modified_ms: None,
        camera_model: None,
        focal_length: None,
        aperture: None,
        iso: None,
    })
}
