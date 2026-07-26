//! Output (png saving / clipboard copy).
//!
//! Currently just png saving and clipboard copy. gif/mp4 saving will be added here later.

use std::borrow::Cow;
use std::path::PathBuf;

use chrono::Local;
use image::RgbaImage;

/// A cropped selection region (RGBA8, top-to-bottom, left-to-right).
pub struct Shot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Saves the selection region as a png, returning the saved path.
pub fn save_png(shot: &Shot) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = output_path("png")?;
    save_png_to(shot, &path)?;
    Ok(path)
}

/// Saves the selection region as a png to a given path (for "save as";
/// unlike `output_path`, doesn't go through the filename template/counter).
pub fn save_png_to(shot: &Shot, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let img = RgbaImage::from_raw(shot.width, shot.height, shot.rgba.clone())
        .ok_or("画像バッファのサイズが width*height*4 と一致しません")?;
    img.save(path)?;
    Ok(())
}

/// Writes a png to the temp directory, returning its path (for handing off to the editor process).
///
/// The name is unique by pid + nanoseconds, so calling this multiple times in quick succession never collides.
pub fn save_temp_png(shot: &Shot) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = format!("pashari_edit_{}_{}.png", std::process::id(), nanos);
    let path = std::env::temp_dir().join(name);
    let img = RgbaImage::from_raw(shot.width, shot.height, shot.rgba.clone())
        .ok_or("画像バッファのサイズが width*height*4 と一致しません")?;
    img.save(&path)?;
    Ok(path)
}

/// Encodes as png in memory (for uses that don't go through a file, like upload).
pub fn encode_png(shot: &Shot) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let img = RgbaImage::from_raw(shot.width, shot.height, shot.rgba.clone())
        .ok_or("画像バッファのサイズが width*height*4 と一致しません")?;
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)?;
    Ok(buf)
}

/// Loads a png into a `Shot` (RGBA8) (used when the editor process starts).
pub fn load_shot(path: &std::path::Path) -> Result<Shot, Box<dyn std::error::Error>> {
    let img = image::open(path)?.to_rgba8();
    let (width, height) = (img.width(), img.height());
    Ok(Shot {
        width,
        height,
        rgba: img.into_raw(),
    })
}

/// Info for one filename-template counter token (the parse result of `%[#][0-9]*n`).
#[derive(Clone, Copy)]
struct CounterToken {
    /// Whether it has a `#` (persistent counter).
    persistent: bool,
    /// Zero-padding width (the `4` in `%04n`; defaults to 4 if omitted).
    width: usize,
}

/// The placeholder character for a counter token (a Unicode private-use
/// codepoint, which never collides with chrono's format or ordinary user input).
fn placeholder_char(i: usize) -> char {
    char::from_u32(0xE000 + i as u32).expect("私用領域の範囲内")
}

/// Assuming `s` starts with `%`, checks whether that's a counter token
/// (`%[#][0-9]*n`). Returns `(bytes consumed, token)` if it matches.
fn try_parse_counter_token(s: &str) -> Option<(usize, CounterToken)> {
    let rest = &s[1..];
    let bytes = rest.as_bytes();
    let mut idx = 0;
    let persistent = bytes.first() == Some(&b'#');
    if persistent {
        idx += 1;
    }
    let digit_start = idx;
    while bytes.get(idx).is_some_and(u8::is_ascii_digit) {
        idx += 1;
    }
    let width = if idx > digit_start {
        rest[digit_start..idx].parse::<usize>().ok()?
    } else {
        4
    };
    if bytes.get(idx) == Some(&b'n') {
        idx += 1;
        Some((1 + idx, CounterToken { persistent, width }))
    } else {
        None
    }
}

