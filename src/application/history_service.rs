use crate::state::AppState;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

#[derive(Clone)]
pub struct HistoryService {
    state: Arc<AppState>,
}

impl HistoryService {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    pub async fn add_to_history(&self, path: String) -> std::result::Result<(), String> {
        let mut history = self
            .state
            .history_manager
            .lock()
            .map_err(|e| e.to_string())?;
        let normalized = crate::utils::path_utils::normalize_native_path(PathBuf::from(path))
            .to_string_lossy()
            .to_string();
        history.add(normalized);
        Ok(())
    }

    pub async fn get_history(&self) -> std::result::Result<Vec<String>, String> {
        use futures::future::join_all;
        use tokio::fs;

        let items = {
            let history = self
                .state
                .history_manager
                .lock()
                .map_err(|e| e.to_string())?;
            history.get_all()
        };

        if items.is_empty() {
            return Ok(vec![]);
        }

        let checks = items.iter().map(|path| {
            let p = path.clone();
            async move {
                let exists = fs::metadata(&p).await.is_ok();
                (p, exists)
            }
        });

        let results = join_all(checks).await;
        let valid_items = results
            .into_iter()
            .filter(|(_, exists)| *exists)
            .map(|(path, _)| path)
            .collect();

        Ok(valid_items)
    }

    pub async fn clear_history(&self) -> std::result::Result<(), String> {
        let mut history = self
            .state
            .history_manager
            .lock()
            .map_err(|e| e.to_string())?;
        history.clear();
        Ok(())
    }

    pub async fn remove_from_history(&self, path: String) -> std::result::Result<(), String> {
        info!("HistoryService: Removing folder from history: {}", path);
        let mut history = self
            .state
            .history_manager
            .lock()
            .map_err(|e| e.to_string())?;
        history.remove(path);
        Ok(())
    }
}
