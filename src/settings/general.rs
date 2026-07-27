//! General tab: filename template, update check, launch at startup.

use winit::keyboard::{Key, NamedKey};

use super::{
    ACCENT, Btn, CONTENT_X, Settings, SettingsResult, TextCursor, apply_common_edit_key,
    char_index_for_x, field, hover_tint_for, next_row_y, next_row_y_with_extra_gap, theme_colors,
    x_for_char_index,
};
use crate::app::UserEvent;
use crate::ui::text::TextRenderer;
use crate::ui::{Canvas, Rect};
use crate::update::{self, ReleaseInfo};

const FILENAME_FORMAT_ROW_Y: usize = 88;
const FILENAME_FORMAT_ROW_H: usize = 26;

/// Filename template field: right of the label, follows window width.
fn filename_format_row_layout(sw: usize) -> Rect {
    const LABEL_W: usize = 140;
    field(
        CONTENT_X + LABEL_W,
        FILENAME_FORMAT_ROW_Y,
        sw.saturating_sub(CONTENT_X + LABEL_W + 20),
        FILENAME_FORMAT_ROW_H,
    )
}

/// Version/update-check row, below the filename field + 2 lines of help
/// text (hence the extra gap).
const UPDATE_ROW_Y: usize =
    next_row_y_with_extra_gap(FILENAME_FORMAT_ROW_Y, FILENAME_FORMAT_ROW_H, 50);
const UPDATE_ROW_H: usize = 26;
/// A second row below the check button for the check's outcome (a status
/// message, and — once a newer version is found — a button to go get it).
/// Always reserved, even before anything's been checked.
const UPDATE_STATUS_ROW_Y: usize = next_row_y(UPDATE_ROW_Y, UPDATE_ROW_H);
const UPDATE_STATUS_ROW_H: usize = 26;
const STARTUP_ROW_Y: usize = next_row_y_with_extra_gap(UPDATE_STATUS_ROW_Y, UPDATE_STATUS_ROW_H, 6);
const UPDATE_BTN_PAD: usize = 24;
const UPDATE_BTN_W_FALLBACK: usize = 200;
const DOWNLOAD_BTN_LABEL: &str = "Update";
const DOWNLOAD_BTN_W_FALLBACK: usize = 100;
const UPDATE_STATUS_GAP: usize = 16;

/// Outcome of the last manual check made from this session, shown in the
/// status row below the check button. `update_available` (a persistent,
/// app-wide value) covers the "found a newer version" case on its own;
/// this only needs to distinguish the other two.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum UpdateCheckStatus {
    UpToDate,
    Failed,
}

/// Measured width of the version label + gap; the check button sits to
/// its right.
fn update_label_x(text: Option<&TextRenderer>) -> usize {
    let label = format!("Version {}", update::CURRENT_VERSION);
    CONTENT_X
        + text
            .map(|tr| tr.text_width(&label, 15.0).ceil() as usize + 24)
            .unwrap_or(160)
}

const CHECK_BTN_LABEL: &str = "Check for updates";

/// Sized to fit the label text, like the other buttons. The label is
/// always "Check for updates" now — the outcome shows in the status row
/// below instead of replacing this button's own label.
fn update_button_rect(text: Option<&TextRenderer>) -> Rect {
    let w = text
        .map(|tr| tr.text_width(CHECK_BTN_LABEL, 15.0).ceil() as usize + UPDATE_BTN_PAD)
        .unwrap_or(UPDATE_BTN_W_FALLBACK);
    field(update_label_x(text), UPDATE_ROW_Y, w, UPDATE_ROW_H)
}

/// The status row's message, if there's anything to show yet (`None`
/// before the first check this session). An install error (from clicking
/// "Update") takes priority, then the in-progress state, then whether a
/// newer version is known, then the plain check outcome.
fn update_status_text(
    update_available: Option<&ReleaseInfo>,
    status: Option<UpdateCheckStatus>,
    updating: bool,
    install_error: Option<&str>,
) -> Option<String> {
    if let Some(msg) = install_error {
        Some(format!("Update failed: {msg}"))
    } else if updating {
        Some("Downloading update...".to_string())
    } else if let Some(info) = update_available {
        Some(format!("A new version is available: v{}", info.version))
    } else {
        match status {
            Some(UpdateCheckStatus::UpToDate) => Some("You're up to date.".to_string()),
            Some(UpdateCheckStatus::Failed) => {
                Some("Update check failed. Try again later.".to_string())
            }
            None => None,
        }
    }
}

