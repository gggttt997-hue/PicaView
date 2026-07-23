/// PicaView core data models
///
/// Defines the main data structures used in the application
use serde::{Deserialize, Serialize};

/// Window status information
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct WindowState {
    pub is_maximized: bool,
    pub is_minimized: bool,
    pub is_fullscreen: bool,
}

/// Image information structure
///
/// Contains the path of the image and is used to transmit image information between the backend and frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    /// Full path to the image file
    pub path: String,
    /// Name of the image file (e.g., image.jpg)
    pub name: String,
    /// Original image width (Logical width)
    pub original_width: Option<u32>,
    /// Original image height (Logical height)
    pub original_height: Option<u32>,
    /// Rendered proxy width (Physical width in layout)
    pub proxy_width: Option<u32>,
    /// Rendered proxy height (Physical height in layout)
    pub proxy_height: Option<u32>,
    /// The ratio between proxy and original (proxy_width / original_width)
    pub proxy_scale: Option<f32>,
    /// Image width (backward compatibility, matches original_width)
    pub width: Option<u32>,
    /// Image height (backward compatibility, matches original_height)
    pub height: Option<u32>,
    /// Image size in bytes
    pub size: Option<u64>,
    /// Last modified timestamp in milliseconds
    pub modified_ms: Option<u64>,

    /// Path to the cached proxy/thumbnail file on disk
    pub proxy_path: Option<String>,

    /// In-memory pixel buffer for zero-copy rendering
    #[serde(skip)]
    pub pixel_buffer: Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>>,

    /// Camera model from EXIF
    pub camera_model: Option<String>,
    /// Focal length from EXIF (mm)
    pub focal_length: Option<f32>,
    /// Aperture from EXIF (f-number)
    pub aperture: Option<f32>,
    /// ISO speed from EXIF
    pub iso: Option<u32>,
}

impl ImageInfo {
    /// Create a new ImageInfo instance
    pub fn new(path: String) -> Self {
        let name = std::path::Path::new(&path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        Self {
            path,
            name,
            original_width: None,
            original_height: None,
            proxy_width: None,
            proxy_height: None,
            proxy_scale: None,
            width: None,
            height: None,
            size: None,
            modified_ms: None,
            proxy_path: None,
            pixel_buffer: None,
            camera_model: None,
            focal_length: None,
            aperture: None,
            iso: None,
        }
    }

    /// Create ImageInfo from a path
    pub fn from_path(path: impl Into<String>) -> Self {
        let path_str = path.into();
        let name = std::path::Path::new(&path_str)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        Self {
            path: path_str,
            name,
            original_width: None,
            original_height: None,
            proxy_width: None,
            proxy_height: None,
            proxy_scale: None,
            width: None,
            height: None,
            size: None,
            modified_ms: None,
            proxy_path: None,
            pixel_buffer: None,
            camera_model: None,
            focal_length: None,
            aperture: None,
            iso: None,
        }
    }

    /// Get the filename (excluding the path)
    pub fn filename(&self) -> Option<String> {
        std::path::Path::new(&self.path)
            .file_name()
            .and_then(|name| name.to_str())
            .map(|s| s.to_string())
    }

    /// Get the estimated memory size of the image pixel buffer in bytes
    pub fn pixel_weight(&self) -> usize {
        self.pixel_buffer
            .as_ref()
            .map(|b| b.width() as usize * b.height() as usize * 4)
            .unwrap_or(0)
    }
}

/// Sorting criteria
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum SortCriteria {
    Name,
    Size,
    Date,
}

/// Sorting order
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_info_metadata_fields() {
        let mut info = ImageInfo::new("/path/to/image.jpg".to_string());
        info.size = Some(1024);
        info.modified_ms = Some(1672531200000); // Example timestamp

        assert_eq!(info.size, Some(1024));
        assert_eq!(info.modified_ms, Some(1672531200000));
    }

    #[test]
    fn test_image_info_creation() {
        let info = ImageInfo::new("/path/to/image.jpg".to_string());

        assert_eq!(info.path, "/path/to/image.jpg");
        assert_eq!(info.width, None);
        assert_eq!(info.height, None);
        assert_eq!(info.filename(), Some("image.jpg".to_string()));
    }

    #[test]
    fn test_image_info_from_path() {
        let info = ImageInfo::from_path("/path/to/test.png");

        assert_eq!(info.path, "/path/to/test.png");
        assert_eq!(info.filename(), Some("test.png".to_string()));
    }

    #[test]
    fn test_filename_extraction() {
        // Use platform-specific separators for testing or test both
        let info = ImageInfo::from_path("/home/user/images/vacation.png");
        assert_eq!(info.filename(), Some("vacation.png".to_string()));

        #[cfg(windows)]
        {
            let info = ImageInfo::from_path("C:\\Users\\Test\\Pictures\\photo.jpg");
            assert_eq!(info.filename(), Some("photo.jpg".to_string()));
        }
    }

    #[test]
    fn test_image_info_pixel_weight() {
        let mut info = ImageInfo::from_path("/path/to/test.png");
        assert_eq!(info.pixel_weight(), 0);

        // Create a buffer and set it (2x2 pixel buffer)
        let buffer = slint::SharedPixelBuffer::clone_from_slice(&[0u8; 16], 2, 2);
        info.pixel_buffer = Some(buffer);
        // 2 * 2 * 4 = 16 bytes
        assert_eq!(info.pixel_weight(), 16);
    }
}
