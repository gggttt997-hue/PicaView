# PicaView

> A local image and manga viewer.

[简体中文](./README_zh.md)

PicaView is a desktop tool built with **Native Rust + Slint + WGPU**, primarily designed for standard image viewing and local manga reading. By utilizing **WGPU 28** and a VRAM streaming strategy, it maintains decent interactive performance and reasonable memory usage when handling higher-resolution images.

---

## 🚀 Quick Start

### Option A: Linux (Ubuntu/Debian/Fedora)
```bash
# Install Rust and system dependencies (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
sudo apt-get update && sudo apt-get install -y libfontconfig1-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev

# Clone and run the development version
git clone https://github.com/MikeWu-GM/PicaView.git
cd PicaView
cargo run --release -- /path/to/your/image.jpg
```

### Option B: Windows (Installer & PowerShell)
**For Users:** Download the latest `picaview-0.1.0-x86_64.msi` installer from the Releases page. It automatically configures the system `PATH`, registers image file associations, integrates with Windows 11 Default Apps, and natively manages clean upgrades using Windows Registry-backed installation directory memory.

**For Developers:**
```powershell
# Clone the repository
git clone https://github.com/MikeWu-GM/PicaView.git
cd PicaView

# Run the development version
cargo run --release -- "C:\path\to\your\image.jpg"
```

---

## 📦 Installation & Build

### 1. Prerequisites
*   **Rust**: 1.77.2+ (1.93.0+ recommended)
*   **C Compiler**: `build-essential` on Linux, MSVC on Windows.

### 2. Production Build
**Linux (Standard Binary):**
```bash
cargo build --release
```
The build artifacts will be located in `target/release/`.

**Windows (MSI Installer):**
```powershell
$env:PATH += ";$PWD\wix-bin"
cargo wix --nocapture
```
The MSI package will be generated at `target/wix/`. Alternatively, you can double-click or run the interactive console script `.\scripts\build.bat` in Windows to choose between building the MSI package or generating a portable ZIP release.

---

## 🛠️ Key Features

*   **GPU Acceleration**: Uses WGPU 28 for hardware-level rendering to improve display smoothness.
*   **Large Image Tiling**: Implements VRAM Streaming to slice and load high-resolution images in the background, maintaining basic responsiveness while reducing memory overhead.
*   **Native Execution**: Compiled as a native Rust binary for standard startup speeds.
*   **Format Support**: Reads WebP, TIFF, JPEG, PNG, BMP, GIF, and ICO files.
*   **Directory Scanning**: Features progressive slide-window scanning and snapshot caching to keep folder navigation functional in directories with many files.
*   **Background Pre-caching**: Low-priority pre-caching of adjacent folders during continuous reading to smooth out transitions.
*   **Manga Mode**: Provides cascading navigation for continuous cross-folder reading.
*   **Archive Viewing (ZIP/CBZ/7Z/RAR)**: Allows previewing images inside archives without manual extraction. ZIP/CBZ files are processed by Rust libraries, while RAR/7Z rely on installed CLI tools. Includes basic password prompts and thumbnail parsing.
*   **Long Image Mode**: Stitches images vertically for continuous scrolling, which is useful for long manga strips or web captures. Includes a basic auto-scroll toggle.
*   **Scroll Behavior Toggle**: Switch mouse wheel behavior between zooming and image navigation, complete with basic debounce filtering.
*   **Wallpaper Settings**: Click to set the current image as the desktop wallpaper (supports Windows and GNOME).
*   **Basic File Operations**: Right-click to copy files, save as, or open in the system's default editor.
*   **Context Menu Integration**: Scripts provided for system right-click menu integration (Windows .reg / Linux .desktop).
*   **Process Limits**: Defaults to single-instance to save memory, with an optional multi-instance mode that has a hard cap of 5 windows.
*   **State Persistence**: Locally saves the last viewed image position for each folder.
*   **Quick Rating**: Use 1-5 keys to categorize images into subdirectories locally.
*   **EXIF Info**: Press 'I' to view basic camera metadata or filter the current directory based on EXIF parameters.
*   **Viewport Lock**: Locks zoom and offset coordinates across different images for straightforward comparisons.
*   **Channel Mode**: View R/G/B/A color channels individually.
*   **Split-Screen Compare**: Dual-viewport design supporting horizontal, vertical, and swipe layouts for side-by-side comparison.
*   **ICC Color Management**: Uses lcms2 to parse ICC profiles and convert colors to the sRGB space, reducing color shift on wide-gamut displays.
*   **Borderless Drag & Drop**: Support for dragging the borderless window and dropping files directly into the UI.

---

## 📖 Usage Guide

### CLI Arguments
PicaView supports opening files or directories directly from the command line:
```bash
picaview [path] [options]
```
*   **Passing a file path**: Opens the image directly in **Viewer Mode**.
*   **Passing a directory path**: Opens the directory in **Gallery Mode** for asset browsing.

### Gallery Interactions & Thumbnail Center-sync
*   **Scrolling**: High-performance virtual scrolling, regardless of directory size. Selecting a thumbnail automatically center-aligns/offsets the thumbnail strip.
*   **Search**: Real-time filename filtering.
*   **Sorting**: Supports natural language sorting by Name, Size, and Date.

