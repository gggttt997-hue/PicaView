use crate::application::runtime::Runtime;
use crate::MainWindow;
use slint::Model;
use std::sync::Arc;

pub struct WorkshopController {
    ui: slint::Weak<MainWindow>,
    runtime: Arc<Runtime>,
}

impl WorkshopController {
    pub fn new(ui: slint::Weak<MainWindow>, runtime: Arc<Runtime>) -> Self {
        Self { ui, runtime }
    }

    pub fn batch_convert(&self, format_str: String) {
        let ui_weak = self.ui.clone();
        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            let (tx_data, rx_data) = tokio::sync::oneshot::channel::<(Vec<String>, Vec<String>)>();

            let ui_weak_for_scan = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let mut assets = Vec::new();
                let mut selected = Vec::new();
                if let Some(ui) = ui_weak_for_scan.upgrade() {
                    for item in ui.get_thumbnails().iter() {
                        assets.push(item.path.to_string());
                        if item.selected {
                            selected.push(item.path.to_string());
                        }
                    }
                }
                let _ = tx_data.send((assets, selected));
            });

            let (assets, selected_assets) = rx_data.await.unwrap_or_default();
            let final_assets = if !selected_assets.is_empty() {
                selected_assets
            } else {
                assets
            };

            if final_assets.is_empty() {
                return;
            }

            let target_format = match format_str.as_str() {
                "jpg" => crate::core::image_processor::OutputFormat::Jpeg,
                "png" => crate::core::image_processor::OutputFormat::Png,
                "webp" => crate::core::image_processor::OutputFormat::WebP,
                _ => crate::core::image_processor::OutputFormat::Jpeg,
            };

            let first_path = std::path::PathBuf::from(&final_assets[0]);
            let output_dir = first_path.parent().unwrap().join("workshop_output");
            let _ = std::fs::create_dir_all(&output_dir);

            let ui_inner = ui_weak.clone();
            let _ = runtime
                .workshop_service
                .batch_convert(
                    final_assets,
                    target_format,
                    output_dir.to_string_lossy().to_string(),
                    move |current, total| {
                        let ui = ui_inner.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui.upgrade() {
                                ui.set_status_text(
                                    format!("Processing: {}/{}", current, total).into(),
                                );
                            }
                        });
                    },
                )
                .await;

            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_status_text("Batch conversion complete".into());
                }
            });
        });
    }

    pub fn stitch(&self, dir_str: String, gap: i32) {
        let ui_weak = self.ui.clone();
        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            let (tx_data, rx_data) = tokio::sync::oneshot::channel::<(Vec<String>, Vec<String>)>();

            let ui_weak_for_scan = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let mut assets = Vec::new();
                let mut selected = Vec::new();
                if let Some(ui) = ui_weak_for_scan.upgrade() {
                    for item in ui.get_thumbnails().iter() {
                        assets.push(item.path.to_string());
                        if item.selected {
                            selected.push(item.path.to_string());
                        }
                    }
                }
                let _ = tx_data.send((assets, selected));
            });

            let (assets, selected_assets) = rx_data.await.unwrap_or_default();
            let final_assets = if !selected_assets.is_empty() {
                selected_assets
            } else {
                assets
            };

            if final_assets.is_empty() {
                return;
            }

            let config = crate::core::image_processor::StitchConfig {
                direction: if dir_str == "v" {
                    crate::core::image_processor::StitchDirection::Vertical
                } else {
                    crate::core::image_processor::StitchDirection::Horizontal
                },
                gap: gap as u32,
                align: true,
                background_color: [0, 0, 0],
            };

            let first_path = std::path::PathBuf::from(&final_assets[0]);
            let output_path = first_path.parent().unwrap().join("stitched_result.jpg");

            let ui_weak_inner = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak_inner.upgrade() {
                    ui.set_status_text("Stitching images...".into());
                }
            });

            let ui_weak_done = ui_weak.clone();
            match runtime
                .workshop_service
                .synthesize_images(
                    final_assets,
                    config,
                    output_path.to_string_lossy().to_string(),
                )
                .await
            {
                Ok(_) => {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak_done.upgrade() {
                            ui.set_status_text("Stitching complete".into());
                        }
                    });
                }
                Err(e) => tracing::error!("Stitching failed: {}", e),
            }
        });
    }
}
