use crate::error::PicaViewError;
use std::path::PathBuf;

/// Verify if the input path exists and return its PathBuf form.
/// Returns PicaViewError::FileNotFound if the path does not exist.
pub fn resolve_input_path(path_str: &str) -> Result<PathBuf, PicaViewError> {
    let mut check_path = path_str;

    // 如果是带有 '|' 的虚拟压缩包路径，只校验前半段（物理压缩包）是否存在
    if let Some((archive, _)) = path_str.split_once('|') {
        check_path = archive;
    }

    let path = PathBuf::from(check_path);
    if !path.exists() {
        return Err(PicaViewError::file_not_found(check_path.to_string()));
    }

    // Force conversion to platform-native canonical path to ensure unification of \ and /
    // 对于带有 | 的虚拟路径，canonicalize 会失败并触发后备方案，这也正是我们期望的
    Ok(normalize_native_path(PathBuf::from(path_str)))
}

/// Normalize path to platform-native format
pub fn normalize_native_path(path: PathBuf) -> PathBuf {
    // Attempt to get absolute/canonical path
    if let Ok(p) = path.canonicalize() {
        // On Windows, canonicalize prepends the \\?\ prefix; we need to remove it for a UI-friendly path
        #[cfg(windows)]
        {
            let path_str = p.to_string_lossy();
            if let Some(stripped) = path_str.strip_prefix(r"\\?\") {
                return PathBuf::from(stripped);
            }
        }
        return p;
    }

    // Fallback: if the path doesn't exist, at least unify the separators
    #[cfg(windows)]
    {
        let path_str = path.to_string_lossy().replace('/', "\\");
        PathBuf::from(path_str)
    }

    #[cfg(not(windows))]
    {
        let path_str = path.to_string_lossy().replace('\\', "/");
        PathBuf::from(path_str)
    }
}

/// Get the application configuration directory
pub fn get_app_config_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("PicaView"))
            .unwrap_or_else(|_| PathBuf::from(".config"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME")
            .map(|p| PathBuf::from(p).join(".config").join("picaview"))
            .unwrap_or_else(|_| PathBuf::from(".config"))
    }
}

/// Get the application cache directory
pub fn get_app_cache_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("PicaView").join("cache"))
            .unwrap_or_else(|_| PathBuf::from(".cache"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME")
            .map(|p| PathBuf::from(p).join(".cache").join("picaview"))
            .unwrap_or_else(|_| PathBuf::from(".cache"))
    }
}

/// Get the application log directory
pub fn get_app_log_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("PicaView").join("logs"))
            .unwrap_or_else(|_| PathBuf::from(".logs"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME")
            .map(|p| {
                PathBuf::from(p)
                    .join(".cache")
                    .join("picaview")
                    .join("logs")
            })
            .unwrap_or_else(|_| PathBuf::from(".logs"))
    }
}

/// Natural language sorting comparison function
pub fn compare_natural(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();

    let mut a_chars = a_lower.chars().peekable();
    let mut b_chars = b_lower.chars().peekable();

    loop {
        match (a_chars.peek(), b_chars.peek()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(&ac), Some(&bc)) => {
                if ac.is_ascii_digit() && bc.is_ascii_digit() {
                    // Both are digits, extract the numeric block for comparison
                    let mut a_num = 0u64;
                    while let Some(&c) = a_chars.peek() {
                        if let Some(digit) = c.to_digit(10) {
                            a_num = a_num.wrapping_mul(10).wrapping_add(digit as u64);
                            a_chars.next();
                        } else {
                            break;
                        }
                    }
                    let mut b_num = 0u64;
                    while let Some(&c) = b_chars.peek() {
                        if let Some(digit) = c.to_digit(10) {
                            b_num = b_num.wrapping_mul(10).wrapping_add(digit as u64);
                            b_chars.next();
                        } else {
                            break;
                        }
                    }
                    if a_num != b_num {
                        return a_num.cmp(&b_num);
                    }
                } else {
                    // Non-digit or mixed, compare by character
                    match ac.cmp(&bc) {
                        Ordering::Equal => {
                            a_chars.next();
                            b_chars.next();
                        }
                        other => return other,
                    }
                }
            }
        }
    }
}

