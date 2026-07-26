//! Region recording, saved as mp4 or gif.
//!
//! Captures a monitor via `windows-capture`, crops each frame to the
//! selected region, and encodes to mp4 (H264) or gif. Recording runs on a
//! separate thread via `start_free_threaded`; [`Recorder::stop`] stops it
//! and finalizes the file. Desktop audio and mic are held by
//! [`audio_session`] as a dynamic audio session that lives for the whole
//! recording, toggled on/off via [`Recorder::set_desktop_audio`]/
//! [`Recorder::set_mic`]. An mp4 that never actually used audio has its
//! audio track stripped by [`mp4_strip`] after stopping.

mod audio;
mod audio_session;
/// Re-exports the type so `overlay` can start a preview audio session
/// directly (for the volume indicator while setting up a recording),
/// without exposing the submodule layout itself.
pub(crate) use audio_session::AudioSession;
mod click_ripple;
mod gdi;
mod gif;
mod mic;
mod mixer;
mod mp4_strip;

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use audio::AudioFormat;
use click_ripple::{ClickTracker, unpack_color};
use gif::{GifFlags, GifHandler};

type HandlerError = Box<dyn std::error::Error + Send + Sync>;

/// Desktop-audio (render) device names, for the settings GUI's dropdown.
pub fn audio_output_device_names() -> Vec<String> {
    audio::output_device_names()
}

/// Microphone (input) device names, for the settings GUI's dropdown.
pub fn audio_input_device_names() -> Vec<String> {
    mic::input_device_names()
}

/// Recording output format.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RecordFormat {
    Mp4,
    Gif,
}

/// A recording request (caller → Recorder). The region is absolute
/// virtual-desktop coordinates (physical pixels, `x1`/`y1` exclusive;
/// negative values are possible depending on monitor layout), plus output
/// path, format, and whether desktop audio/mic are on (mp4 only).
/// Recording is limited to the single monitor containing the region's
/// center (a Graphics Capture API constraint); a selection spanning
/// monitors is clamped to that monitor's bounds.
pub struct RecordRequest {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub path: String,
    pub format: RecordFormat,
    pub desktop_audio: bool,
    pub mic: bool,
    pub fps: u32,
    /// Whether to show the mouse cursor.
    pub show_cursor: bool,
    /// MP4 bitrate in Mbps (ignored for gif, which has no such concept).
    pub bitrate_mbps: u32,
    /// Width cap in px; exceeding it shrinks the output preserving aspect
    /// ratio (shared by mp4/gif). 0 = unlimited.
    pub max_width: u32,
    /// Height cap in px (0 = unlimited).
    pub max_height: u32,
    /// Whether to show ripples expanding from click positions.
    pub show_click_ripple: bool,
    /// Left-click ripple color (`0x00RRGGBB`).
    pub click_color_left: u32,
    /// Right-click ripple color (`0x00RRGGBB`).
    pub click_color_right: u32,
    /// Desktop-audio loopback device name (empty = system default).
    pub audio_output_device: String,
    /// Microphone device name (empty = system default).
    pub audio_input_device: String,
    /// Target sample rate (Hz) for recorded audio; both desktop and mic are
    /// resampled to this rate before mixing.
    pub audio_sample_rate: u32,
    /// Whether to strip the mp4's audio track afterward if it stayed
    /// effectively silent throughout the recording (ignored for gif, which
    /// has no such concept).
    pub strip_silent_audio: bool,
}

/// Audio spec passed to the encoder: format plus PCM receiver.
type AudioSpec = (AudioFormat, Receiver<Vec<u8>>);

/// Internal flags passed to the handler (`Send` since they're moved to
/// another thread).
struct RecordFlags {
    crop: (u32, u32, u32, u32),
    path: String,
    /// (format, PCM receiver) when capturing desktop audio.
    audio: Option<(AudioFormat, Receiver<Vec<u8>>)>,
    fps: u32,
    bitrate_mbps: u32,
    max_width: u32,
    max_height: u32,
    /// Whether to bake in click ripples.
    show_click_ripple: bool,
    click_color_left: u32,
    click_color_right: u32,
    /// Absolute screen coordinates of the buffer's (0,0) pixel (monitor
    /// origin plus crop origin), used to transform ripple coordinates.
    capture_origin: (i32, i32),
}