/// The "Update" button, shown to the right of `status_label` once a newer
/// version is known (`status_label` is whatever `update_status_text`
/// returned, so the button starts right after that text).
fn download_button_rect(text: Option<&TextRenderer>, status_label: &str) -> Rect {
    let status_w = text
        .map(|tr| tr.text_width(status_label, 15.0).ceil() as usize)
        .unwrap_or(0);
    let w = text
        .map(|tr| tr.text_width(DOWNLOAD_BTN_LABEL, 15.0).ceil() as usize + UPDATE_BTN_PAD)
        .unwrap_or(DOWNLOAD_BTN_W_FALLBACK);
    let x0 = CONTENT_X + status_w + UPDATE_STATUS_GAP;
    field(x0, UPDATE_STATUS_ROW_Y, w, UPDATE_STATUS_ROW_H)
}

/// Commit the buffer: trims and keeps it, or falls back to `current` if
/// empty.
fn commit_filename_format_value(buf: &str, current: &str) -> String {
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        current.to_string()
    } else {
        trimmed.to_string()
    }
}

impl Settings {
    pub(super) fn buttons_general(&self, sw: usize) -> Vec<(Btn, Rect)> {
        let mut v = vec![
            (Btn::FilenameFormatField, filename_format_row_layout(sw)),
            (Btn::CheckForUpdates, update_button_rect(self.text.as_ref())),
            (
                Btn::LaunchAtStartup,
                field(CONTENT_X, STARTUP_ROW_Y, 260, 30),
            ),
        ];
        if let Some(info) = self.update_available.as_ref() {
            let status_label = update_status_text(
                Some(info),
                self.update_check_status,
                self.updating,
                self.update_install_error.as_deref(),
            )
            .expect("update_available is Some, so update_status_text always returns Some");
            v.push((
                Btn::DownloadUpdate,
                download_button_rect(self.text.as_ref(), &status_label),
            ));
        }
        v
    }

    pub(super) fn activate_general(&mut self, btn: Btn) -> Option<SettingsResult> {
        match btn {
            Btn::FilenameFormatField => {
                self.focus_filename_format();
                None
            }
            Btn::CheckForUpdates => {
                // A fresh check always starts over, clearing any stale
                // up-to-date/failed status from a previous click this session.
                self.update_check_status = None;
                let proxy = self.update_proxy.clone();
                std::thread::spawn(move || {
                    let result = update::check_latest();
                    let _ = proxy.send_event(UserEvent::UpdateCheckResult(result));
                });
                self.request_redraw();
                None
            }
            Btn::DownloadUpdate => {
                if self.updating {
                    return None;
                }
                if let Some(info) = self.update_available.clone() {
                    match update::update_target(&info) {
                        Some(target) => {
                            self.updating = true;
                            self.update_install_error = None;
                            let proxy = self.update_proxy.clone();
                            std::thread::spawn(move || {
                                let result = update::download_update(target);
                                let _ = proxy.send_event(UserEvent::UpdateReady(result));
                            });
                            self.request_redraw();
                        }
                        // No matching asset (an old release predating this
                        // feature) — fall back to the manual-download page.
                        None => crate::shell::open_url(&info.url),
                    }
                }
                None
            }
            Btn::LaunchAtStartup => {
                self.launch_at_startup = !self.launch_at_startup;
                self.request_redraw();
                None
            }
            _ => None,
        }
    }

