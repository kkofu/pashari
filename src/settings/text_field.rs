//! Shared single-line text field editing: caret/selection state and key
//! handling used by several Settings tabs.

use winit::keyboard::{ModifiersState, NamedKey};

use crate::ui::text::TextRenderer;

/// Caret/selection state for a text field; the value itself lives in the
/// caller's `String` buffer. Offsets are byte-based but always kept on char
/// boundaries.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) struct TextCursor {
    pub(super) cursor: usize,
    /// Other end of the selection; equal to `cursor` means no selection.
    pub(super) anchor: usize,
}

fn prev_char_boundary(s: &str, idx: usize) -> Option<usize> {
    if idx == 0 {
        return None;
    }
    s[..idx].char_indices().next_back().map(|(i, _)| i)
}

fn next_char_boundary(s: &str, idx: usize) -> Option<usize> {
    if idx >= s.len() {
        return None;
    }
    s[idx..].chars().next().map(|c| idx + c.len_utf8())
}

impl TextCursor {
    /// Caret at the end, no selection.
    pub(super) fn at_end(buf: &str) -> Self {
        Self {
            cursor: buf.len(),
            anchor: buf.len(),
        }
    }

    /// Selection range (low, high), or `None` if `cursor == anchor`.
    pub(super) fn selection(&self) -> Option<(usize, usize)> {
        if self.cursor == self.anchor {
            None
        } else {
            Some((self.cursor.min(self.anchor), self.cursor.max(self.anchor)))
        }
    }

    /// Without `extend` (Shift), an active selection just collapses to its
    /// left edge; otherwise moves one char left.
    pub(super) fn move_left(&mut self, buf: &str, extend: bool) {
        if !extend && let Some((lo, _)) = self.selection() {
            self.cursor = lo;
            self.anchor = lo;
            return;
        }
        if let Some(prev) = prev_char_boundary(buf, self.cursor) {
            self.cursor = prev;
        }
        if !extend {
            self.anchor = self.cursor;
        }
    }

    pub(super) fn move_right(&mut self, buf: &str, extend: bool) {
        if !extend && let Some((_, hi)) = self.selection() {
            self.cursor = hi;
            self.anchor = hi;
            return;
        }
        if let Some(next) = next_char_boundary(buf, self.cursor) {
            self.cursor = next;
        }
        if !extend {
            self.anchor = self.cursor;
        }
    }

    pub(super) fn move_home(&mut self, extend: bool) {
        self.cursor = 0;
        if !extend {
            self.anchor = 0;
        }
    }

    pub(super) fn move_end(&mut self, buf: &str, extend: bool) {
        self.cursor = buf.len();
        if !extend {
            self.anchor = buf.len();
        }
    }

    pub(super) fn select_all(&mut self, buf: &str) {
        self.anchor = 0;
        self.cursor = buf.len();
    }

    /// Click/drag positioning; with `extend` (dragging), only `cursor` moves
    /// so the selection grows.
    pub(super) fn set_from_click(&mut self, idx: usize, extend: bool) {
        self.cursor = idx;
        if !extend {
            self.anchor = idx;
        }
    }

    /// Deletes the selection if any, returns whether it did.
    pub(super) fn delete_selection(&mut self, buf: &mut String) -> bool {
        if let Some((lo, hi)) = self.selection() {
            buf.replace_range(lo..hi, "");
            self.cursor = lo;
            self.anchor = lo;
            true
        } else {
            false
        }
    }

    /// Replaces the selection, or inserts at the cursor.
    pub(super) fn insert(&mut self, buf: &mut String, s: &str) {
        self.delete_selection(buf);
        buf.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.anchor = self.cursor;
    }

    /// Backspace: deletes the selection, or the char before the cursor.
    pub(super) fn backspace(&mut self, buf: &mut String) {
        if self.delete_selection(buf) {
            return;
        }
        if let Some(prev) = prev_char_boundary(buf, self.cursor) {
            buf.replace_range(prev..self.cursor, "");
            self.cursor = prev;
            self.anchor = prev;
        }
    }

    /// Delete: deletes the selection, or the char after the cursor.
    pub(super) fn delete_forward(&mut self, buf: &mut String) {
        if self.delete_selection(buf) {
            return;
        }
        if let Some(next) = next_char_boundary(buf, self.cursor) {
            buf.replace_range(self.cursor..next, "");
        }
    }
}

/// Per-char fallback width when no font is loaded yet.
const FALLBACK_CHAR_W: f32 = 8.0;