/// Returns (width, height) shrunk to stay within `max_w`/`max_h` (each 0 =
/// unlimited) while preserving aspect ratio; returned unchanged (never
/// upscaled) if both already fit. An OS-independent pure function, shared
/// by all 4 paths (mp4/gif × WGC/GDI).
fn scale_to_fit(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let mut scale = 1.0f64;
    if max_w != 0 && w > max_w {
        scale = scale.min(max_w as f64 / w as f64);
    }
    if max_h != 0 && h > max_h {
        scale = scale.min(max_h as f64 / h as f64);
    }
    if scale >= 1.0 {
        return (w, h);
    }
    (
        ((w as f64 * scale).round() as u32).max(1),
        ((h as f64 * scale).round() as u32).max(1),
    )
}

/// Downscales `src` (`src_w x src_h`, 4 bytes/px; works for either
/// BGRA/RGBA) to `dst_w x dst_h` via nearest-neighbor sampling, writing
/// into `dst`. An OS-independent pure function.
fn scale_pixels_nearest(
    src: &[u8],
    src_w: usize,
    src_h: usize,
    dst: &mut [u8],
    dst_w: usize,
    dst_h: usize,
) {
    debug_assert_eq!(dst.len(), dst_w * dst_h * 4);
    for dy in 0..dst_h {
        let sy = (dy * src_h / dst_h).min(src_h.saturating_sub(1));
        for dx in 0..dst_w {
            let sx = (dx * src_w / dst_w).min(src_w.saturating_sub(1));
            let s = (sy * src_w + sx) * 4;
            let d = (dy * dst_w + dx) * 4;
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
}

struct RecordHandler {
    encoder: Option<VideoEncoder>,
    /// Crop rect (x0, y0, x1, y1).
    crop: (u32, u32, u32, u32),
    /// Timestamp of the first frame's arrival (used as pts=0); `None` before it arrives.
    started: Option<Instant>,
    /// Scratch buffer for `as_nopadding_buffer`.
    scratch: Vec<u8>,
    /// Buffer after vertical flip (bottom-to-top), same raw size as the crop rect.
    flipped: Vec<u8>,
    /// Output size after applying the resolution cap (equal to the crop
    /// rect's raw size if no downscaling is needed).
    out_w: u32,
    out_h: u32,
    /// Buffer downscaled to `out_w x out_h` (unused if no downscaling is needed).
    resized: Vec<u8>,
    /// Desktop-audio PCM receiver (`None` if disabled).
    audio_rx: Option<Receiver<Vec<u8>>>,
    /// Timestamp of the most recently accepted frame, for fps dropping.
    last_frame: Option<Instant>,
    /// Minimum interval between accepted frames, derived from fps.
    frame_interval: Duration,
    /// Click ripples (`None` = not baked in).
    click_tracker: Option<ClickTracker>,
    /// Absolute screen coordinates of the buffer's (0,0) pixel.
    capture_origin: (i32, i32),
}

impl RecordHandler {
    /// Sends all buffered audio PCM to the encoder.
    fn drain_audio(&mut self) -> Result<(), HandlerError> {
        if let Some(rx) = self.audio_rx.as_ref() {
            while let Ok(chunk) = rx.try_recv() {
                if let Some(enc) = self.encoder.as_mut() {
                    // The timestamp is ignored — pacing is derived monotonically from sample count.
                    enc.send_audio_buffer(&chunk, 0)?;
                }
            }
        }
        Ok(())
    }

    /// Discards buffered audio PCM without sending it (drops the pre-roll before start).
    fn discard_audio(&mut self) {
        if let Some(rx) = self.audio_rx.as_ref() {
            while rx.try_recv().is_ok() {}
        }
    }
}

impl GraphicsCaptureApiHandler for RecordHandler {
    type Flags = RecordFlags;
    type Error = HandlerError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let f = ctx.flags;
        let (x0, y0, x1, y1) = f.crop;
        let (w, h) = (x1 - x0, y1 - y0);
        let (out_w, out_h) = scale_to_fit(w, h, f.max_width, f.max_height);

        let audio_settings = match &f.audio {
            Some((fmt, _)) => AudioSettingsBuilder::new()
                .sample_rate(fmt.sample_rate)
                .channel_count(fmt.channels)
                .bit_per_sample(16),
            None => AudioSettingsBuilder::default().disabled(true),
        };
        let fps = f.fps.max(1);
        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(out_w, out_h)
                .frame_rate(fps)
                .bitrate(f.bitrate_mbps.max(1) * 1_000_000),
            audio_settings,
            ContainerSettingsBuilder::default(),
            &f.path,
        )?;

        Ok(Self {
            encoder: Some(encoder),
            crop: f.crop,
            started: None,
            scratch: Vec::new(),
            flipped: Vec::new(),
            out_w,
            out_h,
            resized: Vec::new(),
            audio_rx: f.audio.map(|(_, rx)| rx),
            last_frame: None,
            frame_interval: Duration::from_millis(1000 / fps as u64),
            click_tracker: f.show_click_ripple.then(|| {
                ClickTracker::new(
                    unpack_color(f.click_color_left),
                    unpack_color(f.click_color_right),
                )
            }),
            capture_origin: f.capture_origin,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let now = Instant::now();
        // Drop frames to hit fps; the first frame always passes, as the pts=0 baseline.
        if self.started.is_some()
            && let Some(last) = self.last_frame
            && now.duration_since(last) < self.frame_interval
        {
            return Ok(());
        }
        self.last_frame = Some(now);

        // Use the first frame as the pts=0 baseline, discarding any audio
        // before it as pre-roll to keep A/V in sync.
        let first = self.started.is_none();
        let started = *self.started.get_or_insert(now);
        if first {
            self.discard_audio();
        } else {
            self.drain_audio()?;
        }

        let (x0, y0, x1, y1) = self.crop;
        let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let row = w * 4;
        if self.flipped.len() != row * h {
            self.flipped.resize(row * h, 0);
        }

        {
            // Extract just the selected region (BGRA, top-to-bottom, no padding).
            let fb = frame.buffer_crop(x0, y0, x1, y1)?;
            let src = fb.as_nopadding_buffer(&mut self.scratch);
            // send_frame_buffer expects bottom-to-top, so flip the rows vertically.
            for y in 0..h {
                let s = &src[y * row..y * row + row];
                self.flipped[(h - 1 - y) * row..(h - y) * row].copy_from_slice(s);
            }
        }

        if let Some(tracker) = &mut self.click_tracker {
            tracker.poll();
            // self.flipped is already flipped (bottom-to-top), so pass
            // flip_y=true (origin stays in the original top-to-bottom coordinate system).
            tracker.draw_onto(
                &mut self.flipped,
                w as i32,
                h as i32,
                self.capture_origin,
                true,
                true,
            );
        }

        let timestamp = (started.elapsed().as_nanos() / 100) as i64;
        let buf = if self.out_w != w as u32 || self.out_h != h as u32 {
            let need = self.out_w as usize * self.out_h as usize * 4;
            if self.resized.len() != need {
                self.resized.resize(need, 0);
            }
            scale_pixels_nearest(
                &self.flipped,
                w,
                h,
                &mut self.resized,
                self.out_w as usize,
                self.out_h as usize,
            );
            &self.resized
        } else {
            &self.flipped
        };
        self.encoder
            .as_mut()
            .unwrap()
            .send_frame_buffer(buf, timestamp)?;
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        // Send any remaining audio, then finalize the encoder and write out the mp4.
        let _ = self.drain_audio();
        if let Some(encoder) = self.encoder.take() {
            encoder.finish()?;
        }
        Ok(())
    }
}

/// A running recording handle; `stop()` stops it and finalizes the file.
pub struct Recorder(RecorderInner);

/// Held as an enum since the handler type differs per format (private, so
/// the handler type stays hidden).
enum RecorderInner {
    Mp4 {
        control: CaptureControl<RecordHandler, HandlerError>,
        /// Audio session that lives for the whole recording (desktop/mic can
        /// be attached/detached on the fly).
        audio: audio_session::AudioSession,
        /// Output path, used for the post-hoc audio-track strip ([`mp4_strip`]).
        path: String,
        /// Device names passed straight through to
        /// `AudioSession::set_desktop`/`set_mic` when turned on during
        /// recording (empty = system default).
        audio_output_device: String,
        audio_input_device: String,
        /// Whether to strip the audio track afterward if the recording
        /// stayed effectively silent throughout.
        strip_silent_audio: bool,
    },
    Gif {
        control: CaptureControl<GifHandler, HandlerError>,
    },
    /// GDI fallback path for a selection spanning multiple monitors (shared
    /// by mp4/gif). `audio` is always `None` for gif, which doesn't support
    /// audio.
    Gdi {
        recorder: gdi::GdiRecorder,
        audio: Option<audio_session::AudioSession>,
        path: String,
        audio_output_device: String,
        audio_input_device: String,
        strip_silent_audio: bool,
    },
}

/// Builds the common Settings for a given monitor (only `flags` and the
/// color format differ per call site).
fn settings_for<F>(
    monitor: Monitor,
    color: ColorFormat,
    flags: F,
    show_cursor: bool,
) -> Result<Settings<F, Monitor>, Box<dyn std::error::Error>> {
    let cursor = if show_cursor {
        CursorCaptureSettings::WithCursor
    } else {
        CursorCaptureSettings::WithoutCursor
    };
    Ok(Settings::new(
        monitor,
        cursor,
        // Suppress the "capturing" yellow border (applies on Windows 11).
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Default,
        // Keep sending frames at ~60fps even when the screen is static.
        // Without this, no frames arrive until something changes, delaying
        // recording start and blocking audio output (mic-only recording
        // wouldn't work).
        MinimumUpdateIntervalSettings::Custom(Duration::from_millis(16)),
        DirtyRegionSettings::Default,
        color,
        flags,
    ))
}

/// Returns the monitor nearest absolute screen coordinates `(x, y)` and its
/// origin (absolute top-left in virtual-desktop coordinates).
/// `MONITOR_DEFAULTTONEAREST` means this never fails as long as at least
/// one monitor exists.
fn monitor_at(x: i32, y: i32) -> Result<(Monitor, (i32, i32)), Box<dyn std::error::Error>> {
    let point = POINT { x, y };
    // SAFETY: read-only call, just gets the nearest monitor handle for the given point.
    let hmonitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    if hmonitor.is_invalid() {
        return Err("モニタが見つかりません".into());
    }

    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: just queries the rect for the valid HMONITOR obtained above.
    if unsafe { !GetMonitorInfoW(hmonitor, &mut info).as_bool() } {
        return Err("モニタ情報の取得に失敗しました".into());
    }
    let origin = (info.rcMonitor.left, info.rcMonitor.top);
    // Build the Monitor directly from the same HMONITOR (rather than
    // relying on xcap's enumeration order) so it matches exactly what
    // windows-capture itself will capture.
    let monitor = Monitor::from_raw_hmonitor(hmonitor.0);
    Ok((monitor, origin))
}

/// Whether `req` fits entirely within `bounds` (an OS-independent pure
/// function), used to detect a selection spanning monitors.
fn rect_fits(req: (i32, i32, i32, i32), bounds: (i32, i32, i32, i32)) -> bool {
    req.0 >= bounds.0 && req.1 >= bounds.1 && req.2 <= bounds.2 && req.3 <= bounds.3
}

/// Converts an absolute virtual-desktop crop rect to coordinates relative
/// to the monitor's origin, clamped to the monitor's size. `None` if the
/// result has zero width/height (recording is limited to a single monitor,
/// so anything outside it is trimmed off). An OS-independent pure function.
fn clamp_to_monitor(
    req: (i32, i32, i32, i32),
    origin: (i32, i32),
    size: (u32, u32),
) -> Option<(u32, u32, u32, u32)> {
    let (mw, mh) = (size.0 as i32, size.1 as i32);
    let clamp = |v: i32, o: i32, max: i32| (v - o).clamp(0, max);
    let x0 = clamp(req.0, origin.0, mw);
    let y0 = clamp(req.1, origin.1, mh);
    let x1 = clamp(req.2, origin.0, mw);
    let y1 = clamp(req.3, origin.1, mh);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((x0 as u32, y0 as u32, x1 as u32, y1 as u32))
}

impl Recorder {
    /// Starts recording on a separate thread. Uses Windows Graphics Capture
    /// (the fast path) when the selected region fits within a single
    /// monitor, falling back to GDI polling — captured as-is, unclamped —
    /// when it spans monitors (see [`gdi`]).
    pub fn start(req: RecordRequest) -> Result<Self, Box<dyn std::error::Error>> {
        let req_abs = (req.x0, req.y0, req.x1, req.y1);
        let (cx, cy) = ((req.x0 + req.x1) / 2, (req.y0 + req.y1) / 2);
        let (monitor, origin) = monitor_at(cx, cy)?;
        let mon_size = (monitor.width()?, monitor.height()?);
        let mon_bounds = (
            origin.0,
            origin.1,
            origin.0 + mon_size.0 as i32,
            origin.1 + mon_size.1 as i32,
        );

        if !rect_fits(req_abs, mon_bounds) {
            return Self::start_gdi(req, req_abs);
        }

        let crop =
            clamp_to_monitor(req_abs, origin, mon_size).ok_or("録画領域がモニタ範囲外です")?;
        // Absolute screen coordinates of the buffer's (0,0) pixel (monitor origin plus crop origin).
        let capture_origin = (origin.0 + crop.0 as i32, origin.1 + crop.1 as i32);
        match req.format {
            RecordFormat::Mp4 => {
                let path = req.path.clone();
                // Always create the session regardless of the on/off state
                // at start, so desktop/mic can be toggled at any point
                // during recording.
                let (audio, fmt, rx) = audio_session::AudioSession::start(
                    req.desktop_audio,
                    req.mic,
                    &req.audio_output_device,
                    &req.audio_input_device,
                    req.audio_sample_rate,
                );
                let audio_output_device = req.audio_output_device.clone();
                let audio_input_device = req.audio_input_device.clone();
                let strip_silent_audio = req.strip_silent_audio;
                let flags = RecordFlags {
                    crop,
                    path: req.path,
                    audio: Some((fmt, rx)),
                    fps: req.fps,
                    bitrate_mbps: req.bitrate_mbps,
                    max_width: req.max_width,
                    max_height: req.max_height,
                    show_click_ripple: req.show_click_ripple,
                    click_color_left: req.click_color_left,
                    click_color_right: req.click_color_right,
                    capture_origin,
                };
                let settings = settings_for(monitor, ColorFormat::Bgra8, flags, req.show_cursor)?;
                let control = RecordHandler::start_free_threaded(settings)
                    .map_err(|e| format!("録画開始に失敗: {e}"))?;
                Ok(Recorder(RecorderInner::Mp4 {
                    control,
                    audio,
                    path,
                    audio_output_device,
                    audio_input_device,
                    strip_silent_audio,
                }))
            }
            RecordFormat::Gif => {
                let flags = GifFlags {
                    crop,
                    path: req.path,
                    fps: req.fps,
                    max_width: req.max_width,
                    max_height: req.max_height,
                    show_click_ripple: req.show_click_ripple,
                    click_color_left: req.click_color_left,
                    click_color_right: req.click_color_right,
                    capture_origin,
                };
                let settings = settings_for(monitor, ColorFormat::Rgba8, flags, req.show_cursor)?;
                let control = GifHandler::start_free_threaded(settings)
                    .map_err(|e| format!("録画開始に失敗: {e}"))?;
                Ok(Recorder(RecorderInner::Gif { control }))
            }
        }
    }

    /// Records a monitor-spanning selection via GDI polling (fallback path).
    fn start_gdi(
        req: RecordRequest,
        req_abs: (i32, i32, i32, i32),
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let path = req.path.clone();
        // gif has no audio, so only create the session for mp4.
        let (audio, gdi_audio) = if matches!(req.format, RecordFormat::Mp4) {
            let (session, fmt, rx) = audio_session::AudioSession::start(
                req.desktop_audio,
                req.mic,
                &req.audio_output_device,
                &req.audio_input_device,
                req.audio_sample_rate,
            );
            (Some(session), Some((fmt, rx)))
        } else {
            (None, None)
        };
        let audio_output_device = req.audio_output_device.clone();
        let audio_input_device = req.audio_input_device.clone();
        let strip_silent_audio = req.strip_silent_audio;
        let recorder = gdi::GdiRecorder::start(
            req_abs,
            req.path,
            req.format,
            req.fps,
            gdi_audio,
            req.show_cursor,
            req.bitrate_mbps,
            req.max_width,
            req.max_height,
            req.show_click_ripple,
            req.click_color_left,
            req.click_color_right,
        )?;
        Ok(Recorder(RecorderInner::Gdi {
            recorder,
            audio,
            path,
            audio_output_device,
            audio_input_device,
            strip_silent_audio,
        }))
    }

    /// Toggles desktop audio during recording (a no-op for gif, or any
    /// recording without an audio session).
    pub fn set_desktop_audio(&mut self, on: bool) {
        match &mut self.0 {
            RecorderInner::Mp4 {
                audio,
                audio_output_device,
                ..
            } => audio.set_desktop(on, audio_output_device),
            RecorderInner::Gdi {
                audio: Some(audio),
                audio_output_device,
                ..
            } => audio.set_desktop(on, audio_output_device),
            _ => {}
        }
    }

    /// Toggles the mic during recording (a no-op for gif, or any recording
    /// without an audio session).
    pub fn set_mic(&mut self, on: bool) {
        match &mut self.0 {
            RecorderInner::Mp4 {
                audio,
                audio_input_device,
                ..
            } => audio.set_mic(on, audio_input_device),
            RecorderInner::Gdi {
                audio: Some(audio),
                audio_input_device,
                ..
            } => audio.set_mic(on, audio_input_device),
            _ => {}
        }
    }

    /// Latest desktop/mic volume levels (0.0..=1.0); always (0.0, 0.0) for
    /// a recording with no audio session (e.g. gif). For the control bar's
    /// indicator.
    pub fn levels(&self) -> (f32, f32) {
        match &self.0 {
            RecorderInner::Mp4 { audio, .. } => audio.levels(),
            RecorderInner::Gdi {
                audio: Some(audio), ..
            } => audio.levels(),
            _ => (0.0, 0.0),
        }
    }

    /// Stops recording and finalizes the file. If `strip_silent_audio` is
    /// enabled and the mp4's audio stayed effectively silent throughout
    /// (whether it was never turned on, or was on but silent), strips the
    /// audio track afterward ([`mp4_strip`] — failure there doesn't fail
    /// the recording itself).
    pub fn stop(self) -> Result<(), Box<dyn std::error::Error>> {
        match self.0 {
            RecorderInner::Mp4 {
                control,
                audio,
                path,
                strip_silent_audio,
                ..
            } => {
                let silent = audio.was_silent();
                // Stop the audio pipeline (loopback/mic/combiner) before finalizing.
                audio.stop();
                control.stop().map_err(|e| format!("録画停止に失敗: {e}"))?;
                if strip_silent_audio && silent {
                    mp4_strip::strip_unused_audio(Path::new(&path));
                }
            }
            RecorderInner::Gif { control } => {
                control.stop().map_err(|e| format!("録画停止に失敗: {e}"))?;
            }
            RecorderInner::Gdi {
                recorder,
                audio,
                path,
                strip_silent_audio,
                ..
            } => {
                let silent = audio.as_ref().is_some_and(|a| a.was_silent());
                let has_audio = audio.is_some();
                if let Some(a) = audio {
                    a.stop();
                }
                recorder.stop()?;
                if has_audio && strip_silent_audio && silent {
                    mp4_strip::strip_unused_audio(Path::new(&path));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_fits_true_when_fully_inside() {
        assert!(rect_fits((100, 100, 500, 500), (0, 0, 1920, 1080)));
    }

    #[test]
    fn rect_fits_true_when_exactly_matches_bounds() {
        assert!(rect_fits((0, 0, 1920, 1080), (0, 0, 1920, 1080)));
    }

    #[test]
    fn rect_fits_false_when_exceeds_right_edge() {
        assert!(!rect_fits((1800, 100, 2000, 300), (0, 0, 1920, 1080)));
    }

    #[test]
    fn rect_fits_false_when_exceeds_left_edge_with_negative_bounds() {
        assert!(!rect_fits((-2000, 50, -100, 250), (-1920, 0, 0, 1080)));
    }

    #[test]
    fn clamp_to_monitor_converts_to_relative_coords() {
        // Monitor origin (1920,0), 1920x1080. Absolute (2000,100)-(2200,300) is entirely inside it.
        let r = clamp_to_monitor((2000, 100, 2200, 300), (1920, 0), (1920, 1080));
        assert_eq!(r, Some((80, 100, 280, 300)));
    }

    #[test]
    fn clamp_to_monitor_clamps_when_spanning_boundary() {
        // Monitor origin (0,0), 1920x1080. A selection crossing the right edge is cut off at it.
        let r = clamp_to_monitor((1800, 100, 2200, 300), (0, 0), (1920, 1080));
        assert_eq!(r, Some((1800, 100, 1920, 300)));
    }

    #[test]
    fn clamp_to_monitor_negative_origin() {
        // A monitor with a negative origin (a secondary placed to the left of the primary).
        let r = clamp_to_monitor((-1800, 50, -100, 250), (-1920, 0), (1920, 1080));
        assert_eq!(r, Some((120, 50, 1820, 250)));
    }

    #[test]
    fn clamp_to_monitor_none_when_fully_outside() {
        // A selection entirely outside the monitor ends up 0 width/height after clamping: None.
        let r = clamp_to_monitor((3000, 100, 3200, 300), (0, 0), (1920, 1080));
        assert_eq!(r, None);
    }

    #[test]
    fn scale_to_fit_disabled_when_both_max_are_zero() {
        assert_eq!(scale_to_fit(3840, 2160, 0, 0), (3840, 2160));
    }

    #[test]
    fn scale_to_fit_does_not_upscale_when_already_within_bounds() {
        assert_eq!(scale_to_fit(800, 600, 1920, 1920), (800, 600));
    }

    #[test]
    fn scale_to_fit_shrinks_preserving_aspect_ratio() {
        // 2560x1440 (16:9) capped at 1920: scaled 0.75x by the longer edge -> 1920x1080.
        assert_eq!(scale_to_fit(2560, 1440, 1920, 1920), (1920, 1080));
    }

    #[test]
    fn scale_to_fit_handles_extreme_aspect_ratio_without_div_by_zero() {
        // Even an extremely thin/long rect avoids division by zero and stays within both caps.
        let (w, h) = scale_to_fit(5000, 10, 1920, 1920);
        assert!(w <= 1920 && h <= 1920);
        assert!(w >= 1);
        assert!(h >= 1);
    }

    #[test]
    fn scale_to_fit_limits_width_only_when_height_is_unlimited() {
        // Only the width is capped at 1920; height is unlimited (0), so the
        // resulting height from preserving aspect ratio may exceed it.
        assert_eq!(scale_to_fit(3840, 1000, 1920, 0), (1920, 500));
    }

    #[test]
    fn scale_to_fit_limits_height_only_when_width_is_unlimited() {
        assert_eq!(scale_to_fit(1000, 3840, 0, 1920), (500, 1920));
    }

    #[test]
    fn scale_to_fit_applies_the_tighter_of_independent_width_and_height_limits() {
        // Width cap (1920) alone gives 0.5x; height cap (2000) alone gives
        // 0.833x. The tighter (smaller) factor, from the width cap, wins.
        assert_eq!(scale_to_fit(3840, 2400, 1920, 2000), (1920, 1200));
    }

    #[test]
    fn scale_pixels_nearest_downscales_2x2_to_1x1_picking_top_left_pixel() {
        // Downscaling a 2x2 of 4 colors (assumed BGRA, but content doesn't
        // matter) to 1x1: nearest-neighbor sampling picks (0,0).
        #[rustfmt::skip]
        let src: [u8; 2 * 2 * 4] = [
            10, 20, 30, 255,   40, 50, 60, 255,
            70, 80, 90, 255,   100, 110, 120, 255,
        ];
        let mut dst = [0u8; 4];
        scale_pixels_nearest(&src, 2, 2, &mut dst, 1, 1);
        assert_eq!(dst, [10, 20, 30, 255]);
    }

    #[test]
    fn scale_pixels_nearest_identity_when_same_size() {
        let src: [u8; 2 * 2 * 4] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let mut dst = [0u8; 2 * 2 * 4];
        scale_pixels_nearest(&src, 2, 2, &mut dst, 2, 2);
        assert_eq!(dst, src);
    }
}