    pub(super) fn on_filename_format_key(&mut self, event: &winit::event::KeyEvent) {
        if apply_common_edit_key(
            &mut self.filename_format_cursor,
            &mut self.filename_format_buf,
            event,
            self.mods,
        ) {
            self.request_redraw();
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => self.commit_filename_format(),
            Key::Named(NamedKey::Escape) => {
                self.filename_format_focus = false;
                self.filename_format_buf.clear();
                self.filename_format_cursor = TextCursor::default();
                self.request_redraw();
            }
            Key::Character(s) if self.mods.control_key() && s.eq_ignore_ascii_case("c") => {
                self.copy_filename_format_selection();
            }
            Key::Character(s) if self.mods.control_key() && s.eq_ignore_ascii_case("x") => {
                self.copy_filename_format_selection();
                self.filename_format_cursor
                    .delete_selection(&mut self.filename_format_buf);
                self.request_redraw();
            }
            Key::Character(s) if self.mods.control_key() && s.eq_ignore_ascii_case("v") => {
                if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                    self.filename_format_cursor
                        .insert(&mut self.filename_format_buf, text.trim());
                    self.request_redraw();
                }
            }
            // Other Ctrl shortcuts aren't typed as text.
            Key::Character(_) if self.mods.control_key() => {}
            Key::Character(s) => {
                self.filename_format_cursor
                    .insert(&mut self.filename_format_buf, s);
                self.request_redraw();
            }
            Key::Named(NamedKey::Space) => {
                self.filename_format_cursor
                    .insert(&mut self.filename_format_buf, " ");
                self.request_redraw();
            }
            _ => {}
        }
    }

    /// Focus the field and load the current value into the buffer.
    fn focus_filename_format(&mut self) {
        self.filename_format_buf = self.filename_format.clone();
        self.filename_format_cursor = TextCursor::at_end(&self.filename_format_buf);
        self.filename_format_focus = true;
        self.request_redraw();
    }

    /// Left-aligned text draw-start x (matches the layout math in `draw`).
    pub(super) fn filename_format_text_x0(&self) -> f32 {
        filename_format_row_layout(self.size.0).x0 as f32 + 8.0
    }

    /// Mouse press: focus if needed, place caret at click, start drag-select.
    pub(super) fn begin_filename_format_press(&mut self, click_x: f64) {
        if !self.filename_format_focus {
            self.focus_filename_format();
        }
        let rel_x = click_x as f32 - self.filename_format_text_x0();
        let idx = char_index_for_x(self.text.as_ref(), &self.filename_format_buf, 15.0, rel_x);
        self.filename_format_cursor.set_from_click(idx, false);
        self.text_drag = true;
        self.request_redraw();
    }

    /// Commit the focused field. No-op if unfocused, so it's safe to call
    /// unconditionally.
    pub(super) fn commit_filename_format(&mut self) {
        if !self.filename_format_focus {
            return;
        }
        self.filename_format_focus = false;
        self.filename_format =
            commit_filename_format_value(&self.filename_format_buf, &self.filename_format);
        self.filename_format_buf.clear();
        self.filename_format_cursor = TextCursor::default();
        self.request_redraw();
    }

    /// Copy the current selection to the clipboard, if any.
    fn copy_filename_format_selection(&self) {
        if let Some((lo, hi)) = self.filename_format_cursor.selection()
            && let Ok(mut clip) = arboard::Clipboard::new()
        {
            let _ = clip.set_text(self.filename_format_buf[lo..hi].to_string());
        }
    }
}

