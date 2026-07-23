#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::redundant_guards)]
#![allow(clippy::manual_strip)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::unnecessary_to_owned)]
#![allow(clippy::collapsible_if)]
#![allow(dead_code)]
pub mod adapters;
pub mod application;
pub mod core;
pub mod ep;
pub mod error;
pub mod models;
pub mod state;
pub mod utils;

use state::AppState;
use std::sync::Arc;
use tracing::info;

/// Core initialization for PicaView (Shared by all entry points)
pub fn init_core() -> Arc<AppState> {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let console_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stdout);

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,picaview_lib=info"));

    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer)
        .try_init();

    info!("PicaView Core Initialized (Fast Boot)");

    Arc::new(AppState::new())
}

slint::include_modules!();

fn setup_early_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
        let log_dir = crate::utils::path_utils::get_app_log_dir();
        let _ = std::fs::create_dir_all(&log_dir);
        let panic_log_path = log_dir.join("panic.log");

        let mut msg = String::new();
        msg.push_str("PicaView Critical Panic Event\n");

        if let Some(location) = panic_info.location() {
            msg.push_str(&format!(
                "Location: {}:{}:{}\n",
                location.file(),
                location.line(),
                location.column()
            ));
        }

        if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            msg.push_str(&format!("Details: {}\n", s));
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            msg.push_str(&format!("Details: {}\n", s));
        } else {
            msg.push_str("Details: Unknown panic payload\n");
        }

        let _ = std::fs::write(&panic_log_path, &msg);
        tracing::error!("{}", msg);
    }));
}

/// Dispatcher run: Default to native entry point
pub fn run() {
    setup_early_panic_hook();
    let start = std::time::Instant::now();

    if crate::utils::single_instance::SingleInstance::check_and_forward() {
        info!("Handled by existing instance. Exiting.");
        return;
    }
    info!(
        "[BOOT] SingleInstance check: {}ms",
        start.elapsed().as_millis()
    );

    let t_init = std::time::Instant::now();
    let state = init_core();
    info!(
        "[BOOT] init_core (State/Logs): {}ms",
        t_init.elapsed().as_millis()
    );

    run_native(state);
}

