//! Serializes and restores a session (source image plus placed items). The
//! only place that touches `Annotation`'s (`pub(super)`) internals. The
//! directory layout is `meta.json` (read/written here), one `img_{n}.png`
//! per Image item, and `thumb.png` (pre-rendered bytes passed in by the
//! caller).
//!
//! Picking the target folder and pruning old sessions is left to
//! `crate::session`, a thin module that knows nothing about `Annotation`.

use std::path::Path;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::ui::{packed_to_rgba_alpha, rgba_to_packed_alpha};

use super::annotation::{Annotation, MosaicMode};

#[derive(Serialize, Deserialize)]
struct SessionMeta {
    width: u32,
    height: u32,
    items: Vec<SerAnnotation>,
}

/// A serialization mirror with a 1:1 mapping to `Annotation`. Only `Image`
/// differs: it holds the sidecar PNG's filename instead of `pixels`.
#[derive(Serialize, Deserialize)]
enum SerAnnotation {
    Image {
        r: (f64, f64, f64, f64),
        src_w: i64,
        src_h: i64,
        file: String,
        rot: f64,
    },
    Arrow {
        a: (i64, i64),
        b: (i64, i64),
        color: u32,
        thick: i64,
    },
    Polyline {
        points: Vec<(i64, i64)>,
        color: u32,
        thick: i64,
    },
    Rect {
        r: (f64, f64, f64, f64),
        color: u32,
        thick: i64,
        rot: f64,
        /// Defaults to `false` so sessions saved before `filled` existed still load.
        #[serde(default)]
        filled: bool,
    },
    Ellipse {
        r: (f64, f64, f64, f64),
        color: u32,
        thick: i64,
        rot: f64,
        /// Defaults to `false` so sessions saved before `filled` existed still load.
        #[serde(default)]
        filled: bool,
    },
    Text {
        pos: (i64, i64),
        text: String,
        color: u32,
        size: f32,
        rot: f64,
    },
    NumberMarker {
        pos: (i64, i64),
        number: u32,
        color: u32,
        size: f32,
    },
    Guide {
        r: (i64, i64, i64, i64),
    },
    Mosaic {
        r: (f64, f64, f64, f64),
        rot: f64,
        block: f32,
        /// Stored as a bool rather than carrying over `Annotation::Mosaic`'s
        /// `mode` (Pixelate/Blur), matching `SerAnnotation`'s convention of
        /// primitive-only fields. Like `filled`, defaults to `false`
        /// (Pixelate) so sessions without it still load.
        #[serde(default)]
        blur: bool,
        /// PRNG seed for Blur. Defaults to 0 for sessions predating it
        /// (harmless, since it's unused for Pixelate).
        #[serde(default)]
        seed: u32,
    },
}

