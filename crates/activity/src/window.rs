#[cfg(windows)]
pub use imp::{app_windows, foreground_window, idle_time};

/// The window in front and the program that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundWindow {
    pub pid: u32,
    pub exe_path: String,
    /// The program's file name, e.g. `WINWORD.EXE`.
    pub exe: String,
    pub title: String,
}

/// A program with a window open, as offered when linking an app to a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppWindow {
    pub exe_path: String,
    pub exe: String,
    /// The program's own description ("Microsoft Word"), or its file name without `.exe`.
    pub name: String,
    /// One of its window titles, to help tell programs apart.
    pub title: String,
}

#[cfg(not(windows))]
pub fn foreground_window() -> Option<ForegroundWindow> {
    None
}

#[cfg(not(windows))]
pub fn app_windows() -> Vec<AppWindow> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn idle_time() -> std::time::Duration {
    std::time::Duration::ZERO
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::path::Path;

    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, EnumWindows, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW,
        GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, GW_OWNER, WS_EX_TOOLWINDOW,
    };

    use super::{AppWindow, ForegroundWindow};

    /// Hosts the windows of Store apps; the real app is a child window's process.
    const FRAME_HOST: &str = "ApplicationFrameHost.exe";

    pub fn foreground_window() -> Option<ForegroundWindow> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return None;
        }
        let title = window_title(hwnd);
        let mut pid = window_pid(hwnd);
        let mut exe_path = process_path(pid)?;
        if file_name(&exe_path).eq_ignore_ascii_case(FRAME_HOST) {
            if let Some(child) = store_app_pid(hwnd, pid) {
                pid = child;
                exe_path = process_path(pid)?;
            }
        }
        Some(ForegroundWindow { pid, exe: file_name(&exe_path).to_owned(), exe_path, title })
    }

    /// How long since the last keyboard or mouse input anywhere on the computer.
    pub fn idle_time() -> std::time::Duration {
        let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return std::time::Duration::ZERO;
        }
        // Both are 32-bit millisecond tick counts, so wrapping_sub stays right when the counter rolls over (~49 days).
        let idle_ms = unsafe { GetTickCount() }.wrapping_sub(info.dwTime);
        std::time::Duration::from_millis(idle_ms as u64)
    }

    /// Programs with a normal window open (not tool windows or hidden ones), one entry per program.
    pub fn app_windows() -> Vec<AppWindow> {
        let mut windows: Vec<HWND> = Vec::new();
        unsafe { EnumWindows(Some(collect), &mut windows as *mut Vec<HWND> as LPARAM) };

        let me = unsafe { GetCurrentProcessId() };
        let mut apps: Vec<AppWindow> = Vec::new();
        for hwnd in windows {
            if !is_app_window(hwnd) {
                continue;
            }
            let mut pid = window_pid(hwnd);
            let Some(mut exe_path) = process_path(pid) else { continue };
            if file_name(&exe_path).eq_ignore_ascii_case(FRAME_HOST) {
                let Some(child) = store_app_pid(hwnd, pid) else { continue };
                pid = child;
                let Some(path) = process_path(pid) else { continue };
                exe_path = path;
            }
            if pid == me || apps.iter().any(|a| a.exe_path.eq_ignore_ascii_case(&exe_path)) {
                continue;
            }
            apps.push(AppWindow {
                name: describe(&exe_path),
                exe: file_name(&exe_path).to_owned(),
                title: window_title(hwnd),
                exe_path,
            });
        }
        apps.sort_by_key(|a| a.name.to_lowercase());
        apps
    }

    unsafe extern "system" fn collect(hwnd: HWND, list: LPARAM) -> BOOL {
        unsafe { (*(list as *mut Vec<HWND>)).push(hwnd) };
        1
    }

    fn is_app_window(hwnd: HWND) -> bool {
        unsafe {
            if IsWindowVisible(hwnd) == 0 || !GetWindow(hwnd, GW_OWNER).is_null() {
                return false;
            }
            if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0 {
                return false;
            }
            // Suspended Store apps keep "visible" windows that are cloaked (not actually shown).
            let mut cloaked: u32 = 0;
            let size = std::mem::size_of::<u32>() as u32;
            if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, &mut cloaked as *mut u32 as *mut c_void, size) == 0
                && cloaked != 0
            {
                return false;
            }
        }
        !window_title(hwnd).is_empty()
    }

    fn window_title(hwnd: HWND) -> String {
        let len = unsafe { GetWindowTextLengthW(hwnd) };
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let copied = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf16_lossy(&buf[..copied.max(0) as usize])
    }

    fn window_pid(hwnd: HWND) -> u32 {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        pid
    }

    fn process_path(pid: u32) -> Option<String> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut buf = vec![0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
            CloseHandle(handle);
            (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
        }
    }

    fn store_app_pid(frame: HWND, frame_pid: u32) -> Option<u32> {
        let mut children: Vec<HWND> = Vec::new();
        unsafe { EnumChildWindows(frame, Some(collect), &mut children as *mut Vec<HWND> as LPARAM) };
        children.into_iter().map(window_pid).find(|&pid| pid != frame_pid && pid != 0)
    }

    fn file_name(path: &str) -> &str {
        path.rsplit(['\\', '/']).next().unwrap_or(path)
    }

    /// The program's FileDescription from its version info, falling back to its file name.
    fn describe(exe_path: &str) -> String {
        version_description(exe_path)
            .filter(|d| !d.trim().is_empty())
            .unwrap_or_else(|| Path::new(exe_path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
    }

    fn version_description(exe_path: &str) -> Option<String> {
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        let path = wide(exe_path);
        unsafe {
            let size = GetFileVersionInfoSizeW(path.as_ptr(), std::ptr::null_mut());
            if size == 0 {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr() as *mut c_void) == 0 {
                return None;
            }

            let query = |sub: &str| -> Option<(*mut c_void, u32)> {
                let mut ptr: *mut c_void = std::ptr::null_mut();
                let mut len = 0u32;
                let ok = VerQueryValueW(data.as_ptr() as *const c_void, wide(sub).as_ptr(), &mut ptr, &mut len);
                (ok != 0 && !ptr.is_null() && len > 0).then_some((ptr, len))
            };

            // Version strings are stored per language and code page; use the first one the file declares.
            let (lang, codepage) = match query("\\VarFileInfo\\Translation") {
                Some((ptr, len)) if len >= 4 => {
                    let pair = ptr as *const u16;
                    (*pair, *pair.add(1))
                }
                _ => (0x0409, 0x04b0),
            };
            let (ptr, len) = query(&format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\FileDescription"))?;
            let text = std::slice::from_raw_parts(ptr as *const u16, len as usize);
            Some(String::from_utf16_lossy(text).trim_end_matches('\0').trim().to_owned())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn describes_programs_from_version_info() {
            let notepad = r"C:\Windows\System32\notepad.exe";
            if Path::new(notepad).exists() {
                assert!(!describe(notepad).is_empty());
            }
            assert_eq!(describe(r"C:\definitely\missing\Thing.exe"), "Thing");
        }

        #[test]
        fn lists_app_windows_without_crashing() {
            for app in app_windows() {
                assert!(!app.exe.is_empty() && !app.name.is_empty());
            }
            let _ = foreground_window();
            assert!(idle_time() < std::time::Duration::from_secs(60 * 60 * 24 * 50));
        }
    }
}
