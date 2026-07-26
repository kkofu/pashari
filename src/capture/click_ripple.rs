//! Detects left/right mouse clicks during recording and bakes an expanding,
//! fading ring at the click position into each captured frame.
//!
//! No hooks — just polls `GetAsyncKeyState` inside the existing per-frame
//! callback (no dedicated thread or message pump needed, fewer failure modes).

use std::time::Instant;

use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// Ripple lifetime in ms.
const RIPPLE_DURATION_MS: u128 = 500;
/// Start/end radius in px.
const MIN_RADIUS: f64 = 6.0;
const MAX_RADIUS: f64 = 40.0;
/// Ring thickness in px.
const STROKE_WIDTH: f64 = 3.0;
/// Starting alpha (0-255); fades linearly to 0 by the end.
const MAX_ALPHA: f32 = 180.0;
/// Anti-aliasing feather width in px, applied to both the inner and outer
/// edge so the ring looks smooth rather than a hard-pixel outline.
const AA_WIDTH: f64 = 1.0;

/// Unpacks `0x00RRGGBB` into `(r, g, b)` (the format `Config`'s color
/// settings are stored in).
pub(super) fn unpack_color(c: u32) -> (u8, u8, u8) {
    (
        ((c >> 16) & 0xff) as u8,
        ((c >> 8) & 0xff) as u8,
        (c & 0xff) as u8,
    )
}

struct Ripple {
    /// Click position (absolute screen coordinates).
    pos: (i32, i32),
    started: Instant,
    /// The color at the moment of the click (a later settings change doesn't
    /// affect existing ripples).
    color: (u8, u8, u8),
}

/// Unshared state owned by each recording handler
/// (`RecordHandler`/`GifHandler`/`GdiCapturer`) individually; never shared
/// across threads.
pub(super) struct ClickTracker {
    left_color: (u8, u8, u8),
    right_color: (u8, u8, u8),
    /// Whether each button was down on the previous poll — kept separately
    /// from `GetAsyncKeyState`'s low bit so click edges are also detected
    /// independently.
    was_down_left: bool,
    was_down_right: bool,
    ripples: Vec<Ripple>,
}

impl ClickTracker {
    pub(super) fn new(left_color: (u8, u8, u8), right_color: (u8, u8, u8)) -> Self {
        Self {
            left_color,
            right_color,
            was_down_left: false,
            was_down_right: false,
            ripples: Vec::new(),
        }
    }

    /// Call once per frame. Adds a ripple on a detected click, and prunes
    /// ripples past their lifetime.
    pub(super) fn poll(&mut self) {
        let left_clicked = Self::poll_button(VK_LBUTTON.0 as i32, &mut self.was_down_left);
        let right_clicked = Self::poll_button(VK_RBUTTON.0 as i32, &mut self.was_down_right);

        if (left_clicked || right_clicked)
            && let Some(pos) = Self::cursor_pos()
        {
            if left_clicked {
                self.ripples.push(Ripple {
                    pos,
                    started: Instant::now(),
                    color: self.left_color,
                });
            }
            if right_clicked {
                self.ripples.push(Ripple {
                    pos,
                    started: Instant::now(),
                    color: self.right_color,
                });
            }
        }

        self.ripples
            .retain(|r| r.started.elapsed().as_millis() < RIPPLE_DURATION_MS);
    }

    /// Detects a press of one virtual key. `was_down` is the state from the
    /// previous poll (kept per-button by the caller).
    fn poll_button(vk: i32, was_down: &mut bool) -> bool {
        // SAFETY: only queries the current button state.
        let state = unsafe { GetAsyncKeyState(vk) };
        let is_down = (state as u16 & 0x8000) != 0;
        // The low bit ("pressed since the last call") is global state shared
        // with any other process polling the same virtual key, so it's
        // OR'd with our own edge detection (is_down && !was_down) for
        // redundancy.
        let low_bit_hit = (state & 1) != 0;
        let clicked = low_bit_hit || (is_down && !*was_down);
        *was_down = is_down;
        clicked
    }

    fn cursor_pos() -> Option<(i32, i32)> {
        let mut point = POINT::default();
        // SAFETY: only queries the current cursor position.
        unsafe { GetCursorPos(&mut point) }
            .ok()
            .map(|()| (point.x, point.y))
    }

    /// Composites active ripples onto `buf` (4 bytes/pixel: the first 3
    /// bytes are color, the 4th is untouched since video has no real
    /// alpha). `origin` is the absolute screen coordinates of the buffer's
    /// (0,0) pixel, top-to-bottom (i.e. `origin` is the topmost row's
    /// coordinate). If `flip_y` is true, the buffer itself is treated as
    /// bottom-to-top and Y is flipped when drawing (`origin` still refers
    /// to the topmost row in the original top-to-bottom coordinate system)
    /// — used for the MP4 path's flipped buffer.
    pub(super) fn draw_onto(
        &self,
        buf: &mut [u8],
        w: i32,
        h: i32,
        origin: (i32, i32),
        bgr: bool,
        flip_y: bool,
    ) {
        for r in &self.ripples {
            let elapsed = r.started.elapsed().as_millis().min(RIPPLE_DURATION_MS) as f64;
            let t = elapsed / RIPPLE_DURATION_MS as f64;
            let radius = MIN_RADIUS + t * (MAX_RADIUS - MIN_RADIUS);
            let alpha = MAX_ALPHA * (1.0 - t) as f32;
            if alpha <= 0.0 {
                continue;
            }
            let cx = r.pos.0 - origin.0;
            let cy_top = r.pos.1 - origin.1;
            let cy = if flip_y { h - 1 - cy_top } else { cy_top };
            let r0 = (radius - STROKE_WIDTH).max(0.0);
            let r1 = radius;
            let bbox = (r1 + AA_WIDTH).ceil() as i32 + 1;

            for dy in -bbox..=bbox {
                let y = cy + dy;
                if y < 0 || y >= h {
                    continue;
                }
                for dx in -bbox..=bbox {
                    let x = cx + dx;
                    if x < 0 || x >= w {
                        continue;
                    }
                    let d = ((dx * dx + dy * dy) as f64).sqrt();
                    let coverage = ring_coverage(d, r0, r1);
                    if coverage <= 0.0 {
                        continue;
                    }
                    let px_alpha = alpha * coverage;
                    let idx = (y as usize * w as usize + x as usize) * 4;
                    let (ri, gi, bi) = if bgr {
                        (idx + 2, idx + 1, idx)
                    } else {
                        (idx, idx + 1, idx + 2)
                    };
                    blend(&mut buf[ri], r.color.0, px_alpha);
                    blend(&mut buf[gi], r.color.1, px_alpha);
                    blend(&mut buf[bi], r.color.2, px_alpha);
                }
            }
        }
    }
}

/// Coverage (0.0-1.0) of a ring (inner radius `r0`, outer radius `r1`) at
/// distance `d` from the center. Both edges fade linearly over `AA_WIDTH`
/// for anti-aliasing (same idea as `Canvas::blend_i`'s coverage blending).
fn ring_coverage(d: f64, r0: f64, r1: f64) -> f32 {
    let outer = ((r1 + AA_WIDTH - d) / AA_WIDTH).clamp(0.0, 1.0);
    let inner = ((d - (r0 - AA_WIDTH)) / AA_WIDTH).clamp(0.0, 1.0);
    outer.min(inner) as f32
}

fn blend(dst: &mut u8, src: u8, alpha: f32) {
    let a = alpha / 255.0;
    *dst = (*dst as f32 * (1.0 - a) + src as f32 * a)
        .round()
        .clamp(0.0, 255.0) as u8;
}
