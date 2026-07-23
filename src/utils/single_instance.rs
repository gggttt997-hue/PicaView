pub struct SingleInstance;

#[cfg(windows)]
mod win32 {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::OnceLock;
    use std::thread;
    use tracing::{debug, error, info};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND,
        ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING,
        PIPE_ACCESS_INBOUND,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW,
        PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::CreateMutexW;

    const PIPE_NAME: &str = "\\\\.\\pipe\\PicaView_IPC_Pipe";
    const MUTEX_NAME: &str = "Local\\PicaView_SingleInstance_Mutex";

    static MUTEX_HANDLE: OnceLock<windows_sys::Win32::Foundation::HANDLE> = OnceLock::new();

    fn get_pipe_name_w() -> Vec<u16> {
        OsStr::new(PIPE_NAME)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn get_mutex_name_w() -> Vec<u16> {
        OsStr::new(MUTEX_NAME)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    pub fn check_multi_instance_limit() -> bool {
        for i in 1..=5 {
            let name = format!("Local\\PicaView_MultiInstance_Mutex_{}", i);
            let mutex_name_w: Vec<u16> = OsStr::new(&name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let mutex_h = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name_w.as_ptr()) };

            if mutex_h != 0 {
                let err = unsafe { GetLastError() };
                if err != ERROR_ALREADY_EXISTS {
                    let _ = MUTEX_HANDLE.set(mutex_h);
                    info!("Acquired multi-instance slot {}", i);
                    return false; // OK to start
                } else {
                    unsafe {
                        CloseHandle(mutex_h);
                    }
                }
            }
        }

        info!("Max instances (5) reached. Routing to primary instance.");
        true // Exceeds limit, caller should exit after forwarding
    }

    pub fn check_and_forward_win32() -> bool {
        let args: Vec<String> = std::env::args().collect();
        let mutex_name_w = get_mutex_name_w();

        // 1. Try to create the named mutex to check instance existence
        let mutex_h = unsafe {
            CreateMutexW(
                std::ptr::null(),
                0, // Initially not owned
                mutex_name_w.as_ptr(),
            )
        };

        if mutex_h == 0 {
            let err = unsafe { GetLastError() };
            error!("Failed to create SingleInstance Named Mutex: error {}", err);
            // Fallback: If mutex creation fails, we return false to let the primary instance start,
            // preventing a cascading lock that blocks startup.
            return false;
        } else {
            let err = unsafe { GetLastError() };
            if err != ERROR_ALREADY_EXISTS {
                // No other instance exists! We are the primary instance.
                // Save the handle to MUTEX_HANDLE to keep it alive during the lifetime of this process
                let _ = MUTEX_HANDLE.set(mutex_h);
                info!("No other instance detected via Named Mutex. Initializing primary instance.");
                return false;
            } else {
                // Another instance exists!
                // Close our temporary handle immediately (since the primary instance already owns it)
                unsafe {
                    CloseHandle(mutex_h);
                }
                info!(
                    "Another active instance detected via Named Mutex. Connecting to Named Pipe..."
                );
            }
        }

        // 2. Since another instance is guaranteed to be running, connect to Named Pipe
        let pipe_name_w = get_pipe_name_w();
        let mut handle = INVALID_HANDLE_VALUE;
        let mut retries = 20;
        let mut delay_ms = 15;

        while retries > 0 {
            handle = unsafe {
                CreateFileW(
                    pipe_name_w.as_ptr(),
                    GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    0,
                )
            };

            if handle != INVALID_HANDLE_VALUE {
                break;
            }

            let err = unsafe { GetLastError() };
            if err == ERROR_PIPE_BUSY || err == ERROR_FILE_NOT_FOUND {
                info!(
                    "Named Pipe is busy or temporarily unavailable (error: {}). Waiting for release (attempts left: {})...",
                    err,
                    retries - 1
                );

                if err == ERROR_PIPE_BUSY {
                    // Wait for the pipe to become available (up to 200ms)
                    let wait_ok = unsafe { WaitNamedPipeW(pipe_name_w.as_ptr(), 200) };
                    if wait_ok == 0 {
                        // Generate high-resolution pseudo-random jitter (0ms - 29ms) using nanoseconds to break synchronization
                        let jitter = (std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                            % 30) as u64;
                        thread::sleep(std::time::Duration::from_millis(delay_ms + jitter));
                    }
                } else {
                    // For FILE_NOT_FOUND (pipe reconstruction gap), sleep briefly with jitter (0ms - 14ms)
                    let jitter = (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                        % 15) as u64;
                    thread::sleep(std::time::Duration::from_millis(delay_ms + jitter));
                }

                // Exponential backoff up to 100ms
                delay_ms = std::cmp::min(delay_ms * 2, 100);
                retries -= 1;
                continue;
            }

            // Any other error means there's no active server listening.
            debug!("No active single-instance pipe found. Error code: {}", err);
            return false;
        }

        if handle == INVALID_HANDLE_VALUE {
            error!("Named Pipe remains unreachable after all retries. No active primary instance is serving. Initializing self as primary.");
            // Physical Anti-Deadlock Shield: If the pipe is unreachable even after retries,
            // the previous instance must have hung or leaked its Mutex. We MUST take over and start up.
            return false;
        }

        info!("Another instance found. Forwarding arguments via Named Pipe...");
        let payload = args.join("\n");
        let mut bytes_written = 0;
        let success = unsafe {
            WriteFile(
                handle,
                payload.as_ptr(),
                payload.len() as u32,
                &mut bytes_written,
                std::ptr::null_mut(),
            )
        };

        if success == 0 {
            let err = unsafe { GetLastError() };
            error!("Failed to write to Named Pipe: error {}", err);
        }

        unsafe {
            CloseHandle(handle);
        }
        true
    }

    pub fn start_listener_win32<F>(on_args_received: F)
    where
        F: Fn(Vec<String>) + Send + Sync + 'static,
    {
        let pipe_name_w = get_pipe_name_w();
        let mutex_name_w = get_mutex_name_w();

        // Ensure that the primary listener owns the Named Mutex (especially for tests)
        let mutex_h = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name_w.as_ptr()) };
        if mutex_h != 0 {
            let _ = MUTEX_HANDLE.set(mutex_h);
        }

