//! HEIC / HEIF pictures, decoded by the operating system's own codec rather than a bundled
//! one: Windows Imaging Component on Windows (needs Microsoft's "HEIF Image Extensions" and
//! "HEVC Video Extensions", which most Windows 11 PCs have), and the built-in `sips` tool on
//! macOS. Both apply the picture's stored rotation, so no EXIF orientation is applied on top.

use std::path::Path;

#[cfg(windows)]
pub fn decode(path: &Path) -> Result<image::DynamicImage, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat48bppRGB, IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
        WICDecodeMetadataCacheOnDemand,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

    let missing = |e: windows::core::Error| {
        format!(
            "Windows could not decode this HEIC file ({}). It needs the \"HEIF Image Extensions\" and \"HEVC Video Extensions\" from the Microsoft Store.",
            e.message()
        )
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: plain COM calls on objects created here; every buffer passed is sized for the
    // call that fills it, and `wide` outlives the call that reads it.
    unsafe {
        // Each thread that decodes must have COM set up; repeating it is harmless.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(missing)?;
        let decoder = factory
            .CreateDecoderFromFilename(PCWSTR(wide.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
            .map_err(missing)?;
        let frame = decoder.GetFrame(0).map_err(missing)?;
        // 16 bits per channel keeps the extra precision of 10-bit HEIC files.
        let converter = factory.CreateFormatConverter().map_err(missing)?;
        converter
            .Initialize(&frame, &GUID_WICPixelFormat48bppRGB, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
            .map_err(missing)?;
        let (mut width, mut height) = (0u32, 0u32);
        converter.GetSize(&mut width, &mut height).map_err(missing)?;
        if width == 0 || height == 0 || width as u64 * height as u64 > 400_000_000 {
            return Err("The HEIC picture has an unusable size.".into());
        }
        let stride = width * 6;
        let mut bytes = vec![0u8; stride as usize * height as usize];
        converter.CopyPixels(std::ptr::null(), stride, &mut bytes).map_err(missing)?;
        let pixels: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        image::ImageBuffer::from_raw(width, height, pixels)
            .map(image::DynamicImage::ImageRgb16)
            .ok_or_else(|| "The HEIC picture could not be read.".to_owned())
    }
}

#[cfg(target_os = "macos")]
pub fn decode(path: &Path) -> Result<image::DynamicImage, String> {
    // `sips` ships with macOS and reads HEIC natively; convert to a temporary PNG and read that.
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let temp = std::env::temp_dir().join(format!("develop-heic-{}-{unique}.png", std::process::id()));
    let output = std::process::Command::new("sips")
        .args(["-s", "format", "png"])
        .arg(path)
        .arg("--out")
        .arg(&temp)
        .output()
        .map_err(|e| format!("Could not run sips to read the HEIC file: {e}"))?;
    let result = if output.status.success() {
        image::open(&temp).map_err(|e| format!("Could not read the converted HEIC picture: {e}"))
    } else {
        Err(format!("macOS could not decode this HEIC file: {}", String::from_utf8_lossy(&output.stderr).trim()))
    };
    let _ = std::fs::remove_file(&temp);
    result
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn decode(_path: &Path) -> Result<image::DynamicImage, String> {
    Err("HEIC pictures are only supported on Windows and macOS.".into())
}

pub fn is_heic(path: &Path) -> bool {
    matches!(crate::filmstrip::extension_of(path).as_str(), "heic" | "heif")
}

/// Open any finished picture: HEIC through the system codec, the rest through `image`.
pub fn open_picture(path: &Path) -> Result<image::DynamicImage, String> {
    if is_heic(path) {
        decode(path)
    } else {
        image::open(path).map_err(|e| format!("Could not open the picture: {e}"))
    }
}
