//! One-shot window capture. Only the selected minimized window is restored.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowChoice {
    pub id: u32,
    pub pid: u32,
    pub title: String,
    pub app_name: String,
    pub minimized: bool,
}

pub fn list() -> Result<Vec<WindowChoice>, String> {
    let windows = xcap::Window::all().map_err(|_| "Snippet could not list open windows.")?;
    Ok(windows
        .into_iter()
        .filter_map(|window| {
            let pid = window.pid().ok()?;
            let title = window.title().ok()?;
            if pid == std::process::id() || title.trim().is_empty() {
                return None;
            }
            let id = window.id().ok()?;
            Some(WindowChoice {
                id,
                pid,
                title,
                app_name: window.app_name().unwrap_or_default(),
                minimized: window.is_minimized().unwrap_or(false),
            })
        })
        .collect())
}

pub fn capture(
    id: u32,
    pid: u32,
) -> Result<
    (
        crate::screen::CapturedImage,
        crate::screen::ImagePreview,
        String,
    ),
    String,
> {
    let window = xcap::Window::all()
        .map_err(|_| "Snippet could not list open windows.")?
        .into_iter()
        .find(|window| {
            window.id().ok() == Some(id)
                && window.pid().ok() == Some(pid)
                && pid != std::process::id()
        })
        .ok_or("This window is no longer available. Refresh the window list and try again.")?;
    let title = window
        .title()
        .map_err(|_| "This window is no longer available.")?;
    #[cfg(target_os = "windows")]
    let _restore = native::restore_for_capture(id)?;
    let pixels = window.capture_image().map_err(|_| "Snippet could not capture this window. It may be protected, closed, or not responding. Try restoring it manually.")?;
    // Do not silently attach a blank/protected capture as if it contained context.
    if pixels.width() < 24
        || pixels.height() < 24
        || pixels.pixels().all(|pixel| pixel.0[..3] == [0, 0, 0])
    {
        return Err("This window returned an empty image. Restore it and try again, or capture a region instead.".into());
    }
    let (image, preview) = crate::screen::encode_image(image::DynamicImage::ImageRgba8(pixels))
        .map_err(|error| error.user_message())?;
    Ok((image, preview, title))
}

#[cfg(target_os = "windows")]
mod native {
    use std::{
        mem::size_of,
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            GetWindowPlacement, GetWindowThreadProcessId, IsIconic, IsWindow, SetWindowPlacement,
            ShowWindowAsync, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, WINDOWPLACEMENT,
        },
    };

    pub struct RestoreState {
        hwnd: HWND,
        pid: u32,
        placement: Option<WINDOWPLACEMENT>,
    }
    impl Drop for RestoreState {
        fn drop(&mut self) {
            if let Some(mut placement) = self.placement.take() {
                // Preserve the previous restore bounds/maximized flag without activating it.
                placement.showCmd = SW_SHOWMINNOACTIVE as u32;
                unsafe {
                    let mut pid = 0;
                    GetWindowThreadProcessId(self.hwnd, &mut pid);
                    if IsWindow(self.hwnd) != 0 && pid == self.pid {
                        SetWindowPlacement(self.hwnd, &placement);
                    }
                }
            }
        }
    }

    pub fn restore_for_capture(id: u32) -> Result<RestoreState, String> {
        let hwnd = id as usize as HWND;
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        let mut state = RestoreState {
            hwnd,
            pid,
            placement: None,
        };
        if unsafe { IsIconic(hwnd) } == 0 {
            return Ok(state);
        }
        let mut placement = WINDOWPLACEMENT {
            length: size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        if unsafe { GetWindowPlacement(hwnd, &mut placement) } == 0 {
            return Err("Snippet could not restore this window for capture.".into());
        }
        state.placement = Some(placement);
        unsafe {
            ShowWindowAsync(hwnd, SW_SHOWNOACTIVATE);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { IsIconic(hwnd) } != 0 {
            if Instant::now() >= deadline || unsafe { IsWindow(hwnd) } == 0 {
                return Err(
                    "This window could not be restored. Open it manually and try again.".into(),
                );
            }
            thread::sleep(Duration::from_millis(20));
        }
        // Allow layout to settle; WGC then waits for a newly produced frame (up to 3s).
        thread::sleep(Duration::from_millis(150));
        Ok(state)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::mpsc;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow, GetMessageW,
            PostThreadMessageW, ShowWindow, TranslateMessage, MSG, WM_QUIT, WS_OVERLAPPEDWINDOW,
        };

        #[test]
        #[ignore = "briefly restores a test window; requires an interactive Windows desktop"]
        fn minimized_window_restores_after_capture_failure_without_changing_focus() {
            struct Fixture {
                thread_id: u32,
                owner: Option<thread::JoinHandle<()>>,
            }
            impl Drop for Fixture {
                fn drop(&mut self) {
                    unsafe {
                        PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
                    }
                    self.owner.take().unwrap().join().unwrap();
                }
            }
            let foreground = unsafe { GetForegroundWindow() };
            let (ready, receive) = mpsc::sync_channel(1);
            let owner = thread::spawn(move || unsafe {
                let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
                let hwnd = CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    wide("Snippet capture test").as_ptr(),
                    WS_OVERLAPPEDWINDOW,
                    100,
                    100,
                    400,
                    200,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                );
                assert!(!hwnd.is_null());
                ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
                let thread_id = GetWindowThreadProcessId(hwnd, std::ptr::null_mut());
                ready.send((hwnd as usize, thread_id)).unwrap();
                let mut message = MSG::default();
                while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                DestroyWindow(hwnd);
            });
            let (id, thread_id) = receive.recv_timeout(Duration::from_secs(5)).unwrap();
            let _fixture = Fixture {
                thread_id,
                owner: Some(owner),
            };
            let hwnd = id as HWND;
            assert_ne!(unsafe { IsIconic(hwnd) }, 0);
            let mut before = WINDOWPLACEMENT {
                length: size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            };
            assert_ne!(unsafe { GetWindowPlacement(hwnd, &mut before) }, 0);
            let result: Result<(), &str> = (|| {
                let _restore = restore_for_capture(id as u32).unwrap();
                assert_eq!(unsafe { IsIconic(hwnd) }, 0);
                Err("simulated capture failure")
            })();
            assert!(result.is_err());
            assert_ne!(unsafe { IsIconic(hwnd) }, 0);
            let mut after = WINDOWPLACEMENT {
                length: size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            };
            assert_ne!(unsafe { GetWindowPlacement(hwnd, &mut after) }, 0);
            assert_eq!(before.rcNormalPosition.left, after.rcNormalPosition.left);
            assert_eq!(before.rcNormalPosition.top, after.rcNormalPosition.top);
            assert_eq!(before.rcNormalPosition.right, after.rcNormalPosition.right);
            assert_eq!(
                before.rcNormalPosition.bottom,
                after.rcNormalPosition.bottom
            );
            assert_eq!(unsafe { GetForegroundWindow() }, foreground);
        }
    }
}
