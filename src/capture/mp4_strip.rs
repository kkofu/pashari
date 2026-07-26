//! Removes a silent audio track from an MP4 after the fact, when a
//! recording never actually used audio.
//!
//! `windows-capture`'s `VideoEncoder` fixes the audio track layout at
//! recording start (see [`super::audio_session`]), so if the recording
//! stayed silent throughout, this strips the audio `trak` from the
//! ISO-BMFF (MP4) `moov` afterward. Best-effort and non-destructive above
//! all: any unexpected structure (fragmentation, multiple `mdat`, etc.)
//! makes it bail out and do nothing. Protecting the recorded video from any
//! chance of corruption always outweighs the file-size savings.

use std::fs;
use std::path::{Path, PathBuf};

/// A parsed box: size, type, range. `start..end` is the whole box,
/// `start+header_len..end` is its payload.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BoxHeader {
    fourcc: [u8; 4],
    start: usize,
    header_len: usize,
    end: usize,
}

/// Reads one box at `data[start..]`. Returns `None` for unsupported forms —
/// 64-bit size (`size==1`) or "extends to EOF" (`size==0`) — so the caller
/// aborts safely.
fn parse_box(data: &[u8], start: usize, limit: usize) -> Option<BoxHeader> {
    if start + 8 > limit {
        return None;
    }
    let size32 = u32::from_be_bytes(data[start..start + 4].try_into().ok()?);
    if size32 < 8 {
        return None;
    }
    let fourcc: [u8; 4] = data[start + 4..start + 8].try_into().ok()?;
    let end = start.checked_add(size32 as usize)?;
    if end > limit {
        return None;
    }
    Some(BoxHeader {
        fourcc,
        start,
        header_len: 8,
        end,
    })
}

/// Scans `[start, limit)` as a gapless sequence of boxes. `None` if a box
/// fails to parse or the boxes don't exactly fill the range.
fn list_boxes(data: &[u8], start: usize, limit: usize) -> Option<Vec<BoxHeader>> {
    let mut v = Vec::new();
    let mut pos = start;
    while pos < limit {
        let b = parse_box(data, pos, limit)?;
        pos = b.end;
        v.push(b);
    }
    if pos != limit {
        return None;
    }
    Some(v)
}

/// Reads the handler type (`b"vide"`/`b"soun"`/etc.) from `trak`'s `mdia > hdlr`.
fn handler_type(data: &[u8], trak: &BoxHeader) -> Option<[u8; 4]> {
    let children = list_boxes(data, trak.start + trak.header_len, trak.end)?;
    let mdia = children.iter().find(|b| &b.fourcc == b"mdia")?;
    let mdia_children = list_boxes(data, mdia.start + mdia.header_len, mdia.end)?;
    let hdlr = mdia_children.iter().find(|b| &b.fourcc == b"hdlr")?;
    // FullBox(4: version+flags) + pre_defined(4) + handler_type(4).
    let payload = hdlr.start + hdlr.header_len;
    if payload + 12 > hdlr.end {
        return None;
    }
    data[payload + 8..payload + 12].try_into().ok()
}

/// Adds `shift` to each chunk offset in an `stco` (32-bit), clamped to 0.
fn patch_stco(data: &[u8], box_: &BoxHeader, shift: i64) -> Vec<u8> {
    let mut out = data[box_.start..box_.end].to_vec();
    let payload = box_.header_len;
    if payload + 8 > out.len() {
        return out;
    }
    let Ok(count_bytes) = out[payload + 4..payload + 8].try_into() else {
        return out;
    };
    let entry_count = u32::from_be_bytes(count_bytes) as usize;
    let entries_start = payload + 8;
    for i in 0..entry_count {
        let off = entries_start + i * 4;
        if off + 4 > out.len() {
            break;
        }
        let Ok(v_bytes) = out[off..off + 4].try_into() else {
            break;
        };
        let v = u32::from_be_bytes(v_bytes) as i64;
        let nv = (v + shift).max(0) as u32;
        out[off..off + 4].copy_from_slice(&nv.to_be_bytes());
    }
    out
}

