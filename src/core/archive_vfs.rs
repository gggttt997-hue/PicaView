use crate::error::{PicaViewError, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use tracing::warn;

pub fn archive_passwords() -> &'static RwLock<HashMap<String, String>> {
    static PASSWORDS: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    PASSWORDS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn archive_vfs_cache() -> &'static RwLock<HashMap<String, Arc<Mutex<Box<dyn ArchiveVfs>>>>> {
    static CACHE: OnceLock<RwLock<HashMap<String, Arc<Mutex<Box<dyn ArchiveVfs>>>>>> =
        OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn get_shared_vfs(path: &Path) -> Result<Arc<Mutex<Box<dyn ArchiveVfs>>>> {
    let path_str = path.to_string_lossy().to_string();

    {
        let cache = archive_vfs_cache().read().unwrap();
        if let Some(vfs) = cache.get(&path_str) {
            return Ok(Arc::clone(vfs));
        }
    }

    let mut cache = archive_vfs_cache().write().unwrap();
    if let Some(vfs) = cache.get(&path_str) {
        return Ok(Arc::clone(vfs));
    }

    if cache.len() >= 2 {
        cache.clear();
    }

    let vfs = mount_archive(path)?;
    let shared = Arc::new(Mutex::new(vfs));
    cache.insert(path_str, Arc::clone(&shared));
    Ok(shared)
}

pub trait ArchiveVfs: Send + Sync {
    /// 扫描压缩包中的所有文件名及对应大小（支持图片的入口）
    fn list_entries(&mut self, password: Option<&str>) -> Result<Vec<(String, u64)>>;

    /// 提取指定条目为内存字节流
    fn extract_file(&mut self, entry_name: &str, password: Option<&str>) -> Result<Vec<u8>>;

    /// 返回 VFS 引擎名称
    fn vfs_type(&self) -> &str;
}

pub struct ZipVfs {
    path: PathBuf,
    archive: zip::ZipArchive<File>,
}

impl ZipVfs {
    pub fn new(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| {
            PicaViewError::general(format!("Failed to open zip file {}: {}", path.display(), e))
        })?;
        let archive = zip::ZipArchive::new(file).map_err(|e| {
            PicaViewError::general(format!(
                "Failed to parse zip archive {}: {}",
                path.display(),
                e
            ))
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            archive,
        })
    }
}

impl ArchiveVfs for ZipVfs {
    fn list_entries(&mut self, password: Option<&str>) -> Result<Vec<(String, u64)>> {
        let mut entries = Vec::new();
        let password_bytes = password.unwrap_or("").as_bytes();

        for i in 0..self.archive.len() {
            let file_result = self.archive.by_index_decrypt(i, password_bytes);

            match file_result {
                Ok(Ok(file)) => {
                    if file.is_file() {
                        entries.push((file.name().to_string(), file.size()));
                    }
                }
                Ok(Err(_)) => {
                    return Err(PicaViewError::PasswordRequired);
                }
                Err(zip::result::ZipError::UnsupportedArchive(e)) if e == "Password required" => {
                    return Err(PicaViewError::PasswordRequired);
                }
                Err(e) => {
                    warn!("Failed to read zip entry at index {}: {}", i, e);
                }
            }
        }
        Ok(entries)
    }

    fn extract_file(&mut self, entry_name: &str, password: Option<&str>) -> Result<Vec<u8>> {
        let entry_name_norm = entry_name.replace('\\', "/");
        let password_bytes = password.unwrap_or("").as_bytes();

        let file_result = self
            .archive
            .by_name_decrypt(&entry_name_norm, password_bytes);

        let mut entry = match file_result {
            Ok(Ok(file)) => file,
            Ok(Err(_)) => return Err(PicaViewError::PasswordRequired),
            Err(zip::result::ZipError::UnsupportedArchive(e)) if e == "Password required" => {
                return Err(PicaViewError::PasswordRequired);
            }
            Err(e) => {
                return Err(PicaViewError::general(format!(
                    "Failed to locate entry {} in archive: {}",
                    entry_name, e
                )))
            }
        };

        let mut buffer = Vec::with_capacity(entry.size() as usize);
        if entry.read_to_end(&mut buffer).is_err() {
            return Err(PicaViewError::PasswordRequired);
        }

        Ok(buffer)
    }

    fn vfs_type(&self) -> &str {
        "ZIP"
    }
}

pub struct SevenZVfs {
    path: PathBuf,
}

impl SevenZVfs {
    pub fn new(path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
        })
    }
}

