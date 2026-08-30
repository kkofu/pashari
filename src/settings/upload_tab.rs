//! Upload tab: custom uploader list and edit fields.
//!
//! Same clamped/unclamped row-rect layout as the Recent tab, minus the
//! thumbnail.

use winit::keyboard::{Key, NamedKey};

use super::{
    ACCENT, Btn, CONTENT_X, SCROLLBAR_THUMB, SCROLLBAR_THUMB_HOVER, Settings, SettingsResult,
    TextCursor, UPLOAD_FIELDS, UploadField, apply_common_edit_key, byte_index_for_char_count,
    char_index_for_x, field, hover_tint_for, preedit_caret_index, scrollbar_thumb_rect,
    stroke_top_bottom_aware, theme_colors, with_preedit, x_for_char_index,
};
use crate::store::UploaderProfile;
use crate::ui::text::TextRenderer;
use crate::ui::{Canvas, Rect};

const UPLOADER_ROW_H: usize = 30;
const UPLOADER_ROW_GAP: usize = 4;
const UPLOADER_LIST_Y0: usize = 76;
const UPLOADER_LIST_VISIBLE_H: usize = 132;
const UPLOADER_DELETE_SIZE: usize = 20;
const UPLOADER_DELETE_MARGIN_RIGHT: usize = 10;
/// "Enabled" checkbox size, and its margin from the row's left edge.
const UPLOADER_CHECK_SIZE: usize = 16;
const UPLOADER_CHECK_MARGIN_LEFT: usize = 8;
/// Gap between the checkbox and the label.
const UPLOADER_LABEL_GAP: usize = 8;
const UPLOADER_ADD_BTN_Y: usize = UPLOADER_LIST_Y0 + UPLOADER_LIST_VISIBLE_H + 8;
const UPLOADER_ADD_BTN_W: usize = 120;
const UPLOADER_ADD_BTN_H: usize = 26;
const UPLOADER_FIELDS_Y0: usize = UPLOADER_ADD_BTN_Y + UPLOADER_ADD_BTN_H + 14;
const UPLOADER_FIELD_ROW_H: usize = 32;

struct UploaderRowGeom {
    index: usize,
    /// Viewport-clamped rect (hit-testing, fill).
    visible: Rect,
    /// Unclamped rect (text position).
    raw: Rect,
    /// Clamped delete (×) button rect.
    delete_visible: Rect,
    /// Unclamped; icon center is derived from this.
    delete_raw: Rect,
    /// Clamped "enabled" checkbox rect.
    check_visible: Rect,
    /// Unclamped; checkmark position is derived from this.
    check_raw: Rect,
}

impl UploaderRowGeom {
    fn top_clipped(&self) -> bool {
        self.visible.y0 > self.raw.y0
    }
    fn bottom_clipped(&self) -> bool {
        self.visible.y1 < self.raw.y1
    }
}

/// Content viewport; scrolling hides anything outside it.
pub(super) fn uploader_viewport(sw: usize) -> Rect {
    Rect {
        x0: CONTENT_X,
        y0: UPLOADER_LIST_Y0,
        x1: sw.saturating_sub(16),
        y1: UPLOADER_LIST_Y0 + UPLOADER_LIST_VISIBLE_H,
    }
}

pub(super) fn uploader_content_height(count: usize) -> i64 {
    (count * (UPLOADER_ROW_H + UPLOADER_ROW_GAP)) as i64
}

