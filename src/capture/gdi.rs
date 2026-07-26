//! Recording a selected region that spans multiple monitors, via GDI
//! `BitBlt` polling.
//!
//! The Windows Graphics Capture path ([`super`]'s default, via
//! `windows-capture`) can only capture per-monitor, so this is the fallback
//! when the selected region crosses monitor boundaries. GDI's screen DC
//! treats the whole virtual desktop as one coordinate system (negative
//! values allowed), so a single `BitBlt` captures across monitors directly
//! with no need to composite multiple sources. Being CPU-based software
//! capture, it's heavier and runs at somewhat lower fps than the WGC path.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC, HGDIOBJ, ReleaseDC,
    SRCCOPY, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo,
};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
};

use super::click_ripple::{ClickTracker, unpack_color};
use super::{AudioSpec, RecordFormat, scale_pixels_nearest, scale_to_fit};

const GIF_SPEED: i32 = 30;

/// Polling interval from fps, shared by mp4/gif.
fn interval_for_fps(fps: u32) -> Duration {
    Duration::from_millis((1000 / fps.max(1) as u64).max(1))
}

/// GIF frame display time (centiseconds = 1/100 s) from fps.
fn gif_delay_cs(fps: u32) -> u16 {
    ((100 / fps.max(1)).max(1)) as u16
}

/// A running GDI recording thread; `stop()` signals it to stop and joins it.
pub struct GdiRecorder {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<Result<(), String>>,
}

impl GdiRecorder {
    /// Starts polling-based recording on a separate thread (`rect` is
    /// absolute virtual-desktop coordinates).
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        rect: (i32, i32, i32, i32),
        path: String,
        format: RecordFormat,
        fps: u32,
        audio: Option<AudioSpec>,
        show_cursor: bool,
        bitrate_mbps: u32,
        max_width: u32,
        max_height: u32,
        show_click_ripple: bool,
        click_color_left: u32,
        click_color_right: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if rect.2 <= rect.0 || rect.3 <= rect.1 {
            return Err("録画領域が不正です".into());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_th = stop.clone();
        let handle = std::thread::spawn(move || {
            run(
                rect,
                path,
                format,
                fps,
                audio,
                show_cursor,
                bitrate_mbps,
                max_width,
                max_height,
                show_click_ripple,
                click_color_left,
                click_color_right,
                stop_th,
            )
        });
        Ok(Self { stop, handle })
    }

    /// Signals a stop, waits for the thread to finish, and propagates its result.
    pub fn stop(self) -> Result<(), Box<dyn std::error::Error>> {
        self.stop.store(true, Ordering::SeqCst);
        match self.handle.join() {
            Ok(r) => r.map_err(|e| e.into()),
            Err(_) => Err("録画スレッドがパニックしました".into()),
        }
    }
}

/// The recording thread's body: sets up an encoder for the format, then
/// polls until the stop flag is set.
#[allow(clippy::too_many_arguments)]
fn run(
    rect: (i32, i32, i32, i32),
    path: String,
    format: RecordFormat,
    fps: u32,
    audio: Option<AudioSpec>,
    show_cursor: bool,
    bitrate_mbps: u32,
    max_width: u32,
    max_height: u32,
    show_click_ripple: bool,
    click_color_left: u32,
    click_color_right: u32,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut cap = GdiCapturer::new(
        rect,
        show_cursor,
        show_click_ripple,
        click_color_left,
        click_color_right,
    )?;
    match format {
        RecordFormat::Mp4 => run_mp4(
            &mut cap,
            &path,
            fps,
            bitrate_mbps,
            max_width,
            max_height,
            audio,
            &stop,
        ),
        RecordFormat::Gif => run_gif(&mut cap, &path, fps, max_width, max_height, &stop),
    }
}

/// Sends all buffered audio PCM to the encoder (same as
/// `RecordHandler::drain_audio`).
fn drain_audio(encoder: &mut VideoEncoder, rx: Option<&Receiver<Vec<u8>>>) -> Result<(), String> {
    if let Some(rx) = rx {
        while let Ok(chunk) = rx.try_recv() {
            encoder
                .send_audio_buffer(&chunk, 0)
                .map_err(|e| format!("音声送出に失敗: {e}"))?;
        }
    }
    Ok(())
}

