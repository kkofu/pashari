//! Capture tab: screenshot (PNG) save location.

use super::{
    ACCENT, Btn, CONTENT_X, SAVE_KINDS, SaveKind, Settings, TextCursor, preedit_caret_index,
    save_row_layout, theme_colors, with_preedit, x_for_char_index,
};
use crate::ui::text::TextRenderer;
use crate::ui::{Canvas, Rect};

const CAPTURE_SAVE_ROW_Y: usize = 88;

impl Settings {
    pub(super) fn buttons_capture(&self, sw: usize) -> Vec<(Btn, Rect)> {
        let mut v = Vec::new();
        let (png_kind, _) = SAVE_KINDS[0];
        let (path_rect, browse_rect) = save_row_layout(sw, CAPTURE_SAVE_ROW_Y);
        v.push((Btn::SaveDirField(png_kind), path_rect));
        v.push((Btn::Browse(png_kind), browse_rect));
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
    save_dir_focus: Option<SaveKind>,
    save_dir_buf: &str,
    save_dir_cursor: TextCursor,
    ime_preedit: &str,
    ime_preedit_cursor: Option<(usize, usize)>,
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
    let (png_kind, label) = SAVE_KINDS[0];
    let (path_rect, _) = save_row_layout(sw, CAPTURE_SAVE_ROW_Y);
    let baseline = t.baseline_for_center((CAPTURE_SAVE_ROW_Y + 13) as f32, 15.0);
    t.draw(canvas, CONTENT_X as f32, baseline, label, 15.0, DIM);

    let focused = save_dir_focus == Some(png_kind);
    let save_dir = &save_dirs[0];
    canvas.fill(path_rect, FIELD_BG);
    canvas.stroke(path_rect, if focused { ACCENT } else { 0x0080_8080 });
    let (shown, preedit_range) = if focused && !ime_preedit.is_empty() {
        let (display, range) = with_preedit(save_dir_buf, save_dir_cursor, ime_preedit);
        (display, Some(range))
    } else if focused {
        (save_dir_buf.to_string(), None)
    } else if save_dir.is_empty() {
        ("(default: Pictures/pashari)".to_string(), None)
    } else {
        (save_dir.clone(), None)
    };
    let path_color = if !focused && save_dir.is_empty() {
        DIM
    } else {
        TEXT
    };
    let path_text_x0 = path_rect.x0 as f32 + 8.0;
    let path_baseline = t.baseline_for_center((path_rect.y0 + path_rect.y1) as f32 / 2.0, 15.0);
    let (ascent, descent) = t.glyph_vextent(15.0);
    let caret_y0 = (path_baseline - ascent) as usize;
    let caret_y1 = (path_baseline - descent) as usize;
    if focused
        && ime_preedit.is_empty()
        && let Some((lo, hi)) = save_dir_cursor.selection()
    {
        let x0 = path_text_x0 + x_for_char_index(Some(t), &shown, 15.0, lo);
        let x1 = path_text_x0 + x_for_char_index(Some(t), &shown, 15.0, hi);
        canvas.fill(
            Rect {
                x0: x0 as usize,
                y0: caret_y0,
                x1: x1 as usize,
                y1: caret_y1,
            },
            TEXT_SELECTION_BG,
        );
    }
    t.draw_clipped(
        canvas,
        path_text_x0,
        path_baseline,
        &shown,
        15.0,
        path_color,
        path_rect,
    );
    if focused {
        let caret_idx = preedit_range.as_ref().map_or_else(
            || Some(save_dir_cursor.cursor),
            |range| preedit_caret_index(range.start, ime_preedit, ime_preedit_cursor),
        );
        if let Some(range) = preedit_range.as_ref() {
            let x0 = path_text_x0 + x_for_char_index(Some(t), &shown, 15.0, range.start);
            let x1 = path_text_x0 + x_for_char_index(Some(t), &shown, 15.0, range.end);
            let underline_y = caret_y1.min(path_rect.y1.saturating_sub(2));
            canvas.fill(
                Rect {
                    x0: x0 as usize,
                    y0: underline_y,
                    x1: x1 as usize,
                    y1: underline_y + 1,
                },
                ACCENT,
            );
        }
        if let Some(caret_idx) = caret_idx {
            let cx = (path_text_x0 + x_for_char_index(Some(t), &shown, 15.0, caret_idx)) as usize;
            canvas.fill(
                Rect {
                    x0: cx,
                    y0: caret_y0,
                    x1: cx + 1,
                    y1: caret_y1,
                },
                TEXT,
            );
        }
    }
}
