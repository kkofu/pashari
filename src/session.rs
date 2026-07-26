//! A lightweight listing of Editor session history (knows nothing about `Annotation`).
//!
//! Actual item restoration (`Annotation` serialization) is handled by
//! `editor::session`. This only handles what the settings GUI's Recent
//! tab needs — scanning "folder list + thumbnail + date/time label" — and
//! deleting old sessions beyond the retention limit.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::ui::rgba_to_packed;

/// The target size for thumbnails written for the Recent tab (fit within
/// this box, keeping aspect ratio; referenced from `editor::session::record`).
pub const THUMB_W: u32 = 160;
pub const THUMB_H: u32 = 100;

/// Info for one session shown in the Recent tab.
pub struct SessionSummary {
    pub dir: PathBuf,
    /// The display label in "YYYY-MM-DD HH:MM:SS" format.
    pub label: String,
    pub thumb_w: usize,
    pub thumb_h: usize,
    pub thumb: Rc<Vec<u32>>,
}

/// The root of the session save location (`%APPDATA%\pashari\sessions`).
pub fn sessions_dir() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    Some(PathBuf::from(appdata).join("pashari").join("sessions"))
}

/// The session list, newest first. Silently skips entries whose
/// `thumb.png` can't be read/is corrupt (so one broken entry doesn't stop
/// the Recent tab from showing the rest).
pub fn list() -> Vec<SessionSummary> {
    match sessions_dir() {
        Some(dir) => list_in(&dir),
        None => Vec::new(),
    }
}

/// Trims the retained count down to `limit` (removes the oldest first).
pub fn prune(limit: usize) {
    if let Some(dir) = sessions_dir() {
        prune_in(&dir, limit);
    }
}

/// A session folder name (`{timestamp}_{pid}_{nanoseconds}`). pid +
/// nanoseconds, the same idea as `export::save_temp_png`, avoids
/// collisions even if multiple processes close at the same time. The
/// folder names' lexicographic order matches chronological order.
pub(crate) fn timestamp_dir_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}_{}_{}",
        chrono::Local::now().format("%Y-%m-%d_%H-%M-%S"),
        std::process::id(),
        nanos
    )
}

fn list_in(dir: &Path) -> Vec<SessionSummary> {
    let mut names = subdir_names(dir);
    names.sort_unstable();
    names.reverse(); // newest first.
    names
        .into_iter()
        .filter_map(|name| {
            let session_dir = dir.join(&name);
            let img = image::open(session_dir.join("thumb.png")).ok()?.to_rgba8();
            let (w, h) = (img.width() as usize, img.height() as usize);
            let thumb = rgba_to_packed(w, h, img.as_raw());
            Some(SessionSummary {
                dir: session_dir,
                label: label_from_dir_name(&name),
                thumb_w: w,
                thumb_h: h,
                thumb: Rc::new(thumb),
            })
        })
        .collect()
}

fn prune_in(dir: &Path, limit: usize) {
    let mut names = subdir_names(dir);
    names.sort_unstable(); // oldest first.
    if names.len() > limit {
        for name in &names[..names.len() - limit] {
            let _ = std::fs::remove_dir_all(dir.join(name));
        }
    }
}

fn subdir_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

/// Formats a folder name's first 19 characters ("YYYY-MM-DD_HH-MM-SS") as "YYYY-MM-DD HH:MM:SS".
fn label_from_dir_name(name: &str) -> String {
    if name.len() < 19 {
        return name.to_string();
    }
    let date = &name[0..10];
    let time = &name[11..19];
    format!("{date} {}", time.replace('-', ":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_dir_name_has_expected_shape() {
        let name = timestamp_dir_name();
        let parts: Vec<&str> = name.splitn(3, '_').collect();
        // Should split into 3 parts: "YYYY-MM-DD", "HH-MM-SS", and
        // "{pid}_{nanos}" (splitn(3) leaves pid and nanos together in the third part).
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].len(), 10);
        assert_eq!(parts[1].len(), 8);
        assert!(!parts[2].is_empty());
    }

    #[test]
    fn label_from_dir_name_formats_date_and_time() {
        assert_eq!(
            label_from_dir_name("2026-07-18_15-30-02_1234_5678"),
            "2026-07-18 15:30:02"
        );
    }

    #[test]
    fn label_from_dir_name_falls_back_to_raw_name_when_too_short() {
        assert_eq!(label_from_dir_name("weird"), "weird");
    }

    fn make_tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pashari_session_test_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn prune_in_removes_oldest_beyond_limit() {
        let root = make_tempdir("prune");
        for name in [
            "2026-01-01_00-00-01_1_1",
            "2026-01-02_00-00-01_1_1",
            "2026-01-03_00-00-01_1_1",
        ] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        prune_in(&root, 2);
        let mut remaining = subdir_names(&root);
        remaining.sort_unstable();
        assert_eq!(
            remaining,
            vec!["2026-01-02_00-00-01_1_1", "2026-01-03_00-00-01_1_1"]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn prune_in_keeps_everything_when_under_limit() {
        let root = make_tempdir("prune_under");
        std::fs::create_dir_all(root.join("2026-01-01_00-00-01_1_1")).unwrap();
        prune_in(&root, 10);
        assert_eq!(subdir_names(&root).len(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