/// Discards buffered audio PCM without sending it (drops the pre-roll before start).
fn discard_audio(rx: Option<&Receiver<Vec<u8>>>) {
    if let Some(rx) = rx {
        while rx.try_recv().is_ok() {}
    }
}

/// The mp4 polling loop. Applies the same conversion as
/// `RecordHandler::on_frame_arrived` (flip vertically, then
/// `send_frame_buffer`), driven by its own timer instead of a WGC callback.
#[allow(clippy::too_many_arguments)]
fn run_mp4(
    cap: &mut GdiCapturer,
    path: &str,
    fps: u32,
    bitrate_mbps: u32,
    max_width: u32,
    max_height: u32,
    audio: Option<AudioSpec>,
    stop: &Arc<AtomicBool>,
) -> Result<(), String> {
    let (w, h) = cap.size();
    let (ow, oh) = scale_to_fit(w, h, max_width, max_height);
    let audio_rx = audio.as_ref().map(|(_, rx)| rx);
    let audio_settings = match &audio {
        Some((fmt, _)) => AudioSettingsBuilder::new()
            .sample_rate(fmt.sample_rate)
            .channel_count(fmt.channels)
            .bit_per_sample(16),
        None => AudioSettingsBuilder::default().disabled(true),
    };
    let mut encoder = VideoEncoder::new(
        VideoSettingsBuilder::new(ow, oh)
            .frame_rate(fps.max(1))
            .bitrate(bitrate_mbps.max(1) * 1_000_000),
        audio_settings,
        ContainerSettingsBuilder::default(),
        path,
    )
    .map_err(|e| format!("エンコーダの初期化に失敗: {e}"))?;

    let row = w as usize * 4;
    let mut raw = Vec::new();
    let mut flipped = vec![0u8; row * h as usize];
    let mut resized = vec![0u8; ow as usize * oh as usize * 4];
    let started = Instant::now();
    // Drop pre-roll audio from before start, to keep A/V in sync (same as the WGC path).
    discard_audio(audio_rx);

    let interval = interval_for_fps(fps);
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now < next {
            std::thread::sleep(next - now);
        }
        next += interval;

        cap.capture_frame(&mut raw)?;
        // GdiCapturer returns top-down, so flip to the bottom-up order
        // send_frame_buffer expects (same logic as the WGC path).
        for y in 0..h as usize {
            let s = &raw[y * row..y * row + row];
            flipped[(h as usize - 1 - y) * row..(h as usize - y) * row].copy_from_slice(s);
        }
        drain_audio(&mut encoder, audio_rx)?;
        let timestamp = (started.elapsed().as_nanos() / 100) as i64;
        let buf = if (ow, oh) != (w, h) {
            scale_pixels_nearest(
                &flipped,
                w as usize,
                h as usize,
                &mut resized,
                ow as usize,
                oh as usize,
            );
            &resized
        } else {
            &flipped
        };
        encoder
            .send_frame_buffer(buf, timestamp)
            .map_err(|e| format!("フレーム送出に失敗: {e}"))?;
    }

    drain_audio(&mut encoder, audio_rx)?;
    encoder
        .finish()
        .map_err(|e| format!("mp4 の確定に失敗: {e}"))?;
    Ok(())
}

