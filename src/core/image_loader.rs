/// Image loading and processing module
///
/// Provides loading and information retrieval for image files.
use crate::error::{PicaViewError, Result};
use crate::models::ImageInfo;
use std::path::Path;
use tracing::{debug, info, instrument, warn};

/// Load image file information
///
/// This function verifies the image path and returns an ImageInfo containing that path.
/// It no longer attempts to convert resource URLs; it only provides the original path.
///
/// # Arguments
/// * `path` - Path to the image file
///
/// # Returns
/// * `Ok(ImageInfo)` - Returns ImageInfo containing the path on success
/// * `Err(PicaViewError)` - Returns an error on failure
pub fn parse_archive_path(path: &str) -> Option<(&str, &str)> {
    path.split_once('|')
}

pub fn load_archive_image_bytes(archive_path: &str, entry_name: &str) -> Result<Vec<u8>> {
    let shared_vfs = crate::core::archive_vfs::get_shared_vfs(std::path::Path::new(archive_path))?;
    let pass_store = crate::core::archive_vfs::archive_passwords()
        .read()
        .unwrap();
    let pass = pass_store.get(archive_path).map(|s: &String| s.as_str());

    let mut vfs = shared_vfs.lock().unwrap();
    vfs.extract_file(entry_name, pass)
}

#[instrument(skip_all, fields(path = %path.display()))]
pub async fn load_image_info(path: &Path) -> Result<ImageInfo> {
    let path_str = path.to_string_lossy().to_string();
    debug!("Starting to load image info: {}", path_str);

    if let Some((archive_path, entry_name)) = parse_archive_path(&path_str) {
        let bytes = tokio::task::spawn_blocking({
            let archive_path = archive_path.to_string();
            let entry_name = entry_name.to_string();
            move || load_archive_image_bytes(&archive_path, &entry_name)
        })
        .await
        .map_err(|e| PicaViewError::general(format!("Task spawn failed: {}", e)))??;

        let (width, height, _format) =
            match image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format() {
                Ok(reader) => {
                    let fmt = reader.format();
                    match reader.into_dimensions() {
                        Ok((w, h)) => (Some(w), Some(h), fmt),
                        Err(_) => (None, None, fmt),
                    }
                }
                Err(_) => (None, None, None),
            };

        let mut image_info = ImageInfo::from_path(&path_str);
        image_info.size = Some(bytes.len() as u64);
        image_info.width = width;
        image_info.height = height;

        debug!(
            "Archive image info loaded successfully: {}",
            image_info.path
        );
        return Ok(image_info);
    }

    // 1. Basic checks
    if !path.exists() {
        warn!("Image file does not exist: {}", path.display());
        return Err(PicaViewError::file_not_found(path.to_string_lossy()));
    }
    if !path.is_file() {
        warn!("Path is not a file: {}", path.display());
        return Err(PicaViewError::general(format!(
            "Path is not a file: {}",
            path.display()
        )));
    }

    // Get file metadata, including size
    let metadata = std::fs::metadata(path).map_err(|e| {
        warn!("Failed to get file metadata: {} - {}", path.display(), e);
        PicaViewError::from(e)
    })?;
    let file_size = metadata.len(); // Get file size in bytes

    // 2. Check format and dimensions to decide on proxy strategy
    let (width, height, format) =
        match image::ImageReader::open(path).and_then(|r| r.with_guessed_format()) {
            Ok(reader) => {
                let fmt = reader.format();
                match reader.into_dimensions() {
                    Ok((w, h)) => (Some(w), Some(h), fmt),
                    Err(_) => (None, None, fmt),
                }
            }
            Err(_) => (None, None, None),
        };

    let needs_proxy = match (width, height, format) {
        (_, _, Some(image::ImageFormat::Tiff)) => true,
        (Some(w), Some(h), _) if w > 4096 || h > 4096 => true,
        _ => false,
    };

    let path_str = path.to_string_lossy().to_string();
    let mut image_info = ImageInfo::from_path(&path_str);
    image_info.size = Some(file_size);
    image_info.width = width;
    image_info.height = height;

    if let Some((camera, focal, aperture, iso)) = extract_exif(path) {
        image_info.camera_model = camera;
        image_info.focal_length = focal;
        image_info.aperture = aperture;
        image_info.iso = iso;
    }

    // --- 🚀 Milestone 2: Native Optimization ---
    // We skip proxy generation (Base64) in Native mode because WGPU
    // handles full-res textures directly. We only need the metadata.
    if needs_proxy && format == Some(image::ImageFormat::Tiff) {
        info!(
            path = %path.display(),
            "TIFF detected, spawning proxy task for compatibility..."
        );

        let path_clone = path_str.clone();
        image_info = tokio::task::spawn_blocking(move || {
            crate::core::image_generator::generate_thumbnail(&path_clone, 4096)
        })
        .await
        .map_err(|e| PicaViewError::general(format!("Proxy task join failed: {}", e)))??;

        image_info.size = Some(file_size);
    }
    debug!("Image info loaded successfully: {}", image_info.path);
    Ok(image_info)
}

