//! Editor tab: external editor path, Recent tab session limit.

use winit::keyboard::{Key, NamedKey};

use super::{
    ACCENT, Btn, CONTENT_X, Settings, SettingsResult, TextCursor, apply_common_edit_key,
    char_index_for_x, field, hover_tint_for, theme_colors, x_for_char_index,
};
use crate::ui::text::TextRenderer;
use crate::ui::{Canvas, Rect};

/// Session limit row: label + stepper (-/value/+), same layout as other
/// numeric fields.
const SESSION_LIMIT_LABEL: &str = "Sessions to keep in Recent tab:";
const SESSION_LIMIT_ROW_Y: usize = 178;
const SESSION_LIMIT_STEP_W: usize = 22;
const SESSION_LIMIT_FIELD_W: usize = 48;
const SESSION_LIMIT_FIELD_H: usize = 26;
/// Label-to-control gap (with fallback when text can't be measured).
const SESSION_LIMIT_LABEL_GAP: usize = 12;
const SESSION_LIMIT_LABEL_W_FALLBACK: usize = 230;

/// Measured label width + gap (fixed fallback if no font yet), so the label
/// column always lines up.
fn session_limit_label_w(text: Option<&TextRenderer>) -> usize {
    text.map(|tr| {
        tr.text_width(SESSION_LIMIT_LABEL, 15.0).ceil() as usize + SESSION_LIMIT_LABEL_GAP
    })
    .unwrap_or(SESSION_LIMIT_LABEL_W_FALLBACK)
}

/// (minus button, value field, plus button) rects.
pub(super) fn session_limit_row_layout(text: Option<&TextRenderer>) -> (Rect, Rect, Rect) {
    let x0 = CONTENT_X + session_limit_label_w(text);
    let minus = field(
        x0,
        SESSION_LIMIT_ROW_Y,
        SESSION_LIMIT_STEP_W,
        SESSION_LIMIT_FIELD_H,
    );
    let fld = field(
        x0 + SESSION_LIMIT_STEP_W,
        SESSION_LIMIT_ROW_Y,
        SESSION_LIMIT_FIELD_W,
        SESSION_LIMIT_FIELD_H,
    );
    let plus = field(
        x0 + SESSION_LIMIT_STEP_W + SESSION_LIMIT_FIELD_W,
        SESSION_LIMIT_ROW_Y,
        SESSION_LIMIT_STEP_W,
        SESSION_LIMIT_FIELD_H,
    );
    (minus, fld, plus)
}

/// Parse and clamp the input buffer; falls back to `current` if empty/invalid.
fn parse_session_limit(buf: &str, current: usize) -> usize {
    match buf.trim().parse::<usize>() {
        Ok(v) => v.clamp(1, 999),
        Err(_) => current,
    }
}

impl Settings {
    pub(super) fn buttons_editor(&self) -> Vec<(Btn, Rect)> {
        let mut v = Vec::new();
        v.push((Btn::BrowseEditor, field(CONTENT_X, 80, 100, 30)));
        let (minus_rect, field_rect, plus_rect) = session_limit_row_layout(self.text.as_ref());
        v.push((Btn::SessionLimitStep(false), minus_rect));
        v.push((Btn::SessionLimitField, field_rect));
        v.push((Btn::SessionLimitStep(true), plus_rect));
        v
    }

    pub(super) fn activate_editor(&mut self, btn: Btn) -> Option<SettingsResult> {
        match btn {
            Btn::SessionLimitField => {
                self.focus_session_limit();
                None
            }
            Btn::SessionLimitStep(up) => {
                self.step_session_limit(up);
                None
            }
            Btn::BrowseEditor => {
                if let Some(file) = rfd::FileDialog::new()
                    .set_title("外部エディタの実行ファイルを選択")
                    .add_filter("実行ファイル", &["exe"])
                    .pick_file()
                {
                    self.external_editor = file.to_string_lossy().into_owned();
                    self.request_redraw();
                }
                None
            }
            _ => None,
        }
    }