/// The gif polling loop: same frame-dropping and quantization as
/// `GifHandler::on_frame_arrived`.
fn run_gif(
    cap: &mut GdiCapturer,
    path: &str,
    fps: u32,
    max_width: u32,
    max_height: u32,
    stop: &Arc<AtomicBool>,
) -> Result<(), String> {
    let (w, h) = cap.size();
    let (ow, oh) = scale_to_fit(w, h, max_width, max_height);
    let file = std::fs::File::create(path).map_err(|e| format!("ファイル作成に失敗: {e}"))?;
    let mut encoder = gif::Encoder::new(std::io::BufWriter::new(file), ow as u16, oh as u16, &[])
        .map_err(|e| format!("GIF エンコーダの初期化に失敗: {e}"))?;
    encoder
        .set_repeat(gif::Repeat::Infinite)
        .map_err(|e| format!("GIF 設定に失敗: {e}"))?;

    let mut raw = Vec::new();
    let mut rgba = vec![0u8; w as usize * h as usize * 4];
    let mut resized = vec![0u8; ow as usize * oh as usize * 4];

    let interval = interval_for_fps(fps);
    let delay_cs = gif_delay_cs(fps);
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now < next {
            std::thread::sleep(next - now);
        }
        next += interval;

        cap.capture_frame(&mut raw)?;
        // GDI returns BGRA, so swap R/B for RGBA (alpha is fixed opaque).
        for (px, s) in rgba.chunks_exact_mut(4).zip(raw.chunks_exact(4)) {
            px[0] = s[2];
            px[1] = s[1];
            px[2] = s[0];
            px[3] = 255;
        }
        let (fw, fh, buf): (u16, u16, &mut Vec<u8>) = if (ow, oh) != (w, h) {
            scale_pixels_nearest(
                &rgba,
                w as usize,
                h as usize,
                &mut resized,
                ow as usize,
                oh as usize,
            );
            (ow as u16, oh as u16, &mut resized)
        } else {
            (w as u16, h as u16, &mut rgba)
        };
        let mut gframe = gif::Frame::from_rgba_speed(fw, fh, buf, GIF_SPEED);
        gframe.delay = delay_cs;
        encoder
            .write_frame(&gframe)
            .map_err(|e| format!("GIF フレーム書き出しに失敗: {e}"))?;
    }
    // Dropping encoder here writes the trailer (same as GifHandler in gif.rs).
    Ok(())
}

/// Captures the screen rect via `BitBlt` on each call, reusing its DC/bitmap.
struct GdiCapturer {
    screen_dc: HDC,
    mem_dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    rect: (i32, i32, i32, i32),
    w: i32,
    h: i32,
    info: BITMAPINFO,
    /// Whether to composite the mouse cursor in.
    show_cursor: bool,
    /// Click ripples (`None` = not baked in).
    click_tracker: Option<ClickTracker>,
}

impl GdiCapturer {
    fn new(
        rect: (i32, i32, i32, i32),
        show_cursor: bool,
        show_click_ripple: bool,
        click_color_left: u32,
        click_color_right: u32,
    ) -> Result<Self, String> {
        let (x0, y0, x1, y1) = rect;
        let (w, h) = (x1 - x0, y1 - y0);

        // SAFETY: read-only call, just gets the full-screen (NULL) DC.
        let screen_dc = unsafe { GetDC(None) };
        if screen_dc.is_invalid() {
            return Err("画面 DC の取得に失敗しました".into());
        }
        // SAFETY: just creates a new DC/bitmap compatible with screen_dc.
        let mem_dc = unsafe { CreateCompatibleDC(screen_dc) };
        let bitmap = unsafe { CreateCompatibleBitmap(screen_dc, w, h) };
        if mem_dc.is_invalid() || bitmap.is_invalid() {
            // SAFETY: just releases the screen_dc obtained above.
            unsafe {
                let _ = ReleaseDC(None, screen_dc);
            }
            return Err("キャプチャ用ビットマップの作成に失敗しました".into());
        }
        // SAFETY: just selects the created bitmap into the created mem_dc.
        let old_bitmap = unsafe { SelectObject(mem_dc, HGDIOBJ::from(bitmap)) };

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // Negative makes it a top-down DIB (the gif path then needs no row flip).
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        Ok(Self {
            screen_dc,
            mem_dc,
            bitmap,
            old_bitmap,
            rect,
            w,
            h,
            info,
            show_cursor,
            click_tracker: show_click_ripple.then(|| {
                ClickTracker::new(
                    unpack_color(click_color_left),
                    unpack_color(click_color_right),
                )
            }),
        })
    }

