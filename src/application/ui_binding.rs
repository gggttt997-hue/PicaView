use crate::application::controller::AppController;
use crate::application::protocol::{AppCommand, EventEmitter};
use crate::MainWindow;
use slint::ComponentHandle;
use std::sync::Arc;

pub fn bind_ui(ui: &MainWindow, controller: &Arc<AppController>, emitter: &Arc<dyn EventEmitter>) {
    // 0. Initialize UI properties from persisted settings
    let init_settings = crate::utils::settings::get_settings();
    ui.set_scroll_mode_zoom(init_settings.scroll_mode == "zoom");
    let is_manga = init_settings.default_view_mode == "manga";
    let is_waterfall = init_settings.default_view_mode == "waterfall";
    ui.set_manga_mode_active(is_manga);
    ui.set_long_image_mode_active(is_waterfall);
    if is_waterfall {
        controller.rendering_runtime().set_long_image_mode(true);
    }

    // 1. Mouse move
    {
        let c = controller.clone();
        ui.on_mouse_move(move |mx, my| {
            c.handle_mouse_move(mx, my);
        });
    }

    // 2. Navigation
    {
        let c = controller.clone();
        ui.on_request_next(move || {
            let c = c.clone();
            tokio::spawn(async move {
                c.handle_next_image().await;
            });
        });
    }

    {
        let c = controller.clone();
        ui.on_request_prev(move || {
            let c = c.clone();
            tokio::spawn(async move {
                c.handle_prev_image().await;
            });
        });
    }

    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_request_open(move |path| {
            let c = c.clone();
            let em = em.clone();
            let path_str = path.to_string();
            tokio::spawn(async move {
                c.handle_open_target(path_str, em).await;
            });
        });
    }

    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_submit_password(move |path, password| {
            let c = c.clone();
            let em = em.clone();
            let path_str = path.to_string();
            let pass_str = password.to_string();

            crate::core::archive_vfs::archive_passwords()
                .write()
                .unwrap()
                .insert(path_str.clone(), pass_str);

            tokio::spawn(async move {
                c.handle_open_target(path_str, em).await;
            });
        });
    }

    // 3. Transformation & Interaction
    {
        let c = controller.clone();
        ui.on_request_rotate(move |delta| {
            c.handle_rotate(delta as f32);
        });
    }

    {
        let c = controller.clone();
        ui.on_request_flip(move |axis| {
            c.handle_flip(axis.to_string());
        });
    }

    {
        let c = controller.clone();
        ui.on_request_reset_transform(move || {
            c.handle_reset_transform();
        });
    }

    {
        let c = controller.clone();
        ui.on_request_zoom(move |delta, mx, my, has_control, has_shift| {
            c.handle_zoom(delta, mx, my, has_control, has_shift);
        });
    }

    {
        let c = controller.clone();
        ui.on_request_toggle_scroll_mode(move || {
            c.handle_toggle_scroll_mode();
        });
    }

    {
        let c = controller.clone();
        ui.on_request_toggle_long_image_mode(move || {
            c.handle_toggle_long_image_mode();
        });
    }

    {
        let c = controller.clone();
        ui.on_request_scroll_to_percent(move |percent| {
            c.handle_scroll_to_percent(percent);
        });
    }

    {
        let c = controller.clone();
        ui.on_request_adjust_width_percent(move |percent| {
            c.handle_adjust_width_percent(percent);
        });
    }

    {
        let c = controller.clone();
        ui.on_request_scroll_navigate(move |delta| {
            c.handle_scroll_navigate(delta);
        });
    }

    // 4. Asset Gallery
    {
        let c = controller.clone();
        ui.on_request_asset_search(move |text| {
            c.handle_asset_search(text.to_string());
        });
    }

    {
        let c = controller.clone();
        ui.on_request_asset_filter(move |format| {
            c.handle_asset_filter(format.to_string());
        });
    }

    {
        let c = controller.clone();
        ui.on_request_exif_filter(move |min_f, max_f, min_a, max_a, min_i, max_i, cam| {
            let min_focal = min_f.as_str().parse::<f32>().ok();
            let max_focal = max_f.as_str().parse::<f32>().ok();
            let min_ap = min_a.as_str().parse::<f32>().ok();
            let max_ap = max_a.as_str().parse::<f32>().ok();
            let min_iso = min_i.as_str().parse::<u32>().ok();
            let max_iso = max_i.as_str().parse::<u32>().ok();
            let camera = if !cam.is_empty() {
                Some(cam.to_string())
            } else {
                None
            };

            c.handle_exif_filter(
                min_focal, max_focal, min_ap, max_ap, min_iso, max_iso, camera,
            );
        });
    }

    {
        let c = controller.clone();
        ui.on_request_toggle_compare_mode(move || {
            c.handle_toggle_compare_mode();
        });
    }

    {
        let c = controller.clone();
        ui.on_request_toggle_compare_orientation(move || {
            c.handle_toggle_compare_orientation();
        });
    }

    {
        let c = controller.clone();
        ui.on_request_compare_with_thumbnail(move |index| {
            c.handle_compare_with_thumbnail(index);
        });
    }

    {
        let c = controller.clone();
        ui.on_request_asset_toggle_selection(move |index| {
            c.handle_asset_toggle_selection(index);
        });
    }

    {
        let c = controller.clone();
        ui.on_request_asset_select_all(move |select| {
            c.handle_asset_select_all(select);
        });
    }

    {
        let c = controller.clone();
        ui.on_thumbnail_clicked(move |index| {
            c.handle_thumbnail_clicked(index);
        });
    }

    {
        let c = controller.clone();
        ui.on_long_image_scrolled_to(move |index| {
            c.handle_long_image_scrolled_to(index);
        });
    }

    {
        let c = controller.clone();
        ui.on_request_pan(move |dx, dy| {
            c.handle_pan(dx, dy);
        });

        let c_auto_scroll = controller.clone();
        ui.on_request_toggle_auto_scroll(move || {
            c_auto_scroll.handle_toggle_auto_scroll();
        });

        let c_adjust_speed = controller.clone();
        ui.on_request_adjust_auto_scroll_speed(move |delta| {
            c_adjust_speed.handle_adjust_auto_scroll_speed(delta);
        });
    }

    {
        let c = controller.clone();
        ui.on_request_inertia(move |vx, vy| {
            c.handle_inertia(vx, vy);
        });
    }

    {
        let c = controller.clone();
        ui.on_request_drag(move |dx, dy| {
            c.handle_drag(dx, dy);
        });
        let c_drag = controller.clone();
        ui.on_request_window_drag(move |has_control| {
            c_drag.handle_window_drag(has_control);
        });
    }

    // 5. Window States (Close, Minimize, Maximize)
    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_request_close(move || {
            let c = c.clone();
            let em = em.clone();
            tokio::spawn(async move {
                c.handle_dispatch_event(AppCommand::CloseWindow, em).await;
            });
        });
    }

    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_request_minimize(move || {
            let c = c.clone();
            let em = em.clone();
            tokio::spawn(async move {
                c.handle_dispatch_event(AppCommand::MinimizeWindow, em)
                    .await;
            });
        });
    }

    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_request_toggle_maximize(move || {
            let c = c.clone();
            let em = em.clone();
            tokio::spawn(async move {
                c.handle_dispatch_event(AppCommand::ToggleMaximize, em)
                    .await;
            });
        });
    }

    // 6. Slideshow
    {
        let c = controller.clone();
        ui.on_request_toggle_slideshow(move || {
            c.handle_toggle_slideshow();
        });
    }

    {
        let ui_weak: slint::Weak<MainWindow> = ui.as_weak();
        ui.on_slideshow_tick(move || {
            if let Some(ui_instance) = ui_weak.upgrade() {
                ui_instance.invoke_request_next();
            }
        });
    }

    // 7. Clipboard & Explorer Declarative High-Order Bindings
    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_request_reveal_in_explorer(move || {
            c.handle_current_image_command(
                |p| AppCommand::RevealInExplorer { path: p },
                em.clone(),
            );
        });
    }

    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_request_copy_to_clipboard(move || {
            c.handle_current_image_command(
                |p| AppCommand::CopyImageToClipboard { path: p },
                em.clone(),
            );
        });
    }

    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_request_copy_path_to_clipboard(move || {
            c.handle_current_image_command(
                |p| AppCommand::WriteToClipboard { text: p },
                em.clone(),
            );
        });
    }

    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_request_copy_file_to_clipboard(move || {
            c.handle_current_image_command(
                |p| AppCommand::CopyFileToClipboard { path: p },
                em.clone(),
            );
        });
    }

    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_request_save_as(move || {
            c.handle_current_image_command(|p| AppCommand::SaveImageAs { path: p }, em.clone());
        });
    }

    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_request_open_in_external_editor(move || {
            c.handle_current_image_command(
                |p| AppCommand::OpenInExternalEditor { path: p },
                em.clone(),
            );
        });
    }

    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_request_set_wallpaper(move || {
            c.handle_current_image_command(
                |p| AppCommand::SetWallpaper {
                    path: p,
                    mode: String::new(),
                },
                em.clone(),
            );
        });
    }

    // 8. Delete & Drag-Drop & L10n & Workshop
    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_request_delete(move || {
            let c = c.clone();
            let em = em.clone();
            tokio::spawn(async move {
                c.handle_delete_current(em).await;
            });
        });
    }

    {
        let c = controller.clone();
        ui.on_files_dropped(move |data| {
            c.handle_drag_drop(data.to_string());
        });
    }

    {
        let c = controller.clone();
        ui.on_request_switch_language(move || {
            c.handle_switch_language();
        });
    }

    {
        let c = controller.clone();
        let em = emitter.clone();
        ui.on_request_update_shell(move || {
            let c = c.clone();
            let em = em.clone();
            tokio::spawn(async move {
                c.handle_update_shell(em).await;
            });
        });
    }

    {
        let c = controller.clone();
        ui.on_request_workshop_batch_convert(move |format| {
            c.handle_workshop_batch_convert(format.to_string());
        });
    }

    {
        let c = controller.clone();
        ui.on_request_workshop_stitch(move |dir, gap, _bg| {
            c.handle_workshop_stitch(dir.to_string(), gap);
        });
    }

    {
        let c = controller.clone();
        ui.on_request_toggle_manga_mode(move || {
            c.handle_toggle_manga_mode();
        });
    }

    {
        let c = controller.clone();
        ui.on_request_shortcut(move |key_text, control_pressed| {
            let key_str = key_text.to_string();
            let c = c.clone();

            if control_pressed {
                match key_str.as_str() {
                    "0" => {
                        let c_clone = c.clone();
                        tokio::spawn(async move {
                            c_clone.handle_go_to_original_dir().await;
                        });
                        true
                    }
                    "1" | "2" | "3" | "4" | "5" => {
                        let c_clone = c.clone();
                        if let Ok(rating) = key_str.parse::<i32>() {
                            tokio::spawn(async move {
                                c_clone.handle_switch_to_rating_dir(rating).await;
                            });
                        }
                        true
                    }
                    _ => false,
                }
            } else {
                match key_str.to_lowercase().as_str() {
                    "1" | "2" | "3" | "4" | "5" => {
                        let c_clone = c.clone();
                        if let Ok(rating) = key_str.parse::<i32>() {
                            tokio::spawn(async move {
                                c_clone.handle_rate_image(rating).await;
                            });
                        }
                        true
                    }
                    "r" => {
                        c.handle_set_channel_mode(1);
                        true
                    }
                    "g" => {
                        c.handle_set_channel_mode(2);
                        true
                    }
                    "b" => {
                        c.handle_set_channel_mode(3);
                        true
                    }
                    "a" => {
                        c.handle_set_channel_mode(4);
                        true
                    }
                    "c" => {
                        let current_mode = c.channel_mode.load(std::sync::atomic::Ordering::SeqCst);
                        if current_mode != 0 {
                            c.handle_set_channel_mode(0); // 恢复正常 RGB 模式
                        } else {
                            c.handle_toggle_compare_mode();
                        }
                        true
                    }
                    _ => false,
                }
            }
        });
    }

    // 11. Request Toggle Settings Callback
    {
        let ui_weak = ui.as_weak();
        ui.on_request_toggle_settings(move || {
            if let Some(ui_inst) = ui_weak.upgrade() {
                let current = ui_inst.get_settings_visible();
                ui_inst.set_settings_visible(!current);
            }
        });
    }

    // 10. Save Settings Callback
    {
        let controller_clone = controller.clone();
        ui.on_request_toggle_viewport_lock(move || {
            controller_clone.handle_toggle_viewport_lock();
        });
    }

    {
        let controller_clone = controller.clone();
        ui.on_request_toggle_exif_hud(move || {
            controller_clone.handle_toggle_exif_hud();
        });
    }

    // 12. Save Settings Callback
    {
        let controller_clone = controller.clone();
        let ui_weak = ui.as_weak();
        ui.on_save_settings(
            move |tile_str, cache_str, hud_str, fallback_str, zune_str, mem_str, multi_inst| {
                let current = crate::utils::settings::get_settings();
                let new_tile = tile_str
                    .as_str()
                    .parse::<u32>()
                    .unwrap_or(current.tile_size);
                let new_cache = cache_str
                    .as_str()
                    .parse::<usize>()
                    .unwrap_or(current.max_cache_size_mb);
                let new_hud = hud_str
                    .as_str()
                    .parse::<u64>()
                    .unwrap_or(current.hud_dismiss_delay_ms);
                let new_fallback = fallback_str
                    .as_str()
                    .parse::<u32>()
                    .unwrap_or(current.fallback_size);
                let new_zune = zune_str
                    .as_str()
                    .parse::<u32>()
                    .unwrap_or(current.max_zune_dimension);
                let new_mem = mem_str
                    .as_str()
                    .parse::<f32>()
                    .unwrap_or(current.mem_limit_fraction);

                let new_settings = crate::utils::settings::AppSettings {
                    tile_size: new_tile,
                    max_cache_size_mb: new_cache,
                    hud_dismiss_delay_ms: new_hud,
                    fallback_size: new_fallback,
                    max_zune_dimension: new_zune,
                    mem_limit_fraction: new_mem,
                    allow_multi_instance: multi_inst,
                    scroll_mode: current.scroll_mode,
                    default_view_mode: current.default_view_mode,
                    language: current.language,
                    remember_window_geometry: current.remember_window_geometry,
                    window_x: current.window_x,
                    window_y: current.window_y,
                    window_width: current.window_width,
                    window_height: current.window_height,
                    window_maximized: current.window_maximized,
                };

                if let Err(e) = crate::utils::settings::update_settings(new_settings.clone()) {
                    tracing::error!("Failed to save settings: {:?}", e);
                } else {
                    tracing::info!("Settings updated successfully: {:?}", new_settings);

                    // Hot reload Cache size
                    let weight = new_settings.max_cache_size_mb * 1024 * 1024;
                    if let Ok(mut cache) = controller_clone.runtime().state.thumbnail_cache.lock() {
                        cache.set_max_weight(weight);
                    }

                    // Sync properties back to Slint UI for visual update
                    if let Some(ui_inst) = ui_weak.upgrade() {
                        ui_inst.set_settings_tile_size(new_settings.tile_size.to_string().into());
                        ui_inst.set_settings_max_cache_mb(
                            new_settings.max_cache_size_mb.to_string().into(),
                        );
                        ui_inst.set_settings_hud_delay_ms(
                            new_settings.hud_dismiss_delay_ms.to_string().into(),
                        );
                        ui_inst.set_settings_fallback_size(
                            new_settings.fallback_size.to_string().into(),
                        );
                        ui_inst.set_settings_zune_limit(
                            new_settings.max_zune_dimension.to_string().into(),
                        );
                        ui_inst.set_settings_mem_fraction(
                            new_settings.mem_limit_fraction.to_string().into(),
                        );
                        ui_inst.set_settings_multi_instance(new_settings.allow_multi_instance);

                        if new_tile != current.tile_size {
                            ui_inst.set_status_text("设置已保存，修改瓦片大小需重启生效".into());
                        } else {
                            ui_inst.set_status_text("设置已成功保存并应用！".into());
                        }
                        ui_inst.set_manga_hud_visible(true);

                        let ui_clone = ui_weak.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(tokio::time::Duration::from_millis(new_hud)).await;
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui_active) = ui_clone.upgrade() {
                                    ui_active.set_manga_hud_visible(false);
                                }
                            });
                        });
                    }
                }
            },
        );
    }

    // Sync initial settings state back to UI properties
    let settings = crate::utils::settings::get_settings();
    ui.set_settings_tile_size(settings.tile_size.to_string().into());
    ui.set_settings_max_cache_mb(settings.max_cache_size_mb.to_string().into());
    ui.set_settings_hud_delay_ms(settings.hud_dismiss_delay_ms.to_string().into());
    ui.set_settings_fallback_size(settings.fallback_size.to_string().into());
    ui.set_settings_zune_limit(settings.max_zune_dimension.to_string().into());
    ui.set_settings_mem_fraction(settings.mem_limit_fraction.to_string().into());
    ui.set_settings_multi_instance(settings.allow_multi_instance);
    // Sync initial manga mode state
    let initial_manga = *controller
        .runtime()
        .state
        .manga_mode_enabled
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    ui.set_manga_mode_active(initial_manga);
}