/// Preload an image into the memory cache
///
/// This function decodes the image in a background thread and stores it in the AppState's preload_cache.
pub async fn preload_image(path: &std::path::Path, state: std::sync::Arc<crate::state::AppState>) {
    let path_str = path.to_string_lossy().to_string();

    // Check if already in cache
    {
        if let Ok(mut cache) = state.preload_cache.lock() {
            if cache.get(&path_str).is_some() {
                debug!("Image already in preload cache: {}", path_str);
                return;
            }
        }
    }

    let path_buf = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        debug!("Background pre-decoding: {}", path_buf.display());
        let res = if let Some((archive_path, entry_name)) = parse_archive_path(&path_str) {
            load_archive_image_bytes(archive_path, entry_name)
                .map_err(|e| e.to_string())
                .and_then(|bytes| image::load_from_memory(&bytes).map_err(|e| e.to_string()))
        } else {
            // Physical Defense: Block massive assets (>20MB) from being fully decoded into the preload cache pool
            if let Ok(meta) = std::fs::metadata(&path_buf) {
                if meta.len() > 20 * 1024 * 1024 {
                    return Err(
                        "Image too large for full pixel preload (exceeds 20MB limit)".into(),
                    );
                }
            }
            image::ImageReader::open(&path_buf)
                .map_err(|e| e.to_string())
                .and_then(|r| r.with_guessed_format().map_err(|e| e.to_string()))
                .and_then(|r| r.decode().map_err(|e| e.to_string()))
        };

        match res {
            Ok(mut img) => {
                crate::utils::color_space::correct_image_colors(&path_str, &mut img, &state);
                if let Ok(mut cache) = state.preload_cache.lock() {
                    cache.put(path_str.clone(), std::sync::Arc::new(img));
                    info!("Successfully preloaded image to cache: {}", path_str);
                }
                Ok(())
            }
            Err(e) => {
                warn!("Failed to preload image {}: {}", path_buf.display(), e);
                Err(e)
            }
        }
    });
}

pub fn extract_exif(
    path: &std::path::Path,
) -> Option<(Option<String>, Option<f32>, Option<f32>, Option<u32>)> {
    let file = std::fs::File::open(path).ok()?;
    let mut buf_reader = std::io::BufReader::new(file);
    let exifreader = exif::Reader::new();
    let exif = exifreader.read_from_container(&mut buf_reader).ok()?;

    let camera_model = exif
        .get_field(exif::Tag::Model, exif::In::PRIMARY)
        .and_then(|field| match &field.value {
            exif::Value::Ascii(vec) => vec.first().and_then(|ascii| {
                std::str::from_utf8(ascii)
                    .ok()
                    .map(|s| s.trim().to_string())
            }),
            _ => None,
        });

    let focal_length = exif
        .get_field(exif::Tag::FocalLength, exif::In::PRIMARY)
        .and_then(|field| match &field.value {
            exif::Value::Rational(vec) => vec.first().map(|r| {
                if r.denom != 0 {
                    r.num as f32 / r.denom as f32
                } else {
                    0.0
                }
            }),
            _ => None,
        });

    let aperture = exif
        .get_field(exif::Tag::FNumber, exif::In::PRIMARY)
        .and_then(|field| match &field.value {
            exif::Value::Rational(vec) => vec.first().map(|r| {
                if r.denom != 0 {
                    r.num as f32 / r.denom as f32
                } else {
                    0.0
                }
            }),
            _ => None,
        });

    let iso = exif
        .get_field(exif::Tag::ISOSpeed, exif::In::PRIMARY)
        .and_then(|field| match &field.value {
            exif::Value::Short(vec) => vec.first().map(|&v| v as u32),
            exif::Value::Long(vec) => vec.first().copied(),
            _ => None,
        });

    Some((camera_model, focal_length, aperture, iso))
}

