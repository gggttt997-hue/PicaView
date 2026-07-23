use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum StartupMode {
    Viewer,
    Gallery,
    None,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StartupMetadata {
    pub mode: StartupMode,
    pub path: Option<String>,
}

#[derive(Clone)]
pub struct StartupService {
    state: Arc<AppState>,
}

impl StartupService {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    pub fn get_metadata(&self, args: Vec<String>) -> StartupMetadata {
        debug!(
            "[Evidence] StartupService.get_metadata processing args: {:?}",
            args
        );

        if args.len() > 1 {
            let mut startup_path = args[1].clone();

            startup_path = startup_path
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();

            if startup_path.starts_with("file://") {
                startup_path = startup_path.replace("file://", "");
                if let Ok(decoded) = urlencoding::decode(&startup_path) {
                    startup_path = decoded.into_owned();
                }
            }

            let path = Path::new(&startup_path);
            let final_path = if path.is_relative() {
                std::env::current_dir()
                    .map(|cwd| cwd.join(path))
                    .unwrap_or_else(|_| path.to_path_buf())
            } else {
                path.to_path_buf()
            };

            if final_path.exists() {
                let path_str = final_path.to_string_lossy().to_string();
                if final_path.is_dir() {
                    return StartupMetadata {
                        mode: StartupMode::Gallery,
                        path: Some(path_str),
                    };
                } else {
                    return StartupMetadata {
                        mode: StartupMode::Viewer,
                        path: Some(path_str),
                    };
                }
            }
        }

        StartupMetadata {
            mode: StartupMode::None,
            path: None,
        }
    }

    pub fn take_startup_file(&self) -> Option<String> {
        let file_path = self.state.take_startup_file();
        if let Some(ref path) = file_path {
            info!("StartupService: Acquiring startup file path: {}", path);
        }
        file_path
    }

    pub fn get_args(&self) -> Vec<String> {
        std::env::args().collect()
    }
}
