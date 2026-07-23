/// PicaView error type definition
/// Defines a simple but complete error handling mechanism based on MVP principles.
#[derive(Debug, thiserror::Error)]
pub enum PicaViewError {
    /// File not found error
    #[error("File not found: {path}")]
    FileNotFound { path: String },

    /// Unsupported image format
    #[error("Unsupported format: {format}")]
    UnsupportedFormat { format: String },

    /// Image decoding failed
    #[error("Image decoding failed: {path} - {reason}")]
    DecodeError { path: String, reason: String },

    /// IO operation error
    #[error("File operation error: {0}")]
    IoError(#[from] std::io::Error),

    /// Image processing error
    #[error("Image processing error: {0}")]
    ImageError(#[from] image::ImageError),

    /// Directory scan error
    #[error("Directory scan failed: {path}")]
    DirectoryScanError { path: String },

    /// Out of memory error
    #[error("Out of memory")]
    OutOfMemory,

    /// General application error
    #[error("Application error: {message}")]
    General { message: String },

    /// Archive requires password
    #[error("Password required to extract archive")]
    PasswordRequired,

    /// Tauri command error
    #[error("Tauri command error: {0}")]
    ThumbnailCommandError(String),
    /// Thumbnail generation error
    #[error("Thumbnail generation failed: {0}")]
    ThumbnailGenerationError(String),

    /// Manga mode: detected next chapter directory
    #[error("Entering next folder: {0:?}")]
    MangaNextFolder(std::path::PathBuf),
    /// Manga mode: detected previous chapter directory
    #[error("Entering previous folder: {0:?}")]
    MangaPrevFolder(std::path::PathBuf),
}

/// Result type alias to simplify error handling
pub type Result<T> = std::result::Result<T, PicaViewError>;

impl PicaViewError {
    /// Create file not found error
    pub fn file_not_found(path: impl Into<String>) -> Self {
        Self::FileNotFound { path: path.into() }
    }

    /// Create unsupported format error
    pub fn unsupported_format(format: impl Into<String>) -> Self {
        Self::UnsupportedFormat {
            format: format.into(),
        }
    }

    /// Create decoding error
    pub fn decode_error(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::DecodeError {
            path: path.into(),
            reason: reason.into(),
        }
    }

    /// Create directory scan error
    pub fn directory_scan_error(path: impl Into<String>) -> Self {
        Self::DirectoryScanError { path: path.into() }
    }

    /// Create general application error
    pub fn general(message: impl Into<String>) -> Self {
        Self::General {
            message: message.into(),
        }
    }

    /// Create image processing error
    pub fn image_error(err: image::ImageError) -> Self {
        Self::ImageError(err)
    }

    /// Convert error to a string suitable for display on the frontend
    pub fn to_frontend_message(&self) -> String {
        match self {
            Self::FileNotFound { .. } => "File not found".to_string(),
            Self::UnsupportedFormat { .. } => "Unsupported image format".to_string(),
            Self::DecodeError { .. } => "Image file is corrupt or cannot be decoded".to_string(),
            Self::IoError(_) => "Failed to read file".to_string(),
            Self::ImageError(_) => "Failed to process image".to_string(),
            Self::DirectoryScanError { .. } => "Directory scan failed".to_string(),
            Self::OutOfMemory => "Out of memory".to_string(),
            Self::PasswordRequired => "Password required".to_string(),
            Self::General { message } => message.clone(),
            Self::ThumbnailGenerationError(_) => "Thumbnail generation failed".to_string(),
            Self::ThumbnailCommandError(_) => "Tauri command execution failed".to_string(),
            Self::MangaNextFolder(_) => "Entering next folder...".to_string(),
            Self::MangaPrevFolder(_) => "Entering previous folder...".to_string(),
        }
    }

    /// Convert error to a more accurate frontend message
    pub fn to_accurate_frontend_message(&self) -> String {
        match self {
            Self::FileNotFound { .. } => "File not found".to_string(),
            Self::UnsupportedFormat { format } => {
                if format.contains("Unrecognized") {
                    "Unsupported image format".to_string()
                } else {
                    format!("Unsupported image format: {}", format)
                }
            }
            Self::DecodeError { reason, .. } => {
                if reason.contains("No such file") || reason.contains("not found") {
                    "File not found".to_string()
                } else if reason.contains("Permission denied") {
                    "Insufficient file permissions".to_string()
                } else if reason.contains("Invalid")
                    || reason.contains("Corrupt")
                    || reason.contains("corrupt")
                {
                    "Image file is corrupt".to_string()
                } else if reason.contains("memory") || reason.contains("Memory") {
                    "Out of memory".to_string()
                } else if reason.contains("format") || reason.contains("Format") {
                    "Image format error".to_string()
                } else if reason.contains("too small") {
                    "File too small or incomplete".to_string()
                } else {
                    format!("Image decoding failed: {}", reason)
                }
            }
            Self::IoError(io_err) => match io_err.kind() {
                std::io::ErrorKind::NotFound => "File not found".to_string(),
                std::io::ErrorKind::PermissionDenied => "Insufficient file permissions".to_string(),
                std::io::ErrorKind::InvalidData => "Invalid file data".to_string(),
                std::io::ErrorKind::UnexpectedEof => "File incomplete".to_string(),
                _ => format!("File operation failed: {}", io_err),
            },
            Self::ImageError(img_err) => {
                let err_str = img_err.to_string().to_lowercase();
                if err_str.contains("unsupported") {
                    "Unsupported image format".to_string()
                } else if err_str.contains("corrupt") || err_str.contains("invalid") {
                    "Image file is corrupt".to_string()
                } else if err_str.contains("memory") {
                    "Out of memory".to_string()
                } else {
                    format!("Failed to process image: {}", img_err)
                }
            }
            Self::DirectoryScanError { .. } => "Directory scan failed".to_string(),
            Self::OutOfMemory => "Out of memory".to_string(),
            Self::PasswordRequired => "Password required".to_string(),
            Self::General { message } => message.clone(),
            Self::ThumbnailGenerationError(msg) => format!("Thumbnail generation failed: {}", msg),
            Self::ThumbnailCommandError(msg) => format!("Tauri command execution failed: {}", msg),
            Self::MangaNextFolder(path) => format!("Entering next folder: {:?}", path),
            Self::MangaPrevFolder(path) => format!("Entering previous folder: {:?}", path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_messages() {
        let error = PicaViewError::file_not_found("test.jpg");
        assert_eq!(error.to_frontend_message(), "File not found");
        assert_eq!(error.to_accurate_frontend_message(), "File not found");

        let error = PicaViewError::unsupported_format("webp");
        assert_eq!(error.to_frontend_message(), "Unsupported image format");
        assert_eq!(
            error.to_accurate_frontend_message(),
            "Unsupported image format: webp"
        );

        let error = PicaViewError::decode_error("test.jpg", "Permission denied");
        assert_eq!(
            error.to_accurate_frontend_message(),
            "Insufficient file permissions"
        );

        let error = PicaViewError::decode_error("test.jpg", "Invalid image data");
        assert_eq!(
            error.to_accurate_frontend_message(),
            "Image file is corrupt"
        );

        let error = PicaViewError::decode_error("test.jpg", "Out of memory");
        assert_eq!(error.to_accurate_frontend_message(), "Out of memory");
    }

    #[test]
    fn test_manga_folder_errors() {
        use std::path::PathBuf;
        let path = PathBuf::from("next_folder");
        let error = PicaViewError::MangaNextFolder(path.clone());
        assert!(error
            .to_accurate_frontend_message()
            .contains("Entering next folder"));

        let path_prev = PathBuf::from("prev_folder");
        let error_prev = PicaViewError::MangaPrevFolder(path_prev);
        assert!(error_prev
            .to_accurate_frontend_message()
            .contains("Entering previous folder"));
    }
}