fn io_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// Writes `meta.json`, `thumb.png`, and (for any Image items) `img_{n}.png`
/// into `dir` (already created by the caller).
pub(crate) fn save(
    dir: &Path,
    annotations: &[Annotation],
    width: u32,
    height: u32,
    thumb_png: &[u8],
) -> std::io::Result<()> {
    let mut img_index = 0usize;
    let mut items = Vec::with_capacity(annotations.len());
    for ann in annotations {
        items.push(match ann {
            Annotation::Image {
                r,
                src_w,
                src_h,
                pixels,
                rot,
            } => {
                let file = format!("img_{img_index}.png");
                img_index += 1;
                let rgba = packed_to_rgba_alpha(*src_w as usize, *src_h as usize, pixels);
                let img = image::RgbaImage::from_raw(*src_w as u32, *src_h as u32, rgba)
                    .ok_or_else(|| {
                        io_err("画像バッファのサイズが width*height*4 と一致しません")
                    })?;
                img.save(dir.join(&file)).map_err(io_err)?;
                SerAnnotation::Image {
                    r: *r,
                    src_w: *src_w,
                    src_h: *src_h,
                    file,
                    rot: *rot,
                }
            }
            Annotation::Arrow { a, b, color, thick } => SerAnnotation::Arrow {
                a: *a,
                b: *b,
                color: *color,
                thick: *thick,
            },
            Annotation::Polyline {
                points,
                color,
                thick,
            } => SerAnnotation::Polyline {
                points: points.clone(),
                color: *color,
                thick: *thick,
            },
            Annotation::Rect {
                r,
                color,
                thick,
                rot,
                filled,
            } => SerAnnotation::Rect {
                r: *r,
                color: *color,
                thick: *thick,
                rot: *rot,
                filled: *filled,
            },
            Annotation::Ellipse {
                r,
                color,
                thick,
                rot,
                filled,
            } => SerAnnotation::Ellipse {
                r: *r,
                color: *color,
                thick: *thick,
                rot: *rot,
                filled: *filled,
            },
            Annotation::Text {
                pos,
                text,
                color,
                size,
                rot,
            } => SerAnnotation::Text {
                pos: *pos,
                text: text.clone(),
                color: *color,
                size: *size,
                rot: *rot,
            },
            Annotation::NumberMarker {
                pos,
                number,
                color,
                size,
            } => SerAnnotation::NumberMarker {
                pos: *pos,
                number: *number,
                color: *color,
                size: *size,
            },
            Annotation::Guide { r } => SerAnnotation::Guide { r: *r },
            Annotation::Mosaic {
                r,
                rot,
                block,
                mode,
                seed,
            } => SerAnnotation::Mosaic {
                r: *r,
                rot: *rot,
                block: *block,
                blur: matches!(mode, MosaicMode::Blur),
                seed: *seed,
            },
        });
    }

    let meta = SessionMeta {
        width,
        height,
        items,
    };
    let meta_json = serde_json::to_vec_pretty(&meta).map_err(io_err)?;
    std::fs::write(dir.join("meta.json"), meta_json)?;
    std::fs::write(dir.join("thumb.png"), thumb_png)?;
    Ok(())
}

/// Loads a session written by `save` and restores its `Annotation` list.
pub(crate) fn load(dir: &Path) -> std::io::Result<(usize, usize, Vec<Annotation>)> {
    let text = std::fs::read_to_string(dir.join("meta.json"))?;
    let meta: SessionMeta = serde_json::from_str(&text).map_err(io_err)?;

    let mut annotations = Vec::with_capacity(meta.items.len());
    for item in meta.items {
        annotations.push(match item {
            SerAnnotation::Image {
                r,
                src_w,
                src_h,
                file,
                rot,
            } => {
                let img = image::open(dir.join(&file)).map_err(io_err)?.to_rgba8();
                let pixels = rgba_to_packed_alpha(src_w as usize, src_h as usize, img.as_raw());
                Annotation::Image {
                    r,
                    src_w,
                    src_h,
                    pixels: Rc::new(pixels),
                    rot,
                }
            }
            SerAnnotation::Arrow { a, b, color, thick } => Annotation::Arrow { a, b, color, thick },
            SerAnnotation::Polyline {
                points,
                color,
                thick,
            } => Annotation::Polyline {
                points,
                color,
                thick,
            },
            SerAnnotation::Rect {
                r,
                color,
                thick,
                rot,
                filled,
            } => Annotation::Rect {
                r,
                color,
                thick,
                rot,
                filled,
            },
            SerAnnotation::Ellipse {
                r,
                color,
                thick,
                rot,
                filled,
            } => Annotation::Ellipse {
                r,
                color,
                thick,
                rot,
                filled,
            },
            SerAnnotation::Text {
                pos,
                text,
                color,
                size,
                rot,
            } => Annotation::Text {
                pos,
                text,
                color,
                size,
                rot,
            },
            SerAnnotation::NumberMarker {
                pos,
                number,
                color,
                size,
            } => Annotation::NumberMarker {
                pos,
                number,
                color,
                size,
            },
            SerAnnotation::Guide { r } => Annotation::Guide { r },
            SerAnnotation::Mosaic {
                r,
                rot,
                block,
                blur,
                seed,
            } => Annotation::Mosaic {
                r,
                rot,
                block,
                mode: if blur {
                    MosaicMode::Blur
                } else {
                    MosaicMode::Pixelate
                },
                seed,
            },
        });
    }
    Ok((meta.width as usize, meta.height as usize, annotations))
}