impl ArchiveVfs for SevenZVfs {
    fn list_entries(&mut self, password: Option<&str>) -> Result<Vec<(String, u64)>> {
        let mut file = File::open(&self.path).map_err(|e| PicaViewError::general(e.to_string()))?;
        let len = file.metadata().unwrap().len();
        let pass = password.map(|p| p.as_bytes().to_vec()).unwrap_or_default();

        let archive = sevenz_rust::Archive::read(&mut file, len, pass.as_slice()).map_err(|e| {
            if e.to_string().to_lowercase().contains("password")
                || e.to_string().to_lowercase().contains("decrypt")
            {
                PicaViewError::PasswordRequired
            } else {
                PicaViewError::general(e.to_string())
            }
        })?;

        let mut entries = Vec::new();
        for file in &archive.files {
            if file.has_stream() {
                entries.push((file.name().to_string(), file.size()));
            }
        }
        Ok(entries)
    }

    fn extract_file(&mut self, entry_name: &str, password: Option<&str>) -> Result<Vec<u8>> {
        let entry_name_norm = entry_name.replace('\\', "/");
        let mut file = File::open(&self.path).map_err(|e| PicaViewError::general(e.to_string()))?;
        let pass = password
            .map(sevenz_rust::Password::from)
            .unwrap_or_else(sevenz_rust::Password::empty);

        let mut buffer = Vec::new();
        let target_name = entry_name_norm.clone();

        sevenz_rust::decompress_with_extract_fn_and_password(
            &mut file,
            "",
            pass,
            |entry: &sevenz_rust::SevenZArchiveEntry,
             reader: &mut dyn std::io::Read,
             _|
             -> std::result::Result<bool, sevenz_rust::Error> {
                if entry.name() == target_name {
                    std::io::copy(reader, &mut buffer).map_err(sevenz_rust::Error::io)?;
                }
                Ok(true)
            },
        )
        .map_err(|e: sevenz_rust::Error| {
            if e.to_string().to_lowercase().contains("password")
                || e.to_string().to_lowercase().contains("decrypt")
            {
                PicaViewError::PasswordRequired
            } else {
                PicaViewError::general(e.to_string())
            }
        })?;

        if buffer.is_empty() {
            Err(PicaViewError::general(format!(
                "Entry {} not found or empty",
                target_name
            )))
        } else {
            Ok(buffer)
        }
    }

    fn vfs_type(&self) -> &str {
        "7Z"
    }
}

pub struct CliStreamVfs {
    path: PathBuf,
    executable: String,
}

impl CliStreamVfs {
    pub fn new(path: &Path, executable: &str) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            executable: executable.to_string(),
        })
    }
}

impl ArchiveVfs for CliStreamVfs {
    fn list_entries(&mut self, password: Option<&str>) -> Result<Vec<(String, u64)>> {
        let mut cmd = std::process::Command::new(&self.executable);
        cmd.arg("l").arg("-slt").arg(&self.path);

        let path_lossy_norm = self.path.to_string_lossy().to_string().replace('\\', "/");

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        if let Some(pass) = password {
            cmd.arg(format!("-p{}", pass));
        } else {
            cmd.stdin(std::process::Stdio::null());
        }

        let output = cmd
            .output()
            .map_err(|e| PicaViewError::general(e.to_string()))?;
        let output_str = String::from_utf8_lossy(&output.stdout);

        if !output.status.success() {
            let err_str = String::from_utf8_lossy(&output.stderr).to_lowercase();
            let out_str = output_str.to_lowercase();
            if err_str.contains("wrong password")
                || out_str.contains("wrong password")
                || err_str.contains("enter password")
                || out_str.contains("enter password")
            {
                return Err(PicaViewError::PasswordRequired);
            }
            return Err(PicaViewError::general(format!(
                "CLI list failed: {}",
                err_str
            )));
        }

        let mut entries = Vec::new();
        let mut current_path = String::new();
        let mut current_size = 0u64;
        let mut is_dir = false;

        for line in output_str.lines() {
            let line = line.trim();
            if line.is_empty() {
                if !current_path.is_empty() && !is_dir {
                    let current_norm = current_path.replace('\\', "/");
                    if current_norm != path_lossy_norm
                        && current_path
                            != self.path.file_name().unwrap_or_default().to_string_lossy()
                    {
                        entries.push((current_path.clone(), current_size));
                    }
                }
                current_path.clear();
                current_size = 0;
                is_dir = false;
                continue;
            }

            if let Some(path) = line.strip_prefix("Path = ") {
                current_path = path.to_string();
            } else if let Some(size) = line.strip_prefix("Size = ") {
                current_size = size.parse().unwrap_or(0);
            } else if let Some(attr) = line.strip_prefix("Attributes = ") {
                if attr.contains('D') {
                    is_dir = true;
                }
            }
        }

        // Push last entry if exists
        if !current_path.is_empty() && !is_dir {
            let current_norm = current_path.replace('\\', "/");
            if current_norm != path_lossy_norm
                && current_path != self.path.file_name().unwrap_or_default().to_string_lossy()
            {
                entries.push((current_path, current_size));
            }
        }

        // 7z l -slt sometimes includes the archive itself as the first Path=.
        // We filter it out if its name matches the archive name exactly and it's the first one, or we just rely on it not being an image extension.

        Ok(entries)
    }