/// The `co64` (64-bit) version.
fn patch_co64(data: &[u8], box_: &BoxHeader, shift: i64) -> Vec<u8> {
    let mut out = data[box_.start..box_.end].to_vec();
    let payload = box_.header_len;
    if payload + 8 > out.len() {
        return out;
    }
    let Ok(count_bytes) = out[payload + 4..payload + 8].try_into() else {
        return out;
    };
    let entry_count = u32::from_be_bytes(count_bytes) as usize;
    let entries_start = payload + 8;
    for i in 0..entry_count {
        let off = entries_start + i * 8;
        if off + 8 > out.len() {
            break;
        }
        let Ok(v_bytes) = out[off..off + 8].try_into() else {
            break;
        };
        let v = u64::from_be_bytes(v_bytes) as i64;
        let nv = (v + shift).max(0) as u64;
        out[off..off + 8].copy_from_slice(&nv.to_be_bytes());
    }
    out
}

/// Box types whose contents need recursing into.
const CONTAINERS: [&[u8; 4]; 5] = [b"moov", b"trak", b"mdia", b"minf", b"stbl"];

/// Rebuilds `box_`, dropping any direct child whose start matches
/// `skip_start` entirely. If an `stco`/`co64` is found, adds `shift` (0 =
/// no-op) to each of its offsets. Other leaves are copied as-is.
fn rebuild_box(data: &[u8], box_: &BoxHeader, skip_start: usize, shift: i64) -> Vec<u8> {
    if shift != 0 && &box_.fourcc == b"stco" {
        return patch_stco(data, box_, shift);
    }
    if shift != 0 && &box_.fourcc == b"co64" {
        return patch_co64(data, box_, shift);
    }
    if !CONTAINERS.contains(&&box_.fourcc) {
        return data[box_.start..box_.end].to_vec();
    }
    let Some(children) = list_boxes(data, box_.start + box_.header_len, box_.end) else {
        return data[box_.start..box_.end].to_vec();
    };
    let mut body = Vec::new();
    for child in &children {
        if child.start == skip_start {
            continue;
        }
        body.extend(rebuild_box(data, child, skip_start, shift));
    }
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(&box_.fourcc);
    out.extend_from_slice(&body);
    out
}

/// Returns a whole new file with the audio `trak` removed. `None` if there's
/// nothing to remove, or the structure is unexpected (fragmented, multiple
/// `mdat`, etc.) — the caller then leaves the original file untouched.
fn strip_bytes(data: &[u8]) -> Option<Vec<u8>> {
    let top = list_boxes(data, 0, data.len())?;
    if top.iter().any(|b| &b.fourcc == b"moof") {
        return None; // fragmented MP4s aren't supported
    }
    let mdat_boxes: Vec<&BoxHeader> = top.iter().filter(|b| &b.fourcc == b"mdat").collect();
    if mdat_boxes.len() != 1 {
        return None;
    }
    let mdat = *mdat_boxes[0];
    let moov = *top.iter().find(|b| &b.fourcc == b"moov")?;

    let moov_children = list_boxes(data, moov.start + moov.header_len, moov.end)?;
    if moov_children.iter().any(|b| &b.fourcc == b"mvex") {
        return None; // a fragmented moov isn't supported either
    }
    let audio_trak = moov_children
        .iter()
        .filter(|b| &b.fourcc == b"trak")
        .find(|t| handler_type(data, t) == Some(*b"soun"))?;

    let delta = (audio_trak.end - audio_trak.start) as i64;
    // If moov comes before mdat, shrinking moov shifts mdat earlier, so the
    // remaining track's absolute offsets (stco/co64) need correcting. Not
    // needed if moov comes after.
    let shift = if moov.start < mdat.start { -delta } else { 0 };

    let new_moov = rebuild_box(data, &moov, audio_trak.start, shift);

    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..moov.start]);
    out.extend_from_slice(&new_moov);
    out.extend_from_slice(&data[moov.end..]);
    Some(out)
}

