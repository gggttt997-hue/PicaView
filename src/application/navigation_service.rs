use crate::core::image_loader::load_image_info;
use crate::models::ImageInfo;
use crate::state::AppState;
use std::sync::Arc;
use tracing::{debug, info, warn};

#[derive(Clone)]
pub struct NavigationService {
    state: Arc<AppState>,
}

impl NavigationService {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    /// Get the next image
    pub async fn get_next_image(&self) -> std::result::Result<Option<ImageInfo>, String> {
        debug!("NavigationService: Requesting next image");

        // Manga mode boundary interception
        let manga_mode = *self
            .state
            .manga_mode_enabled
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if manga_mode {
            let (current_index, total_count) = self
                .state
                .with_file_manager(|manager| (manager.current_index(), manager.total_count()))
                .unwrap_or((0, 0));

            if total_count > 0 && current_index == total_count - 1 {
                let next_dir = self
                    .state
                    .with_file_manager(|manager| manager.find_neighboring_dir(true))
                    .unwrap_or(Ok(None));

                if let Ok(Some(path)) = next_dir {
                    info!("Manga mode: Reached end boundary, suggesting switch to next directory: {:?}", path);
                    return Err(crate::error::PicaViewError::MangaNextFolder(path)
                        .to_accurate_frontend_message());
                }
            }
        }

        // Get the path of the next image
        let next_path = self
            .state
            .with_file_manager_mut(|manager| manager.next_image());

        match next_path {
            Ok(Some(path)) => {
                debug!("Navigating to next image: {}", path.display());

                // Trigger preloading for the NEXT NEXT image
                let state_clone = Arc::clone(&self.state);
                let next_next_path = self
                    .state
                    .with_file_manager(|m| {
                        let total = m.total_count();
                        if total > 0 {
                            let idx = (m.current_index() + 1) % total;
                            m.get_all_infos()
                                .get(idx)
                                .map(|info| std::path::PathBuf::from(&info.path))
                        } else {
                            None
                        }
                    })
                    .unwrap_or(None);

                if let Some(p) = next_next_path {
                    tokio::spawn(async move {
                        crate::core::image_loader::preload_image(&p, state_clone).await;
                    });
                }

                match load_image_info(&path).await {
                    Ok(image_info) => Ok(Some(image_info)),
                    Err(e) => {
                        let error_msg = e.to_accurate_frontend_message();
                        warn!(
                            "Failed to load next image: {} - {}",
                            path.display(),
                            error_msg
                        );
                        Err(error_msg)
                    }
                }
            }
            Ok(None) => Ok(None),
            Err(lock_error) => Err(format!("Failed to acquire state lock: {}", lock_error)),
        }
    }

    /// Get the previous image
    pub async fn get_prev_image(&self) -> std::result::Result<Option<ImageInfo>, String> {
        debug!("NavigationService: Requesting previous image");

        // Manga mode boundary interception
        let manga_mode = *self
            .state
            .manga_mode_enabled
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if manga_mode {
            let current_index = self
                .state
                .with_file_manager(|manager| manager.current_index())
                .unwrap_or(0);

            if current_index == 0 {
                let prev_dir = self
                    .state
                    .with_file_manager(|manager| manager.find_neighboring_dir(false))
                    .unwrap_or(Ok(None));

                if let Ok(Some(path)) = prev_dir {
                    info!("Manga mode: Reached start boundary, suggesting switch to previous directory: {:?}", path);
                    return Err(crate::error::PicaViewError::MangaPrevFolder(path)
                        .to_accurate_frontend_message());
                }
            }
        }

        // Get the path of the previous image
        let prev_path = self.state.with_file_manager_mut(|manager| manager.prev());

        match prev_path {
            Ok(Some(path)) => {
                debug!("Navigating to previous image: {}", path.display());

                // Trigger preloading for the PREV PREV image
                let state_clone = Arc::clone(&self.state);
                let prev_prev_path = self
                    .state
                    .with_file_manager(|m| {
                        let total = m.total_count();
                        if total > 0 {
                            let idx = if m.current_index() == 0 {
                                total - 1
                            } else {
                                m.current_index() - 1
                            };
                            m.get_all_infos()
                                .get(idx)
                                .map(|info| std::path::PathBuf::from(&info.path))
                        } else {
                            None
                        }
                    })
                    .unwrap_or(None);

                if let Some(p) = prev_prev_path {
                    tokio::spawn(async move {
                        crate::core::image_loader::preload_image(&p, state_clone).await;
                    });
                }

                match load_image_info(&path).await {
                    Ok(image_info) => Ok(Some(image_info)),
                    Err(e) => {
                        let error_msg = e.to_accurate_frontend_message();
                        warn!(
                            "Failed to load previous image: {} - {}",
                            path.display(),
                            error_msg
                        );
                        Err(error_msg)
                    }
                }
            }
            Ok(None) => Ok(None),
            Err(lock_error) => Err(format!("Failed to acquire state lock: {}", lock_error)),
        }
    }

    /// Set manga mode toggle
    pub fn set_manga_mode(&self, enabled: bool) -> std::result::Result<(), String> {
        match self.state.manga_mode_enabled.lock() {
            Ok(mut mode) => {
                *mode = enabled;
                info!("Manga mode state changed: {}", enabled);
                Ok(())
            }
            Err(e) => Err(format!("Failed to set manga mode: {}", e)),
        }
    }
}
