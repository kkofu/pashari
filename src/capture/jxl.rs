//! JXL (JPEG XL) output for region recording, via the external `cjxl`
//! reference encoder (assumed on PATH).
//!
//! Crops captured frames to the selected region. Since `cjxl` takes a
//! single animated input (not a raw frame stream), recording spools raw
//! RGBA frames to a temp dir (`frames.raw`, bounded RAM: only ~1 frame
//! resident), assembles a transient APNG at stop (`frames.apng`), then
//! runs `cjxl anim.apng out.jxl -d 1.0 -e 3` (visually lossless lossy).
//! Frames are paced to the target fps (`JxlFlags::fps`); the APNG carries
//! 1/fps per-frame delays so the JXL keeps the same timing.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;

use super::HandlerError;
use super::click_ripple::{ClickTracker, unpack_color};
use super::{scale_pixels_nearest, scale_to_fit};

/// Hard cap on retained JXL frames (e.g. 30 s at 30 fps). Bounds temp-dir
/// size plus the APNG-assembly/`cjxl` time at stop; frames past the cap are
/// dropped (the saved animation keeps the first N frames).
pub(crate) const MAX_JXL_FRAMES: u32 = 900;
/// The `cjxl` binary name (resolved via PATH).
pub(crate) const CJXL_BIN: &str = "cjxl";
/// Quality: `-d 1.0` = visually lossless (lossy VarDCT). Far smaller than
/// `-d 0` (mathematically lossless) on screen content, with no visible
/// difference in practice.
const CJXL_DISTANCE: &str = "1.0";
/// Encoder effort (3 = falcon; fast stop at some size cost).
const CJXL_EFFORT: &str = "3";

/// Argument vector for the `cjxl` invocation, split out for testability.
pub(crate) fn cjxl_args(apng: &Path, out: &Path) -> Vec<String> {
    vec![
        apng.to_string_lossy().into_owned(),
        out.to_string_lossy().into_owned(),
        "-d".into(),
        CJXL_DISTANCE.into(),
        "-e".into(),
        CJXL_EFFORT.into(),
    ]
}

/// Fails fast when `cjxl` can't be run (called at recording start so a
/// minutes-long recording doesn't end in "binary not found").
pub(crate) fn check_cjxl_available() -> Result<(), String> {
    match Command::new(CJXL_BIN).arg("--version").output() {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let detail: String = String::from_utf8_lossy(&out.stderr).chars().take(300).collect();
            Err(format!("cjxl が使えません: {detail}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!(
            "{CJXL_BIN} が見つかりません。libjxl-tools (cjxl) をインストールして PATH を通してください"
        )),
        Err(e) => Err(format!("cjxl の確認に失敗: {e}")),
    }
}

/// Runs `cjxl <apng> <out.jxl> -d 1.0 -e 3`, capturing stderr for diagnostics.
pub(crate) fn run_cjxl(bin: &str, apng: &Path, out: &Path) -> Result<(), String> {
    let mut command = Command::new(bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = command
        .args(cjxl_args(apng, out))
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("{bin} が見つかりません。libjxl-tools (cjxl) をインストールして PATH を通してください")
            } else {
                format!("cjxl の起動に失敗: {e}")
            }
        })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr: String = String::from_utf8_lossy(&output.stderr)
        .chars()
        .take(500)
        .collect();
    Err(format!("cjxl が失敗 ({}): {stderr}", output.status))
}

/// Flags passed to the JXL handler.
pub struct JxlFlags {
    pub crop: (u32, u32, u32, u32),
    pub fps: u32,
    /// Width cap in px (0 = unlimited).
    pub max_width: u32,
    /// Height cap in px (0 = unlimited).
    pub max_height: u32,
    /// Whether to bake in click ripples.
    pub show_click_ripple: bool,
    pub click_color_left: u32,
    pub click_color_right: u32,
    /// Absolute screen coordinates of the buffer's (0,0) pixel.
    pub capture_origin: (i32, i32),
}

/// Shared recording state: spools raw frames during capture, assembles the
/// transient APNG and runs `cjxl` upon closing or stop.
pub struct JxlShared {
    path: PathBuf,
    fps: u32,
    out_w: u32,
    out_h: u32,
    dir: PathBuf,
    spool_path: PathBuf,
    spool: Option<File>,
    frames: u32,
    truncated: bool,
    saved: bool,
}

