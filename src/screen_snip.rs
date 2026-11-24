//! 从本次屏幕框选复制图片、文字或二维码内容。调用方在后台线程运行。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    CopyImage,
    Ocr,
    Qr,
}

#[cfg(not(windows))]
pub fn run(_: Action) -> Result<String, String> {
    Err("屏幕框选仅支持 Windows".into())
}

#[cfg(windows)]
pub use platform::run;

#[cfg(windows)]
mod platform {
    use super::Action;
    use base64::Engine;
    use image::GrayImage;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::{Mutex, TryLockError};
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;
    use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

    static SELECTION: Mutex<()> = Mutex::new(());
    const SELECTOR: &str = include_str!("../scripts/screen-snip.ps1");

    fn powershell(script: &str) -> Command {
        let source: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let encoded = base64::engine::general_purpose::STANDARD.encode(source);
        let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let exe = std::path::PathBuf::from(system)
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let mut cmd = Command::new(exe);
        cmd.creation_flags(0x08000000) // CREATE_NO_WINDOW; the selection form stays visible.
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-STA",
                "-EncodedCommand",
            ])
            .arg(encoded);
        cmd
    }

    pub fn run(action: Action) -> Result<String, String> {
        let _session = match SELECTION.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return Err("已有框选正在进行".into()),
            Err(TryLockError::Poisoned(err)) => err.into_inner(),
        };
        let output = powershell(SELECTOR)
            .env(
                "DAOZAI_SNIP_MODE",
                if action == Action::CopyImage {
                    "image"
                } else {
                    "read"
                },
            )
            .output()
            .map_err(|_| "无法打开屏幕框选，请检查 Windows PowerShell".to_owned())?;
        match output.status.code() {
            Some(2) => return Ok("已取消框选".into()),
            Some(0) => {}
            _ => return Err("框选失败，请检查桌面或剪贴板是否可用".into()),
        }
        if action == Action::CopyImage {
            return Ok("截图已复制".into());
        }
        let pixels = image::load_from_memory_with_format(&output.stdout, image::ImageFormat::Png)
            .map_err(|_| "无法读取本次截图".to_owned())?
            .into_luma8();
        let (contents, status) = match action {
            Action::Ocr => (recognize_text(pixels)?, "文字已复制"),
            Action::Qr => (decode_qr(pixels)?, "二维码内容已复制"),
            Action::CopyImage => unreachable!(),
        };
        clipboard_win::set_clipboard_string(&contents)
            .map_err(|_| "识别成功，写入剪贴板失败".to_owned())?;
        Ok(status.into())
    }

    fn decode_qr(pixels: GrayImage) -> Result<String, String> {
        let mut image = rqrr::PreparedImage::prepare(pixels);
        let mut results = Vec::new();
        for grid in image.detect_grids() {
            if let Ok((_, value)) = grid.decode() {
                if !value.is_empty() && !results.contains(&value) {
                    results.push(value);
                }
            }
        }
        if results.is_empty() {
            Err("未识别到可读取的二维码".into())
        } else {
            Ok(results.join("\n"))
        }
    }

    fn recognize_text(mut pixels: GrayImage) -> Result<String, String> {
        // All WinRT objects are created and released on this worker before uninitializing it.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
            .map_err(|_| "无法初始化 Windows OCR".to_owned())?;
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe { RoUninitialize() }
            }
        }
        let _apartment = Apartment;
        let engine = OcrEngine::TryCreateFromUserProfileLanguages()
            .or_else(|_| {
                let language = OcrEngine::AvailableRecognizerLanguages()?.GetAt(0)?;
                OcrEngine::TryCreateFromLanguage(&language)
            })
            .map_err(|_| "Windows 未安装可用的 OCR 语言包".to_owned())?;
        let ocr_error = |_: windows::core::Error| "Windows 文字识别失败".to_owned();
        let max = OcrEngine::MaxImageDimension().map_err(ocr_error)?;
        let longest = pixels.width().max(pixels.height());
        if longest > max {
            let scale = |n: u32| (u64::from(n) * u64::from(max) / u64::from(longest)).max(1) as u32;
            pixels = image::imageops::resize(
                &pixels,
                scale(pixels.width()),
                scale(pixels.height()),
                image::imageops::FilterType::Triangle,
            );
        }
        let bytes = DataWriter::new().map_err(ocr_error)?;
        bytes.WriteBytes(pixels.as_raw()).map_err(ocr_error)?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
            &bytes.DetachBuffer().map_err(ocr_error)?,
            BitmapPixelFormat::Gray8,
            pixels.width() as i32,
            pixels.height() as i32,
        )
        .map_err(ocr_error)?;
        let lines = engine
            .RecognizeAsync(&bitmap)
            .and_then(|task| task.get())
            .and_then(|result| result.Lines())
            .map_err(ocr_error)?;
        let mut result = Vec::new();
        for line in lines {
            let text = line.Text().map_err(ocr_error)?.to_string_lossy();
            if !text.trim().is_empty() {
                result.push(text);
            }
        }
        if result.is_empty() {
            Err("框选区域未识别到文字".into())
        } else {
            Ok(result.join("\r\n"))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn reads_synthetic_qr_and_rejects_an_empty_image() {
            // "DAOZAI", encoded using OpenCV's QRCodeEncoder.
            let fixture = "iVBORw0KGgoAAAANSUhEUgAAAJYAAACWAQAAAAAUekxPAAAA3klEQVR42uWWMW4DQQhF35iRUuIb+Cjs1Xyz9VF8A7a0xOq78G6KpKdIqNA0XwMPPkP8jP3C7/grbygBLAMCgKgm3ZBkEi6TlD26F2CMhW0UsI3ZXnt7Lo39nWcS+y1nr65g+n2y4GLr5tmyl+czijiStv+6LI/c+uYo4PG6UjBdr2tXf6UiLMElrV17AyVFlK/lgrC+/gYuy1Ae6q08f9SbeQbTWkS5cDX7kUnVtzdOP+JR25gfsNu4Iuygiy6uvv3oYc9lv61f1ee/0gqU35VU933Fsaya6jz+2R37BpFHrliRfARYAAAAAElFTkSuQmCC";
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(fixture)
                .unwrap();
            let pixels = image::load_from_memory(&bytes).unwrap().into_luma8();
            assert_eq!(decode_qr(pixels).unwrap(), "DAOZAI");
            assert_eq!(
                decode_qr(GrayImage::new(160, 160)).unwrap_err(),
                "未识别到可读取的二维码"
            );
        }

        #[test]
        #[ignore = "requires a Windows OCR language pack"]
        fn reads_generated_text_without_accessing_desktop_or_clipboard() {
            let script = r#"
                Add-Type -AssemblyName System.Drawing
                $bitmap = [System.Drawing.Bitmap]::new(560, 120)
                $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
                $graphics.Clear([System.Drawing.Color]::White)
                $font = [System.Drawing.Font]::new('Arial', 42)
                $graphics.DrawString('HELLO 2026', $font, [System.Drawing.Brushes]::Black, 12, 15)
                $buf = [System.IO.MemoryStream]::new()
                $bitmap.Save($buf, [System.Drawing.Imaging.ImageFormat]::Png)
                $buf.WriteTo([Console]::OpenStandardOutput())
            "#;
            let out = powershell(script).output().unwrap();
            assert!(out.status.success());
            let pixels = image::load_from_memory(&out.stdout).unwrap().into_luma8();
            let text = recognize_text(pixels).unwrap();
            assert!(
                text.split_whitespace()
                    .collect::<String>()
                    .contains("HELLO2026"),
                "{text}"
            );
            assert_eq!(
                recognize_text(GrayImage::new(160, 160)).unwrap_err(),
                "框选区域未识别到文字"
            );
        }
    }
}
