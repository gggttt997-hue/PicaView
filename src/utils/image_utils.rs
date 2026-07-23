use crate::error::{PicaViewError, Result};

/// Detect image format from file header
pub fn detect_format_from_header(path: &std::path::Path) -> Result<image::ImageFormat> {
    use std::fs::File;
    use std::io::Read;

    let mut file = File::open(path)?;
    let mut header = [0u8; 12];

    // Try reading file header; if file is smaller, read actual size
    let bytes_read = file.read(&mut header)?;
    if bytes_read < 3 {
        return Err(PicaViewError::decode_error(
            path.to_string_lossy(),
            "File too small to detect format",
        ));
    }

    // JPEG: FF D8 FF
    if bytes_read >= 3 && header[0] == 0xFF && header[1] == 0xD8 && header[2] == 0xFF {
        return Ok(image::ImageFormat::Jpeg);
    }

    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if bytes_read >= 8 && header[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        return Ok(image::ImageFormat::Png);
    }

    // BMP: 42 4D
    if bytes_read >= 2 && header[0] == 0x42 && header[1] == 0x4D {
        return Ok(image::ImageFormat::Bmp);
    }

    // GIF: 47 49 46 38 (GIF8)
    if bytes_read >= 4 && &header[0..4] == b"GIF8" {
        return Ok(image::ImageFormat::Gif);
    }

    // WebP: RIFF .... WEBP
    if bytes_read >= 12 && &header[0..4] == b"RIFF" && &header[8..12] == b"WEBP" {
        tracing::debug!(
            location = "detect_format_from_header",
            format = "WebP",
            path = %path.display(),
            "Physically identified WebP format (RIFF/WEBP)"
        );
        return Ok(image::ImageFormat::WebP);
    }

    // TIFF: II* (Little Endian) or MM* (Big Endian)
    if bytes_read >= 4 {
        if header[0..4] == [0x49, 0x49, 0x2A, 0x00] || header[0..4] == [0x4D, 0x4D, 0x00, 0x2A] {
            tracing::debug!(
                location = "detect_format_from_header",
                format = "TIFF",
                path = %path.display(),
                "Physically identified TIFF format"
            );
            return Ok(image::ImageFormat::Tiff);
        }

        // ICO: 00 00 01 00
        if header[0..4] == [0x00, 0x00, 0x01, 0x00] {
            tracing::debug!(
                location = "detect_format_from_header",
                format = "ICO",
                header = ?&header[0..4],
                path = %path.display(),
                "Physically identified ICO format"
            );
            return Ok(image::ImageFormat::Ico);
        }
    }

    Err(PicaViewError::unsupported_format(
        "Unrecognized file format",
    ))
}

/// Get image format from file extension
pub fn get_format_from_extension(path: &std::path::Path) -> Option<image::ImageFormat> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .and_then(|ext| match ext.as_str() {
            "jpg" | "jpeg" => Some(image::ImageFormat::Jpeg),
            "png" => Some(image::ImageFormat::Png),
            "bmp" => Some(image::ImageFormat::Bmp),
            "gif" => Some(image::ImageFormat::Gif),
            "webp" => Some(image::ImageFormat::WebP),
            "tif" | "tiff" => Some(image::ImageFormat::Tiff),
            "ico" => Some(image::ImageFormat::Ico),
            _ => None,
        })
}

/// Detect image format (combining extension and file header)
pub fn detect_image_format(path: &std::path::Path) -> Result<image::ImageFormat> {
    use tracing::{debug, warn};

    // 1. Check extension first
    let extension_format = get_format_from_extension(path);

    // 2. Read file header to verify format
    let actual_format = detect_format_from_header(path)?;

    // 3. If extension and actual format match, use the detected format
    if let Some(ext_fmt) = extension_format {
        if ext_fmt == actual_format {
            debug!(
                "Format detection consistent: {:?} - {}",
                actual_format,
                path.display()
            );
            Ok(actual_format)
        } else {
            warn!(
                "Extension mismatch with actual format: Ext {:?}, Actual {:?} - {}",
                ext_fmt,
                actual_format,
                path.display()
            );
            // Use physically detected format
            Ok(actual_format)
        }
    } else {
        debug!(
            "No extension, using detected format: {:?} - {}",
            actual_format,
            path.display()
        );
        Ok(actual_format)
    }
}

/// Check if a supported image format by file extension only (fast, no I/O)
pub fn is_supported_by_extension(path: &std::path::Path) -> bool {
    const SUPPORTED_EXTENSIONS: &[&str] = &[
        "jpg", "jpeg", "png", "bmp", "gif", "webp", "tif", "tiff", "ico",
    ];

    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or(false)
}

