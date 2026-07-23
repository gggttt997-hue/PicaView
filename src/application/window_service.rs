use crate::models::WindowState;

/// Trait to abstract window operations across different platforms (Tauri/Slint)
pub trait WindowService: Send + Sync {
    fn close(&self);
    fn minimize(&self);
    fn toggle_maximize(&self);
    fn get_state(&self) -> WindowState;
}
