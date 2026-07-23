use std::path::PathBuf;
use tracing::debug;

/// Generate context menu integration scripts
pub fn generate_shell_integration_scripts(
    executable_path: PathBuf,
) -> Result<(String, String), String> {
    debug!(
        "[Evidence] Generating shell integration scripts for: {:?}",
        executable_path
    );

    // 1. Windows .reg script content
    let win_path = executable_path.to_string_lossy().replace("\\", "\\\\");
    let reg_content = format!(
        r#"Windows Registry Editor Version 5.00

[HKEY_CURRENT_USER\Software\Classes\Directory\shell\PicaView]
@="使用 PicaView 打开"
"Icon"="{}"

[HKEY_CURRENT_USER\Software\Classes\Directory\shell\PicaView\command]
@="\"{}\" \"%1\""

[HKEY_CURRENT_USER\Software\Classes\Directory\Background\shell\PicaView]
@="使用 PicaView 打开"
"Icon"="{}"

[HKEY_CURRENT_USER\Software\Classes\Directory\Background\shell\PicaView\command]
@="\"{}\" \"%V\""

[HKEY_CURRENT_USER\Software\Classes\*\shell\PicaView]
@="使用 PicaView 打开"
"Icon"="{}"

[HKEY_CURRENT_USER\Software\Classes\*\shell\PicaView\command]
@="\"{}\" \"%1\""
"#,
        win_path, win_path, win_path, win_path, win_path, win_path
    );

    // 2. Linux .desktop content
    let desktop_content = format!(
        r#"[Desktop Entry]
Name=PicaView
Comment=极简桌面看图软件
Exec={} %u
Icon=picaview
Terminal=false
Type=Application
Categories=Graphics;
MimeType=image/jpeg;image/png;image/gif;image/webp;image/tiff;inode/directory;
Keywords=image;viewer;gallery;
"#,
        executable_path.to_string_lossy()
    );

    debug!("[Evidence] Shell integration scripts generated successfully.");
    Ok((reg_content, desktop_content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reg_generation() {
        let path = PathBuf::from("C:\\Program Files\\PicaView\\PicaView.exe");
        let (reg, _) = generate_shell_integration_scripts(path).unwrap();
        // Verify path is correctly escaped
        assert!(reg.contains("C:\\\\Program Files\\\\PicaView\\\\PicaView.exe"));
        assert!(reg.contains("Windows Registry Editor Version 5.00"));
        assert!(reg.contains("HKEY_CURRENT_USER\\Software\\Classes\\Directory\\shell\\PicaView"));
    }

    #[test]
    fn test_desktop_generation() {
        let path = PathBuf::from("/usr/bin/picaview");
        let (_, desktop) = generate_shell_integration_scripts(path).unwrap();
        assert!(desktop.contains("Exec=/usr/bin/picaview %u"));
        assert!(desktop.contains("MimeType="));
        assert!(desktop.contains("inode/directory"));
    }
}
