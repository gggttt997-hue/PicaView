use image::DynamicImage;
use std::path::Path;
use tracing::debug;

/// Extract ICC Profile raw bytes from in-memory image file bytes.
pub fn extract_icc_profile_from_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    let cursor = std::io::Cursor::new(bytes);
    if let Ok(reader) = image::ImageReader::new(cursor).with_guessed_format() {
        match reader.format() {
            Some(image::ImageFormat::Jpeg) => {
                let mut dec =
                    image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(bytes)).ok()?;
                use image::ImageDecoder;
                dec.icc_profile().ok().flatten()
            }
            Some(image::ImageFormat::Png) => {
                let mut dec =
                    image::codecs::png::PngDecoder::new(std::io::Cursor::new(bytes)).ok()?;
                use image::ImageDecoder;
                dec.icc_profile().ok().flatten()
            }
            Some(image::ImageFormat::WebP) => {
                let mut dec =
                    image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes)).ok()?;
                use image::ImageDecoder;
                dec.icc_profile().ok().flatten()
            }
            Some(image::ImageFormat::Tiff) => {
                let mut dec =
                    image::codecs::tiff::TiffDecoder::new(std::io::Cursor::new(bytes)).ok()?;
                use image::ImageDecoder;
                dec.icc_profile().ok().flatten()
            }
            _ => None,
        }
    } else {
        None
    }
}

/// Extract ICC Profile raw bytes compatible with both normal paths and archive virtual paths.
pub fn extract_icc_profile(path: &str) -> Option<Vec<u8>> {
    if path.contains('|') {
        if let Some((archive_path, entry_name)) = path.split_once('|') {
            if let Ok(bytes) =
                crate::core::image_loader::load_archive_image_bytes(archive_path, entry_name)
            {
                return extract_icc_profile_from_bytes(&bytes);
            }
        }
    } else if let Ok(bytes) = std::fs::read(Path::new(path)) {
        return extract_icc_profile_from_bytes(&bytes);
    }
    None
}

/// Apply ICC Profile color mapping to sRGB space in-place using Little CMS.
pub fn apply_icc_color_correction(img: &mut DynamicImage, icc_bytes: &[u8]) -> Option<()> {
    let input_profile = lcms2::Profile::new_icc(icc_bytes).ok()?;
    let output_profile = lcms2::Profile::new_srgb(); // target output to standard sRGB space

    let mut rgba_img = img.to_rgba8();
    let pixels = rgba_img.as_flat_samples_mut().samples;

    let transform = lcms2::Transform::new(
        &input_profile,
        lcms2::PixelFormat::RGBA_8,
        &output_profile,
        lcms2::PixelFormat::RGBA_8,
        lcms2::Intent::Perceptual,
    )
    .ok()?;

    transform.transform_in_place(pixels);

    *img = DynamicImage::ImageRgba8(rgba_img);
    Some(())
}

/// Correct colors of the image if it has an embedded ICC profile.
/// Leverages AppState's active_icc_profile cache to avoid redundant IO.
pub fn correct_image_colors(path: &str, img: &mut DynamicImage, state: &crate::state::AppState) {
    let cached_icc = state.get_active_icc(path);

    let icc_bytes = match cached_icc {
        Some(icc_opt) => icc_opt,
        None => {
            let extracted = extract_icc_profile(path);
            state.set_active_icc(path.to_string(), extracted.clone());
            extracted
        }
    };

    if let Some(ref bytes) = icc_bytes {
        if apply_icc_color_correction(img, bytes).is_some() {
            debug!("Applied ICC Profile color correction for image: {}", path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use image::{DynamicImage, ImageBuffer, Rgba};

    #[test]
    fn test_extract_icc_profile_no_icc() {
        // Create a blank image without ICC
        let img = DynamicImage::ImageRgba8(ImageBuffer::new(10, 10));
        let mut buffer = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut buffer),
            image::ImageFormat::Png,
        )
        .unwrap();

        let profile = extract_icc_profile_from_bytes(&buffer);
        assert!(profile.is_none());
    }

    #[test]
    fn test_correct_image_colors_cache_integration() {
        let state = AppState::new();
        let mut img =
            DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([255, 0, 0, 255])));

        // Assert: no ICC initially cached
        assert!(state.get_active_icc("nonexistent_path.jpg").is_none());

        // Call color correction (should search file, fail, and cache None)
        correct_image_colors("nonexistent_path.jpg", &mut img, &state);

        // Assert: cache now successfully holds Some(None) to prevent subsequent file access
        assert_eq!(state.get_active_icc("nonexistent_path.jpg"), Some(None));
    }
}
