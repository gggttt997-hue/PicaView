use crate::application::runtime::Runtime;
use crate::core::asset_browser::AssetInfo;
use crate::MainWindow;
use crate::Thumbnail;
use slint::Model;
use std::sync::Arc;

pub struct GalleryController {
    ui: slint::Weak<MainWindow>,
    runtime: Arc<Runtime>,
}

impl GalleryController {
    pub fn new(ui: slint::Weak<MainWindow>, runtime: Arc<Runtime>) -> Self {
        Self { ui, runtime }
    }

    pub fn update_active_item(&self, current_path: String) {
        let ui_weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                // 1. Update Gallery (Thumbnails)
                let mut thumbnails: Vec<Thumbnail> = ui.get_thumbnails().iter().collect();
                let mut active_idx = -1;
                for (i, item) in thumbnails.iter_mut().enumerate() {
                    item.is_active = item.path == current_path;
                    if item.is_active {
                        active_idx = i as i32;
                    }
                }
                ui.set_thumbnails(slint::ModelRc::from(thumbnails.as_slice()));
                ui.set_active_thumbnail_index(active_idx);

                // 2. Update Master Model
                let mut all: Vec<Thumbnail> = ui.get_all_thumbnails().iter().collect();
                for item in all.iter_mut() {
                    item.is_active = item.path == current_path;
                }
                ui.set_all_thumbnails(slint::ModelRc::from(all.as_slice()));
            }
        });
    }

    pub fn update_thumbnails_model(&self, mut assets: Vec<AssetInfo>) {
        // 1. Natural Sort by name
        assets.sort_by(|a, b| crate::utils::path_utils::compare_natural(&a.name, &b.name));

        let assets_for_ui = assets.clone();
        let ui_weak_for_set = self.ui.clone();

        // 2. Sync Gallery Model (Thumbnails placeholders) with State Preservation
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak_for_set.upgrade() {
                let current_path = ui.get_current_image_path().to_string();

                // Map existing images to preserve them during batch updates
                let mut existing_images = std::collections::HashMap::new();
                for item in ui.get_all_thumbnails().iter() {
                    if item.image.size().width > 0 {
                        existing_images.insert(item.path.to_string(), item.image.clone());
                    }
                }

                let mut active_idx = -1;
                let slint_items: Vec<Thumbnail> = assets_for_ui
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let is_active = a.path == current_path;
                        if is_active {
                            active_idx = i as i32;
                        }

                        let image = existing_images
                            .get(&a.path)
                            .cloned()
                            .unwrap_or_else(slint::Image::default);

                        Thumbnail {
                            path: a.path.clone().into(),
                            label: a.name.clone().into(),
                            is_active,
                            image,
                            visible: true,
                            selected: false,
                        }
                    })
                    .collect();

                let model = slint::ModelRc::from(slint_items.as_slice());
                ui.set_thumbnails(model.clone());
                ui.set_all_thumbnails(model);

                if active_idx != -1 {
                    ui.set_active_thumbnail_index(active_idx);
                }
            }
        });

        // 3. Lazy Load Thumbnails
        for asset in assets.into_iter() {
            let ui_weak_inner = self.ui.clone();
            let runtime_inner = self.runtime.clone();
            let path = asset.path.clone();

            tokio::spawn(async move {
                let cache_dir = crate::utils::path_utils::get_app_cache_dir();
                if !cache_dir.exists() {
                    let _ = std::fs::create_dir_all(&cache_dir);
                }

                if let Ok(info) = runtime_inner
                    .thumbnail_service
                    .get_thumbnail_data(path.clone(), 200, cache_dir)
                    .await
                {
                    if let Some(buffer) = info.pixel_buffer {
                        let ui_weak_update = ui_weak_inner.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak_update.upgrade() {
                                let slint_img = slint::Image::from_rgba8(buffer);
                                let mut items: Vec<Thumbnail> =
                                    ui.get_all_thumbnails().iter().collect();

                                if let Some(item) = items.iter_mut().find(|it| it.path == path) {
                                    if item.image.size().width == 0 {
                                        item.image = slint_img;
                                        let new_model = slint::ModelRc::from(items.as_slice());
                                        ui.set_thumbnails(new_model.clone());
                                        ui.set_all_thumbnails(new_model);
                                    }
                                }
                            }
                        });
                    }
                }
            });
        }
    }

    pub fn search_assets(&self, text: String) {
        let ui_weak = self.ui.clone();
        let text = text.to_lowercase();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let current_path = ui.get_current_image_path().to_string();

                let filtered: Vec<Thumbnail> = ui
                    .get_all_thumbnails()
                    .iter()
                    .filter(|item| {
                        item.label.to_lowercase().contains(&text)
                            || item.path.to_lowercase().contains(&text)
                    })
                    .collect();

                let final_active_idx = filtered
                    .iter()
                    .position(|t| t.path == current_path)
                    .map(|i| i as i32)
                    .unwrap_or(-1);

                ui.set_thumbnails(slint::ModelRc::from(filtered.as_slice()));
                ui.set_active_thumbnail_index(final_active_idx);
            }
        });
    }

    pub fn filter_assets(&self, format: String) {
        let ui_weak = self.ui.clone();
        let format = format.to_lowercase();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let current_path = ui.get_current_image_path().to_string();

                let filtered: Vec<Thumbnail> = ui
                    .get_all_thumbnails()
                    .iter()
                    .filter(|item| {
                        if format == "all" {
                            true
                        } else {
                            let path = item.path.to_lowercase();
                            path.ends_with(&format)
                                || path.ends_with(&format.replace("jpg", "jpeg"))
                        }
                    })
                    .collect();

                let final_active_idx = filtered
                    .iter()
                    .position(|t| t.path == current_path)
                    .map(|i| i as i32)
                    .unwrap_or(-1);

                ui.set_thumbnails(slint::ModelRc::from(filtered.as_slice()));
                ui.set_active_thumbnail_index(final_active_idx);
            }
        });
    }

    pub fn toggle_selection(&self, index: i32) {
        let ui_weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let mut current_visible: Vec<Thumbnail> = ui.get_thumbnails().iter().collect();
                if let Some(item) = current_visible.get_mut(index as usize) {
                    item.selected = !item.selected;
                    let path = item.path.clone();
                    let is_selected = item.selected;

                    ui.set_thumbnails(slint::ModelRc::from(current_visible.as_slice()));

                    let mut all: Vec<Thumbnail> = ui.get_all_thumbnails().iter().collect();
                    for a in all.iter_mut() {
                        if a.path == path {
                            a.selected = is_selected;
                            break;
                        }
                    }
                    ui.set_all_thumbnails(slint::ModelRc::from(all.as_slice()));
                }
            }
        });
    }

    pub fn select_all(&self, select: bool) {
        let ui_weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let mut visible: Vec<Thumbnail> = ui.get_thumbnails().iter().collect();
                let mut paths_to_update = std::collections::HashSet::new();

                for item in visible.iter_mut() {
                    item.selected = select;
                    paths_to_update.insert(item.path.clone());
                }
                ui.set_thumbnails(slint::ModelRc::from(visible.as_slice()));

                let mut all: Vec<Thumbnail> = ui.get_all_thumbnails().iter().collect();
                for a in all.iter_mut() {
                    if paths_to_update.contains(&a.path) {
                        a.selected = select;
                    }
                }
                ui.set_all_thumbnails(slint::ModelRc::from(all.as_slice()));
            }
        });
    }

    pub fn apply_exif_filter_to_thumbnails(
        &self,
        filtered_paths: std::collections::HashSet<String>,
    ) {
        let ui_weak = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let current_path = ui.get_current_image_path().to_string();

                let filtered: Vec<Thumbnail> = ui
                    .get_all_thumbnails()
                    .iter()
                    .filter(|item| filtered_paths.contains(&item.path.to_string()))
                    .collect();

                let final_active_idx = filtered
                    .iter()
                    .position(|t| t.path == current_path)
                    .map(|i| i as i32)
                    .unwrap_or(-1);

                ui.set_thumbnails(slint::ModelRc::from(filtered.as_slice()));
                ui.set_active_thumbnail_index(final_active_idx);
            }
        });
    }
}
