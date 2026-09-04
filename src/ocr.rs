use std::{
    error::Error,
    fs::File,
    io::{copy, Cursor},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use crate::export::Shot;
use rusto::{DetectTextResult, ImageSource, InitializeConfig, OcrRunOptions, OutputGranularity, RustO};

/// Lazily-initialized OCR engine. Loaded once on first use and reused for
/// every subsequent `recognize()` call, avoiding the cost of re-reading and
/// re-parsing the MNN model files each time.
static ENGINE: OnceLock<Mutex<RustO>> = OnceLock::new();

const DET_MODEL: &str = "det.mnn";
const REC_MODEL: &str = "rec.mnn";
const DICT_FILE: &str = "dict.txt";

/// Checks if all required model files exist and are non-empty.
fn is_valid_model_dir(dir: &Path) -> bool {
    [DET_MODEL, REC_MODEL, DICT_FILE]
        .iter()
        .all(|file| {
            dir.join(file)
                .metadata()
                .map(|m| m.len() > 0)
                .unwrap_or(false)
        })
}

fn default_models_dir() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(appdata)
        .join("pashari")
        .join("models")
}

fn find_models_dir() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let dir = parent.join("models");
            if is_valid_model_dir(&dir) {
                return Some(dir);
            }
        }
    }
    let appdata_dir = default_models_dir();
    if is_valid_model_dir(&appdata_dir) {
        return Some(appdata_dir);
    }
    let cwd_dir = PathBuf::from("models");
    if is_valid_model_dir(&cwd_dir) {
        return Some(cwd_dir);
    }
    None
}

/// Downloads a zip from `url` and extracts files matching `keep` into `dest`.
fn download_and_extract(
    url: &str,
    dest: &Path,
    keep: &[&str],
) -> Result<(), Box<dyn Error>> {
    let mut response = ureq::get(url).call()?;
    let mut zip_bytes = Vec::new();
    copy(
        &mut response.body_mut().as_reader(),
        &mut zip_bytes,
    )?;
    let mut archive = zip::ZipArchive::new(Cursor::new(zip_bytes))?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        let name = file.name();
        if name.ends_with('/') {
            continue;
        }
        let Some(filename) = Path::new(name).file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        if !keep.contains(&filename) {
            continue;
        }

        // Write to a temporary file first and then rename to prevent leaving
        // corrupted 0-byte or partial files if interrupted midway.
        let output_path = dest.join(filename);
        let temp_path = dest.join(format!("{filename}.downloading"));
        {
            let mut output = File::create(&temp_path)?;
            copy(&mut file, &mut output)?;
        }
        std::fs::rename(&temp_path, &output_path)?;
    }
    Ok(())
}

/// Ensures OCR models exist, downloading them if missing.
/// Uses PPOCRv6-Tiny for the language-agnostic text detection model
/// (`det.mnn`) and PPOCRv4-Japanese for Japanese-specific recognition
/// (`rec.mnn`, `dict.txt`).
pub fn ensure_models() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(dir) = find_models_dir() {
        return Ok(dir);
    }

    let models_dir = default_models_dir();
    std::fs::create_dir_all(&models_dir)?;
    println!(
        "OCRモデルが見つかりません。初回ダウンロード中（約11MB）: {}",
        models_dir.display()
    );

    // det.mnn: text detection (language-agnostic) from PPOCRv6-Tiny (~1.7MB)
    download_and_extract(
        "https://github.com/byrizki/rusto-rs/releases/download/v0.2.5/RustO-Models-PPOCRv6-Tiny.zip",
        &models_dir,
        &["det.mnn"],
    )?;

    // rec.mnn + dict.txt: Japanese text recognition from PPOCRv4-Japanese (~9.3MB)
    download_and_extract(
        "https://github.com/byrizki/rusto-rs/releases/download/v0.2.5/RustO-Models-PPOCRv4-Japanese.zip",
        &models_dir,
        &["rec.mnn", "dict.txt"],
    )?;

    println!("OCRモデルのダウンロードと展開が完了しました");
    Ok(models_dir)
}

fn get_engine() -> Result<&'static Mutex<RustO>, Box<dyn Error>> {
    if let Some(engine) = ENGINE.get() {
        return Ok(engine);
    }

    let models_dir = ensure_models()?;
    let det = models_dir.join(DET_MODEL);
    let rec = models_dir.join(REC_MODEL);
    let dict = models_dir.join(DICT_FILE);
    let config = InitializeConfig::ppv4(det, rec, dict);
    let initialized = RustO::initialize(config)?;

    // If another thread initialized concurrently, keep the first one.
    Ok(ENGINE.get_or_init(|| Mutex::new(initialized)))
}

/// Runs OCR on the selected screenshot pixels and returns the extracted text.
///
/// Takes the `Shot` by value so the pixel buffer can be handed directly to
/// the image encoder without cloning.
pub fn recognize(shot: Shot) -> Result<String, Box<dyn Error>> {
    // Encode image buffer to uncompressed BMP format outside the engine lock.
    let image = image::RgbaImage::from_raw(
        shot.width,
        shot.height,
        shot.rgba,
    )
    .ok_or("invalid shot image data")?;
    let mut bmp_bytes = Vec::new();
    image.write_to(
        &mut Cursor::new(&mut bmp_bytes),
        image::ImageFormat::Bmp,
    )?;

    // Acquire engine lock only during text detection.
    let engine_mutex = get_engine()?;
    let mut engine = engine_mutex
        .lock()
        .map_err(|e| format!("OCR engine lock poisoned: {e}"))?;

    let options = OcrRunOptions {
        output: OutputGranularity::Spatial,
        ..Default::default()
    };

    let result = engine.detect_text(
        &ImageSource::Bytes(bmp_bytes),
        &options,
    )?;

    let text = match result {
        DetectTextResult::Spatial(text) => text,
        DetectTextResult::Structured(items) => items
            .into_iter()
            .map(|item| item.text)
            .collect::<Vec<_>>()
            .join("\n"),
    };

    Ok(text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_models_dir_ends_with_models() {
        assert!(default_models_dir().ends_with("models"));
    }

    #[test]
    fn test_recognize_japanese() {
        let dir = ensure_models().expect("OCR models not found or failed to download");

        println!("Testing with models dir: {}", dir.display());

        let renderer =
            crate::ui::text::TextRenderer::load().expect("TextRenderer load");

        let width = 300;
        let height = 80;

        let mut pixels = vec![0x00FFFFFFu32; width * height];

        let mut canvas = crate::ui::Canvas {
            buf: &mut pixels,
            w: width,
            h: height,
            scale: 1.0,
        };

        let font_size = 28.0;
        let baseline = renderer.baseline_for_center(40.0, font_size);

        renderer.draw(
            &mut canvas,
            20.0,
            baseline,
            "こんにちは世界",
            font_size,
            0x000000,
        );

        // Convert 0x00RRGGBB pixels to RGBA bytes.
        let mut rgba = Vec::with_capacity(width * height * 4);

        for &pixel in &pixels {
            let r = ((pixel >> 16) & 0xFF) as u8;
            let g = ((pixel >> 8) & 0xFF) as u8;
            let b = (pixel & 0xFF) as u8;

            rgba.extend_from_slice(&[r, g, b, 255]);
        }

        let shot = Shot {
            rgba,
            width: width as u32,
            height: height as u32,
        };

        let result = recognize(shot).expect("recognize failed");

        println!("OCR recognized text: '{result}'");

        assert!(
            result.contains("こんにちは") || result.contains("世界"),
            "Unexpected OCR text: {result}"
        );
    }
}