/// Extracts every `%[#][0-9]*n` from `template`, returning a "skeleton"
/// string with those positions replaced by placeholders, plus the
/// extracted tokens (the token order matches the placeholder sequence).
/// Any `%` that isn't a counter token is left as-is, to be expanded later
/// as a chrono strftime specifier.
fn extract_counter_tokens(template: &str) -> (String, Vec<CounterToken>) {
    let mut out = String::new();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < template.len() {
        if template.as_bytes()[i] == b'%'
            && let Some((len, token)) = try_parse_counter_token(&template[i..])
        {
            out.push(placeholder_char(tokens.len()));
            tokens.push(token);
            i += len;
            continue;
        }
        let ch = template[i..].chars().next().expect("i は文字境界");
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, tokens)
}

/// Expands only the template's date/time part (counter tokens stay as
/// placeholders). `chrono`'s `Display` impl can return an error for an
/// invalid format specifier, and calling `to_string()` directly would
/// panic via its internal `expect`. Writes via `write!` instead, falling
/// back to the (unexpanded) skeleton on failure so the app never crashes.
fn render_skeleton(
    template: &str,
    now: chrono::DateTime<chrono::Local>,
) -> (String, Vec<CounterToken>) {
    let (skeleton, tokens) = extract_counter_tokens(template);
    let mut dated = String::new();
    if std::fmt::Write::write_fmt(&mut dated, format_args!("{}", now.format(&skeleton))).is_err() {
        dated = skeleton.clone();
    }
    (dated, tokens)
}

/// Substitutes placeholders in a date-expanded skeleton with actual
/// values (the persistent counter's value is fetched once by the caller
/// and reused; `candidate` is used for the collision-avoidance counter).
/// Finally replaces characters forbidden in Windows filenames (`\ / : * ? " < > |`) with `_`.
fn substitute_counter_placeholders(
    dated: &str,
    tokens: &[CounterToken],
    persistent_value: Option<u32>,
    candidate: u32,
) -> String {
    let mut out = dated.to_string();
    for (i, tok) in tokens.iter().enumerate() {
        let ph = placeholder_char(i);
        let val = if tok.persistent {
            persistent_value.unwrap_or(0)
        } else {
            candidate
        };
        let rendered = format!("{val:0width$}", width = tok.width);
        out = out.replace(ph, &rendered);
    }
    sanitize_filename(&out)
}

/// Replaces characters forbidden in Windows filenames (`\ / : * ? " < > |`)
/// with `_` (a safeguard for freely-typed templates).
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c })
        .collect()
}

/// Prepares the output directory and returns the full path (extension
/// `ext`) from expanding the config's filename template. Resolved in one
/// pass if the template has no `%n` (non-persistent counter); otherwise
/// increments the candidate number until no file of that name exists in the destination.
pub fn output_path(ext: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = output_dir(ext)?;
    std::fs::create_dir_all(&dir)?;
    let (dated, tokens) = render_skeleton(&crate::store::filename_format(), Local::now());

    let persistent_value = tokens
        .iter()
        .any(|t| t.persistent)
        .then(crate::store::next_filename_counter);
    let has_fs_counter = tokens.iter().any(|t| !t.persistent);

    let mut candidate = 0u32;
    loop {
        let name = substitute_counter_placeholders(&dated, &tokens, persistent_value, candidate);
        let path = dir.join(format!("{name}.{ext}"));
        if !has_fs_counter || !path.exists() {
            return Ok(path);
        }
        candidate += 1;
    }
}

/// Copies the selection region to the clipboard as an image (no file saved).
///
/// On Windows, clipboard data is duplicated by the OS, so it survives after the process exits.
pub fn copy_to_clipboard(shot: &Shot) -> Result<(), Box<dyn std::error::Error>> {
    let image = arboard::ImageData {
        width: shot.width as usize,
        height: shot.height as usize,
        bytes: Cow::Borrowed(&shot.rgba),
    };
    let mut clipboard = arboard::Clipboard::new()?;
    clipboard.set_image(image)?;
    Ok(())
}