#[allow(non_snake_case, unused_variables, clippy::too_many_arguments)]
pub(super) fn draw_general(
    canvas: &mut Canvas,
    t: &TextRenderer,
    dark: bool,
    hover: Option<Btn>,
    buttons: &[(Btn, Rect)],
    sw: usize,
    filename_format: &str,
    filename_format_focus: bool,
    filename_format_buf: &str,
    filename_format_cursor: TextCursor,
    text: Option<&TextRenderer>,
    update_available: &Option<ReleaseInfo>,
    update_check_status: Option<UpdateCheckStatus>,
    updating: bool,
    update_install_error: &Option<String>,
    launch_at_startup: bool,
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

    let format_rect = filename_format_row_layout(sw);
    let format_label_baseline =
        t.baseline_for_center((format_rect.y0 + format_rect.y1) as f32 / 2.0, 15.0);
    t.draw(
        canvas,
        CONTENT_X as f32,
        format_label_baseline,
        "Filename format:",
        15.0,
        DIM,
    );
    canvas.fill(format_rect, FIELD_BG);
    canvas.stroke(
        format_rect,
        if filename_format_focus {
            ACCENT
        } else {
            0x0080_8080
        },
    );
    let format_shown = if filename_format_focus {
        filename_format_buf.to_string()
    } else {
        filename_format.to_string()
    };
    let format_text_x0 = format_rect.x0 as f32 + 8.0;
    let format_baseline =
        t.baseline_for_center((format_rect.y0 + format_rect.y1) as f32 / 2.0, 15.0);
    let (format_ascent, format_descent) = t.glyph_vextent(15.0);
    let format_caret_y0 = (format_baseline - format_ascent) as usize;
    let format_caret_y1 = (format_baseline - format_descent) as usize;
    if filename_format_focus && let Some((lo, hi)) = filename_format_cursor.selection() {
        let x0 = format_text_x0 + x_for_char_index(Some(t), &format_shown, 15.0, lo);
        let x1 = format_text_x0 + x_for_char_index(Some(t), &format_shown, 15.0, hi);
        canvas.fill(
            Rect {
                x0: x0 as usize,
                y0: format_caret_y0,
                x1: x1 as usize,
                y1: format_caret_y1,
            },
            TEXT_SELECTION_BG,
        );
    }
    t.draw_clipped(
        canvas,
        format_text_x0,
        format_baseline,
        &format_shown,
        15.0,
        TEXT,
        format_rect,
    );
    if filename_format_focus {
        let cx = (format_text_x0
            + x_for_char_index(Some(t), &format_shown, 15.0, filename_format_cursor.cursor))
            as usize;
        canvas.fill(
            Rect {
                x0: cx,
                y0: format_caret_y0,
                x1: cx + 1,
                y1: format_caret_y1,
            },
            TEXT,
        );
    }
    // Format help text (2 lines to fit the width).
    t.draw(
        canvas,
        CONTENT_X as f32,
        (format_rect.y1 + 16) as f32,
        "Date/time: any chrono format, e.g. %Y %m %d %H %M %S",
        13.0,
        DIM,
    );
    t.draw(
        canvas,
        CONTENT_X as f32,
        (format_rect.y1 + 34) as f32,
        "Counter: %n (skip existing files) / %#n (persistent), e.g. %04n",
        13.0,
        DIM,
    );

    let version_baseline = t.baseline_for_center((UPDATE_ROW_Y + UPDATE_ROW_H / 2) as f32, 15.0);
    t.draw(
        canvas,
        CONTENT_X as f32,
        version_baseline,
        &format!("Version {}", update::CURRENT_VERSION),
        15.0,
        DIM,
    );

    // "Check for updates" button: same fill+centered-text look as other
    // buttons, always the same label now.
    let btn_rect = update_button_rect(text);
    let btn_color = if hover == Some(Btn::CheckForUpdates) {
        hover_tint(BTN_BG)
    } else {
        BTN_BG
    };
    canvas.fill(btn_rect, btn_color);
    let tw = t.text_width(CHECK_BTN_LABEL, 15.0);
    let lx = btn_rect.x0 as f32 + ((btn_rect.x1 - btn_rect.x0) as f32 - tw) / 2.0;
    let btn_baseline = t.baseline_for_center((btn_rect.y0 + btn_rect.y1) as f32 / 2.0, 15.0);
    t.draw(canvas, lx, btn_baseline, CHECK_BTN_LABEL, 15.0, TEXT);

    // Status row: the last check's outcome, plus an "Update" button once a
    // newer version is known.
    if let Some(status_label) = update_status_text(
        update_available.as_ref(),
        update_check_status,
        updating,
        update_install_error.as_deref(),
    ) {
        let status_baseline =
            t.baseline_for_center((UPDATE_STATUS_ROW_Y + UPDATE_STATUS_ROW_H / 2) as f32, 15.0);
        t.draw(
            canvas,
            CONTENT_X as f32,
            status_baseline,
            &status_label,
            15.0,
            DIM,
        );

        if update_available.is_some() {
            let dl_rect = download_button_rect(text, &status_label);
            let dl_color = if hover == Some(Btn::DownloadUpdate) {
                hover_tint(ACCENT)
            } else {
                ACCENT
            };
            canvas.fill(dl_rect, dl_color);
            let dl_tw = t.text_width(DOWNLOAD_BTN_LABEL, 15.0);
            let dl_lx = dl_rect.x0 as f32 + ((dl_rect.x1 - dl_rect.x0) as f32 - dl_tw) / 2.0;
            let dl_baseline = t.baseline_for_center((dl_rect.y0 + dl_rect.y1) as f32 / 2.0, 15.0);
            t.draw(
                canvas,
                dl_lx,
                dl_baseline,
                DOWNLOAD_BTN_LABEL,
                15.0,
                0x0011_1111,
            );
        }
    }

    let (_, cb_rect) = buttons
        .iter()
        .find(|(b, _)| *b == Btn::LaunchAtStartup)
        .expect("LaunchAtStartup は General タブに常に存在する");
    let box_size = 18;
    let box_y = cb_rect.y0 + (cb_rect.height() - box_size) / 2;
    let box_rect = Rect {
        x0: cb_rect.x0,
        y0: box_y,
        x1: cb_rect.x0 + box_size,
        y1: box_y + box_size,
    };
    canvas.fill(box_rect, if launch_at_startup { ACCENT } else { FIELD_BG });
    canvas.stroke(
        box_rect,
        if hover == Some(Btn::LaunchAtStartup) {
            ACCENT
        } else {
            0x0080_8080
        },
    );
    if launch_at_startup {
        // Checkmark (two short lines).
        let (x0, y0, x1, y1) = (
            box_rect.x0 as i64,
            box_rect.y0 as i64,
            box_rect.x1 as i64,
            box_rect.y1 as i64,
        );
        canvas.line(x0 + 3, y0 + 9, x0 + 7, y1 - 4, 2, 0x00FF_FFFF);
        canvas.line(x0 + 7, y1 - 4, x1 - 3, y0 + 3, 2, 0x00FF_FFFF);
    }
    let label_x = (cb_rect.x0 + box_size + 8) as f32;
    let baseline = t.baseline_for_center((cb_rect.y0 + cb_rect.y1) as f32 / 2.0, 15.0);
    t.draw(
        canvas,
        label_x,
        baseline,
        "Launch at Windows startup",
        15.0,
        TEXT,
    );
}

