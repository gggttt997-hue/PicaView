#[cfg(windows)]
#[allow(clippy::upper_case_acronyms)]
mod win32 {
    use std::os::windows::ffi::OsStrExt;
    use std::thread;
    use tracing::warn;
    use windows_sys::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
    };

    type HRESULT = i32;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct GUID {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    #[repr(C)]
    struct SIZE {
        cx: i32,
        cy: i32,
    }

    const IID_ISHELL_ITEM: GUID = GUID {
        data1: 0x43826d1e,
        data2: 0xe718,
        data3: 0x42ee,
        data4: [0xbc, 0x55, 0xa1, 0xe2, 0x61, 0xc3, 0x7b, 0xfe],
    };

    const IID_ISHELL_ITEM_IMAGE_FACTORY: GUID = GUID {
        data1: 0xbcc18b79,
        data2: 0xba16,
        data3: 0x442f,
        data4: [0x80, 0xc4, 0x8a, 0x59, 0xc3, 0x0c, 0x46, 0x3b],
    };

    const SIIGBF_INCACHEONLY: u32 = 0x00000008;
    const SIIGBF_THUMBNAILONLY: u32 = 0x00000002;

    extern "system" {
        fn SHCreateItemFromParsingName(
            pszPath: *const u16,
            pbc: *const std::ffi::c_void,
            riid: *const GUID,
            ppv: *mut *mut std::ffi::c_void,
        ) -> HRESULT;
        fn CoInitialize(pvReserved: *const std::ffi::c_void) -> HRESULT;
        fn CoUninitialize();
    }

    #[repr(C)]
    struct IShellItemVtbl {
        pub query_interface: unsafe extern "system" fn(
            this: *mut std::ffi::c_void,
            riid: *const GUID,
            ppv_object: *mut *mut std::ffi::c_void,
        ) -> HRESULT,
        pub add_ref: unsafe extern "system" fn(this: *mut std::ffi::c_void) -> u32,
        pub release: unsafe extern "system" fn(this: *mut std::ffi::c_void) -> u32,
    }

    #[repr(C)]
    struct IShellItemImageFactoryVtbl {
        pub query_interface: unsafe extern "system" fn(
            this: *mut std::ffi::c_void,
            riid: *const GUID,
            ppv_object: *mut *mut std::ffi::c_void,
        ) -> HRESULT,
        pub add_ref: unsafe extern "system" fn(this: *mut std::ffi::c_void) -> u32,
        pub release: unsafe extern "system" fn(this: *mut std::ffi::c_void) -> u32,
        pub get_image: unsafe extern "system" fn(
            this: *mut std::ffi::c_void,
            size: SIZE,
            flags: u32,
            phbmp: *mut HBITMAP,
        ) -> HRESULT,
    }

    pub fn extract_wic_thumbnail_win32(
        path_str: &str,
        target_size: u32,
    ) -> Option<(Vec<u8>, u32, u32)> {
        let path_str_owned = path_str.to_string();

        let handle = thread::spawn(move || {
            let mut path_wide: Vec<u16> = std::ffi::OsStr::new(&path_str_owned)
                .encode_wide()
                .collect();
            path_wide.push(0);

            unsafe {
                let hr = CoInitialize(std::ptr::null());
                if hr < 0 {
                    warn!("[WIC Cache] CoInitialize failed: hr = 0x{:X}", hr);
                    return None;
                }

                // 1. Create ShellItem (IShellItem)
                let mut shell_item_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
                let hr = SHCreateItemFromParsingName(
                    path_wide.as_ptr(),
                    std::ptr::null(),
                    &IID_ISHELL_ITEM,
                    &mut shell_item_ptr,
                );

                if hr < 0 || shell_item_ptr.is_null() {
                    CoUninitialize();
                    return None;
                }

                // 2. QueryInterface for IShellItemImageFactory
                let shell_item_vtbl = *(shell_item_ptr as *mut *mut IShellItemVtbl);
                if shell_item_vtbl.is_null() {
                    ((*shell_item_vtbl).release)(shell_item_ptr);
                    CoUninitialize();
                    return None;
                }

                let mut image_factory_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
                let hr_query = ((*shell_item_vtbl).query_interface)(
                    shell_item_ptr,
                    &IID_ISHELL_ITEM_IMAGE_FACTORY,
                    &mut image_factory_ptr,
                );

                // Release base shell item
                ((*shell_item_vtbl).release)(shell_item_ptr);

                if hr_query < 0 || image_factory_ptr.is_null() {
                    CoUninitialize();
                    return None;
                }

                // 3. GetImage from factory
                let image_factory_vtbl =
                    *(image_factory_ptr as *mut *mut IShellItemImageFactoryVtbl);
                if image_factory_vtbl.is_null() {
                    CoUninitialize();
                    return None;
                }

                let mut hbitmap: HBITMAP = 0;
                let size_param = SIZE {
                    cx: target_size as i32,
                    cy: target_size as i32,
                };

                // Try in-cache lookup first
                let mut hr_get = ((*image_factory_vtbl).get_image)(
                    image_factory_ptr,
                    size_param,
                    SIIGBF_INCACHEONLY,
                    &mut hbitmap,
                );

                // Fallback to normal thumbnail generation/loading if not in cache
                if hr_get < 0 || hbitmap == 0 {
                    hr_get = ((*image_factory_vtbl).get_image)(
                        image_factory_ptr,
                        SIZE {
                            cx: target_size as i32,
                            cy: target_size as i32,
                        },
                        SIIGBF_THUMBNAILONLY,
                        &mut hbitmap,
                    );
                }

                // Release image factory
                ((*image_factory_vtbl).release)(image_factory_ptr);

                if hr_get < 0 || hbitmap == 0 {
                    CoUninitialize();
                    return None;
                }

                // 4. Extract pixel buffer from HBITMAP
                let mut bmp: BITMAP = std::mem::zeroed();
                let info_ok = GetObjectW(
                    hbitmap,
                    std::mem::size_of::<BITMAP>() as i32,
                    &mut bmp as *mut BITMAP as *mut std::ffi::c_void,
                );

                if info_ok == 0 {
                    DeleteObject(hbitmap);
                    CoUninitialize();
                    return None;
                }

                let width = bmp.bmWidth;
                let height = bmp.bmHeight;

                let hdc = GetDC(0);
                if hdc == 0 {
                    DeleteObject(hbitmap);
                    CoUninitialize();
                    return None;
                }

                let mut bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: width,
                        biHeight: -height, // top-down format
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB,
                        biSizeImage: 0,
                        biXPelsPerMeter: 0,
                        biYPelsPerMeter: 0,
                        biClrUsed: 0,
                        biClrImportant: 0,
                    },
                    bmiColors: [std::mem::zeroed(); 1],
                };

                let buf_size = (width * height * 4) as usize;
                let mut pixels = vec![0u8; buf_size];

                let copied = GetDIBits(
                    hdc,
                    hbitmap,
                    0,
                    height as u32,
                    pixels.as_mut_ptr() as *mut std::ffi::c_void,
                    &mut bmi,
                    DIB_RGB_COLORS,
                );

                ReleaseDC(0, hdc);
                DeleteObject(hbitmap);
                CoUninitialize();

                if copied == 0 {
                    return None;
                }

                // Swap BGRA to RGBA
                for i in (0..pixels.len()).step_by(4) {
                    if i + 2 < pixels.len() {
                        let b = pixels[i];
                        let r = pixels[i + 2];
                        pixels[i] = r;
                        pixels[i + 2] = b;
                    }
                }

                Some((pixels, width as u32, height as u32))
            }
        });

        handle.join().unwrap_or(None)
    }
}

/// Extracts WIC thumbnail from Windows Shell Thumbnail Cache.
/// Returns Option<(RGBA_pixels, width, height)>.
pub fn extract_wic_thumbnail(path_str: &str, target_size: u32) -> Option<(Vec<u8>, u32, u32)> {
    #[cfg(windows)]
    {
        win32::extract_wic_thumbnail_win32(path_str, target_size)
    }
    #[cfg(not(windows))]
    {
        let _ = path_str;
        let _ = target_size;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wic_nonexistent_file_fallback() {
        let res = extract_wic_thumbnail("C:\\this_file_does_not_exist_12345.png", 256);
        assert!(res.is_none());
    }

    #[test]
    fn test_wic_invalid_path_fallback() {
        let res = extract_wic_thumbnail("", 256);
        assert!(res.is_none());
    }
}