/// The save directory: the configured one if set, else
/// `%USERPROFILE%\Pictures\pashari`. Has no side effects (doesn't create
/// the directory), so it's also usable as-is for "look but don't create"
/// purposes like a dialog's initial folder.
pub(crate) fn output_dir(ext: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let save_dir = crate::store::save_dir_for(ext);
    if !save_dir.is_empty() {
        return Ok(PathBuf::from(save_dir));
    }
    let profile = std::env::var("USERPROFILE")?;
    Ok(PathBuf::from(profile).join("Pictures").join("pashari"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_now() -> chrono::DateTime<chrono::Local> {
        chrono::Local
            .with_ymd_and_hms(2026, 7, 21, 9, 5, 3)
            .unwrap()
    }

    #[test]
    fn extract_counter_tokens_parses_default_and_persistent_forms() {
        let (skeleton, tokens) = extract_counter_tokens("shot_%n");
        assert_eq!(skeleton, format!("shot_{}", placeholder_char(0)));
        assert_eq!(tokens.len(), 1);
        assert!(!tokens[0].persistent);
        assert_eq!(tokens[0].width, 4);

        let (skeleton, tokens) = extract_counter_tokens("shot_%#n");
        assert_eq!(skeleton, format!("shot_{}", placeholder_char(0)));
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].persistent);
        assert_eq!(tokens[0].width, 4);
    }

    #[test]
    fn extract_counter_tokens_parses_explicit_width() {
        let (_, tokens) = extract_counter_tokens("%04n");
        assert_eq!(tokens[0].width, 4);
        assert!(!tokens[0].persistent);

        let (_, tokens) = extract_counter_tokens("%#012n");
        assert_eq!(tokens[0].width, 12);
        assert!(tokens[0].persistent);
    }

    #[test]
    fn extract_counter_tokens_leaves_unrelated_percent_sequences_untouched() {
        let (skeleton, tokens) = extract_counter_tokens("pashari_%Y-%m-%d_%H-%M-%S");
        assert_eq!(skeleton, "pashari_%Y-%m-%d_%H-%M-%S");
        assert!(tokens.is_empty());
    }

    #[test]
    fn extract_counter_tokens_returns_input_unchanged_when_no_token_present() {
        let (skeleton, tokens) = extract_counter_tokens("no_counter_here");
        assert_eq!(skeleton, "no_counter_here");
        assert!(tokens.is_empty());
    }

    #[test]
    fn substitute_counter_placeholders_pads_persistent_and_candidate_values() {
        let tokens = vec![
            CounterToken {
                persistent: true,
                width: 4,
            },
            CounterToken {
                persistent: false,
                width: 2,
            },
        ];
        let dated = format!("{}_{}", placeholder_char(0), placeholder_char(1));
        let out = substitute_counter_placeholders(&dated, &tokens, Some(7), 3);
        assert_eq!(out, "0007_03");
    }

    #[test]
    fn substitute_counter_placeholders_sanitizes_forbidden_windows_characters() {
        let out = substitute_counter_placeholders(r#"a:b/c\d*e?f"g<h>i|j"#, &[], None, 0);
        assert_eq!(out, "a_b_c_d_e_f_g_h_i_j");
    }

    #[test]
    fn render_skeleton_default_template_matches_old_hardcoded_format() {
        let (dated, tokens) = render_skeleton("pashari_%Y-%m-%d_%H-%M-%S", sample_now());
        assert_eq!(dated, "pashari_2026-07-21_09-05-03");
        assert!(tokens.is_empty());
    }

    #[test]
    fn render_skeleton_does_not_panic_on_invalid_specifier() {
        // Even with an invalid format specifier (chrono doesn't
        // recognize %Q), it should return some string without panicking.
        let (dated, _) = render_skeleton("weird_%Q_shot", sample_now());
        assert!(!dated.is_empty());
    }

    #[test]
    fn render_skeleton_keeps_counter_tokens_as_placeholders() {
        let (dated, tokens) = render_skeleton("shot_%Y_%n", sample_now());
        assert_eq!(dated, format!("shot_2026_{}", placeholder_char(0)));
        assert_eq!(tokens.len(), 1);
    }
}