        info!(
            "Starting single-instance Named Pipe listener on {}",
            PIPE_NAME
        );

        thread::spawn(move || {
            let pipe_handle = unsafe {
                CreateNamedPipeW(
                    pipe_name_w.as_ptr(),
                    PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    4096,
                    4096,
                    0,
                    std::ptr::null(),
                )
            };

            if pipe_handle == INVALID_HANDLE_VALUE {
                let err = unsafe { GetLastError() };
                error!("Failed to create Named Pipe instance: error {}", err);
                return;
            }

            loop {
                let connected = unsafe { ConnectNamedPipe(pipe_handle, std::ptr::null_mut()) };
                let success = if connected != 0 {
                    true
                } else {
                    let err = unsafe { GetLastError() };
                    err == ERROR_PIPE_CONNECTED
                };

                if success {
                    let mut total_buffer = Vec::new();
                    let mut temp_buffer = [0u8; 1024];
                    loop {
                        let mut bytes_read = 0;
                        let read_ok = unsafe {
                            ReadFile(
                                pipe_handle,
                                temp_buffer.as_mut_ptr() as *mut _,
                                temp_buffer.len() as u32,
                                &mut bytes_read,
                                std::ptr::null_mut(),
                            )
                        };

                        if read_ok != 0 && bytes_read > 0 {
                            total_buffer.extend_from_slice(&temp_buffer[..bytes_read as usize]);
                        } else {
                            let err = unsafe { GetLastError() };
                            // ERROR_BROKEN_PIPE is the standard Windows EOF signal for named pipes
                            if err != ERROR_BROKEN_PIPE && err != 0 {
                                error!("Error reading from Named Pipe: {}", err);
                            }
                            break;
                        }
                    }

                    if !total_buffer.is_empty() {
                        if let Ok(content) = String::from_utf8(total_buffer) {
                            let args: Vec<String> =
                                content.lines().map(|s| s.to_string()).collect();
                            on_args_received(args);
                        }
                    }
                }

                unsafe {
                    DisconnectNamedPipe(pipe_handle);
                }
            }
        });
    }
}

impl SingleInstance {
    #[cfg(not(windows))]
    const PORT: u16 = 42069; // PicaView dedicated port

    /// Checks if another instance is running.
    /// If yes, sends arguments to it and returns true (caller should exit).
    /// If no, returns false (caller is the primary instance).
    pub fn check_and_forward() -> bool {
        let is_test = std::env::var("PICAVIEW_TEST").is_ok();
        if is_test {
            return false;
        }

        #[allow(unused_variables)]
        let settings = crate::utils::settings::get_settings();

        #[cfg(windows)]
        {
            if settings.allow_multi_instance {
                if win32::check_multi_instance_limit() {
                    // Limit exceeded, fallback to forwarding to the primary instance (which holds the pipe)
                    win32::check_and_forward_win32()
                } else {
                    false
                }
            } else {
                win32::check_and_forward_win32()
            }
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// Starts a listener in the background to receive paths from other instances.
    pub fn start_listener<F>(on_args_received: F)
    where
        F: Fn(Vec<String>) + Send + Sync + 'static,
    {
        #[cfg(windows)]
        {
            win32::start_listener_win32(on_args_received);
        }
        #[cfg(not(windows))]
        {
            use std::io::Read;
            use std::net::TcpListener;
            use std::thread;
            use tracing::{error, info, warn};

            let listener = match TcpListener::bind(("127.0.0.1", Self::PORT)) {
                Ok(l) => l,
                Err(e) => {
                    error!(
                        "Failed to bind single-instance listener on port {}: {}",
                        Self::PORT,
                        e
                    );
                    return;
                }
            };

            info!(
                "Single-instance listener started on 127.0.0.1:{}",
                Self::PORT
            );

            thread::spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(mut stream) => {
                            let mut buffer = String::new();
                            if stream.read_to_string(&mut buffer).is_ok() {
                                let args: Vec<String> =
                                    buffer.lines().map(|s| s.to_string()).collect();
                                on_args_received(args);
                            }
                        }
                        Err(e) => {
                            warn!("Error accepting incoming connection: {}", e);
                        }
                    }
                }
            });
        }
    }
}
