use crate::state::AppState;
use std::sync::Arc;
use tokio::time::{self, Duration};
use tracing::info;

#[derive(Clone)]
pub struct SlideshowService {
    state: Arc<AppState>,
}

impl SlideshowService {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    /// Toggle slideshow playback status
    pub async fn toggle_slideshow<F>(
        &self,
        interval_secs: u64,
        on_tick: F,
    ) -> std::result::Result<bool, String>
    where
        F: Fn() + Send + Sync + 'static,
    {
        info!(
            "SlideshowService: Toggle slideshow request, interval: {}s",
            interval_secs
        );

        if interval_secs == 0 {
            return Err("Slideshow interval cannot be 0".to_string());
        }

        let mut handle = self
            .state
            .slideshow_handle
            .lock()
            .map_err(|e| e.to_string())?;

        if let Some(existing_handle) = handle.take() {
            info!("SlideshowService: Stopping existing task");
            existing_handle.abort();
            Ok(false)
        } else {
            info!("SlideshowService: Starting new task");

            let mut interval_state = self
                .state
                .slideshow_interval_secs
                .lock()
                .map_err(|e| e.to_string())?;
            *interval_state = interval_secs;

            let new_handle = tokio::spawn(async move {
                let mut interval = time::interval(Duration::from_secs(interval_secs));
                loop {
                    interval.tick().await;
                    info!("SlideshowService: Timer triggered");
                    on_tick();
                }
            });

            *handle = Some(new_handle);
            Ok(true)
        }
    }
}