pub fn extract_exif_formatted(path: &std::path::Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut buf_reader = std::io::BufReader::new(file);
    let exifreader = exif::Reader::new();
    let exif = exifreader.read_from_container(&mut buf_reader).ok()?;

    let get_ascii = |tag| -> Option<String> {
        exif.get_field(tag, exif::In::PRIMARY)
            .and_then(|f| match &f.value {
                exif::Value::Ascii(vec) => vec.first().and_then(|ascii| {
                    std::str::from_utf8(ascii)
                        .ok()
                        .map(|s| s.trim().to_string())
                }),
                _ => None,
            })
    };

    let get_rational = |tag| -> Option<f32> {
        exif.get_field(tag, exif::In::PRIMARY)
            .and_then(|f| match &f.value {
                exif::Value::Rational(vec) => vec.first().map(|r| {
                    if r.denom != 0 {
                        r.num as f32 / r.denom as f32
                    } else {
                        0.0
                    }
                }),
                _ => None,
            })
    };

    let get_rational_raw = |tag| -> Option<(u32, u32)> {
        exif.get_field(tag, exif::In::PRIMARY)
            .and_then(|f| match &f.value {
                exif::Value::Rational(vec) => vec.first().map(|r| (r.num, r.denom)),
                _ => None,
            })
    };

    let make = get_ascii(exif::Tag::Make);
    let model = get_ascii(exif::Tag::Model);
    let focal_length = get_rational(exif::Tag::FocalLength);
    let aperture = get_rational(exif::Tag::FNumber);
    let iso = exif
        .get_field(exif::Tag::ISOSpeed, exif::In::PRIMARY)
        .or_else(|| exif.get_field(exif::Tag::PhotographicSensitivity, exif::In::PRIMARY))
        .and_then(|f| match &f.value {
            exif::Value::Short(vec) => vec.first().map(|&v| v as u32),
            exif::Value::Long(vec) => vec.first().copied(),
            _ => None,
        });
    let exposure_time = get_rational_raw(exif::Tag::ExposureTime);

    let mut parts = Vec::new();

    let camera = match (make, model) {
        (Some(mk), Some(md)) => {
            if md.starts_with(&mk) {
                Some(md)
            } else {
                Some(format!("{} {}", mk, md))
            }
        }
        (Some(mk), None) => Some(mk),
        (None, Some(md)) => Some(md),
        _ => None,
    };
    if let Some(cam) = camera {
        parts.push(cam);
    }
    if let Some(fl) = focal_length {
        parts.push(format!("{:.0}mm", fl));
    }
    if let Some(ap) = aperture {
        parts.push(format!("f/{:.1}", ap));
    }
    if let Some((num, den)) = exposure_time {
        if den > num && num != 0 {
            parts.push(format!("1/{}s", den / num));
        } else {
            parts.push(format!("{:.1}s", num as f32 / den as f32));
        }
    }
    if let Some(i) = iso {
        parts.push(format!("ISO {}", i));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" | "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_load_image_info_success() {
        let temp_dir = TempDir::new().unwrap();
        let test_image = temp_dir.path().join("test.jpg");
        fs::write(&test_image, "fake image data").unwrap();

        let result = load_image_info(&test_image).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.path, test_image.to_string_lossy());
    }

    #[tokio::test]
    async fn test_load_nonexistent_file() {
        let result = load_image_info(Path::new("/nonexistent/file.jpg")).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            PicaViewError::FileNotFound { .. } => {
                // Expected error type
            }
            _ => panic!("Expected FileNotFound error"),
        }
    }

    #[tokio::test]
    async fn test_tiff_loading_with_preview() {
        use image::{DynamicImage, ImageFormat, RgbImage};

        let temp_dir = TempDir::new().unwrap();
        let tiff_path = temp_dir.path().join("test.tiff");

        // Create a real TIFF
        let img = DynamicImage::ImageRgb8(RgbImage::new(10, 10));
        img.save_with_format(&tiff_path, ImageFormat::Tiff).unwrap();

        // Invoke loading logic
        let result = load_image_info(&tiff_path)
            .await
            .expect("TIFF loading failed");

        // Verify if a pixel buffer was generated (proxy)
        assert!(result.pixel_buffer.is_some());
        let proxy_buffer = result.pixel_buffer.unwrap();
        assert!(proxy_buffer.width() > 0);
        assert_eq!(result.width, Some(10));
    }
}
