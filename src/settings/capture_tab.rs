//! Capture tab: screenshot (PNG) save location.

use super::{Btn, CONTENT_X, SAVE_KINDS, Settings, save_row_layout, theme_colors};
use crate::ui::text::TextRenderer;
use crate::ui::{Canvas, Rect};

const CAPTURE_SAVE_ROW_Y: usize = 88;

impl Settings {
    pub(super) fn buttons_capture(&self, sw: usize) -> Vec<(Btn, Rect)> {
        let mut v = Vec::new();
        let (png_kind, _) = SAVE_KINDS[0];
        let (_, browse_rect, default_rect) = save_row_layout(sw, CAPTURE_SAVE_ROW_Y);
        v.push((Btn::Browse(png_kind), browse_rect));
        v.push((Btn::DefaultDir(png_kind), default_rect));
        v
    }
}

#[allow(non_snake_case, unused_variables, clippy::too_many_arguments)]
pub(super) fn draw_capture(
    canvas: &mut Canvas,
    t: &TextRenderer,
    dark: bool,
    sw: usize,
    save_dirs: &[String; 3],
) {
    let (
        BG,
        SIDEBAR_BG,
        FIELD_BG,
        BTN_BG,
        TEXT,
        DIM,
        UPLOADER_ACTIVE_BG,
        TEXT_SELECTION_BG,
        PICK_BG,
        VERY_DIM,
        SWATCH_HOVER,
    ) = theme_colors(dark);

    t.draw(canvas, CONTENT_X as f32, 72.0, "Save to:", 15.0, DIM);
    let (_, label) = SAVE_KINDS[0];
    let (path_rect, _, _) = save_row_layout(sw, CAPTURE_SAVE_ROW_Y);
    let baseline = t.baseline_for_center((CAPTURE_SAVE_ROW_Y + 13) as f32, 15.0);
    t.draw(canvas, CONTENT_X as f32, baseline, label, 15.0, DIM);

    let save_dir = &save_dirs[0];
    let path_label = if save_dir.is_empty() {
        "(default: Pictures/pashari)".to_string()
    } else {
        save_dir.clone()
    };
    canvas.fill(path_rect, FIELD_BG);
    let path_color = if save_dir.is_empty() { DIM } else { TEXT };
    let path_baseline = t.baseline_for_center((path_rect.y0 + path_rect.y1) as f32 / 2.0, 15.0);
    t.draw(
        canvas,
        path_rect.x0 as f32 + 8.0,
        path_baseline,
        &path_label,
        15.0,
        path_color,
    );
}