    pub(super) fn on_session_limit_key(&mut self, event: &winit::event::KeyEvent) {
        if apply_common_edit_key(
            &mut self.session_limit_cursor,
            &mut self.session_limit_buf,
            event,
            self.mods,
        ) {
            self.request_redraw();
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => self.commit_session_limit(),
            Key::Named(NamedKey::Escape) => {
                self.session_limit_focus = false;
                self.session_limit_buf.clear();
                self.session_limit_cursor = TextCursor::default();
                self.request_redraw();
            }
            Key::Character(s) if self.mods.control_key() && s.eq_ignore_ascii_case("c") => {
                self.copy_session_limit_selection();
            }
            Key::Character(s) if self.mods.control_key() && s.eq_ignore_ascii_case("x") => {
                self.copy_session_limit_selection();
                self.session_limit_cursor
                    .delete_selection(&mut self.session_limit_buf);
                self.request_redraw();
            }
            Key::Character(s) if self.mods.control_key() && s.eq_ignore_ascii_case("v") => {
                if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
                    if !digits.is_empty() {
                        self.session_limit_cursor
                            .insert(&mut self.session_limit_buf, &digits);
                        self.request_redraw();
                    }
                }
            }
            // Other Ctrl shortcuts aren't typed as text.
            Key::Character(_) if self.mods.control_key() => {}
            Key::Character(s) if s.chars().all(|c| c.is_ascii_digit()) => {
                self.session_limit_cursor
                    .insert(&mut self.session_limit_buf, s);
                self.request_redraw();
            }
            _ => {}
        }
    }

    /// Focus the field and load the current value into the buffer.
    fn focus_session_limit(&mut self) {
        self.session_limit_buf = self.session_history_limit.to_string();
        self.session_limit_cursor = TextCursor::at_end(&self.session_limit_buf);
        self.session_limit_focus = true;
        self.request_redraw();
    }

    /// Centered text draw-start x (matches the centering math in `draw`).
    pub(super) fn session_limit_text_x0(&self) -> f32 {
        let (_, field_rect, _) = session_limit_row_layout(self.text.as_ref());
        let tw = self
            .text
            .as_ref()
            .map(|tr| tr.text_width(&self.session_limit_buf, 15.0))
            .unwrap_or(0.0);
        field_rect.x0 as f32 + (field_rect.width() as f32 - tw) / 2.0
    }

    /// Mouse press: focus if needed, place caret at click, start drag-select.
    pub(super) fn begin_session_limit_press(&mut self, click_x: f64) {
        if !self.session_limit_focus {
            self.focus_session_limit();
        }
        let rel_x = click_x as f32 - self.session_limit_text_x0();
        let idx = char_index_for_x(self.text.as_ref(), &self.session_limit_buf, 15.0, rel_x);
        self.session_limit_cursor.set_from_click(idx, false);
        self.text_drag = true;
        self.request_redraw();
    }

    /// Commit the focused field (parse, clamp, apply). No-op if unfocused,
    /// so it's safe to call unconditionally.
    pub(super) fn commit_session_limit(&mut self) {
        if !self.session_limit_focus {
            return;
        }
        self.session_limit_focus = false;
        self.session_history_limit =
            parse_session_limit(&self.session_limit_buf, self.session_history_limit);
        self.session_limit_buf.clear();
        self.session_limit_cursor = TextCursor::default();
        self.request_redraw();
    }

    /// Copy the current selection to the clipboard, if any.
    fn copy_session_limit_selection(&self) {
        if let Some((lo, hi)) = self.session_limit_cursor.selection()
            && let Ok(mut clip) = arboard::Clipboard::new()
        {
            let _ = clip.set_text(self.session_limit_buf[lo..hi].to_string());
        }
    }

    /// Step by ±1; commits any focused input first.
    fn step_session_limit(&mut self, up: bool) {
        self.commit_session_limit();
        let delta: i64 = if up { 1 } else { -1 };
        self.session_history_limit =
            (self.session_history_limit as i64 + delta).clamp(1, 999) as usize;
        self.request_redraw();
    }
}

