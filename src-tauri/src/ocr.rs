//! Local Windows OCR, independent of Ollama and the clipboard.
use crate::provider::ProviderError;
use image::DynamicImage;

pub fn recognize(image: &DynamicImage) -> Result<String, ProviderError> {
    #[cfg(target_os = "windows")]
    {
        native::recognize(image).map_err(ProviderError::RequestFailed)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = image;
        Err(ProviderError::RequestFailed(
            "Windows OCR is available on Windows only. Choose Ollama in Settings → Read screen."
                .into(),
        ))
    }
}

#[cfg(target_os = "windows")]
mod native {
    use super::*;
    use windows::{
        Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
        Media::Ocr::OcrEngine,
        Storage::Streams::DataWriter,
        Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
    };

    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { RoUninitialize() };
        }
    }

    pub(super) fn recognize(image: &DynamicImage) -> Result<String, String> {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(failure)?;
        let _apartment = Apartment;
        let engine = OcrEngine::TryCreateFromUserProfileLanguages().or_else(|_| {
            let languages = OcrEngine::AvailableRecognizerLanguages()?;
            OcrEngine::TryCreateFromLanguage(&languages.GetAt(0)?)
        }).map_err(|_| "Windows OCR has no recognizer for your preferred languages. Install an OCR language pack in Windows Settings → Time & language → Language & region, or choose Ollama in Snippet Settings → Read screen.".to_string())?;
        let limit = OcrEngine::MaxImageDimension().map_err(failure)?;
        let image = prepare(image, limit);
        let mut pixels = image.to_rgba8();
        // Windows expects BGRA bytes; ignore the screenshot's alpha channel.
        for pixel in pixels.pixels_mut() {
            pixel.0.swap(0, 2);
            pixel.0[3] = 255;
        }
        let writer = DataWriter::new().map_err(failure)?;
        writer.WriteBytes(pixels.as_raw()).map_err(failure)?;
        let buffer = writer.DetachBuffer().map_err(failure)?;
        let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            pixels.width() as i32,
            pixels.height() as i32,
            BitmapAlphaMode::Ignore,
        )
        .map_err(failure)?;
        let recognition = (|| {
            let result = engine.RecognizeAsync(&bitmap)?.get()?;
            let mut lines = Vec::new();
            for line in result.Lines()? {
                lines.push(line.Text()?.to_string());
            }
            Ok::<_, windows::core::Error>(lines.join("\n").trim().to_owned())
        })();
        let _ = bitmap.Close();
        recognition.map_err(failure)
    }

    fn failure(error: windows::core::Error) -> String {
        format!("Windows OCR could not read this capture: {error}. Try again or choose Ollama in Settings → Read screen.")
    }

    fn prepare(image: &DynamicImage, limit: u32) -> DynamicImage {
        let edge = image.width().max(image.height());
        let scale = (f64::from(limit) / f64::from(edge)).min(1.5);
        let width = (f64::from(image.width()) * scale).round().max(1.0) as u32;
        let height = (f64::from(image.height()) * scale).round().max(1.0) as u32;
        let resized = image.resize_exact(width, height, image::imageops::FilterType::CatmullRom);
        // Small strips need some surrounding background for reliable recognition.
        let output_width = width.max(80).min(limit);
        let output_height = height.max(80).min(limit);
        if (width, height) == (output_width, output_height) {
            return resized;
        }
        use image::GenericImageView;
        let mut output =
            image::RgbaImage::from_pixel(output_width, output_height, resized.get_pixel(0, 0));
        image::imageops::overlay(
            &mut output,
            &resized.to_rgba8(),
            i64::from((output_width - width) / 2),
            i64::from((output_height - height) / 2),
        );
        DynamicImage::ImageRgba8(output)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        #[ignore = "requires installed Windows English OCR language pack"]
        fn recognizes_known_text_without_package_identity_or_ollama() {
            let image =
                image::load_from_memory(include_bytes!("../tests/fixtures/ocr.png")).unwrap();
            let text = super::super::recognize(&image).unwrap();
            assert!(
                text.contains("Snippet OCR 12345"),
                "Unexpected OCR: {text:?}"
            );
            let blank = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                600,
                100,
                image::Rgba([255, 255, 255, 255]),
            ));
            assert_eq!(super::super::recognize(&blank).unwrap(), "");
        }

        #[test]
        fn prepares_tiny_strips_and_large_captures_within_engine_limit() {
            let small = prepare(&DynamicImage::new_rgba8(100, 24), 2048);
            assert_eq!((small.width(), small.height()), (150, 80));
            let large = prepare(&DynamicImage::new_rgba8(3840, 2160), 2048);
            assert_eq!((large.width(), large.height()), (2048, 1152));
        }
    }
}