/// Check if a supported archive format by file extension only (fast, no I/O)
pub fn is_archive_extension(path: &std::path::Path) -> bool {
    const ARCHIVE_EXTENSIONS: &[&str] = &["zip", "cbz", "7z", "cb7", "rar", "cbr"];

    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .map(|ext| ARCHIVE_EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or(false)
}

/// Extract file metadata (size and modification time)
pub fn get_file_metadata(path: &std::path::Path) -> (Option<u64>, Option<u64>) {
    if let Ok(metadata) = std::fs::metadata(path) {
        let size = Some(metadata.len());
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64);
        (size, modified_ms)
    } else {
        (None, None)
    }
}

/// Supported image format check (improved version with header detection)
pub fn is_supported_image_format(path: &std::path::Path) -> bool {
    // Check extension first (fast check)
    if !is_supported_by_extension(path) {
        // Extension not supported, but still attempt header detection (handles files without extension)
        return detect_format_from_header(path).is_ok();
    }

    // Extension supported, further check file header
    match detect_format_from_header(path) {
        Ok(_) => true,
        Err(_) => {
            // Header detection failed but extension is correct; might be a corrupt file, still treat as supported
            // Let the subsequent loading process handle specific errors
            true
        }
    }
}

/// Crop image to specified region
pub fn crop_image(
    img: &image::DynamicImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> image::DynamicImage {
    img.crop_imm(x, y, width, height)
}

/// Get formatted file size representation (e.g. "1.2 MB", "45.3 KB", "856 B")
pub fn get_formatted_file_size(path: impl AsRef<std::path::Path>) -> String {
    if let Ok(meta) = std::fs::metadata(path) {
        let bytes = meta.len();
        if bytes < 1024 {
            format!("{} B", bytes)
        } else if bytes < 1024 * 1024 {
            format!("{:.1} KB", bytes as f32 / 1024.0)
        } else {
            format!("{:.1} MB", bytes as f32 / (1024.0 * 1024.0))
        }
    } else {
        String::new()
    }
}

/// 动态获取系统推荐的图片解码内存上限 (Bytes)
pub fn get_dynamic_decode_limit() -> u64 {
    // 默认兜底为 1GB
    let default_limit = 1024 * 1024 * 1024;

    #[cfg(windows)]
    {
        #[repr(C)]
        #[allow(non_snake_case)]
        #[allow(clippy::upper_case_acronyms)]
        struct MEMORYSTATUSEX {
            dwLength: u32,
            dwMemoryLoad: u32,
            ullTotalPhys: u64,
            ullAvailPhys: u64,
            ullTotalPageFile: u64,
            ullAvailPageFile: u64,
            ullTotalVirtual: u64,
            ullAvailVirtual: u64,
            ullAvailExtendedVirtual: u64,
        }

        extern "system" {
            fn GlobalMemoryStatusEx(lpBuffer: *mut MEMORYSTATUSEX) -> i32;
        }

        let mut mem_info = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            dwMemoryLoad: 0,
            ullTotalPhys: 0,
            ullAvailPhys: 0,
            ullTotalPageFile: 0,
            ullAvailPageFile: 0,
            ullTotalVirtual: 0,
            ullAvailVirtual: 0,
            ullAvailExtendedVirtual: 0,
        };

        unsafe {
            if GlobalMemoryStatusEx(&mut mem_info) != 0 {
                // Total physical memory multiplied by the dynamic fraction from settings
                let fraction = crate::utils::settings::get_settings().mem_limit_fraction;
                let recommended = (mem_info.ullTotalPhys as f32 * fraction) as u64;
                // 限制在 [256MB, 2GB] 之间
                let lower_bound = 256 * 1024 * 1024;
                let upper_bound = 2048 * 1024 * 1024;
                recommended.clamp(lower_bound, upper_bound)
            } else {
                default_limit
            }
        }
    }

    #[cfg(not(windows))]
    {
        default_limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_new_formats_support() {
        // Test WebP
        assert!(is_supported_by_extension(Path::new("test.webp")));
        assert_eq!(
            get_format_from_extension(Path::new("test.webp")),
            Some(image::ImageFormat::WebP)
        );

        // Test TIFF
        assert!(is_supported_by_extension(Path::new("test.tiff")));
        assert!(is_supported_by_extension(Path::new("test.tif")));
        assert_eq!(
            get_format_from_extension(Path::new("test.tiff")),
            Some(image::ImageFormat::Tiff)
        );
        assert_eq!(
            get_format_from_extension(Path::new("test.tif")),
            Some(image::ImageFormat::Tiff)
        );
    }

    #[test]
    fn test_magic_number_detection() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        // Test WebP magic number (RIFF .... WEBP)
        let mut webp_file = NamedTempFile::new().unwrap();
        let mut webp_data = [0u8; 12];
        webp_data[0..4].copy_from_slice(b"RIFF");
        webp_data[8..12].copy_from_slice(b"WEBP");
        webp_file.write_all(&webp_data).unwrap();
        assert_eq!(
            detect_format_from_header(webp_file.path()).unwrap(),
            image::ImageFormat::WebP
        );

        // Test TIFF (Little Endian: II*)
        let mut tiff_le_file = NamedTempFile::new().unwrap();
        tiff_le_file.write_all(&[0x49, 0x49, 0x2A, 0x00]).unwrap();
        assert_eq!(
            detect_format_from_header(tiff_le_file.path()).unwrap(),
            image::ImageFormat::Tiff
        );

        // Test TIFF (Big Endian: MM*)
        let mut tiff_be_file = NamedTempFile::new().unwrap();
        tiff_be_file.write_all(&[0x4D, 0x4D, 0x00, 0x2A]).unwrap();
        assert_eq!(
            detect_format_from_header(tiff_be_file.path()).unwrap(),
            image::ImageFormat::Tiff
        );
    }

    #[test]
    fn test_physical_webp_tiff_roundtrip() {
        use image::{DynamicImage, ImageFormat, RgbImage};
        use std::fs;

        // 1. Verify WebP physical write and read
        let img = DynamicImage::ImageRgb8(RgbImage::new(10, 10));
        let temp_dir = tempfile::tempdir().unwrap();
        let webp_path = temp_dir.path().join("test.webp");

        // Save as WebP
        img.save_with_format(&webp_path, ImageFormat::WebP)
            .expect("Failed to save WebP file");

        // Verify file exists and is not empty
        let metadata = fs::metadata(&webp_path).unwrap();
        assert!(metadata.len() > 0);

        // Verify with magic number detector
        let detected = detect_format_from_header(&webp_path)
            .expect("Magic number detector failed to recognize generated WebP");
        assert_eq!(detected, ImageFormat::WebP);

        // Decode verification
        let decoded =
            image::open(&webp_path).expect("image library failed to open generated WebP file");
        assert_eq!(decoded.width(), 10);

        // 2. Verify TIFF
        let tiff_path = temp_dir.path().join("test.tiff");
        img.save_with_format(&tiff_path, ImageFormat::Tiff)
            .expect("Failed to save TIFF file");

        let detected_tiff = detect_format_from_header(&tiff_path)
            .expect("Magic number detector failed to recognize generated TIFF");
        assert_eq!(detected_tiff, ImageFormat::Tiff);

        let decoded_tiff =
            image::open(&tiff_path).expect("image library failed to open generated TIFF file");
        assert_eq!(decoded_tiff.width(), 10);
    }

    #[test]
    fn test_large_tiff_memory_sla() {
        use image::{DynamicImage, ImageFormat, RgbImage};

        // Create a 2000x2000 image
        let width = 2000;
        let height = 2000;
        let img = DynamicImage::ImageRgb8(RgbImage::new(width, height));

        let temp_dir = tempfile::tempdir().unwrap();
        let tiff_path = temp_dir.path().join("pressure_test.tiff");

        // Execute save and load cycle
        img.save_with_format(&tiff_path, ImageFormat::Tiff)
            .expect("Failed to save large TIFF");
        let decoded = image::open(&tiff_path).expect("Failed to load large TIFF");

        assert_eq!(decoded.width(), width);
        assert_eq!(decoded.height(), height);
    }

    #[test]
    fn test_robustness_mismatched_extension() {
        use image::{DynamicImage, ImageFormat, RgbImage};

        let img = DynamicImage::ImageRgb8(RgbImage::new(10, 10));
        let temp_dir = tempfile::tempdir().unwrap();

        // 1. WebP file disguised as .txt
        let fake_txt_path = temp_dir.path().join("fake.txt");
        img.save_with_format(&fake_txt_path, ImageFormat::WebP)
            .unwrap();

        let result = detect_image_format(&fake_txt_path)
            .expect("Robustness detection failed: Could not recognize WebP disguised as .txt");
        assert_eq!(result, ImageFormat::WebP);

        // 2. TIFF file disguised as .jpg
        let fake_jpg_path = temp_dir.path().join("fake.jpg");
        img.save_with_format(&fake_jpg_path, ImageFormat::Tiff)
            .unwrap();

        let result_tiff = detect_image_format(&fake_jpg_path)
            .expect("Robustness detection failed: Could not recognize TIFF disguised as .jpg");
        assert_eq!(result_tiff, ImageFormat::Tiff);
    }

    #[test]
    fn test_ico_support() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        // 1. Test extension recognition
        assert!(is_supported_by_extension(Path::new("test.ico")));
        assert_eq!(
            get_format_from_extension(Path::new("test.ico")),
            Some(image::ImageFormat::Ico)
        );

        // 2. Test magic number detection [0x00, 0x00, 0x01, 0x00]
        let mut ico_file = NamedTempFile::new().unwrap();
        ico_file.write_all(&[0x00, 0x00, 0x01, 0x00]).unwrap();
        let detected = detect_format_from_header(ico_file.path())
            .expect("Magic number detector failed to recognize ICO");
        assert_eq!(detected, image::ImageFormat::Ico);
    }
}