#[allow(non_snake_case, unused_variables, clippy::too_many_arguments)]
pub(super) fn draw_editor(
    canvas: &mut Canvas,
    t: &TextRenderer,
    dark: bool,
    hover: Option<Btn>,
    sw: usize,
    external_editor: &str,
    session_limit_focus: bool,
    session_limit_buf: &str,
    session_limit_cursor: TextCursor,
    session_history_limit: usize,
    text: Option<&TextRenderer>,
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
    let hover_tint = |c: u32| hover_tint_for(c, dark);

    t.draw(canvas, CONTENT_X as f32, 72.0, "Editor:", 15.0, DIM);
    // Used by Shift+E if set (E always opens the built-in editor).
    let editor_active = !external_editor.is_empty();
    let editor_label = if external_editor.is_empty() {
        "(none selected)".to_string()
    } else {
        external_editor.to_string()
    };
    canvas.fill(
        field(CONTENT_X, 118, sw.saturating_sub(CONTENT_X + 20), 26),
        FIELD_BG,
    );
    let editor_color = if editor_active { TEXT } else { DIM };
    t.draw(
        canvas,
        CONTENT_X as f32 + 8.0,
        136.0,
        &editor_label,
        15.0,
        editor_color,
    );

    // Recent tab's session limit (stepper field, same UI as other numeric
    // fields).
    let (minus_rect, field_rect, plus_rect) = session_limit_row_layout(text);
    let row_baseline = t.baseline_for_center((minus_rect.y0 + minus_rect.y1) as f32 / 2.0, 15.0);
    t.draw(
        canvas,
        CONTENT_X as f32,
        row_baseline,
        SESSION_LIMIT_LABEL,
        15.0,
        DIM,
    );

    for (r, sym, btn) in [
        (minus_rect, "-", Btn::SessionLimitStep(false)),
        (plus_rect, "+", Btn::SessionLimitStep(true)),
    ] {
        canvas.fill(
            r,
            if hover == Some(btn) {
                hover_tint(BTN_BG)
            } else {
                BTN_BG
            },
        );
        let tw = t.text_width(sym, 15.0);
        let lx = r.x0 as f32 + (r.width() as f32 - tw) / 2.0;
        let baseline = t.baseline_for_center((r.y0 + r.y1) as f32 / 2.0, 15.0);
        t.draw(canvas, lx, baseline, sym, 15.0, TEXT);
    }

    canvas.fill(field_rect, FIELD_BG);
    canvas.stroke(
        field_rect,
        if session_limit_focus {
            ACCENT
        } else {
            0x0080_8080
        },
    );
    let limit_shown = if session_limit_focus {
        session_limit_buf.to_string()
    } else {
        session_history_limit.to_string()
    };
    let tw = t.text_width(&limit_shown, 15.0);
    let lx = field_rect.x0 as f32 + (field_rect.width() as f32 - tw) / 2.0;
    let baseline = t.baseline_for_center((field_rect.y0 + field_rect.y1) as f32 / 2.0, 15.0);
    // Caret/selection height follows the glyph's actual ascent/descent, not
    // the row height.
    let (ascent, descent) = t.glyph_vextent(15.0);
    let caret_y0 = (baseline - ascent) as usize;
    let caret_y1 = (baseline - descent) as usize;
    if session_limit_focus && let Some((lo, hi)) = session_limit_cursor.selection() {
        let x0 = lx + x_for_char_index(Some(t), &limit_shown, 15.0, lo);
        let x1 = lx + x_for_char_index(Some(t), &limit_shown, 15.0, hi);
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
    t.draw(canvas, lx, baseline, &limit_shown, 15.0, TEXT);
    if session_limit_focus {
        let cx = (lx + x_for_char_index(Some(t), &limit_shown, 15.0, session_limit_cursor.cursor))
            as usize;
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

#[cfg(test)]
mod tests {
    use super::parse_session_limit;

    #[test]
    fn parse_session_limit_clamps_and_keeps_current() {
        assert_eq!(parse_session_limit("20", 10), 20);
        assert_eq!(parse_session_limit("0", 10), 1); // clamps to min
        assert_eq!(parse_session_limit("99999", 10), 999); // clamps to max
        assert_eq!(parse_session_limit("", 10), 10); // empty keeps current
        assert_eq!(parse_session_limit("abc", 10), 10); // invalid keeps current
    }
}
