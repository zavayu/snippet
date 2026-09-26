//! Current-monitor image capture for vision requests.
//!
//! Captures are resized and encoded entirely in memory. Keeping this module
//! independent from the popup makes it possible to add an area-selection
//! overlay later by cropping the same monitor snapshot.

use std::io::Cursor;

use base64::{engine::general_purpose::STANDARD, Engine};
use image::{DynamicImage, ImageFormat};
use serde::Serialize;
use xcap::Monitor;

#[cfg(target_os = "windows")]
use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

const MAX_IMAGE_EDGE: u32 = 1_920;

#[derive(Debug, Clone)]
pub struct CapturedImage {
    pub png_base64: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePreview {
    pub data_url: String,
    pub width: u32,
    pub height: u32,
}

pub fn capture_current_monitor() -> Result<(CapturedImage, ImagePreview), ScreenCaptureError> {
    #[cfg(target_os = "windows")]
    {
        let mut cursor = POINT::default();
        if unsafe { GetCursorPos(&mut cursor) } == 0 {
            return Err(ScreenCaptureError::CursorUnavailable);
        }

        let monitor =
            Monitor::from_point(cursor.x, cursor.y).map_err(|_| ScreenCaptureError::Capture)?;
        let image = monitor
            .capture_image()
            .map_err(|_| ScreenCaptureError::Capture)?;
        let image = DynamicImage::ImageRgba8(image);
        let image = resize_if_needed(image);
        let (width, height) = (image.width(), image.height());
        let mut png = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .map_err(|_| ScreenCaptureError::Capture)?;
        let png_base64 = STANDARD.encode(png);
        let preview = ImagePreview {
            data_url: format!("data:image/png;base64,{png_base64}"),
            width,
            height,
        };

        Ok((CapturedImage { png_base64 }, preview))
    }

    #[cfg(not(target_os = "windows"))]
    Err(ScreenCaptureError::UnsupportedPlatform)
}

fn resize_if_needed(image: DynamicImage) -> DynamicImage {
    let largest_edge = image.width().max(image.height());
    if largest_edge <= MAX_IMAGE_EDGE {
        return image;
    }

    image.resize(
        MAX_IMAGE_EDGE,
        MAX_IMAGE_EDGE,
        image::imageops::FilterType::Triangle,
    )
}

#[derive(Debug)]
pub enum ScreenCaptureError {
    CursorUnavailable,
    Capture,
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    UnsupportedPlatform,
}

impl ScreenCaptureError {
    pub fn user_message(&self) -> String {
        match self {
            Self::CursorUnavailable => "Snippet could not locate the current monitor.".into(),
            Self::Capture => "Snippet could not capture the current screen. Try again.".into(),
            Self::UnsupportedPlatform => {
                "Screen capture is currently available on Windows only.".into()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, RgbaImage};

    use super::{resize_if_needed, MAX_IMAGE_EDGE};

    #[test]
    fn reduces_large_images_without_upscaling_small_ones() {
        let large = DynamicImage::ImageRgba8(RgbaImage::new(3_840, 2_160));
        let scaled = resize_if_needed(large);
        assert_eq!(scaled.width(), MAX_IMAGE_EDGE);
        assert_eq!(scaled.height(), 1_080);

        let small = DynamicImage::ImageRgba8(RgbaImage::new(800, 600));
        let unchanged = resize_if_needed(small);
        assert_eq!((unchanged.width(), unchanged.height()), (800, 600));
    }
}
