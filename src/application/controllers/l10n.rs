use crate::application::runtime::Runtime;
use crate::L10n;
use crate::MainWindow;
use slint::ComponentHandle;
use std::sync::Arc;

pub struct L10nManager {
    ui: slint::Weak<MainWindow>,
    runtime: Arc<Runtime>,
}

impl L10nManager {
    pub fn new(ui: slint::Weak<MainWindow>, runtime: Arc<Runtime>) -> Self {
        Self { ui, runtime }
    }

    pub fn apply_language(&self, lang: &str) {
        let ui_weak = self.ui.clone();
        let i18n = self.runtime.state.i18n.clone();
        i18n.set_language(lang);

        let i18n_clone = i18n.clone();
        let ui_weak_l10n = ui_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak_l10n.upgrade() {
                let l10n = ui.global::<L10n>();
                l10n.set_open_file(i18n_clone.t("open-file").into());
                l10n.set_open_folder(i18n_clone.t("open-folder").into());
                l10n.set_reset_transform(i18n_clone.t("reset-transform").into());
                l10n.set_rotate_left(i18n_clone.t("rotate-left").into());
                l10n.set_rotate_right(i18n_clone.t("rotate-right").into());
                l10n.set_flip_h(i18n_clone.t("flip-h").into());
                l10n.set_flip_v(i18n_clone.t("flip-v").into());
                l10n.set_prev_image(i18n_clone.t("prev-image").into());
                l10n.set_next_image(i18n_clone.t("next-image").into());
                l10n.set_asset_browser(i18n_clone.t("asset-browser").into());
                l10n.set_gallery_toggle(i18n_clone.t("gallery-toggle").into());
                l10n.set_slideshow_play(i18n_clone.t("slideshow-play").into());
                l10n.set_slideshow_pause(i18n_clone.t("slideshow-pause").into());
                l10n.set_pin_toolbar(i18n_clone.t("pin-toolbar").into());
                l10n.set_status_ready(i18n_clone.t("ready").into());
                l10n.set_scroll_zoom_mode(i18n_clone.t("scroll-zoom-mode").into());
                l10n.set_scroll_navigate_mode(i18n_clone.t("scroll-navigate-mode").into());
                l10n.set_long_image_mode_on(i18n_clone.t("long-image-mode-on").into());
                l10n.set_long_image_mode_off(i18n_clone.t("long-image-mode-off").into());
                l10n.set_drag_hint(i18n_clone.t("drag-hint").into());
                l10n.set_supported_formats(i18n_clone.t("supported-formats").into());
                l10n.set_reveal_in_explorer(i18n_clone.t("reveal-in-explorer").into());
                l10n.set_copy_image(i18n_clone.t("copy-image").into());
                l10n.set_copy_file(i18n_clone.t("copy-file").into());
                l10n.set_copy_path(i18n_clone.t("copy-path").into());
                l10n.set_save_as(i18n_clone.t("save-as").into());
                l10n.set_open_external(i18n_clone.t("open-external").into());
                l10n.set_set_wallpaper(i18n_clone.t("set-wallpaper").into());
                l10n.set_delete_file(i18n_clone.t("delete-file").into());
                l10n.set_update_shell(i18n_clone.t("update-shell").into());
                l10n.set_switch_lang(i18n_clone.t("switch-lang").into());

                // Settings UI Translations
                l10n.set_settings_title(i18n_clone.t("settings-title").into());
                l10n.set_tile_size_label(i18n_clone.t("settings-tile-size").into());
                l10n.set_max_cache_label(i18n_clone.t("settings-max-cache").into());
                l10n.set_hud_delay_label(i18n_clone.t("settings-hud-delay").into());
                l10n.set_fallback_label(i18n_clone.t("settings-fallback-size").into());
                l10n.set_zune_limit_label(i18n_clone.t("settings-zune-limit").into());
                l10n.set_mem_fraction_label(i18n_clone.t("settings-mem-fraction").into());
                l10n.set_multi_instance_label(i18n_clone.t("multi-instance-label").into());
                l10n.set_restart_notice(i18n_clone.t("settings-restart-notice").into());
                l10n.set_save_button(i18n_clone.t("settings-save").into());
                l10n.set_cancel_button(i18n_clone.t("settings-cancel").into());

                // Drawers & Modals
                l10n.set_gallery_drawer_title(i18n_clone.t("gallery-drawer-title").into());
                l10n.set_multi_instance_warning_title(
                    i18n_clone.t("multi-instance-warning-title").into(),
                );
                l10n.set_multi_instance_warning_msg(
                    i18n_clone.t("multi-instance-warning-msg").into(),
                );
                l10n.set_multi_instance_warning_confirm(
                    i18n_clone.t("multi-instance-warning-confirm").into(),
                );

                l10n.set_exif_filter_title(i18n_clone.t("exif-filter-title").into());
                l10n.set_exif_camera_model(i18n_clone.t("exif-camera-model").into());
                l10n.set_exif_camera_placeholder(i18n_clone.t("exif-camera-placeholder").into());
                l10n.set_exif_focal_length(i18n_clone.t("exif-focal-length").into());
                l10n.set_exif_aperture(i18n_clone.t("exif-aperture").into());
                l10n.set_exif_iso(i18n_clone.t("exif-iso").into());
                l10n.set_exif_min_placeholder(i18n_clone.t("exif-min-placeholder").into());
                l10n.set_exif_max_placeholder(i18n_clone.t("exif-max-placeholder").into());
                l10n.set_exif_apply(i18n_clone.t("exif-apply").into());
                l10n.set_exif_reset(i18n_clone.t("exif-reset").into());

                l10n.set_password_required_title(i18n_clone.t("password-required-title").into());
                l10n.set_password_archive_prefix(i18n_clone.t("password-archive-prefix").into());
                l10n.set_password_input_placeholder(
                    i18n_clone.t("password-input-placeholder").into(),
                );
                l10n.set_confirm(i18n_clone.t("confirm").into());
                l10n.set_cancel(i18n_clone.t("cancel").into());

                l10n.set_workshop_toggle(i18n_clone.t("workshop-toggle").into());
                l10n.set_workshop_batch_convert(i18n_clone.t("workshop-batch-convert").into());
                l10n.set_to_jpg(i18n_clone.t("to-jpg").into());
                l10n.set_to_png(i18n_clone.t("to-png").into());
                l10n.set_to_webp(i18n_clone.t("to-webp").into());
                l10n.set_workshop_image_stitch(i18n_clone.t("workshop-image-stitch").into());
                l10n.set_vertical_stitch(i18n_clone.t("vertical-stitch").into());
                l10n.set_horizontal_stitch(i18n_clone.t("horizontal-stitch").into());

                l10n.set_manga_mode_on(i18n_clone.t("manga-mode-on").into());
                l10n.set_manga_mode_off(i18n_clone.t("manga-mode-off").into());
                l10n.set_exif_filter_on(i18n_clone.t("exif-filter-on").into());
                l10n.set_exif_filter_off(i18n_clone.t("exif-filter-off").into());
                l10n.set_compare_mode_on(i18n_clone.t("compare-mode-on").into());
                l10n.set_compare_mode_off(i18n_clone.t("compare-mode-off").into());
                l10n.set_split_horizontal(i18n_clone.t("split-horizontal").into());
                l10n.set_split_vertical(i18n_clone.t("split-vertical").into());
                l10n.set_compare_swipe(i18n_clone.t("compare-swipe").into());

                ui.set_status_text(i18n_clone.t("ready").into());
            }
        });
    }

    pub fn switch_language(&self) {
        let current = self.runtime.state.i18n.get_current_lang();
        let next = if current == "zh-CN" { "en-US" } else { "zh-CN" };

        let mut settings = crate::utils::settings::get_settings();
        settings.language = next.to_string();
        let _ = crate::utils::settings::update_settings(settings);

        self.apply_language(next);
    }

    pub fn apply_initial_language(&self) {
        let settings = crate::utils::settings::get_settings();
        self.apply_language(&settings.language);
    }
}
