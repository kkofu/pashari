//! GIF output for region recording.
//!
//! Crops captured frames to the selected region and encodes them to GIF
//! with the `gif` crate. Frames are dropped to the target fps
//! (`GifFlags::fps`) and palette-quantized with `Frame::from_rgba_speed`
//! (real-time, no audio).

use std::fs::File;
use std::io::BufWriter;
use std::time::{Duration, Instant};

use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;

use super::HandlerError;
use super::click_ripple::{ClickTracker, unpack_color};
use super::{scale_pixels_nearest, scale_to_fit};

/// Palette-quantization speed (1..=30; 30 is fastest/lowest quality).
const GIF_SPEED: i32 = 30;

/// Flags passed to the GIF handler.
pub struct GifFlags {
    pub crop: (u32, u32, u32, u32),
    pub path: String,
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

pub struct GifHandler {
    encoder: Option<gif::Encoder<BufWriter<File>>>,
    crop: (u32, u32, u32, u32),
    w: u16,
    h: u16,
    /// Output size after applying the resolution cap (equal to `w`/`h` if no
    /// downscaling is needed).
    out_w: u16,
    out_h: u16,
    /// Timestamp of the last written frame, for pacing.
    last: Option<Instant>,
    /// Frame-pacing interval, derived from fps.
    interval: Duration,
    /// GIF frame display time in centiseconds (1/100 s), derived from fps.
    delay_cs: u16,
    /// Scratch buffer for `as_nopadding_buffer`.
    scratch: Vec<u8>,
    /// RGBA copy passed to quantization (raw size).
    rgba: Vec<u8>,
    /// Buffer downscaled to `out_w x out_h` (unused if no downscaling is needed).
    resized: Vec<u8>,
    /// Click ripples (`None` = not baked in).
    click_tracker: Option<ClickTracker>,
    /// Absolute screen coordinates of the buffer's (0,0) pixel.
    capture_origin: (i32, i32),
}

impl GraphicsCaptureApiHandler for GifHandler {
    type Flags = GifFlags;
    type Error = HandlerError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let f = ctx.flags;
        let (x0, y0, x1, y1) = f.crop;
        let (w, h) = ((x1 - x0) as u16, (y1 - y0) as u16);
        let (out_w, out_h) = scale_to_fit(w as u32, h as u32, f.max_width, f.max_height);
        let (out_w, out_h) = (out_w as u16, out_h as u16);

        let file = File::create(&f.path)?;
        let mut encoder = gif::Encoder::new(BufWriter::new(file), out_w, out_h, &[])?;
        encoder.set_repeat(gif::Repeat::Infinite)?;

        let fps = f.fps.max(1);
        Ok(Self {
            encoder: Some(encoder),
            crop: f.crop,
            w,
            h,
            out_w,
            out_h,
            last: None,
            interval: Duration::from_millis((1000 / fps as u64).max(1)),
            delay_cs: ((100 / fps).max(1)) as u16,
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
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        // Drop frames to hit the target fps.
        let now = Instant::now();
        if let Some(last) = self.last
            && now.duration_since(last) < self.interval
        {
            return Ok(());
        }

        let (x0, y0, x1, y1) = self.crop;
        let fb = frame.buffer_crop(x0, y0, x1, y1)?;
        // ColorFormat::Rgba8, so this is already RGBA, top-to-bottom.
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

        let (fw, fh, buf): (u16, u16, &mut Vec<u8>) =
            if self.out_w != self.w || self.out_h != self.h {
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
                (self.out_w, self.out_h, &mut self.resized)
            } else {
                (self.w, self.h, &mut self.rgba)
            };
        let mut gframe = gif::Frame::from_rgba_speed(fw, fh, buf, GIF_SPEED);
        gframe.delay = self.delay_cs;
        if let Some(enc) = self.encoder.as_mut() {
            enc.write_frame(&gframe)?;
        }
        self.last = Some(now);
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        // Dropping the encoder writes the trailer and flushes the BufWriter.
        self.encoder.take();
        Ok(())
    }
}