    fn extract_file(&mut self, entry_name: &str, password: Option<&str>) -> Result<Vec<u8>> {
        let entry_name_norm = entry_name.replace('\\', "/");
        // Run 7z.exe e -so <archive> <file>
        let mut cmd = std::process::Command::new(&self.executable);
        cmd.arg("e")
            .arg("-so")
            .arg(&self.path)
            .arg(&entry_name_norm);

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        if let Some(pass) = password {
            cmd.arg(format!("-p{}", pass));
        } else {
            cmd.stdin(std::process::Stdio::null());
        }

        let output = cmd
            .output()
            .map_err(|e| PicaViewError::general(e.to_string()))?;

        if !output.status.success() {
            let err_str = String::from_utf8_lossy(&output.stderr).to_lowercase();
            if err_str.contains("wrong password") || err_str.contains("enter password") {
                return Err(PicaViewError::PasswordRequired);
            }
            return Err(PicaViewError::general(format!(
                "CLI extract failed: {}",
                err_str
            )));
        }

        Ok(output.stdout)
    }

    fn vfs_type(&self) -> &str {
        "CLI"
    }
}

pub struct UnRarVfs {
    path: PathBuf,
    executable: String,
}

impl UnRarVfs {
    pub fn new(path: &Path, executable: &str) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            executable: executable.to_string(),
        })
    }
}

impl ArchiveVfs for UnRarVfs {
    fn vfs_type(&self) -> &str {
        "UNRAR"
    }

    fn list_entries(&mut self, password: Option<&str>) -> Result<Vec<(String, u64)>> {
        let mut cmd = std::process::Command::new(&self.executable);
        cmd.arg("vt");
        if let Some(p) = password {
            cmd.arg(format!("-p{}", p));
        } else {
            cmd.arg("-p-");
        }
        cmd.arg("-scu");
        cmd.arg(&self.path);

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let output = cmd
            .output()
            .map_err(|e| PicaViewError::general(e.to_string()))?;
        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_lowercase();
            if err_msg.contains("password") || err_msg.contains("checksum") {
                return Err(PicaViewError::PasswordRequired);
            }
            return Err(PicaViewError::general(format!(
                "UnRar list failed: {}",
                err_msg
            )));
        }

        // Decode UTF-16LE
        let stdout_u16: Vec<u16> = output
            .stdout
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        let output_str = String::from_utf16_lossy(&stdout_u16);

        let mut entries = Vec::new();
        let mut current_name = String::new();
        let mut current_size = 0u64;
        let mut is_file = false;

        for line in output_str.lines() {
            let line = line.trim();
            if line.starts_with("Name:") {
                current_name = line["Name:".len()..].trim().to_string();
            } else if line.starts_with("Type:") {
                is_file = line["Type:".len()..].trim().eq_ignore_ascii_case("File");
            } else if line.starts_with("Size:") {
                if let Ok(sz) = line["Size:".len()..].trim().parse::<u64>() {
                    current_size = sz;
                }
            } else if line.is_empty() {
                if !current_name.is_empty() && is_file {
                    let norm_name = current_name.replace('\\', "/");
                    entries.push((norm_name, current_size));
                }
                current_name.clear();
                current_size = 0;
                is_file = false;
            }
        }
        if !current_name.is_empty() && is_file {
            let norm_name = current_name.replace('\\', "/");
            entries.push((norm_name, current_size));
        }

        Ok(entries)
    }

    fn extract_file(&mut self, entry_name: &str, password: Option<&str>) -> Result<Vec<u8>> {
        let entry_name_win = entry_name.replace('/', "\\");
        let mut cmd = std::process::Command::new(&self.executable);
        cmd.arg("p").arg("-inul");
        if let Some(p) = password {
            cmd.arg(format!("-p{}", p));
        } else {
            cmd.arg("-p-");
        }
        cmd.arg(&self.path).arg(&entry_name_win);

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let output = cmd
            .output()
            .map_err(|e| PicaViewError::general(e.to_string()))?;
        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_lowercase();
            if err_msg.contains("password") || err_msg.contains("checksum") {
                return Err(PicaViewError::PasswordRequired);
            }
            return Err(PicaViewError::general(format!(
                "UnRar extract failed: {}",
                err_msg
            )));
        }

        Ok(output.stdout)
    }
}