static JXL_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl JxlShared {
    pub(crate) fn new(path: String, fps: u32) -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!(
            "pashari-jxl-{}-{}",
            std::process::id(),
            JXL_TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("JXL 一時フォルダの作成に失敗: {e}"))?;
        let spool_path = dir.join("frames.raw");
        let spool = File::create(&spool_path)
            .map_err(|e| format!("JXL 一時ファイルの作成に失敗: {e}"))?;
        Ok(Self {
            path: PathBuf::from(path),
            fps,
            out_w: 0,
            out_h: 0,
            dir,
            spool_path,
            spool: Some(spool),
            frames: 0,
            truncated: false,
            saved: false,
        })
    }

    /// Spools one RGBA frame (fast append, no compression on the hot path).
    /// Past the cap the frame is dropped and a one-time warning is logged.
    pub(crate) fn push_frame(&mut self, buf: &[u8]) -> Result<(), HandlerError> {
        if self.frames >= MAX_JXL_FRAMES {
            if !self.truncated {
                self.truncated = true;
                eprintln!(
                    "JXL 録画が上限 ({MAX_JXL_FRAMES} フレーム) に達したため以降のフレームを破棄します"
                );
            }
            return Ok(());
        }
        let spool = self
            .spool
            .as_mut()
            .ok_or("JXL スプールは確定後に書き込めません")?;
        spool
            .write_all(buf)
            .map_err(|e| format!("JXL フレームの書き込みに失敗: {e}"))?;
        self.frames += 1;
        Ok(())
    }

    fn apng_path(&self) -> PathBuf {
        self.dir.join("frames.apng")
    }

    pub(crate) fn set_output_size(&mut self, w: u32, h: u32) {
        self.out_w = w;
        self.out_h = h;
    }

    /// Streams the spool into a transient APNG (one frame resident at a
    /// time). `set_animated` needs the count upfront, which is why the
    /// spool-then-assemble split exists instead of direct streaming.
    fn assemble_apng(&self) -> Result<PathBuf, HandlerError> {
        let apng_path = self.apng_path();
        let file =
            File::create(&apng_path).map_err(|e| format!("APNG 一時ファイルの作成に失敗: {e}"))?;
        let mut encoder = png::Encoder::new(file, self.out_w, self.out_h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // Transient file: cjxl does the real compression, so be fast here.
        encoder.set_compression(png::Compression::Fast);
        encoder
            .set_animated(self.frames, 0)
            .map_err(|e| format!("APNG の初期化に失敗: {e}"))?;
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("APNG ヘッダの書き込みに失敗: {e}"))?;
        let stride = self.out_w as usize * self.out_h as usize * 4;
        let mut spool =
            File::open(&self.spool_path).map_err(|e| format!("JXL スプールの読み戻しに失敗: {e}"))?;
        let mut frame = vec![0u8; stride];
        // fps fits u16 for any realistic preset; clamp defensively.
        let delay_den = self.fps.clamp(1, 1000) as u16;
        for _ in 0..self.frames {
            spool
                .read_exact(&mut frame)
                .map_err(|e| format!("JXL スプールの読み戻しに失敗: {e}"))?;
            writer
                .set_frame_delay(1, delay_den)
                .map_err(|e| format!("APNG フレーム遅延の設定に失敗: {e}"))?;
            writer
                .write_image_data(&frame)
                .map_err(|e| format!("APNG フレームの書き込みに失敗: {e}"))?;
        }
        writer
            .finish()
            .map_err(|e| format!("APNG の確定に失敗: {e}"))?;
        Ok(apng_path)
    }

    fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    pub fn save_to_file(&mut self) -> Result<(), HandlerError> {
        if self.saved {
            return Ok(());
        }
        if self.frames == 0 {
            self.cleanup();
            return Err("JXL 用のフレームがありません（録画が短すぎます）".into());
        }
        if self.truncated {
            eprintln!("JXL 録画が上限で切り詰められました（{} フレームを保存）", self.frames);
        }
        if let Some(mut spool) = self.spool.take() {
            spool.flush()?;
        }
        let apng = self.assemble_apng()?;
        if let Err(e) = run_cjxl(CJXL_BIN, &apng, &self.path) {
            return Err(e.into());
        }
        self.saved = true;
        self.cleanup();
        Ok(())
    }
}

impl Drop for JxlShared {
    fn drop(&mut self) {
        // Best-effort: covers start-up failures and empty recordings; a
        // no-op when save_to_file already cleaned up.
        self.cleanup();
    }
}

pub struct JxlHandler {
    crop: (u32, u32, u32, u32),
    w: u32,
    h: u32,
    out_w: u32,
    out_h: u32,
    last: Option<Instant>,
    interval: Duration,
    scratch: Vec<u8>,
    rgba: Vec<u8>,
    resized: Vec<u8>,
    click_tracker: Option<ClickTracker>,
    capture_origin: (i32, i32),
    shared: Arc<Mutex<JxlShared>>,
}

impl GraphicsCaptureApiHandler for JxlHandler {
    type Flags = (JxlFlags, Arc<Mutex<JxlShared>>);
    type Error = HandlerError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let (f, shared) = ctx.flags;
        let (x0, y0, x1, y1) = f.crop;
        let (w, h) = (x1 - x0, y1 - y0);
        let (out_w, out_h) = scale_to_fit(w, h, f.max_width, f.max_height);