/// Measured width of `s[..idx]`, or `FALLBACK_CHAR_W` per char without a
/// renderer.
pub(super) fn x_for_char_index(text: Option<&TextRenderer>, s: &str, size: f32, idx: usize) -> f32 {
    match text {
        Some(tr) => tr.text_width(&s[..idx], size),
        None => s[..idx].chars().count() as f32 * FALLBACK_CHAR_W,
    }
}

/// Byte offset of the `n`th char boundary, clamped to `s.len()`.
pub(super) fn byte_index_for_char_count(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len())
}

/// Byte offset of the char boundary closest to `rel_x`. Fields are short,
/// so a linear scan is fine.
pub(super) fn char_index_for_x(
    text: Option<&TextRenderer>,
    s: &str,
    size: f32,
    rel_x: f32,
) -> usize {
    let mut offsets: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
    offsets.push(s.len());
    let mut best = 0usize;
    let mut best_dist = f32::MAX;
    for o in offsets {
        let w = x_for_char_index(text, s, size, o);
        let d = (w - rel_x).abs();
        if d < best_dist {
            best_dist = d;
            best = o;
        }
    }
    best
}

/// Shared handling for arrows/Home/End/Backspace/Delete/Ctrl+A. Returns
/// whether it was handled (caller should redraw).
pub(super) fn apply_common_edit_key(
    cursor: &mut TextCursor,
    buf: &mut String,
    event: &winit::event::KeyEvent,
    mods: ModifiersState,
) -> bool {
    use winit::keyboard::Key;
    let shift = mods.shift_key();
    match &event.logical_key {
        Key::Named(NamedKey::ArrowLeft) => {
            cursor.move_left(buf, shift);
            true
        }
        Key::Named(NamedKey::ArrowRight) => {
            cursor.move_right(buf, shift);
            true
        }
        Key::Named(NamedKey::Home) => {
            cursor.move_home(shift);
            true
        }
        Key::Named(NamedKey::End) => {
            cursor.move_end(buf, shift);
            true
        }
        Key::Named(NamedKey::Backspace) => {
            cursor.backspace(buf);
            true
        }
        Key::Named(NamedKey::Delete) => {
            cursor.delete_forward(buf);
            true
        }
        Key::Character(s) if mods.control_key() && s.eq_ignore_ascii_case("a") => {
            cursor.select_all(buf);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_cursor_move_left_right_stop_at_bounds() {
        let buf = "abc";
        let mut c = TextCursor::at_end(buf); // cursor=3
        c.move_left(buf, false);
        assert_eq!(
            c,
            TextCursor {
                cursor: 2,
                anchor: 2
            }
        );
        c.move_left(buf, false);
        c.move_left(buf, false);
        assert_eq!(
            c,
            TextCursor {
                cursor: 0,
                anchor: 0
            }
        );
        c.move_left(buf, false); // stays at start
        assert_eq!(
            c,
            TextCursor {
                cursor: 0,
                anchor: 0
            }
        );
        c.move_right(buf, false);
        assert_eq!(
            c,
            TextCursor {
                cursor: 1,
                anchor: 1
            }
        );
        c.move_right(buf, false);
        c.move_right(buf, false);
        assert_eq!(
            c,
            TextCursor {
                cursor: 3,
                anchor: 3
            }
        );
        c.move_right(buf, false); // stays at end
        assert_eq!(
            c,
            TextCursor {
                cursor: 3,
                anchor: 3
            }
        );
    }

    #[test]
    fn text_cursor_move_left_right_skip_whole_multibyte_char() {
        let buf = "aあb"; // "あ" is 3 bytes.
        let mut c = TextCursor::at_end(buf);
        c.move_left(buf, false);
        assert_eq!(c.cursor, 4); // before "b" ("aあ" is 4 bytes)
        c.move_left(buf, false);
        assert_eq!(c.cursor, 1); // skips the whole multibyte char, lands after "a"
        c.move_right(buf, false);
        assert_eq!(c.cursor, 4);
    }

    #[test]
    fn text_cursor_shift_extends_selection_and_plain_arrow_collapses_it() {
        let buf = "abcdef";
        let mut c = TextCursor {
            cursor: 2,
            anchor: 2,
        };
        c.move_right(buf, true);
        c.move_right(buf, true);
        assert_eq!(c.selection(), Some((2, 4))); // anchor stays at 2, only cursor advances
        // Arrow without Shift just collapses to the relevant selection edge.
        c.move_right(buf, false);
        assert_eq!(
            c,
            TextCursor {
                cursor: 4,
                anchor: 4
            }
        );
        assert_eq!(c.selection(), None);

        let mut c2 = TextCursor {
            cursor: 4,
            anchor: 2,
        };
        c2.move_left(buf, false);
        assert_eq!(
            c2,
            TextCursor {
                cursor: 2,
                anchor: 2
            }
        );
    }

    #[test]
    fn text_cursor_home_end_and_select_all() {
        let buf = "hello";
        let mut c = TextCursor {
            cursor: 2,
            anchor: 2,
        };
        c.move_home(false);
        assert_eq!(
            c,
            TextCursor {
                cursor: 0,
                anchor: 0
            }
        );
        c.move_end(buf, false);
        assert_eq!(
            c,
            TextCursor {
                cursor: 5,
                anchor: 5
            }
        );

        let mut c2 = TextCursor {
            cursor: 3,
            anchor: 3,
        };
        c2.move_home(true);
        assert_eq!(c2.selection(), Some((0, 3)));

        let mut c3 = TextCursor::default();
        c3.select_all(buf);
        assert_eq!(c3.selection(), Some((0, 5)));
    }

    #[test]
    fn text_cursor_insert_replaces_selection_or_inserts_at_cursor() {
        let mut buf = "abcdef".to_string();
        let mut c = TextCursor {
            cursor: 4,
            anchor: 1,
        }; // selection "bcd" (1..4)
        c.insert(&mut buf, "X");
        assert_eq!(buf, "aXef");
        assert_eq!(
            c,
            TextCursor {
                cursor: 2,
                anchor: 2
            }
        );

        let mut buf2 = "abc".to_string();
        let mut c2 = TextCursor {
            cursor: 1,
            anchor: 1,
        };
        c2.insert(&mut buf2, "Z");
        assert_eq!(buf2, "aZbc");
        assert_eq!(
            c2,
            TextCursor {
                cursor: 2,
                anchor: 2
            }
        );
    }

    #[test]
    fn text_cursor_backspace_and_delete_forward() {
        // No selection: backspace removes the char before the cursor.
        let mut buf2 = "abc".to_string();
        let mut c2 = TextCursor {
            cursor: 2,
            anchor: 2,
        };
        c2.backspace(&mut buf2);
        assert_eq!(buf2, "ac");
        assert_eq!(c2.cursor, 1);

        // No selection: delete_forward removes the char after the cursor
        // (cursor doesn't move).
        let mut buf3 = "abc".to_string();
        let mut c3 = TextCursor {
            cursor: 1,
            anchor: 1,
        };
        c3.delete_forward(&mut buf3);
        assert_eq!(buf3, "ac");
        assert_eq!(c3.cursor, 1);

        // With a selection, both backspace and delete_forward just remove it.
        let mut buf4 = "abcdef".to_string();
        let mut c4 = TextCursor {
            cursor: 4,
            anchor: 1,
        };
        c4.backspace(&mut buf4);
        assert_eq!(buf4, "aef");
        assert_eq!(
            c4,
            TextCursor {
                cursor: 1,
                anchor: 1
            }
        );
    }

    #[test]
    fn x_for_char_index_uses_fallback_width_without_a_text_renderer() {
        assert_eq!(x_for_char_index(None, "abc", 15.0, 0), 0.0);
        assert_eq!(
            x_for_char_index(None, "abc", 15.0, 2),
            2.0 * FALLBACK_CHAR_W
        );
        assert_eq!(
            x_for_char_index(None, "abc", 15.0, 3),
            3.0 * FALLBACK_CHAR_W
        );
    }

    #[test]
    fn byte_index_for_char_count_handles_multibyte_and_out_of_range() {
        // Multibyte chars resolve to the correct byte offset.
        assert_eq!(byte_index_for_char_count("aあb", 0), 0);
        assert_eq!(byte_index_for_char_count("aあb", 1), 1);
        assert_eq!(byte_index_for_char_count("aあb", 2), 1 + 'あ'.len_utf8());
        // Out-of-range clamps to the byte length.
        assert_eq!(byte_index_for_char_count("aあb", 3), "aあb".len());
        assert_eq!(byte_index_for_char_count("aあb", 99), "aあb".len());
    }

    #[test]
    fn char_index_for_x_picks_nearest_boundary_without_a_text_renderer() {
        // Fallback width 8px/char: boundaries of "abc" are at x = 0, 8, 16, 24.
        assert_eq!(char_index_for_x(None, "abc", 15.0, -5.0), 0);
        assert_eq!(char_index_for_x(None, "abc", 15.0, 0.0), 0);
        assert_eq!(char_index_for_x(None, "abc", 15.0, 3.0), 0); // 0 is closer than 8
        assert_eq!(char_index_for_x(None, "abc", 15.0, 5.0), 1); // 8 is closer than 0
        assert_eq!(char_index_for_x(None, "abc", 15.0, 24.0), 3);
        assert_eq!(char_index_for_x(None, "abc", 15.0, 999.0), 3); // clamps at the end
    }
}