fn uploader_layout(count: usize, scroll: i32, sw: usize) -> Vec<UploaderRowGeom> {
    let viewport = uploader_viewport(sw);
    (0..count)
        .filter_map(|i| {
            let y = (i * (UPLOADER_ROW_H + UPLOADER_ROW_GAP)) as i64;
            let raw_y0 = viewport.y0 as i64 + y - scroll as i64;
            let raw_y1 = raw_y0 + UPLOADER_ROW_H as i64;
            if raw_y1 <= viewport.y0 as i64 || raw_y0 >= viewport.y1 as i64 {
                return None; // No overlap with viewport.
            }
            let clamped_y0 = raw_y0.max(viewport.y0 as i64) as usize;
            let clamped_y1 = raw_y1.min(viewport.y1 as i64) as usize;

            let delete_x1 = viewport.x1.saturating_sub(UPLOADER_DELETE_MARGIN_RIGHT);
            let delete_x0 = delete_x1.saturating_sub(UPLOADER_DELETE_SIZE);
            let delete_top = raw_y0 + (UPLOADER_ROW_H as i64 - UPLOADER_DELETE_SIZE as i64) / 2;
            let delete_bottom = delete_top + UPLOADER_DELETE_SIZE as i64;
            let delete_raw = Rect {
                x0: delete_x0,
                y0: delete_top.max(0) as usize,
                x1: delete_x1,
                y1: delete_bottom.max(0) as usize,
            };
            let delete_visible = Rect {
                x0: delete_x0,
                y0: delete_top.clamp(viewport.y0 as i64, viewport.y1 as i64) as usize,
                x1: delete_x1,
                y1: delete_bottom.clamp(viewport.y0 as i64, viewport.y1 as i64) as usize,
            };

            let check_x0 = viewport.x0 + UPLOADER_CHECK_MARGIN_LEFT;
            let check_x1 = check_x0 + UPLOADER_CHECK_SIZE;
            let check_top = raw_y0 + (UPLOADER_ROW_H as i64 - UPLOADER_CHECK_SIZE as i64) / 2;
            let check_bottom = check_top + UPLOADER_CHECK_SIZE as i64;
            let check_raw = Rect {
                x0: check_x0,
                y0: check_top.max(0) as usize,
                x1: check_x1,
                y1: check_bottom.max(0) as usize,
            };
            let check_visible = Rect {
                x0: check_x0,
                y0: check_top.clamp(viewport.y0 as i64, viewport.y1 as i64) as usize,
                x1: check_x1,
                y1: check_bottom.clamp(viewport.y0 as i64, viewport.y1 as i64) as usize,
            };

            Some(UploaderRowGeom {
                index: i,
                visible: Rect {
                    x0: viewport.x0,
                    y0: clamped_y0,
                    x1: viewport.x1,
                    y1: clamped_y1,
                },
                raw: Rect {
                    x0: viewport.x0,
                    y0: raw_y0.max(0) as usize,
                    x1: viewport.x1,
                    y1: raw_y1.max(0) as usize,
                },
                delete_visible,
                delete_raw,
                check_visible,
                check_raw,
            })
        })
        .collect()
}

/// Widest of the 6 field labels, measured + padded (fallback if no font
/// yet), so the label column lines up across rows.
fn upload_field_col_w(text: Option<&TextRenderer>) -> usize {
    match text {
        Some(tr) => {
            UPLOAD_FIELDS
                .iter()
                .map(|f| tr.text_width(f.label(), 15.0).ceil() as usize)
                .max()
                .unwrap_or(0)
                + 12
        }
        None => 110,
    }
}

/// Field rect (right of the label, row `i`); stretches to the window edge.
pub(super) fn upload_field_row_rect(i: usize, sw: usize, text: Option<&TextRenderer>) -> Rect {
    let y = UPLOADER_FIELDS_Y0 + i * UPLOADER_FIELD_ROW_H;
    let x0 = CONTENT_X + upload_field_col_w(text);
    field(x0, y, sw.saturating_sub(x0 + 20), 26)
}

/// Current value of `field` on the selected profile.
fn upload_field_value(p: &UploaderProfile, field: UploadField) -> &str {
    match field {
        UploadField::Name => &p.name,
        UploadField::Url => &p.url,
        UploadField::FileField => &p.file_field,
        UploadField::TokenField => &p.token_field,
        UploadField::Token => &p.token,
        UploadField::ResponseField => &p.response_field,
    }
}

fn upload_field_value_mut(p: &mut UploaderProfile, field: UploadField) -> &mut String {
    match field {
        UploadField::Name => &mut p.name,
        UploadField::Url => &mut p.url,
        UploadField::FileField => &mut p.file_field,
        UploadField::TokenField => &mut p.token_field,
        UploadField::Token => &mut p.token,
        UploadField::ResponseField => &mut p.response_field,
    }
}