#[cfg(test)]
mod tests {
    use super::{UpdateCheckStatus, commit_filename_format_value, update_status_text};
    use crate::update::ReleaseInfo;

    #[test]
    fn commit_filename_format_value_trims_and_keeps_current_when_empty() {
        assert_eq!(
            commit_filename_format_value("pashari_%Y%m%d", "old"),
            "pashari_%Y%m%d"
        );
        assert_eq!(commit_filename_format_value("  spaced  ", "old"), "spaced");
        assert_eq!(commit_filename_format_value("", "old"), "old");
        assert_eq!(commit_filename_format_value("   ", "old"), "old");
    }

    fn test_release_info() -> ReleaseInfo {
        ReleaseInfo {
            version: "9.9.9".to_string(),
            url: "https://example.test/release".to_string(),
            exe_url: None,
            setup_url: None,
        }
    }

    #[test]
    fn update_status_text_is_none_before_any_check() {
        assert_eq!(update_status_text(None, None, false, None), None);
    }

    #[test]
    fn update_status_text_reports_a_newer_version_regardless_of_status() {
        let info = test_release_info();
        let text = update_status_text(Some(&info), None, false, None).unwrap();
        assert!(text.contains("9.9.9"));
        // Takes priority even if a stale status is still set (shouldn't
        // normally happen together, but `update_available` wins either way).
        let text_with_status =
            update_status_text(Some(&info), Some(UpdateCheckStatus::Failed), false, None).unwrap();
        assert!(text_with_status.contains("9.9.9"));
    }

    #[test]
    fn update_status_text_reports_up_to_date_and_failed_when_no_newer_version() {
        assert!(
            update_status_text(None, Some(UpdateCheckStatus::UpToDate), false, None)
                .unwrap()
                .contains("up to date")
        );
        assert!(
            update_status_text(None, Some(UpdateCheckStatus::Failed), false, None)
                .unwrap()
                .contains("failed")
        );
    }

    #[test]
    fn update_status_text_shows_downloading_while_updating() {
        let info = test_release_info();
        let text = update_status_text(Some(&info), None, true, None).unwrap();
        assert!(text.contains("Downloading"));
    }

    #[test]
    fn update_status_text_install_error_takes_priority_over_everything_else() {
        let info = test_release_info();
        let text = update_status_text(
            Some(&info),
            Some(UpdateCheckStatus::Failed),
            true,
            Some("boom"),
        )
        .unwrap();
        assert!(text.contains("boom"));
    }
}