### Keyboard Shortcuts & Interaction Workflow
*   **Scroll & Navigate Toggle**: Click the viewport with your mouse left button (or trigger from settings) to toggle the scroll wheel mode.
    *   *Zoom Mode*: Mouse wheel zooms the image in/out focused on the cursor position.
    *   *Navigation Mode*: Mouse wheel cycles through adjacent images (protected by a 250ms debounce filter to prevent rapid double-triggering).
*   **Continuous Waterfall Long Image Mode**: Toggle this mode to stitch folder images into a unified vertical waterfall. Standard mouse wheel scrolls vertically through the continuous canvas, and zooming automatically unlocks horizontal panning when the scaled width exceeds the viewport.
    *   `M` / `m`: Toggle hands-free **Auto-Scroll** mode.
    *   `UpArrow` / `DownArrow`: Manually pan the view vertically. If Auto-Scroll is active, these keys dynamically adjust the continuous scrolling speed.
    *   `PageUp` / `PageDown`: Rapidly page through the vertical waterfall canvas.
*   **Rating Actions**:
    *   `1` - `5`: Rate the current viewed image. Asynchronously copies the file to a `Star_{rating}` subdirectory inside the parent folder, showing a temporary status HUD.
    *   `Ctrl` + `1` - `5`: Toggle and switch the gallery view to the corresponding rated subdirectory.
    *   `Ctrl` + `0`: Return to the original directory before you navigated into the rated folders.
    *   `Ctrl` + `Left Click`: Drag the window to reposition it on the screen (works in windowed borderless mode). Dragging blank background area when no image is loaded also moves the window.
    *   **File Drag & Drop**: Drag files directly from Windows Explorer (even when running as Administrator) into the windowed borderless viewport to open them instantly.
*   **Color Channel & Compare Mode Shortcuts**:
    *   `R` / `G` / `B` / `A`: Switch the GPU rendering output to Red, Green, Blue, or Alpha channel mode respectively.
    *   `C`: If in color channel mode, resets it to default RGB. If in RGB mode, toggles **Linked Split-Screen Compare Mode** (automatically loads the next image for parallel comparison with synchronized zoom and offsets).
    *   `V`: Toggles **Viewport Lock**, pinning the current zoom and offset coordinates across image navigation.
    *   `I`: Toggles the **EXIF Info HUD** frosted-glass overlay.
    *   *Compare custom image*: Right-click on any thumbnail in the bottom gallery or grid gallery to load it immediately as the comparative image.
    *   *Compare split orientation*: Right-click the viewport and cycle between **Horizontal**, **Vertical**, and **Swipe** compare modes. In Swipe mode, drag the center slider for pixel-perfect spatial comparisons.
*   **EXIF Filter Drawer**: Trigger the filter drawer from the context menu (right click) to parameterize your gallery list based on camera specs.

---

## ⚙️ Advanced Configuration

### System Integration
You can export and apply shell integration scripts to make PicaView your default viewer:
1.  Click "Export Integration Scripts" within the app settings.
2.  **Windows**: Run the exported `.reg` file.
3.  **Linux**: Move the `.desktop` file to `~/.local/share/applications/`.

### Logging & Diagnostics
PicaView incorporates a non-blocking asynchronous logging system for troubleshooting:
*   **Log Locations**: When run as a detached Windows program (`windows_subsystem = "windows"`), logs are automatically redirected to `AppData/Local/PicaView/logs/` (or platform equivalent).
*   **Retention**: The logging subsystem includes an auto-rotation and prune mechanism that clears logs older than 7 days, restricting disk space footprint.
*   **Configuration**: High-frequency operations (LOD tiles allocation, physics ticks) are suppressed in release profiles to avoid I/O bottlenecks.

### Antivirus Whitelisting & False Positives
Due to the use of Win32 subclassing APIs (`SetWindowSubclass` for borderless drag-and-drop), PicaView might trigger false positive warnings on Windows Defender or other strict antivirus suites. (Note: The previous `GetAsyncKeyState` hardware key detection has been strictly purged to eliminate keylogger heuristics).
*   **Exclusion**: You can safely add PicaView's directory to your antivirus exclusions.
*   **False Positive Guide**: For a detailed walkthrough on submitting false positive reports to Microsoft Defender, 360, or Tencent, please refer to the [.documents/2026_07_19_00_49_42_false_positive_submission_guide.md](.documents/2026_07_19_00_49_42_false_positive_submission_guide.md).

### Privacy & Security
PicaView runs entirely locally and **never uploads** your image data. 
*   **Thumbnail Cache**: Stored in your system's default App Cache directory, clearable in app settings.
*   **Positions Cache**: Folder position persistence maps are saved locally in the `folder_positions.json` configurations file (in the app config directory). No telemetry or external connections are established.

---

## 📄 License & Contribution

This project is licensed under the **MIT License**.

Issues and Pull Requests are welcome! Before contributing, please refer to our internal standards in `GEMINI.md` to ensure compliance with project engineering practices (such as the **Three-Strike Rule**, **Meta-Cognitive Diagnosis Protocol**, the strict Test-Driven Development (TDD) dev sequence, and the **Empirical Debugging (empirical_debugging)** scientific validation guidelines for deep-water bug diagnostics).
