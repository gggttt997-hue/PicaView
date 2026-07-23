slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    // --- Initialize logging system (ULP v4.5) ---
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let console_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stdout);

    // Setup file logging for EP Engine
    let file_appender = tracing_appender::rolling::never(".logs", "app_ep.jsonl");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .json()
        .with_target(true)
        .with_thread_ids(true);

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,picaview_ep=debug,picaview_lib=info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer)
        .with(file_layer)
        .init();

    // Store guard to keep logging alive
    Box::leak(Box::new(_guard));
    // --- Logging system initialization complete ---

    tracing::info!(
        location = "main_ep",
        "PicaView EP (Native Rust + Slint) starting..."
    );

    let ui = MainWindow::new()?;

    // Phase 2 will involve initializing WGPU here
    // and passing the window handle to the renderer.

    use picaview_lib::ep::physics::ViewportPhysics;
    use picaview_lib::ep::renderer::WgpuRenderer;
    use picaview_lib::ep::EngineCommand;
    use slint::{GraphicsAPI, RenderingState, Timer, TimerMode};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel::<EngineCommand>();
    let rx = Rc::new(RefCell::new(Some(rx)));
    let tx = Rc::new(tx);

    let renderer: Rc<RefCell<Option<WgpuRenderer>>> = Rc::new(RefCell::new(None));
    let physics = Rc::new(RefCell::new(ViewportPhysics::new()));

    // --- Physics Tick Timer (Targeting 144Hz/7ms) ---
    let physics_timer = Timer::default();
    {
        let physics = physics.clone();
        let ui_handle = ui.as_weak();
        let tx = tx.clone();

        physics_timer.start(
            TimerMode::Repeated,
            std::time::Duration::from_millis(7),
            move || {
                if let Some(ui) = ui_handle.upgrade() {
                    let mut p = physics.borrow_mut();

                    // Sync interpolated matrix to renderer via command bus
                    let width = ui.get_viewport_width();
                    let height = ui.get_viewport_height();

                    p.tick(0.007, width, height); // Fixed delta time for stability
                    let matrix = p.get_view_proj(width, height);

                    let _ = tx.send(EngineCommand::MatrixUpdate {
                        view_proj: matrix.to_cols_array_2d(),
                    });
                }
            },
        );
    }

    {
        let tx = tx.clone();
        let ui_handle = ui.as_weak();

        ui.on_mouse_move(move |mx, my| {
            if let Some(ui) = ui_handle.upgrade() {
                let _ = tx.send(EngineCommand::ViewportUpdate {
                    width: ui.get_viewport_width(),
                    height: ui.get_viewport_height(),
                    mx,
                    my,
                });
            }
        });
    }

    ui.window()
        .set_rendering_notifier(move |state, graphics_api| {
            match (state, graphics_api) {
                (RenderingState::RenderingSetup, GraphicsAPI::WGPU28 { device, queue, .. }) => {
                    let mut r = renderer.borrow_mut();
                    if r.is_none() {
                        if let Some(rx_owned) = rx.borrow_mut().take() {
                            *r = Some(WgpuRenderer::new(device.clone(), queue.clone(), rx_owned));
                        }
                    }
                }
                (RenderingState::BeforeRendering, GraphicsAPI::WGPU28 { .. }) => {
                    if let Some(r) = renderer.borrow_mut().as_mut() {
                        r.execute_commands();
                        // Rendering to texture_view is temporarily disabled due to Slint 1.15 API changes.
                        // We will migrate to Image::try_from(wgpu::Texture) in Phase 2.2.
                        // r.render(texture_view);
                    }
                }
                (RenderingState::RenderingTeardown, _) => {
                    if let Some(mut r) = renderer.borrow_mut().take() {
                        r.teardown();
                    }
                }
                _ => {}
            }
        })
        .expect("Failed to set rendering notifier");

    ui.run()
}
