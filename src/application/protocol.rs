use crate::application::file_service::OpenTargetResponse;
use crate::application::startup_service::StartupMetadata;
use crate::models::{ImageInfo, WindowState};
use serde::{Deserialize, Serialize};

/// Unified Command Protocol for PicaView
/// This enum represents all actions the UI can request from the Backend.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "cmd", content = "payload")]
pub enum AppCommand {
    // --- File Service ---
    OpenTarget {
        path: String,
    },
    OpenFile {
        path: String,
    },
    DeleteCurrentFile,
    RevealInExplorer {
        path: String,
    },

    // --- Navigation Service ---
    GetNextImage,
    GetPrevImage,
    SetMangaMode {
        enabled: bool,
    },
    SetSortConfig {
        criteria: String,
        order: String,
    },
    ToggleSlideshow {
        interval_secs: u64,
    },

    // --- Asset Browser Service ---
    GetAllImagePaths,
    TriggerRecursiveScan {
        path: String,
        depth: Option<u32>,
    },
    GetThumbnailData {
        path: String,
        size: u32,
    },
    CancelThumbnail {
        path: String,
        size: u32,
    },

    // --- History Service ---
    GetHistory,
    AddToHistory {
        path: String,
    },
    RemoveFromHistory {
        path: String,
    },
    ClearHistory,

    // --- Workshop Service ---
    PrepareWorkshopAsset {
        path: String,
    },
    ClearWorkshopCache,
    BatchConvert {
        input_paths: Vec<String>,
        target_format: String,
        output_dir: String,
    },
    SynthesizeImages {
        input_paths: Vec<String>,
        config: serde_json::Value,
        output_path: String,
    },

    // --- Startup Service ---
    GetStartupMetadata,
    GetStartupArgs,

    // --- Platform / Dialogs ---
    OpenFileDialog {
        multiple: bool,
    },
    OpenFolderDialog,
    SaveFileDialog {
        title: Option<String>,
    },

    // --- Window / Platform ---
    CloseWindow,
    MinimizeWindow,
    ToggleMaximize,
    GetWindowState,
    CopyImageToClipboard {
        path: String,
    },
    CopyFileToClipboard {
        path: String,
    },
    SaveImageAs {
        path: String,
    },
    OpenInExternalEditor {
        path: String,
    },
    WriteToClipboard {
        text: String,
    },
    SetWallpaper {
        path: String,
        mode: String,
    },
    ApplyShellIntegration,
}

/// Unified Event Protocol for PicaView
/// This enum represents all notifications the Backend can push to the UI.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "event", content = "payload")]
pub enum AppEvent {
    WorkshopProgress { current: u32, total: u32 },
    SlideshowNextImage,
    AssetBrowserBatch(Vec<crate::core::asset_browser::AssetInfo>),
    AssetBrowserComplete,
    SingleInstanceStartup { args: Vec<String> },
    PasswordRequired { path: String },
}

pub trait EventEmitter: Send + Sync {
    fn emit(&self, event: AppEvent) -> Result<(), String>;
}

/// Response wrapper for unified dispatch
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", content = "data")]
pub enum AppResponse {
    Success,
    OpenTarget(OpenTargetResponse),
    StartupMetadata(StartupMetadata),
    WindowState(WindowState),
    ImageInfo(ImageInfo),
    ImageInfoOption(Option<ImageInfo>),
    StringList(Vec<String>),
    StringOption(Option<String>),
    Json(serde_json::Value),
    Boolean(bool),
    Error(String),
}