impl Settings {
    pub(super) fn buttons_upload(&self, sw: usize) -> Vec<(Btn, Rect)> {
        let mut v = Vec::new();
        for row in uploader_layout(self.uploaders.len(), self.uploader_scroll, sw) {
            // Delete button and checkbox overlap the row; test them first.
            v.push((Btn::DeleteUploader(row.index), row.delete_visible));
            v.push((Btn::ToggleUploaderEnabled(row.index), row.check_visible));
            v.push((Btn::SelectUploader(row.index), row.visible));
        }
        v.push((
            Btn::AddUploader,
            field(
                CONTENT_X,
                UPLOADER_ADD_BTN_Y,
                UPLOADER_ADD_BTN_W,
                UPLOADER_ADD_BTN_H,
            ),
        ));
        if !self.uploaders.is_empty() {
            for (i, f) in UPLOAD_FIELDS.into_iter().enumerate() {
                v.push((
                    Btn::UploadField(f),
                    upload_field_row_rect(i, sw, self.text.as_ref()),
                ));
            }
        }
        v
    }

    pub(super) fn activate_upload(&mut self, btn: Btn) -> Option<SettingsResult> {
        match btn {
            Btn::SelectUploader(i) => {
                if i < self.uploaders.len() {
                    self.editing_uploader = i;
                    self.request_redraw();
                }
                None
            }
            Btn::DeleteUploader(i) => {
                if i < self.uploaders.len() {
                    self.uploaders.remove(i);
                    if self.editing_uploader >= self.uploaders.len() {
                        self.editing_uploader = self.uploaders.len().saturating_sub(1);
                    }
                    self.request_redraw();
                }
                None
            }
            Btn::ToggleUploaderEnabled(i) => {
                if let Some(p) = self.uploaders.get_mut(i) {
                    p.enabled = !p.enabled;
                    self.editing_uploader = i;
                    self.request_redraw();
                }
                None
            }
            Btn::AddUploader => {
                self.uploaders.push(UploaderProfile {
                    name: "New uploader".into(),
                    url: String::new(),
                    file_field: "file".into(),
                    token_field: String::new(),
                    token: String::new(),
                    response_field: "url".into(),
                    enabled: false,
                });
                self.editing_uploader = self.uploaders.len() - 1;
                self.request_redraw();
                None
            }
            Btn::UploadField(field) => {
                self.focus_upload_field(field);
                None
            }
            _ => None,
        }
    }