        {
            let mut s = shared.lock().map_err(|e| e.to_string())?;
            s.set_output_size(out_w, out_h);
        }

        let fps = f.fps.max(1);
        Ok(Self {
            crop: f.crop,
            w,
            h,
            out_w,
            out_h,
            last: None,
            interval: Duration::from_millis((1000 / fps as u64).max(1)),
            scratch: Vec::new(),
            rgba: Vec::new(),
            resized: Vec::new(),
            click_tracker: f.show_click_ripple.then(|| {
                ClickTracker::new(
                    unpack_color(f.click_color_left),
                    unpack_color(f.click_color_right),
                )
            }),
            capture_origin: f.capture_origin,
            shared,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let now = Instant::now();
        if let Some(last) = self.last
            && now.duration_since(last) < self.interval
        {
            return Ok(());
        }

        let (x0, y0, x1, y1) = self.crop;
        let fb = frame.buffer_crop(x0, y0, x1, y1)?;
        let src = fb.as_nopadding_buffer(&mut self.scratch);
        self.rgba.clear();
        self.rgba.extend_from_slice(src);

        if let Some(tracker) = &mut self.click_tracker {
            tracker.poll();
            tracker.draw_onto(
                &mut self.rgba,
                self.w as i32,
                self.h as i32,
                self.capture_origin,
                false,
                false,
            );
        }

        let buf: &[u8] = if self.out_w != self.w || self.out_h != self.h {
            let need = self.out_w as usize * self.out_h as usize * 4;
            if self.resized.len() != need {
                self.resized.resize(need, 0);
            }
            scale_pixels_nearest(
                &self.rgba,
                self.w as usize,
                self.h as usize,
                &mut self.resized,
                self.out_w as usize,
                self.out_h as usize,
            );
            &self.resized
        } else {
            &self.rgba
        };

        let mut shared = self.shared.lock().map_err(|e| e.to_string())?;
        shared.push_frame(buf)?;

        self.last = Some(now);
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        let mut shared = self.shared.lock().map_err(|e| e.to_string())?;
        shared.save_to_file()
    }
}

pub struct JxlRecorderHandle {
    shared: Arc<Mutex<JxlShared>>,
}

impl JxlRecorderHandle {
    pub fn new(path: String, fps: u32) -> Result<(Self, Arc<Mutex<JxlShared>>), String> {
        let shared = Arc::new(Mutex::new(JxlShared::new(path, fps)?));
        Ok((
            Self {
                shared: shared.clone(),
            },
            shared,
        ))
    }

    pub fn finalize(&self) -> Result<(), HandlerError> {
        let mut s = self.shared.lock().map_err(|e| e.to_string())?;
        s.save_to_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_shared(w: u32, h: u32, fps: u32) -> JxlShared {
        let mut s = JxlShared::new("out.jxl".into(), fps).expect("temp dir を作れるはず");
        s.out_w = w;
        s.out_h = h;
        s
    }

    #[test]
    fn push_frame_caps_at_max_frames() {
        let mut s = test_shared(2, 2, 30);
        for _ in 0..MAX_JXL_FRAMES + 10 {
            s.push_frame(&[0u8; 16]).expect("cap 内は成功するはず");
        }
        assert_eq!(s.frames, MAX_JXL_FRAMES);
        assert!(s.truncated);
    }

    #[test]
    fn save_fails_without_frames() {
        let mut s = test_shared(2, 2, 30);
        assert!(s.save_to_file().is_err());
        assert!(!s.saved);
    }

    #[test]
    fn assemble_apng_produces_valid_animated_png() {
        let mut s = test_shared(2, 1, 30);
        // 2x1 RGBA x2 frames with distinct pixels.
        s.push_frame(&[255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
        s.push_frame(&[0, 0, 255, 255, 255, 255, 255, 255]).unwrap();
        if let Some(mut spool) = s.spool.take() {
            spool.flush().unwrap();
        }
        let apng = s.assemble_apng().expect("APNG を組み立てられるはず");
        let bytes = std::fs::read(&apng).unwrap();
        // PNG signature.
        assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        // Must contain acTL (animation control).
        assert!(bytes.windows(4).any(|w| w == b"acTL"));
        assert!(bytes.len() > 100);
    }

    #[test]
    fn cjxl_args_use_visually_lossless_quality() {
        let args = cjxl_args(Path::new("a.apng"), Path::new("b.jxl"));
        let pos = args.iter().position(|a| a == "-d").expect("-d があるはず");
        assert_eq!(args[pos + 1], "1.0");
    }

    #[test]
    fn run_cjxl_reports_missing_binary() {
        let err = run_cjxl("pashari-definitely-not-a-binary", Path::new("a.apng"), Path::new("b.jxl"))
            .expect_err("存在しないバイナリはエラーになるはず");
        assert!(err.contains("見つかりません"), "unexpected: {err}");
    }
}
