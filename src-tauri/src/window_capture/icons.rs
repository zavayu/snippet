//! Read native window icons, falling back to the executable's shell icon.
//! Returned PNG data stays in memory; borrowed app icon handles are copied.

use base64::{engine::general_purpose::STANDARD, Engine};
use std::{io::Cursor, mem::size_of, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HWND},
    Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, SelectObject,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    },
    System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    },
    UI::{
        Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON},
        WindowsAndMessaging::{
            CopyIcon, DestroyIcon, DrawIconEx, GetClassLongPtrW, SendMessageTimeoutW, DI_MASK,
            DI_NORMAL, GCLP_HICON, GCLP_HICONSM, HICON, ICON_BIG, ICON_SMALL2, SMTO_ABORTIFHUNG,
            WM_GETICON,
        },
    },
};

struct OwnedIcon(HICON);
impl Drop for OwnedIcon {
    fn drop(&mut self) {
        unsafe {
            DestroyIcon(self.0);
        }
    }
}

pub(super) fn for_window(id: u32, pid: u32) -> Option<String> {
    let icon = window_icon(id as usize as HWND).or_else(|| executable_icon(pid))?;
    let image = rasterize(icon.0)?;
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(png.into_inner())
    ))
}

fn window_icon(hwnd: HWND) -> Option<OwnedIcon> {
    unsafe {
        for kind in [ICON_SMALL2, ICON_BIG] {
            let mut handle = 0;
            // A hung app must not stall the window picker indefinitely.
            if SendMessageTimeoutW(
                hwnd,
                WM_GETICON,
                kind as usize,
                0,
                SMTO_ABORTIFHUNG,
                40,
                &mut handle,
            ) != 0
                && handle != 0
            {
                let copied = CopyIcon(handle as HICON);
                if !copied.is_null() {
                    return Some(OwnedIcon(copied));
                }
            }
        }
        for kind in [GCLP_HICONSM, GCLP_HICON] {
            let handle = GetClassLongPtrW(hwnd, kind) as HICON;
            if !handle.is_null() {
                let copied = CopyIcon(handle);
                if !copied.is_null() {
                    return Some(OwnedIcon(copied));
                }
            }
        }
    }
    None
}

fn executable_icon(pid: u32) -> Option<OwnedIcon> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        let success = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length);
        CloseHandle(process);
        if success == 0 {
            return None;
        }
        path.truncate(length as usize + 1);
        let mut info = SHFILEINFOW::default();
        if SHGetFileInfoW(
            path.as_ptr(),
            0,
            &mut info,
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_SMALLICON,
        ) == 0
            || info.hIcon.is_null()
        {
            return None;
        }
        Some(OwnedIcon(info.hIcon))
    }
}

fn rasterize(icon: HICON) -> Option<image::RgbaImage> {
    const SIZE: i32 = 32;
    const BYTES: usize = (SIZE * SIZE * 4) as usize;
    unsafe {
        let dc = CreateCompatibleDC(ptr::null_mut());
        if dc.is_null() {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: SIZE,
                biHeight: -SIZE,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = ptr::null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            DeleteDC(dc);
            return None;
        }
        let previous = SelectObject(dc, bitmap);
        let image = (|| {
            ptr::write_bytes(bits as *mut u8, 0, BYTES);
            if DrawIconEx(dc, 0, 0, icon, SIZE, SIZE, 0, ptr::null_mut(), DI_NORMAL) == 0 {
                return None;
            }
            GdiFlush();
            let mut pixels = std::slice::from_raw_parts(bits as *const u8, BYTES).to_vec();
            if pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
                // Legacy icons use an AND mask rather than an alpha channel.
                if DrawIconEx(dc, 0, 0, icon, SIZE, SIZE, 0, ptr::null_mut(), DI_MASK) == 0 {
                    return None;
                }
                GdiFlush();
                let mask = std::slice::from_raw_parts(bits as *const u8, BYTES);
                for (pixel, mask) in pixels.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                    pixel[3] = 255 - mask[0];
                }
            }
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2); // Native DIB pixels are BGRA.
                                  // Windows renders premultiplied alpha; PNG expects straight alpha.
                let alpha = u32::from(pixel[3]);
                if alpha > 0 && alpha < 255 {
                    for color in &mut pixel[..3] {
                        *color = ((u32::from(*color) * 255 + alpha / 2) / alpha).min(255) as u8;
                    }
                }
            }
            image::RgbaImage::from_raw(SIZE as u32, SIZE as u32, pixels)
        })();
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renders_a_native_icon_as_rgba_without_owning_the_shared_handle() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{LoadIconW, IDI_APPLICATION};
        let shared = unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) };
        assert!(!shared.is_null());
        let icon = OwnedIcon(unsafe { CopyIcon(shared) });
        let image = rasterize(icon.0).unwrap();
        assert_eq!(image.dimensions(), (32, 32));
        assert!(image.pixels().any(|pixel| pixel[3] > 0));
        drop(icon);
        assert!(rasterize(shared).is_some());
    }
}