    pub(super) fn on_upload_field_key(&mut self, event: &winit::event::KeyEvent) {
        if apply_common_edit_key(
            &mut self.upload_field_cursor,
            &mut self.upload_field_buf,
            event,
            self.mods,
        ) {
            self.request_redraw();
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => self.commit_upload_field(),
            Key::Named(NamedKey::Escape) => {
                self.upload_field_focus = None;
                self.upload_field_buf.clear();
                self.upload_field_cursor = TextCursor::default();
                self.set_text_ime_allowed(false);
                self.request_redraw();
            }
            Key::Character(s) if self.mods.control_key() && s.eq_ignore_ascii_case("c") => {
                self.copy_upload_field_selection();
            }
            Key::Character(s) if self.mods.control_key() && s.eq_ignore_ascii_case("x") => {
                self.copy_upload_field_selection();
                self.upload_field_cursor
                    .delete_selection(&mut self.upload_field_buf);
                self.request_redraw();
            }
            Key::Character(s) if self.mods.control_key() && s.eq_ignore_ascii_case("v") => {
                if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                    self.upload_field_cursor
                        .insert(&mut self.upload_field_buf, text.trim());
                    self.request_redraw();
                }
            }
            // Other Ctrl shortcuts aren't typed as text.
            Key::Character(_) if self.mods.control_key() => {}
            Key::Character(s) => {
                self.upload_field_cursor
                    .insert(&mut self.upload_field_buf, s);
                self.request_redraw();
            }
            Key::Named(NamedKey::Space) => {
                self.upload_field_cursor
                    .insert(&mut self.upload_field_buf, " ");
                self.request_redraw();
            }
            _ => {}
        }
    }

    /// Copy the current selection to the clipboard, if any.
    fn copy_upload_field_selection(&self) {
        if let Some((lo, hi)) = self.upload_field_cursor.selection()
            && let Ok(mut clip) = arboard::Clipboard::new()
        {
            let _ = clip.set_text(self.upload_field_buf[lo..hi].to_string());
        }
    }

    /// Focus the field and load the selected profile's current value.
    fn focus_upload_field(&mut self, field: UploadField) {
        self.commit_upload_field();
        let value = self
            .uploaders
            .get(self.editing_uploader)
            .map(|p| upload_field_value(p, field).to_string())
            .unwrap_or_default();
        self.upload_field_cursor = TextCursor::at_end(&value);
        self.upload_field_buf = value;
        self.upload_field_focus = Some(field);
        self.set_text_ime_allowed(true);
        self.request_redraw();
    }

    /// Left-aligned text draw-start x for `field` (matches `draw`'s layout).
    pub(super) fn upload_field_text_x0(&self, field: UploadField) -> f32 {
        let i = UPLOAD_FIELDS.iter().position(|&f| f == field).unwrap_or(0);
        let rect = upload_field_row_rect(i, self.size.0, self.text.as_ref());
        rect.x0 as f32 + 8.0
    }

    /// Converts a click/drag x into a byte offset in `upload_field_buf`.
    /// The Token field is masked with `*`, so position is resolved against
    /// the masked string, then mapped back to the real byte offset.
    pub(super) fn upload_field_click_idx(&self, field: UploadField, rel_x: f32) -> usize {
        if field.is_secret() {
            let display = "*".repeat(self.upload_field_buf.chars().count());
            let char_idx = char_index_for_x(self.text.as_ref(), &display, 15.0, rel_x);
            byte_index_for_char_count(&self.upload_field_buf, char_idx)
        } else {
            char_index_for_x(self.text.as_ref(), &self.upload_field_buf, 15.0, rel_x)
        }
    }

    /// Mouse press: focus this field if needed, place caret at click, start
    /// drag-select.
    pub(super) fn begin_upload_field_press(&mut self, field: UploadField, click_x: f64) {
        if self.upload_field_focus != Some(field) {
            self.focus_upload_field(field);
        }
        let rel_x = click_x as f32 - self.upload_field_text_x0(field);
        let idx = self.upload_field_click_idx(field, rel_x);
        self.upload_field_cursor.set_from_click(idx, false);
        self.text_drag = true;
        self.request_redraw();
    }

    /// Commit the focused field back to the selected profile. No-op if
    /// unfocused, so it's safe to call unconditionally.
    pub(super) fn commit_upload_field(&mut self) {
        let Some(field) = self.upload_field_focus.take() else {
            return;
        };
        let value = std::mem::take(&mut self.upload_field_buf);
        if let Some(p) = self.uploaders.get_mut(self.editing_uploader) {
            *upload_field_value_mut(p, field) = value;
        }
        self.upload_field_cursor = TextCursor::default();
        self.set_text_ime_allowed(false);
        self.request_redraw();
    }
}

