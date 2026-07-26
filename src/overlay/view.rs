//! A magnifier view centered on the cursor.
//!
//! The cursor stays at its 1:1 full-screen position (src and screen
//! coincide there), while the area around it is shown magnified by
//! `zoom`. The pixel directly under the cursor never moves.
//!
//!   src(S)    = center + (S - center) / zoom
//!   screen(P) = center + (P - center) * zoom
//!
//! `center` is the cursor position (where screen=src). As long as the
//! cursor is within the image, sampling the whole screen still stays
//! within the image (no black bars).

#[derive(Clone, Copy)]
pub struct View {
    pub zoom: f64,
    /// Magnification center (= cursor position; coincides in screen and src).
    pub center: (f64, f64),
}

impl View {
    pub fn screen_to_src(&self, sx: f64, sy: f64) -> (f64, f64) {
        (
            self.center.0 + (sx - self.center.0) / self.zoom,
            self.center.1 + (sy - self.center.1) / self.zoom,
        )
    }

    pub fn src_to_screen(&self, px: f64, py: f64) -> (f64, f64) {
        (
            self.center.0 + (px - self.center.0) * self.zoom,
            self.center.1 + (py - self.center.1) * self.zoom,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_pixel_is_fixed() {
        let v = View {
            zoom: 4.0,
            center: (100.0, 50.0),
        };
        // The center (directly under the cursor) never moves.
        assert_eq!(v.screen_to_src(100.0, 50.0), (100.0, 50.0));
        assert_eq!(v.src_to_screen(100.0, 50.0), (100.0, 50.0));
    }

    #[test]
    fn screen_src_round_trip() {
        let v = View {
            zoom: 4.0,
            center: (100.0, 50.0),
        };
        let (px, py) = v.screen_to_src(200.0, 150.0);
        let (sx, sy) = v.src_to_screen(px, py);
        assert!((sx - 200.0).abs() < 1e-9 && (sy - 150.0).abs() < 1e-9);
    }

    #[test]
    fn sampled_source_stays_in_image() {
        // With the cursor inside the image, sampling the whole screen stays within the image (no black bars).
        let v = View {
            zoom: 4.0,
            center: (10.0, 10.0),
        };
        let (l, t) = v.screen_to_src(0.0, 0.0);
        let (r, b) = v.screen_to_src(2560.0, 1440.0);
        assert!(l >= 0.0 && t >= 0.0);
        assert!(r <= 2560.0 && b <= 1440.0);
    }
}
