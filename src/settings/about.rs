//! About tab: app name/version/license/repo, and the list of directly
//! depended-on crates (name, version, license — generated at build time
//! from `Cargo.toml`/`cargo metadata`, see `build.rs`, so it can't drift
//! out of sync with a hand-maintained copy).

use super::{
    CONTENT_X, SCROLLBAR_THUMB, SCROLLBAR_THUMB_HOVER, field, scrollbar_thumb_rect, theme_colors,
};
use crate::ui::text::TextRenderer;
use crate::ui::{Canvas, Rect};
use crate::update;

include!(concat!(env!("OUT_DIR"), "/used_crates.rs"));

const LICENSE: &str = "GPL-3.0-or-later";
const REPO_URL: &str = "https://github.com/yozba/pashari";

const HEADER_ROW_Y: usize = 72;
const HEADER_ROW_H: usize = 22;
const DEPS_HEADING_Y: usize = HEADER_ROW_Y + HEADER_ROW_H * 3 + 16;
const DEPS_LIST_Y: usize = DEPS_HEADING_Y + 26;
const DEPS_ROW_H: usize = 24;
const DEPS_VERSION_X: usize = 260;
const DEPS_LICENSE_X: usize = 340;

/// Content viewport; scrolling hides anything outside it (same idea as
/// `recent::session_viewport`).
pub(super) fn about_viewport(sw: usize, sh: usize) -> Rect {
    field(
        CONTENT_X,
        DEPS_LIST_Y,
        sw.saturating_sub(CONTENT_X + 16),
        sh.saturating_sub(DEPS_LIST_Y + 64),
    )
}

pub(super) fn about_content_height(count: usize) -> i64 {
    (count * DEPS_ROW_H) as i64
}

#[allow(non_snake_case, unused_variables)]
pub(super) fn draw_about(
    canvas: &mut Canvas,
    t: &TextRenderer,
    dark: bool,
    sw: usize,
    sh: usize,
    scroll: i32,
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

    let name_baseline = t.baseline_for_center((HEADER_ROW_Y + HEADER_ROW_H / 2) as f32, 15.0);
    t.draw(
        canvas,
        CONTENT_X as f32,
        name_baseline,
        &format!("pashari v{}", update::CURRENT_VERSION),
        15.0,
        TEXT,
    );

    let license_y = HEADER_ROW_Y + HEADER_ROW_H;
    let license_baseline = t.baseline_for_center((license_y + HEADER_ROW_H / 2) as f32, 13.0);
    t.draw(
        canvas,
        CONTENT_X as f32,
        license_baseline,
        &format!("License: {LICENSE}"),
        13.0,
        DIM,
    );

    let repo_y = license_y + HEADER_ROW_H;
    let repo_baseline = t.baseline_for_center((repo_y + HEADER_ROW_H / 2) as f32, 13.0);
    t.draw(canvas, CONTENT_X as f32, repo_baseline, REPO_URL, 13.0, DIM);

    let heading_baseline = t.baseline_for_center((DEPS_HEADING_Y + HEADER_ROW_H / 2) as f32, 15.0);
    t.draw(
        canvas,
        CONTENT_X as f32,
        heading_baseline,
        "Dependencies",
        15.0,
        TEXT,
    );

    let viewport = about_viewport(sw, sh);
    for (i, (name, version, license)) in USED_CRATES.iter().enumerate() {
        let raw_y0 = viewport.y0 as i64 + (i * DEPS_ROW_H) as i64 - scroll as i64;
        let raw_y1 = raw_y0 + DEPS_ROW_H as i64;
        if raw_y1 <= viewport.y0 as i64 || raw_y0 >= viewport.y1 as i64 {
            continue; // No overlap with viewport.
        }
        let baseline = t.baseline_for_center((raw_y0 + DEPS_ROW_H as i64 / 2) as f32, 13.0);
        t.draw_clipped(
            canvas,
            CONTENT_X as f32,
            baseline,
            name,
            13.0,
            TEXT,
            viewport,
        );
        t.draw_clipped(
            canvas,
            (CONTENT_X + DEPS_VERSION_X) as f32,
            baseline,
            version,
            13.0,
            DIM,
            viewport,
        );
        t.draw_clipped(
            canvas,
            (CONTENT_X + DEPS_LICENSE_X) as f32,
            baseline,
            license,
            13.0,
            DIM,
            viewport,
        );
    }

    // Scrollbar (also drag-scrollable; see `Settings::scrollbar_drag`).
    let content_h = about_content_height(USED_CRATES.len());
    let track_x0 = sw.saturating_sub(10);
    if let Some(thumb) = scrollbar_thumb_rect(track_x0, viewport, content_h, scroll) {
        let track = field(track_x0, viewport.y0, 4, (viewport.y1 - viewport.y0).max(1));
        canvas.fill(track, FIELD_BG);
        canvas.fill(
            thumb,
            if scrollbar_active {
                SCROLLBAR_THUMB_HOVER
            } else {
                SCROLLBAR_THUMB
            },
        );
    }
}
