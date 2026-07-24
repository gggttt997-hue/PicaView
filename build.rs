fn main() {
    std::env::set_var("SLINT_ENABLE_EXPERIMENTAL_FEATURES", "1");

    // Spawn a thread with a larger stack size (16MB) to prevent stack overflow (0xc00000fd)
    // on Windows when compiling complex Slint layout trees.
    let handle = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            slint_build::compile("ui/main.slint").unwrap();
        })
        .unwrap();
    handle.join().unwrap();

    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icons/icon.ico");
        res.compile().unwrap();
    }
}