/// Removes a silent audio track from an MP4 that never actually used audio
/// during recording (best-effort; leaves the original file untouched on
/// unexpected structure or I/O failure).
pub(crate) fn strip_unused_audio(path: &Path) {
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("音声トラック除去: ファイル読み込みに失敗（スキップ）: {e}");
            return;
        }
    };
    let Some(new_data) = strip_bytes(&data) else {
        return; // no audio track, or unexpected structure: do nothing
    };
    // Self-verify before writing out (can the top level still be parsed?).
    if list_boxes(&new_data, 0, new_data.len()).is_none() {
        eprintln!("音声トラック除去: 再構成結果の検証に失敗（スキップ）");
        return;
    }
    let tmp: PathBuf = format!("{}.tmp", path.display()).into();
    if let Err(e) = fs::write(&tmp, &new_data) {
        eprintln!("音声トラック除去: 一時ファイル書き出しに失敗（スキップ）: {e}");
        return;
    }
    if let Err(e) = fs::rename(&tmp, path) {
        eprintln!("音声トラック除去: 置き換えに失敗（スキップ）: {e}");
        let _ = fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_box(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(8 + body.len());
        v.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
        v.extend_from_slice(fourcc);
        v.extend_from_slice(body);
        v
    }

    fn mk_hdlr(handler: &[u8; 4]) -> Vec<u8> {
        let mut body = vec![0u8; 4]; // version+flags
        body.extend_from_slice(&[0, 0, 0, 0]); // pre_defined
        body.extend_from_slice(handler);
        body.extend_from_slice(&[0u8; 12]); // reserved
        body.push(0); // name (empty string plus terminator)
        mk_box(b"hdlr", &body)
    }

    fn mk_stco(offsets: &[u32]) -> Vec<u8> {
        let mut body = vec![0, 0, 0, 0];
        body.extend_from_slice(&(offsets.len() as u32).to_be_bytes());
        for &o in offsets {
            body.extend_from_slice(&o.to_be_bytes());
        }
        mk_box(b"stco", &body)
    }

    /// Builds a trak with a placeholder `0` offset in `stco` (rewritten
    /// later to the real mdat position via `set_stco_offset` in tests).
    fn mk_video_trak() -> Vec<u8> {
        let hdlr = mk_hdlr(b"vide");
        let stco = mk_stco(&[0]);
        let stbl = mk_box(b"stbl", &stco);
        let minf = mk_box(b"minf", &stbl);
        let mut mdia_body = hdlr;
        mdia_body.extend_from_slice(&minf);
        let mdia = mk_box(b"mdia", &mdia_body);
        mk_box(b"trak", &mdia)
    }

    fn mk_audio_trak() -> Vec<u8> {
        let hdlr = mk_hdlr(b"soun");
        let mdia = mk_box(b"mdia", &hdlr);
        mk_box(b"trak", &mdia)
    }

    /// Rewrites the video trak's first `stco` offset (big-endian u32, at a
    /// fixed position within `trak`). A small test-only helper tied to
    /// `mk_video_trak`'s structure.
    fn set_stco_offset(video_trak: &mut [u8], value: u32) {
        // trak(8) + mdia(8) + hdlr(8+25=33) + minf(8) + stbl(8) + stco(8) + version/flags(4) + count(4)
        let pos = 8 + 8 + (8 + 25) + 8 + 8 + 8 + 4 + 4;
        video_trak[pos..pos + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn build_file(with_audio: bool, moov_before_mdat: bool, mdat_payload: &[u8]) -> Vec<u8> {
        let ftyp = mk_box(b"ftyp", b"isomiso2avc1mp41");
        let mut video_trak = mk_video_trak();
        let mut moov_body = Vec::new();
        let mdat = mk_box(b"mdat", mdat_payload);

        // The mdat payload's start position isn't known until the final
        // layout is decided, so assemble first, compute the position,
        // rewrite stco, then finalize moov.
        let ftyp_len = ftyp.len();
        let audio_trak = if with_audio {
            Some(mk_audio_trak())
        } else {
            None
        };
        let audio_len = audio_trak.as_ref().map_or(0, |a| a.len());
        let moov_len = 8 + video_trak.len() + audio_len; // moov header + children

        let mdat_payload_start = if moov_before_mdat {
            ftyp_len + moov_len + 8
        } else {
            ftyp_len + 8
        };
        set_stco_offset(&mut video_trak, mdat_payload_start as u32);

        moov_body.extend_from_slice(&video_trak);
        if let Some(a) = &audio_trak {
            moov_body.extend_from_slice(a);
        }
        let moov = mk_box(b"moov", &moov_body);

        let mut file = ftyp;
        if moov_before_mdat {
            file.extend_from_slice(&moov);
            file.extend_from_slice(&mdat);
        } else {
            file.extend_from_slice(&mdat);
            file.extend_from_slice(&moov);
        }
        file
    }

    /// Parses the file and returns `len` bytes from the position pointed to
    /// by the video trak's stco[0] — verifies that following the offset
    /// actually reaches the sample data.
    fn read_via_stco(data: &[u8], len: usize) -> Option<Vec<u8>> {
        let top = list_boxes(data, 0, data.len())?;
        let moov = top.iter().find(|b| &b.fourcc == b"moov")?;
        let moov_children = list_boxes(data, moov.start + moov.header_len, moov.end)?;
        let video = moov_children
            .iter()
            .filter(|b| &b.fourcc == b"trak")
            .find(|t| handler_type(data, t) == Some(*b"vide"))?;
        let vc = list_boxes(data, video.start + video.header_len, video.end)?;
        let mdia = vc.iter().find(|b| &b.fourcc == b"mdia")?;
        let mc = list_boxes(data, mdia.start + mdia.header_len, mdia.end)?;
        let minf = mc.iter().find(|b| &b.fourcc == b"minf")?;
        let mnc = list_boxes(data, minf.start + minf.header_len, minf.end)?;
        let stbl = mnc.iter().find(|b| &b.fourcc == b"stbl")?;
        let sc = list_boxes(data, stbl.start + stbl.header_len, stbl.end)?;
        let stco = sc.iter().find(|b| &b.fourcc == b"stco")?;
        let entries_start = stco.start + stco.header_len + 8;
        let off =
            u32::from_be_bytes(data[entries_start..entries_start + 4].try_into().ok()?) as usize;
        data.get(off..off + len).map(|s| s.to_vec())
    }

    #[test]
    fn strips_audio_trak_and_shrinks_moov() {
        let payload = vec![0xABu8; 64];
        let file = build_file(true, true, &payload);
        let stripped = strip_bytes(&file).expect("audio trak が見つかるはず");
        assert!(stripped.len() < file.len());
        // Confirm the audio track is really gone after stripping.
        let top = list_boxes(&stripped, 0, stripped.len()).unwrap();
        let moov = top.iter().find(|b| &b.fourcc == b"moov").unwrap();
        let children = list_boxes(&stripped, moov.start + moov.header_len, moov.end).unwrap();
        let has_audio = children
            .iter()
            .filter(|b| &b.fourcc == b"trak")
            .any(|t| handler_type(&stripped, t) == Some(*b"soun"));
        assert!(!has_audio);
    }

    #[test]
    fn moov_before_mdat_shifts_stco_and_data_still_reachable() {
        let payload = vec![0xCDu8; 64];
        let file = build_file(true, true, &payload);
        // Sanity check on the test itself: reads correctly via stco before stripping too.
        assert_eq!(read_via_stco(&file, payload.len()), Some(payload.clone()));

        let stripped = strip_bytes(&file).unwrap();
        assert_eq!(read_via_stco(&stripped, payload.len()), Some(payload));
    }

    #[test]
    fn moov_after_mdat_needs_no_shift() {
        let payload = vec![0xEFu8; 32];
        let file = build_file(true, false, &payload);
        assert_eq!(read_via_stco(&file, payload.len()), Some(payload.clone()));

        let stripped = strip_bytes(&file).unwrap();
        assert_eq!(read_via_stco(&stripped, payload.len()), Some(payload));
    }

    #[test]
    fn no_audio_trak_returns_none() {
        let file = build_file(false, true, &[0u8; 16]);
        assert!(strip_bytes(&file).is_none());
    }

    #[test]
    fn fragmented_mp4_is_left_alone() {
        let mut file = build_file(true, true, &[0u8; 16]);
        file.extend_from_slice(&mk_box(b"moof", &[0u8; 4]));
        assert!(strip_bytes(&file).is_none());
    }

    #[test]
    fn truncated_box_header_is_rejected() {
        let data = [0u8, 0, 0, 20, b'f', b't']; // declared size 20, but not enough actual data
        assert!(list_boxes(&data, 0, data.len()).is_none());
    }
}
