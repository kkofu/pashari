//! Recent tab: editor session history.
//!
//! Same clamped/unclamped row-rect layout as the Hotkeys tab, but simpler
//! (one row = one click target).

use std::rc::Rc;

use super::{
    Btn, SCROLLBAR_THUMB, SCROLLBAR_THUMB_HOVER, Settings, SettingsResult, field, hover_tint_for,
    scrollbar_thumb_rect, stroke_top_bottom_aware, theme_colors,
};
use crate::ui::text::TextRenderer;
use crate::ui::{Canvas, Rect};

const SESSION_ROW_H: usize = 64;
const SESSION_ROW_GAP: usize = 8;
const SESSION_THUMB_W: usize = 96;
const SESSION_THUMB_H: usize = 56;
const SESSION_THUMB_MARGIN: usize = 4;
const SESSION_LABEL_X_OFFSET: usize = SESSION_THUMB_MARGIN * 2 + SESSION_THUMB_W + 12;
/// Delete (×) hit target: icon bounding box plus a few px, kept short of the
/// row edge so it doesn't overpaint the row border.
const SESSION_DELETE_SIZE: usize = 20;
const SESSION_DELETE_MARGIN_RIGHT: usize = 10;

struct SessionRowGeom {
    index: usize,
    /// Viewport-clamped rect (hit-testing, fill).
    visible: Rect,
    /// Unclamped rect (thumbnail/label position).
    raw: Rect,
    /// Clamped delete (×) button rect.
    delete_visible: Rect,
    /// Unclamped; icon center is derived from this so it doesn't appear to
    /// resize while scrolling clips it.
    delete_raw: Rect,
}

impl SessionRowGeom {
    fn top_clipped(&self) -> bool {
        self.visible.y0 > self.raw.y0
    }
    fn bottom_clipped(&self) -> bool {
        self.visible.y1 < self.raw.y1
    }
}

/// Content viewport; scrolling hides anything outside it.
pub(super) fn session_viewport(sw: usize, sh: usize) -> Rect {
    Rect {
        x0: super::CONTENT_X,
        y0: 68,
        x1: sw.saturating_sub(16),
        y1: sh.saturating_sub(64),
    }
}

pub(super) fn session_content_height(count: usize) -> i64 {
    (count * (SESSION_ROW_H + SESSION_ROW_GAP)) as i64
}

fn session_layout(count: usize, scroll: i32, sw: usize, sh: usize) -> Vec<SessionRowGeom> {
    let viewport = session_viewport(sw, sh);
    (0..count)
        .filter_map(|i| {
            let y = (i * (SESSION_ROW_H + SESSION_ROW_GAP)) as i64;
            let raw_y0 = viewport.y0 as i64 + y - scroll as i64;
            let raw_y1 = raw_y0 + SESSION_ROW_H as i64;
            if raw_y1 <= viewport.y0 as i64 || raw_y0 >= viewport.y1 as i64 {
                return None; // No overlap with viewport.
            }
            let clamped_y0 = raw_y0.max(viewport.y0 as i64) as usize;
            let clamped_y1 = raw_y1.min(viewport.y1 as i64) as usize;

            // Delete button: small square, right-aligned, vertically centered.
            let delete_x1 = viewport.x1.saturating_sub(SESSION_DELETE_MARGIN_RIGHT);
            let delete_x0 = delete_x1.saturating_sub(SESSION_DELETE_SIZE);
            let delete_top = raw_y0 + (SESSION_ROW_H as i64 - SESSION_DELETE_SIZE as i64) / 2;
            let delete_bottom = delete_top + SESSION_DELETE_SIZE as i64;
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

            Some(SessionRowGeom {
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
            })
        })
        .collect()
}

impl Settings {
    pub(super) fn buttons_recent(&self, sw: usize, sh: usize) -> Vec<(Btn, Rect)> {
        let mut v = Vec::new();
        for row in session_layout(self.sessions.len(), self.recent_scroll, sw, sh) {
            // Delete button overlaps the row; test it first.
            v.push((Btn::DeleteSession(row.index), row.delete_visible));
            v.push((Btn::OpenSession(row.index), row.visible));
        }
        v
    }