    fn size(&self) -> (u32, u32) {
        (self.w as u32, self.h as u32)
    }

    /// Captures one frame into `buf` as BGRA (top-down, no padding, 4 bytes/px).
    fn capture_frame(&mut self, buf: &mut Vec<u8>) -> Result<(), String> {
        let (x0, y0, _, _) = self.rect;
        // SAFETY: just a bit-block transfer from absolute screen coordinates
        // into the already-created compatible DC. GDI's screen DC treats
        // the whole virtual desktop as one coordinate system, so negative
        // coordinates pass through directly — a rect spanning monitors is
        // captured in a single BitBlt.
        unsafe {
            BitBlt(
                self.mem_dc,
                0,
                0,
                self.w,
                self.h,
                self.screen_dc,
                x0,
                y0,
                SRCCOPY,
            )
        }
        .map_err(|e| format!("BitBlt に失敗: {e}"))?;

        if self.show_cursor {
            self.draw_cursor(x0, y0);
        }

        let need = self.w as usize * self.h as usize * 4;
        if buf.len() != need {
            buf.resize(need, 0);
        }
        // SAFETY: just reads pixels from the already-allocated compatible
        // bitmap into the already-allocated buffer.
        let lines = unsafe {
            GetDIBits(
                self.mem_dc,
                self.bitmap,
                0,
                self.h as u32,
                Some(buf.as_mut_ptr().cast()),
                &mut self.info,
                DIB_RGB_COLORS,
            )
        };
        if lines == 0 {
            return Err("GetDIBits に失敗しました".into());
        }

        if let Some(tracker) = &mut self.click_tracker {
            tracker.poll();
            tracker.draw_onto(buf, self.w, self.h, (x0, y0), true, false);
        }
        Ok(())
    }

    /// Draws the cursor onto the compositing DC (no-op if not showing).
    fn draw_cursor(&self, ox: i32, oy: i32) {
        let mut info = CURSORINFO {
            cbSize: std::mem::size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: only queries the current cursor info.
        if unsafe { GetCursorInfo(&mut info) }.is_err() {
            return;
        }
        if info.flags != CURSOR_SHOWING {
            return;
        }
        let (cx, cy) = (info.ptScreenPos.x - ox, info.ptScreenPos.y - oy);
        // SAFETY: just draws the obtained cursor handle onto the
        // already-allocated compatible DC.
        unsafe {
            let _ = DrawIconEx(self.mem_dc, cx, cy, info.hCursor, 0, 0, 0, None, DI_NORMAL);
        }
    }
}

impl Drop for GdiCapturer {
    fn drop(&mut self) {
        // SAFETY: just releases every resource allocated in new() via its matching API.
        unsafe {
            let _ = SelectObject(self.mem_dc, self.old_bitmap);
            let _ = DeleteObject(self.bitmap);
            let _ = DeleteDC(self.mem_dc);
            let _ = ReleaseDC(None, self.screen_dc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_for_fps_matches_expected_values() {
        assert_eq!(interval_for_fps(30), Duration::from_millis(33));
        assert_eq!(interval_for_fps(60), Duration::from_millis(16));
        assert_eq!(interval_for_fps(15), Duration::from_millis(66));
    }

    #[test]
    fn interval_for_fps_guards_against_zero() {
        // fps=0 (an invalid setting) is treated as 1fps, without a division panic.
        assert_eq!(interval_for_fps(0), Duration::from_millis(1000));
    }

    #[test]
    fn gif_delay_cs_matches_expected_values() {
        assert_eq!(gif_delay_cs(15), 6);
        assert_eq!(gif_delay_cs(30), 3);
        assert_eq!(gif_delay_cs(60), 1);
    }

    #[test]
    fn gif_delay_cs_never_zero() {
        // Even at a high fps where 100/fps would be 0, use at least 1 centisecond.
        assert_eq!(gif_delay_cs(200), 1);
        // fps=0 (an invalid setting) is treated as 1fps.
        assert_eq!(gif_delay_cs(0), 100);
    }
}