#[allow(non_snake_case, unused_variables, clippy::too_many_arguments)]
pub(super) fn draw_upload(
    canvas: &mut Canvas,
    t: &TextRenderer,
    dark: bool,
    hover: Option<Btn>,
    sw: usize,
    uploaders: &[UploaderProfile],
    editing_uploader: usize,
    uploader_scroll: i32,
    upload_field_focus: Option<UploadField>,
    upload_field_buf: &str,
    upload_field_cursor: TextCursor,
    ime_preedit: &str,
    ime_preedit_cursor: Option<(usize, usize)>,
    text: Option<&TextRenderer>,
    scrollbar_active: bool,
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

    let list_viewport = uploader_viewport(sw);
    if uploaders.is_empty() {
        t.draw_clipped(
            canvas,
            list_viewport.x0 as f32,
            list_viewport.y0 as f32 + 20.0,
            "No uploaders configured — click + Add uploader to create one.",
            14.0,
            DIM,
            list_viewport,
        );
    }
    for row in uploader_layout(uploaders.len(), uploader_scroll, sw) {
        let profile = &uploaders[row.index];
        let is_editing = row.index == editing_uploader;
        let select_hover = hover == Some(Btn::SelectUploader(row.index));
        let delete_hover = hover == Some(Btn::DeleteUploader(row.index));
        let check_hover = hover == Some(Btn::ToggleUploaderEnabled(row.index));
        // Background marks which row the edit panel below shows (separate
        // from the enabled checkbox).
        let base = if is_editing {
            UPLOADER_ACTIVE_BG
        } else {
            FIELD_BG
        };
        canvas.fill(
            row.visible,
            if select_hover || delete_hover || check_hover {
                hover_tint(base)
            } else {
                base
            },
        );

        // Enabled checkbox; any number of profiles can be checked at once.
        canvas.fill(
            row.check_visible,
            if profile.enabled { ACCENT } else { FIELD_BG },
        );
        canvas.stroke(
            row.check_visible,
            if check_hover { ACCENT } else { 0x0080_8080 },
        );
        if profile.enabled {
            // Checkmark; center comes from `check_raw` (unclamped).
            let (x0, y0, x1, y1) = (
                row.check_raw.x0 as i64,
                row.check_raw.y0 as i64,
                row.check_raw.x1 as i64,
                row.check_raw.y1 as i64,
            );
            canvas.line(x0 + 2, y0 + 8, x0 + 6, y1 - 3, 2, 0x00FF_FFFF);
            canvas.line(x0 + 6, y1 - 3, x1 - 2, y0 + 2, 2, 0x00FF_FFFF);
        }

        let label = if profile.name.trim().is_empty() {
            "(unnamed)"
        } else {
            &profile.name
        };
        let baseline = t.baseline_for_center((row.raw.y0 + row.raw.y1) as f32 / 2.0, 15.0);
        t.draw_clipped(
            canvas,
            (row.check_raw.x1 + UPLOADER_LABEL_GAP) as f32,
            baseline,
            label,
            15.0,
            TEXT,
            list_viewport,
        );

        // Delete icon; center comes from `delete_raw` (unclamped) so it
        // doesn't appear to resize while scrolling clips it.
        if row.delete_visible.width() > 0 && row.delete_visible.height() > 0 {
            let cx = ((row.delete_raw.x0 + row.delete_raw.x1) / 2) as i64;
            let cy = ((row.delete_raw.y0 + row.delete_raw.y1) / 2) as i64;
            let r = 5i64;
            let color = if delete_hover { 0x00FF_6B6B } else { DIM };
            canvas.line(cx - r, cy - r, cx + r, cy + r, 2, color);
            canvas.line(cx - r, cy + r, cx + r, cy - r, 2, color);
        }

        stroke_top_bottom_aware(
            canvas,
            row.visible,
            !row.top_clipped(),
            !row.bottom_clipped(),
            0x0080_8080,
        );
    }

    // Scrollbar (also drag-scrollable; see `Settings::scrollbar_drag`).
    let content_h = uploader_content_height(uploaders.len());
    let track_x0 = sw.saturating_sub(10);
    if let Some(thumb) = scrollbar_thumb_rect(track_x0, list_viewport, content_h, uploader_scroll) {
        let track = field(
            track_x0,
            list_viewport.y0,
            4,
            (list_viewport.y1 - list_viewport.y0).max(1),
        );
        canvas.fill(track, FIELD_BG);
        let thumb_color = if scrollbar_active {
            SCROLLBAR_THUMB_HOVER
        } else {
            SCROLLBAR_THUMB
        };
        canvas.fill(thumb, thumb_color);
    }

    // Edit panel for the selected profile.
    if let Some(profile) = uploaders.get(editing_uploader) {
        for (i, f) in UPLOAD_FIELDS.into_iter().enumerate() {
            let rect = upload_field_row_rect(i, sw, text);
            let label_baseline = t.baseline_for_center((rect.y0 + rect.y1) as f32 / 2.0, 15.0);
            t.draw(
                canvas,
                CONTENT_X as f32,
                label_baseline,
                f.label(),
                15.0,
                DIM,
            );

            canvas.fill(rect, FIELD_BG);
            let focused = upload_field_focus == Some(f);
            canvas.stroke(rect, if focused { ACCENT } else { 0x0080_8080 });
            let (shown, preedit_range) = if focused && !ime_preedit.is_empty() {
                let (display, range) =
                    with_preedit(upload_field_buf, upload_field_cursor, ime_preedit);
                (display, Some(range))
            } else if focused {
                (upload_field_buf.to_string(), None)
            } else {
                (upload_field_value(profile, f).to_string(), None)
            };
            // Token is masked with `*` for display only; the caret is
            // mapped through the char count of the masked string.
            let display = if f.is_secret() {
                "*".repeat(shown.chars().count())
            } else {
                shown.clone()
            };
            let display_idx = |idx: usize| -> usize {
                if f.is_secret() {
                    shown[..idx].chars().count()
                } else {
                    idx
                }
            };
            let value_color = if shown.is_empty() { DIM } else { TEXT };
            let value_baseline = t.baseline_for_center((rect.y0 + rect.y1) as f32 / 2.0, 15.0);
            let text_x0 = rect.x0 as f32 + 8.0;
            // Caret/selection height follows the glyph's actual
            // ascent/descent, not the row height.
            let (ascent, descent) = t.glyph_vextent(15.0);
            let caret_y0 = (value_baseline - ascent) as usize;
            let caret_y1 = (value_baseline - descent) as usize;
            if focused
                && ime_preedit.is_empty()
                && let Some((lo, hi)) = upload_field_cursor.selection()
            {
                let x0 = text_x0 + x_for_char_index(Some(t), &display, 15.0, display_idx(lo));
                let x1 = text_x0 + x_for_char_index(Some(t), &display, 15.0, display_idx(hi));
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
                text_x0,
                value_baseline,
                &display,
                15.0,
                value_color,
                rect,
            );
            if focused {
                let caret_idx = preedit_range.as_ref().map_or_else(
                    || Some(upload_field_cursor.cursor),
                    |range| preedit_caret_index(range.start, ime_preedit, ime_preedit_cursor),
                );
                if let Some(range) = preedit_range.as_ref() {
                    let x0 = text_x0
                        + x_for_char_index(Some(t), &display, 15.0, display_idx(range.start));
                    let x1 =
                        text_x0 + x_for_char_index(Some(t), &display, 15.0, display_idx(range.end));
                    let underline_y = caret_y1.min(rect.y1.saturating_sub(2));
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
                    let cx = (text_x0
                        + x_for_char_index(Some(t), &display, 15.0, display_idx(caret_idx)))
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{uploader_content_height, uploader_layout, uploader_viewport};
    use crate::settings::WIN_W;

    #[test]
    fn uploader_layout_clamps_visible_rows_to_viewport_bounds() {
        let viewport = uploader_viewport(WIN_W);
        let rows = uploader_layout(10, 0, WIN_W);
        // Some rows show, all within the viewport (not all fit without scrolling).
        assert!(!rows.is_empty());
        assert!(rows.len() < 10);
        for row in &rows {
            assert!(row.visible.y0 >= viewport.y0 && row.visible.y1 <= viewport.y1);
        }
    }

    #[test]
    fn uploader_layout_scrolling_reveals_later_rows() {
        let rows_top = uploader_layout(10, 0, WIN_W);
        let viewport = uploader_viewport(WIN_W);
        let max_scroll = (uploader_content_height(10) - (viewport.y1 - viewport.y0) as i64).max(0);
        let rows_bottom = uploader_layout(10, max_scroll as i32, WIN_W);
        assert!(rows_top.iter().any(|r| r.index == 0));
        assert!(!rows_bottom.iter().any(|r| r.index == 0));
        assert!(rows_bottom.iter().any(|r| r.index == 9));
    }
}