/// Creates a new session folder, calls `save`, then prunes down to `limit`
/// sessions. The production entry point called from `Editor`'s save hook
/// (folder selection is delegated to `crate::session`).
pub(crate) fn record(
    annotations: &[Annotation],
    width: u32,
    height: u32,
    thumb_png: &[u8],
    limit: usize,
) {
    let Some(root) = crate::session::sessions_dir() else {
        return;
    };
    let dir = root.join(crate::session::timestamp_dir_name());
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Err(e) = save(&dir, annotations, width, height, thumb_png) {
        eprintln!("セッションの保存に失敗: {e}");
    }
    crate::session::prune(limit);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tempdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pashari_editor_session_test_{tag}_{}_{}",
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
    fn save_then_load_round_trips_non_image_annotations() {
        let dir = make_tempdir("basic");
        let annotations = vec![
            Annotation::Arrow {
                a: (1, 2),
                b: (3, 4),
                color: 0x0011_2233,
                thick: 3,
            },
            Annotation::Polyline {
                points: vec![(1, 2), (3, 4), (5, 2)],
                color: 0x0022_3344,
                thick: 2,
            },
            Annotation::Rect {
                r: (1.0, 2.0, 30.0, 40.0),
                color: 0x0044_5566,
                thick: 2,
                rot: 0.4,
                filled: true,
            },
            Annotation::Ellipse {
                r: (5.0, 6.0, 50.0, 60.0),
                color: 0x0077_8899,
                thick: 4,
                rot: 0.1,
                filled: false,
            },
            Annotation::Text {
                pos: (7, 8),
                text: "hello".into(),
                color: 0x00AA_BBCC,
                size: 14.0,
                rot: 0.2,
            },
            Annotation::NumberMarker {
                pos: (9, 10),
                number: 3,
                color: 0x00DD_EEFF,
                size: 12.0,
            },
            Annotation::Guide { r: (0, 0, 100, 80) },
            Annotation::Mosaic {
                r: (11.0, 12.0, 33.0, 44.0),
                rot: 0.5,
                block: 20.0,
                mode: MosaicMode::Blur,
                seed: 12345,
            },
        ];
        save(&dir, &annotations, 100, 80, &[1, 2, 3]).unwrap();
        let (w, h, restored) = load(&dir).unwrap();
        assert_eq!((w, h), (100, 80));
        assert_eq!(restored.len(), annotations.len());
        match (&restored[0], &annotations[0]) {
            (Annotation::Arrow { a: a1, b: b1, .. }, Annotation::Arrow { a: a2, b: b2, .. }) => {
                assert_eq!((a1, b1), (a2, b2))
            }
            _ => panic!("Arrow のはず"),
        }
        match &restored[1] {
            Annotation::Polyline {
                points,
                color,
                thick,
            } => {
                assert_eq!(*points, vec![(1, 2), (3, 4), (5, 2)]);
                assert_eq!(*color, 0x0022_3344);
                assert_eq!(*thick, 2);
            }
            _ => panic!("Polyline のはず"),
        }
        match &restored[2] {
            Annotation::Rect { filled, .. } => assert!(*filled),
            _ => panic!("Rect のはず"),
        }
        match &restored[3] {
            Annotation::Ellipse { filled, .. } => assert!(!*filled),
            _ => panic!("Ellipse のはず"),
        }
        match &restored[5] {
            Annotation::NumberMarker {
                pos,
                number,
                color,
                size,
            } => {
                assert_eq!(*pos, (9, 10));
                assert_eq!(*number, 3);
                assert_eq!(*color, 0x00DD_EEFF);
                assert_eq!(*size, 12.0);
            }
            _ => panic!("NumberMarker のはず"),
        }
        match &restored[6] {
            Annotation::Guide { r } => assert_eq!(*r, (0, 0, 100, 80)),
            _ => panic!("Guide のはず"),
        }
        match &restored[7] {
            Annotation::Mosaic {
                r,
                rot,
                block,
                mode,
                seed,
            } => {
                assert_eq!(*r, (11.0, 12.0, 33.0, 44.0));
                assert_eq!(*rot, 0.5);
                assert_eq!(*block, 20.0);
                assert_eq!(*mode, MosaicMode::Blur);
                assert_eq!(*seed, 12345);
            }
            _ => panic!("Mosaic のはず"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_defaults_filled_to_false_for_old_sessions_missing_the_field() {
        // A meta.json saved before `filled` existed (no "filled" key on
        // Rect/Ellipse) should still load.
        let dir = make_tempdir("old_format");
        let meta_json = r#"{
            "width": 100,
            "height": 80,
            "items": [
                {"Rect": {"r": [0.0, 0.0, 10.0, 10.0], "color": 0, "thick": 1, "rot": 0.0}},
                {"Ellipse": {"r": [0.0, 0.0, 10.0, 10.0], "color": 0, "thick": 1, "rot": 0.0}}
            ]
        }"#;
        std::fs::write(dir.join("meta.json"), meta_json).unwrap();
        let (_, _, restored) = load(&dir).unwrap();
        match &restored[0] {
            Annotation::Rect { filled, .. } => assert!(!*filled),
            _ => panic!("Rect のはず"),
        }
        match &restored[1] {
            Annotation::Ellipse { filled, .. } => assert!(!*filled),
            _ => panic!("Ellipse のはず"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_defaults_mosaic_to_pixelate_and_zero_seed_for_old_sessions_missing_the_fields() {
        // A meta.json saved before `blur`/`seed` existed (no such keys on
        // Mosaic) should still load, as Pixelate with seed 0.
        let dir = make_tempdir("old_format_mosaic");
        let meta_json = r#"{
            "width": 100,
            "height": 80,
            "items": [
                {"Mosaic": {"r": [0.0, 0.0, 10.0, 10.0], "rot": 0.0, "block": 16.0}}
            ]
        }"#;
        std::fs::write(dir.join("meta.json"), meta_json).unwrap();
        let (_, _, restored) = load(&dir).unwrap();
        match &restored[0] {
            Annotation::Mosaic { mode, seed, .. } => {
                assert_eq!(*mode, MosaicMode::Pixelate);
                assert_eq!(*seed, 0);
            }
            _ => panic!("Mosaic のはず"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_then_load_round_trips_image_pixels() {
        let dir = make_tempdir("image");
        // 2x1: red, green.
        let pixels = vec![0x00FF_0000u32, 0x0000_FF00];
        let annotations = vec![Annotation::Image {
            r: (0.0, 0.0, 2.0, 1.0),
            src_w: 2,
            src_h: 1,
            pixels: Rc::new(pixels.clone()),
            rot: 0.0,
        }];
        save(&dir, &annotations, 2, 1, &[9, 9, 9]).unwrap();
        assert!(dir.join("img_0.png").exists());
        let (_, _, restored) = load(&dir).unwrap();
        match &restored[0] {
            Annotation::Image { pixels: p, .. } => assert_eq!(**p, pixels),
            _ => panic!("Image のはず"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_then_load_round_trips_image_alpha() {
        // Pixels covering semi-transparent, fully opaque, and fully
        // transparent should keep their alpha (top byte) after a round
        // trip through the sidecar PNG.
        let dir = make_tempdir("image_alpha");
        let pixels = vec![0xFFFF_0000u32, 0x8000_FF00, 0x0000_00FFu32];
        let annotations = vec![Annotation::Image {
            r: (0.0, 0.0, 3.0, 1.0),
            src_w: 3,
            src_h: 1,
            pixels: Rc::new(pixels.clone()),
            rot: 0.0,
        }];
        save(&dir, &annotations, 3, 1, &[9, 9, 9]).unwrap();
        let (_, _, restored) = load(&dir).unwrap();
        match &restored[0] {
            Annotation::Image { pixels: p, .. } => assert_eq!(**p, pixels),
            _ => panic!("Image のはず"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
