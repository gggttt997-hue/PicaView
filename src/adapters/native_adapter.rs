use crate::application::protocol::{AppEvent, EventEmitter};
use crate::application::window_service::WindowService;
use crate::models::WindowState;
use crate::MainWindow;
use slint::ComponentHandle;

pub struct SlintWindowService {
    pub ui: slint::Weak<MainWindow>,
}

impl WindowService for SlintWindowService {
    fn close(&self) {
        let ui_weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let _ = ui.hide();
                std::process::exit(0);
            }
        });
    }

    fn minimize(&self) {
        let ui_weak = self.ui.clone();
        tracing::info!("SlintWindowService: request minimize");
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.window().set_minimized(true);
                tracing::info!("SlintWindowService: window minimized set to true");
            }
        });
    }

    fn toggle_maximize(&self) {
        let ui_weak = self.ui.clone();
        tracing::info!("SlintWindowService: request toggle_maximize");
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                // Use the UI property as the source of truth for the toggle
                let current_ui_state = ui.get_is_maximized();
                let next_state = !current_ui_state;

                tracing::info!(
                    "SlintWindowService: UI maximize property is {}, toggling to {}",
                    current_ui_state,
                    next_state
                );

                // 1. Tell the OS window to maximize/unmaximize
                ui.window().set_maximized(next_state);

                // 2. Update the UI property to ensure the icon changes
                ui.set_is_maximized(next_state);
            }
        });
    }

    fn get_state(&self) -> WindowState {
        // Note: This is called from the logic thread, but Slint's window state is on the UI thread.
        // For now, we return the last known state or a default.
        // In a real scenario, we might want to sync this state via properties.
        WindowState {
            is_maximized: false, // Default to false, will be updated by UI interactions
            is_minimized: false,
            is_fullscreen: false,
        }
    }
}

pub struct SlintEventEmitter {
    pub ui: slint::Weak<MainWindow>,
}

impl EventEmitter for SlintEventEmitter {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        let ui_weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                match event {
                    AppEvent::SlideshowNextImage => {
                        ui.invoke_slideshow_tick();
                    }
                    AppEvent::PasswordRequired { path } => {
                        ui.set_password_archive_path(path.into());
                        ui.set_password_modal_visible(true);
                    }
                    _ => {
                        // Handle other events as needed
                    }
                }
            }
        });
        Ok(())
    }
}