    pub(super) fn activate_recent(&mut self, btn: Btn) -> Option<SettingsResult> {
        match btn {
            Btn::OpenSession(i) => self
                .sessions
                .get(i)
                .map(|s| SettingsResult::OpenSession(s.dir.clone())),
            Btn::DeleteSession(i) => {
                if i < self.sessions.len() {
                    let _ = std::fs::remove_dir_all(&self.sessions[i].dir);
                    self.sessions.remove(i);
                    self.request_redraw();
                }
                None
            }
            _ => None,
        }
    }
}

#[allow(non_snake_case, unused_variables, clippy::too_many_arguments)]
pub(super) fn draw_recent(
    canvas: &mut Canvas,
    t: &TextRenderer,
    dark: bool,
    hover: Option<Btn>,
    sw: usize,
    sh: usize,
    session_rows: &[(String, usize, usize, Rc<Vec<u32>>)],
    recent_scroll: i32,
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

    let viewport = session_viewport(sw, sh);
    if session_rows.is_empty() {
        t.draw_clipped(
            canvas,
            viewport.x0 as f32,
            viewport.y0 as f32 + 20.0,
            "No sessions yet — close the Editor after making edits to see them here.",
            14.0,
            DIM,
            viewport,
        );
    }
    for row in session_layout(session_rows.len(), recent_scroll, sw, sh) {
        let (label, thumb_w, thumb_h, thumb) = &session_rows[row.index];
        let delete_hover = hover == Some(Btn::DeleteSession(row.index));
        // Hovering the × also highlights the whole row.
        let row_hover = hover == Some(Btn::OpenSession(row.index)) || delete_hover;
        canvas.fill(
            row.visible,
            if row_hover {
                hover_tint(FIELD_BG)
            } else {
                FIELD_BG
            },
        );

        let thumb_dst = Rect {
            x0: row.raw.x0 + SESSION_THUMB_MARGIN,
            y0: row.raw.y0 + SESSION_THUMB_MARGIN,
            x1: row.raw.x0 + SESSION_THUMB_MARGIN + SESSION_THUMB_W,
            y1: row.raw.y0 + SESSION_THUMB_MARGIN + SESSION_THUMB_H,
        };
        // Clip thumbnail to viewport; blit_scaled no-ops if fully outside.
        let clipped_thumb = Rect {
            x0: thumb_dst.x0.clamp(viewport.x0, viewport.x1),
            y0: thumb_dst.y0.clamp(viewport.y0, viewport.y1),
            x1: thumb_dst.x1.clamp(viewport.x0, viewport.x1),
            y1: thumb_dst.y1.clamp(viewport.y0, viewport.y1),
        };
        if clipped_thumb == thumb_dst {
            canvas.blit_scaled(thumb_dst, *thumb_w, *thumb_h, thumb.as_slice());
        }

        let label_baseline = t.baseline_for_center((row.raw.y0 + row.raw.y1) as f32 / 2.0, 15.0);
        t.draw_clipped(
            canvas,
            (row.raw.x0 + SESSION_LABEL_X_OFFSET) as f32,
            label_baseline,
            label,
            15.0,
            TEXT,
            viewport,
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

        // Draw the row border last so it isn't painted over.
        stroke_top_bottom_aware(
            canvas,
            row.visible,
            !row.top_clipped(),
            !row.bottom_clipped(),
            0x0080_8080,
        );
    }

    // Scrollbar (also drag-scrollable; see `Settings::scrollbar_drag`).
    let content_h = session_content_height(session_rows.len());
    let track_x0 = sw.saturating_sub(10);
    if let Some(thumb) = scrollbar_thumb_rect(track_x0, viewport, content_h, recent_scroll) {
        let track = field(track_x0, viewport.y0, 4, (viewport.y1 - viewport.y0).max(1));
        canvas.fill(track, FIELD_BG);
        let thumb_color = if scrollbar_active {
            SCROLLBAR_THUMB_HOVER
        } else {
            SCROLLBAR_THUMB
        };
        canvas.fill(thumb, thumb_color);
    }
}