fn locate_unrar_executable() -> Option<String> {
    // 1. Check D:\Program Files\WinRAR\UnRar.exe
    let p1 = Path::new("D:\\Program Files\\WinRAR\\UnRar.exe");
    if p1.exists() {
        return Some(p1.to_string_lossy().to_string());
    }

    // 2. Check C:\Program Files\WinRAR\UnRar.exe
    let p2 = Path::new("C:\\Program Files\\WinRAR\\UnRar.exe");
    if p2.exists() {
        return Some(p2.to_string_lossy().to_string());
    }

    // 3. Check C:\Program Files (x86)\WinRAR\UnRar.exe
    let p3 = Path::new("C:\\Program Files (x86)\\WinRAR\\UnRar.exe");
    if p3.exists() {
        return Some(p3.to_string_lossy().to_string());
    }

    // 4. Query Registry via reg query command to avoid winreg dependency
    if let Some(exe) = locate_unrar_executable_via_reg() {
        return Some(exe);
    }

    // 5. Fallback to system path UnRAR command
    if let Ok(output) = std::process::Command::new("UnRAR").arg("-?").output() {
        if output.status.success() {
            return Some("UnRAR".to_string());
        }
    }

    None
}

fn locate_unrar_executable_via_reg() -> Option<String> {
    let mut cmd = std::process::Command::new("reg");
    cmd.arg("query")
        .arg("HKEY_CLASSES_ROOT\\WinRAR\\shell\\open\\command")
        .arg("/ve");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    if let Ok(output) = cmd.output() {
        if output.status.success() {
            let out_str = String::from_utf8_lossy(&output.stdout);
            for line in out_str.lines() {
                if let Some(idx) = line.find("WinRAR.exe") {
                    let start = line.find('"').unwrap_or(0);
                    let path_str = if start > 0 {
                        &line[start + 1..idx + 10]
                    } else {
                        &line[..idx + 10]
                    };
                    let winrar_dir = Path::new(path_str).parent();
                    if let Some(dir) = winrar_dir {
                        let unrar_path = dir.join("UnRar.exe");
                        if unrar_path.exists() {
                            return Some(unrar_path.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

fn locate_7z_executable(ext: &str) -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|parent| parent.to_path_buf()))
        .map(|dir| {
            let p7z = dir.join("vendor").join("bin").join("7z.exe");
            let p7za = dir.join("vendor").join("bin").join("7za.exe");
            if p7z.exists() {
                p7z.to_string_lossy().to_string()
            } else if p7za.exists() {
                p7za.to_string_lossy().to_string()
            } else {
                // 1. 开发调试环境回退查找本地项目根目录下的 vendor
                if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
                    let dev_p7za = std::path::Path::new(&manifest_dir)
                        .join("vendor")
                        .join("bin")
                        .join("7za.exe");
                    if dev_p7za.exists() {
                        return dev_p7za.to_string_lossy().to_string();
                    }
                }
                // 2. 对于 RAR/CBR，优先回退到 "7z"；其余回退到 "7za"
                if ext == "rar" || ext == "cbr" {
                    "7z".to_string()
                } else {
                    "7za".to_string()
                }
            }
        })
        .unwrap_or_else(|| {
            if ext == "rar" || ext == "cbr" {
                "7z".to_string()
            } else {
                "7za".to_string()
            }
        })
}

pub fn mount_archive(path: &Path) -> Result<Box<dyn ArchiveVfs>> {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    match ext.as_str() {
        "zip" | "cbz" => Ok(Box::new(ZipVfs::new(path)?)),
        "rar" | "cbr" => {
            if let Some(unrar_exe) = locate_unrar_executable() {
                Ok(Box::new(UnRarVfs::new(path, &unrar_exe)?))
            } else {
                let exe = locate_7z_executable(&ext);
                Ok(Box::new(CliStreamVfs::new(path, &exe)?))
            }
        }
        "7z" | "cb7" => {
            let exe = locate_7z_executable(&ext);
            Ok(Box::new(CliStreamVfs::new(path, &exe)?))
        }
        _ => Err(PicaViewError::general(format!(
            "Unsupported archive format: {}",
            ext
        ))),
    }
}