/// Native entry point using Slint
pub fn run_native(state: Arc<AppState>) {
    let t_native = std::time::Instant::now();

    // Force Slint to use WGPU renderer
    std::env::set_var("SLINT_BACKEND", "winit-femtovg-wgpu");
    std::env::set_var("SLINT_ENABLE_EXPERIMENTAL_FEATURES", "1");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();

    use crate::application::rendering_runtime::RenderingRuntime;
    use crate::application::runtime::Runtime;
    use slint::ComponentHandle;

    // --- Phase 2.1: Priority UI Creation ---
    let t_ui = std::time::Instant::now();

    // Graphics Fallback Mechanism
    let ui = match MainWindow::new() {
        Ok(window) => window,
        Err(e) => {
            tracing::warn!("MainWindow creation failed with WGPU backend: {:?}. Retrying with default compatible backend...", e);
            std::env::remove_var("SLINT_BACKEND");
            MainWindow::new()
                .expect("Failed to create MainWindow with default compatible graphics backend")
        }
    };
    tracing::info!("[BOOT] MainWindow::new(): {}ms", t_ui.elapsed().as_millis());

    let t_show = std::time::Instant::now();
    ui.show().expect("Failed to show Slint window");
    tracing::info!("[BOOT] ui.show(): {}ms", t_show.elapsed().as_millis());

    // DragAcceptFiles hack restored and upgraded for borderless window IDropTarget compatibility on Windows
    #[cfg(target_os = "windows")]
    {
        use i_slint_backend_winit::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use i_slint_backend_winit::WinitWindowAccessor;
        use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
        use windows::Win32::System::Ole::RevokeDragDrop;
        use windows::Win32::UI::Shell::{
            DefSubclassProc, DragAcceptFiles, DragFinish, DragQueryFileW, SetWindowSubclass, HDROP,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            ChangeWindowMessageFilterEx, MSGFLT_ALLOW, WM_COPYDATA, WM_DROPFILES,
        };

        unsafe extern "system" fn drop_files_subclass_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
            _uids_subclass: usize,
            dwref_data: usize,
        ) -> LRESULT {
            if msg == WM_DROPFILES {
                let hdrop = HDROP(wparam.0 as _);
                let count = DragQueryFileW(hdrop, 0xFFFFFFFF, None);
                let mut paths = String::new();

                for i in 0..count {
                    let len = DragQueryFileW(hdrop, i, None) as usize;
                    let mut buf: Vec<u16> = vec![0; len + 1];
                    DragQueryFileW(hdrop, i, Some(&mut buf));
                    if let Ok(path) = String::from_utf16(&buf[..len]) {
                        paths.push_str("file://");
                        paths.push_str(&path);
                        paths.push('\n');
                    }
                }

                DragFinish(hdrop);

                let ptr = dwref_data as *const slint::Weak<crate::MainWindow>;
                if !ptr.is_null() {
                    let ui_weak = &*ptr;
                    let paths_str = paths.clone();
                    let ui_weak_clone = ui_weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak_clone.upgrade() {
                            ui.invoke_files_dropped(paths_str.into());
                        }
                    });
                }
                return LRESULT(0);
            }
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }

        // 我们必须在事件循环激活 100ms 后再尝试挂载，否则 with_winit_window 会默默返回 None
        let ui_weak_setup = ui.as_weak();
        let setup_timer = slint::Timer::default();
        setup_timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_millis(100),
            move || {
                if let Some(ui) = ui_weak_setup.upgrade() {
                    let success = ui.window().with_winit_window(|winit_window: &i_slint_backend_winit::winit::window::Window| {
                        if let Ok(handle) = winit_window.window_handle() {
                            if let RawWindowHandle::Win32(win32_handle) = handle.as_raw() {
                                let hwnd = HWND(win32_handle.hwnd.get() as _);
                                let weak_ptr = Box::into_raw(Box::new(ui_weak_setup.clone()));
                                unsafe {
                                    // 1. 强制注销 winit 占用的 COM 拖拽以触发 WM_DROPFILES 回退
                                    let _ = RevokeDragDrop(hwnd);
                                    // 2. 赋予传统拖拽接收权限
                                    DragAcceptFiles(hwnd, true);
                                    // 3. 跨越管理员 UIPI 限制
                                    let _ = ChangeWindowMessageFilterEx(hwnd, WM_DROPFILES, MSGFLT_ALLOW, None);
                                    let _ = ChangeWindowMessageFilterEx(hwnd, WM_COPYDATA, MSGFLT_ALLOW, None);
                                    let _ = ChangeWindowMessageFilterEx(hwnd, 0x0049, MSGFLT_ALLOW, None); // WM_COPYGLOBALDATA
                                    // 4. 子类化窗口挂载
                                    let _ = SetWindowSubclass(
                                        hwnd,
                                        Some(drop_files_subclass_proc),
                                        1, // Subclass ID
                                        weak_ptr as usize
                                    );
                                    tracing::info!("Successfully initialized DragAcceptFiles and Subclassed window for borderless window drag support!");
                                }
                            }
                        }
                    });
                    if success.is_none() {
                        tracing::warn!("Failed to obtain winit window handle inside startup timer. Drag and drop may not work.");
                    }
                }
            }
        );
        // 泄漏定时器使其在堆上存活直至触发
        Box::leak(Box::new(setup_timer));
    }

    let ui_weak = ui.as_weak();

    let settings = crate::utils::settings::get_settings();
    let is_single_instance = !settings.allow_multi_instance;

    let ui_weak_close = ui_weak.clone();
    ui.window().on_close_requested(move || {
        if is_single_instance {
            if let Some(ui) = ui_weak_close.upgrade() {
                tracing::info!(
                    "Single instance close requested: hiding window instead of exiting."
                );
                let _ = ui.window().hide();
            }
            slint::CloseRequestResponse::KeepWindowShown
        } else {
            slint::CloseRequestResponse::HideWindow
        }
    });

    // Initialize application runtime
    let t_rt = std::time::Instant::now();
    let runtime = Arc::new(Runtime::new(state));
    tracing::info!("[BOOT] Runtime::new(): {}ms", t_rt.elapsed().as_millis());

    // --- Deferred Initialization Task ---
    {
        let state = runtime.state.clone();
        tokio::spawn(async move {
            if let Ok(mut history) = state.history_manager.lock() {
                let _ = history.load();
                tracing::info!("History loaded asynchronously");
            }

            let log_dir = crate::utils::path_utils::get_app_log_dir();
            if !log_dir.exists() {
                let _ = std::fs::create_dir_all(&log_dir);
            }
        });
    }

    info!("Starting PicaView Services...");

    // Start single instance listener
    let t_listener = std::time::Instant::now();
    {
        if is_single_instance {
            let ui_handle = ui_weak.clone();
            crate::utils::single_instance::SingleInstance::start_listener(move |args| {
                let real_paths: Vec<String> = args
                    .into_iter()
                    .skip(1)
                    .filter(|arg| arg != "--single-instance" && arg != "-s")
                    .collect();

                if !real_paths.is_empty() {
                    let path = real_paths[0].clone();
                    let ui_handle = ui_handle.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_handle.upgrade() {
                            tracing::info!("Single instance: opening {}", path);
                            ui.invoke_request_open(path.into());
                            let _ = ui.window().show();
                            ui.window().set_minimized(false);
                        }
                    });
                } else {
                    let ui_handle = ui_handle.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_handle.upgrade() {
                            tracing::info!("Single instance: focusing window");
                            let _ = ui.window().show();
                            ui.window().set_minimized(false);
                        }
                    });
                }
            });
        }
    }
    tracing::info!(
        "[BOOT] start_listener: {}ms",
        t_listener.elapsed().as_millis()
    );

    // Initialize Rendering Runtime (EP Engine)
    let t_rr = std::time::Instant::now();
    let rendering_runtime = Arc::new({
        let ui_handle = ui_weak.clone();
        let ui_weak_for_rr = ui_weak.clone();
        RenderingRuntime::new(
            ui.window(),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    (
                        ui.get_viewport_width(),
                        ui.get_viewport_height(),
                        ui.window().scale_factor(),
                    )
                } else {
                    (0.0, 0.0, 1.0)
                }
            },
            ui_weak_for_rr,
        )
    });
    tracing::info!(
        "[BOOT] RenderingRuntime::new (WGPU): {}ms",
        t_rr.elapsed().as_millis()
    );

    // --- Register Native Adapters ---
    let window_service = Arc::new(crate::adapters::native_adapter::SlintWindowService {
        ui: ui_weak.clone(),
    });
    runtime.set_window_service(window_service);

    let event_emitter = Arc::new(crate::adapters::native_adapter::SlintEventEmitter {
        ui: ui_weak.clone(),
    });
    let emitter_clone: Arc<dyn crate::application::protocol::EventEmitter> = event_emitter.clone();

    // Create App Controller
    let controller = crate::application::controller::AppController::new(
        ui_weak.clone(),
        runtime.clone(),
        rendering_runtime.clone(),
    );

    // --- Initial L10n setup text ---
    controller.handle_apply_initial_language();

    // --- Bind UI Callbacks to Controller Routing ---
    crate::application::ui_binding::bind_ui(&ui, &controller, &emitter_clone);

    // Trigger startup args check to open initial image
    controller.handle_startup_args();

    tracing::info!(
        "[BOOT] Total run_native till ui.run(): {}ms",
        t_native.elapsed().as_millis()
    );

    ui.run().expect("Failed to run Slint UI");
}
