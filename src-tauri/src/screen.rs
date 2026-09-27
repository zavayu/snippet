//! Current-monitor image capture for vision requests.
//!
//! Captures stay in memory. Region capture crops the original monitor image
//! before encoding, so text is not blurred by resizing a whole display first.

use std::io::Cursor;

use base64::{engine::general_purpose::STANDARD, Engine};
use image::{DynamicImage, ImageFormat};
use serde::{Deserialize, Serialize};
use xcap::Monitor;

#[cfg(target_os = "windows")]
use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

const MAX_IMAGE_EDGE: u32 = 1_920;
const MIN_REGION_EDGE: u32 = 24;

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

#[derive(Debug)]
pub struct MonitorSnapshot {
    image: DynamicImage,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub fn capture_current_monitor() -> Result<(CapturedImage, ImagePreview), ScreenCaptureError> {
    let snapshot = capture_current_monitor_snapshot()?;
    encode_image(snapshot.image)
}

pub fn capture_current_monitor_snapshot() -> Result<MonitorSnapshot, ScreenCaptureError> {
    #[cfg(target_os = "windows")]
    {
        let mut cursor = POINT::default();
        if unsafe { GetCursorPos(&mut cursor) } == 0 {
            return Err(ScreenCaptureError::CursorUnavailable);
        }

        let monitor =
            Monitor::from_point(cursor.x, cursor.y).map_err(|_| ScreenCaptureError::Capture)?;
        let x = monitor.x().map_err(|_| ScreenCaptureError::Capture)?;
        let y = monitor.y().map_err(|_| ScreenCaptureError::Capture)?;
        let width = monitor.width().map_err(|_| ScreenCaptureError::Capture)?;
        let height = monitor.height().map_err(|_| ScreenCaptureError::Capture)?;
        let image = monitor
            .capture_image()
            .map_err(|_| ScreenCaptureError::Capture)?;
        Ok(MonitorSnapshot {
            image: DynamicImage::ImageRgba8(image),
            x,
            y,
            width,
            height,
        })
    }

    #[cfg(not(target_os = "windows"))]
    Err(ScreenCaptureError::UnsupportedPlatform)
}

pub fn crop_region(
    snapshot: &MonitorSnapshot,
    region: NormalizedRegion,
) -> Result<(CapturedImage, ImagePreview), ScreenCaptureError> {
    if !region_is_valid(region) {
        return Err(ScreenCaptureError::InvalidRegion);
    }

    let image_width = snapshot.image.width();
    let image_height = snapshot.image.height();
    let left = (region.x * f64::from(image_width)).floor() as u32;
    let top = (region.y * f64::from(image_height)).floor() as u32;
    let right = ((region.x + region.width) * f64::from(image_width)).ceil() as u32;
    let bottom = ((region.y + region.height) * f64::from(image_height)).ceil() as u32;
    let width = right.saturating_sub(left);
    let height = bottom.saturating_sub(top);

    if width < MIN_REGION_EDGE || height < MIN_REGION_EDGE {
        return Err(ScreenCaptureError::RegionTooSmall);
    }

    encode_image(snapshot.image.crop_imm(left, top, width, height))
}

fn region_is_valid(region: NormalizedRegion) -> bool {
    [region.x, region.y, region.width, region.height]
        .into_iter()
        .all(f64::is_finite)
        && region.x >= 0.0
        && region.y >= 0.0
        && region.width > 0.0
        && region.height > 0.0
        && region.x + region.width <= 1.0
        && region.y + region.height <= 1.0
}

fn encode_image(image: DynamicImage) -> Result<(CapturedImage, ImagePreview), ScreenCaptureError> {
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
    InvalidRegion,
    RegionTooSmall,
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    UnsupportedPlatform,
}

impl ScreenCaptureError {
    pub fn user_message(&self) -> String {
        match self {
            Self::CursorUnavailable => "Snippet could not locate the current monitor.".into(),
            Self::Capture => "Snippet could not capture the current screen. Try again.".into(),
            Self::InvalidRegion => "Select a valid screen region and try again.".into(),
            Self::RegionTooSmall => "Select a larger region so Snippet can read it.".into(),
            Self::UnsupportedPlatform => {
                "Screen capture is currently available on Windows only.".into()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, RgbaImage};

    use super::{
        crop_region, resize_if_needed, MonitorSnapshot, NormalizedRegion, ScreenCaptureError,
        MAX_IMAGE_EDGE,
    };

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

    #[test]
    fn crops_before_resizing() {
        let snapshot = MonitorSnapshot {
            image: DynamicImage::ImageRgba8(RgbaImage::new(3_840, 2_160)),
            x: 0,
            y: 0,
            width: 3_840,
            height: 2_160,
        };

        let (_, preview) = crop_region(
            &snapshot,
            NormalizedRegion {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
            },
        )
        .unwrap();

        assert_eq!((preview.width, preview.height), (1_920, 1_080));
    }

    #[test]
    fn rejects_tiny_or_out_of_bounds_regions() {
        let snapshot = MonitorSnapshot {
            image: DynamicImage::ImageRgba8(RgbaImage::new(800, 600)),
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };

        let tiny = crop_region(
            &snapshot,
            NormalizedRegion {
                x: 0.0,
                y: 0.0,
                width: 0.01,
                height: 0.01,
            },
        )
        .unwrap_err();
        assert!(matches!(tiny, ScreenCaptureError::RegionTooSmall));

        let invalid = crop_region(
            &snapshot,
            NormalizedRegion {
                x: 0.9,
                y: 0.0,
                width: 0.2,
                height: 0.5,
            },
        )
        .unwrap_err();
        assert!(matches!(invalid, ScreenCaptureError::InvalidRegion));
    }
}