/// Copy an image to the system clipboard (Native/Slint friendly)
pub fn copy_image_to_clipboard_native(path: &str) -> Result<(), String> {
    use arboard::{Clipboard, ImageData};
    use std::borrow::Cow;

    let img = if let Some((archive_path, entry_name)) =
        crate::core::image_loader::parse_archive_path(path)
    {
        let bytes = crate::core::image_loader::load_archive_image_bytes(archive_path, entry_name)
            .map_err(|e| format!("Failed to read image from archive: {}", e))?;
        image::load_from_memory(&bytes).map_err(|e| format!("Failed to decode image: {}", e))?
    } else {
        image::open(path).map_err(|e| format!("Failed to open image: {}", e))?
    };
    let img_rgba = img.to_rgba8();

    let (width, height) = img_rgba.dimensions();
    let bytes = img_rgba.into_raw();

    let mut clipboard =
        Clipboard::new().map_err(|e| format!("Failed to initialize clipboard: {}", e))?;

    let image_data = ImageData {
        width: width as usize,
        height: height as usize,
        bytes: Cow::Owned(bytes),
    };

    clipboard
        .set_image(image_data)
        .map_err(|e| format!("Failed to set image to clipboard: {}", e))?;

    Ok(())
}

/// Reveal the file in the system file explorer (Native/Slint friendly)
pub fn reveal_in_explorer_native(path: &str) -> Result<(), String> {
    use std::process::Command;
    let target_path_buf;
    let path_ref =
        if let Some((archive_path, _)) = crate::core::image_loader::parse_archive_path(path) {
            target_path_buf = std::path::PathBuf::from(archive_path);
            &target_path_buf
        } else {
            std::path::Path::new(path)
        };

    if !path_ref.exists() {
        return Err("Path does not exist".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg("/select,")
            .arg(path_ref)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("-R")
            .arg(path_ref)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        // On Linux, xdg-open doesn't support highlighting a file.
        // We open the parent directory.
        let parent = path_ref.parent().unwrap_or(path_ref);
        Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Copy a physical file to the system clipboard (CF_HDROP)
pub fn copy_file_to_clipboard_native(#[allow(unused_variables)] path: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::DataExchange::{
            CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
        };
        use windows::Win32::System::Memory::{
            GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT,
        };
        use windows::Win32::System::Ole::CF_HDROP;
        use windows::Win32::UI::Shell::DROPFILES;

        let mut path_wide: Vec<u16> = path.encode_utf16().collect();
        path_wide.push(0); // null term
        path_wide.push(0); // double null term

        let size = std::mem::size_of::<DROPFILES>() + (path_wide.len() * 2);
        let hglobal =
            unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, size).map_err(|e| e.to_string())? };

        unsafe {
            let ptr = GlobalLock(hglobal) as *mut u8;
            if ptr.is_null() {
                return Err("Failed to lock global memory".into());
            }

            let header = ptr as *mut DROPFILES;

            (*header).pFiles = std::mem::size_of::<DROPFILES>() as u32;
            (*header).fWide = true.into();

            let path_ptr = ptr.add(std::mem::size_of::<DROPFILES>());
            std::ptr::copy_nonoverlapping(
                path_wide.as_ptr(),
                path_ptr as *mut u16,
                path_wide.len(),
            );

            let _ = GlobalUnlock(hglobal);

            let mut success = false;
            for _ in 0..3 {
                if OpenClipboard(None).is_ok() {
                    let _ = EmptyClipboard();
                    let handle = HANDLE(hglobal.0 as *mut core::ffi::c_void);
                    let res = SetClipboardData(CF_HDROP.0 as u32, Some(handle));
                    let _ = CloseClipboard();
                    if res.is_ok() {
                        success = true;
                        break;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if !success {
                return Err("Failed to open clipboard or set data".into());
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Not supported on this OS".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn test_compare_natural_numeric() {
        assert_eq!(compare_natural("2.jpg", "10.jpg"), Ordering::Less);
        assert_eq!(compare_natural("img2.jpg", "img10.jpg"), Ordering::Less);
        assert_eq!(compare_natural("IMG2.jpg", "img10.jpg"), Ordering::Less); // Case insensitive
    }
}
