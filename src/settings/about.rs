//! About tab: app name/version/license/repo, and the list of directly
//! depended-on crates (name + version, generated at build time from
//! `Cargo.toml`'s `[dependencies]` — see `build.rs` — so it can't drift
//! out of sync).

use super::{CONTENT_X, theme_colors};
use crate::ui::Canvas;
use crate::ui::text::TextRenderer;
use crate::update;

include!(concat!(env!("OUT_DIR"), "/used_crates.rs"));

const LICENSE: &str = "GPL-3.0-or-later";
const REPO_URL: &str = "https://github.com/yozba/pashari";

const HEADER_ROW_Y: usize = 72;
const HEADER_ROW_H: usize = 24;
const DEPS_HEADING_Y: usize = HEADER_ROW_Y + HEADER_ROW_H * 3 + 20;
const DEPS_ROW_Y: usize = DEPS_HEADING_Y + 28;
const DEPS_ROW_H: usize = 22;
/// Two columns keep the ~20 direct dependencies comfortably within the
/// window height without needing to scroll.
const DEPS_COL_W: usize = 260;

#[allow(non_snake_case, unused_variables)]
pub(super) fn draw_about(canvas: &mut Canvas, t: &TextRenderer, dark: bool, sw: usize) {
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

    let rows_per_col = USED_CRATES.len().div_ceil(2);
    for (i, (name, version)) in USED_CRATES.iter().enumerate() {
        let col = i / rows_per_col;
        let row = i % rows_per_col;
        let x = CONTENT_X + col * DEPS_COL_W;
        let y = DEPS_ROW_Y + row * DEPS_ROW_H;
        let baseline = t.baseline_for_center((y + DEPS_ROW_H / 2) as f32, 13.0);
        t.draw(canvas, x as f32, baseline, name, 13.0, TEXT);
        t.draw(
            canvas,
            (x + DEPS_COL_W - 70) as f32,
            baseline,
            version,
            13.0,
            DIM,
        );
    }
    let _ = sw;
}
