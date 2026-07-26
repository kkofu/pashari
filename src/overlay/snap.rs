//! Automatic region snapping to windows and child controls.
//!
//! Enumerates top-level windows once at capture start to build a snapshot
//! (matching the frozen-image approach), and every subsequent hover is
//! hit-tested against that snapshot. Within a top-level window,
//! `EnumChildWindows` collects every descendant HWND, adopting whichever
//! contains the cursor with the smallest area.
//!
//! Descendant lookup originally walked one level at a time via
//! `RealChildWindowFromPoint`/`ChildWindowFromPoint`, but on some
//! Chromium-based browsers this got stuck on an intermediate GPU-compositing
//! window (near full-window size) and never reached the content-area HWND
//! behind it (a child window kept around for accessibility compatibility,
//! typically with a `*_RenderWidgetHostHWND`-style class name).
//! `EnumChildWindows` instead enumerates every descendant regardless of
//! depth, so it's unaffected by z-order or transparent hit-testing —
//! preferring the smallest area naturally favors the content area over the
//! intermediate surface.
//!
//! The OS-dependent enumeration/raw Win32 calls are kept separate from the
//! testable pure geometry logic (`fully_covered`/`is_adoptable_descendant`/
//! `smallest_containing`/`abs_to_src_clipped`).

use core::ffi::c_void;

use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GWL_EXSTYLE, GetClassNameW, GetWindowInfo, GetWindowLongW,
    GetWindowRect, GetWindowTextW, IsWindowVisible, WINDOWINFO, WS_CAPTION, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_MINIMIZE,
};

use crate::ui::Rect;

/// Minimum size (px) for a top-level window; anything smaller isn't a candidate.
const MIN_WINDOW_SIZE: i64 = 25;
/// Minimum size (px) to adopt as a child control.
const MIN_WINDOW_CHILD_SIZE: i64 = 200;

/// A rect `(left, top, right, bottom)` in absolute (virtual-desktop)
/// coordinates. right/bottom are exclusive bounds, like RECT. Kept as i64
/// to avoid overflow in difference calculations.
type RectI = (i64, i64, i64, i64);

/// A top-level window found at snapshot time.
struct SnapWindow {
    /// Starting point for child HWND lookup (passed to Win32 APIs as-is, in absolute coordinates).
    hwnd: isize,
    /// The true bounding rect (absolute coordinates), used for child lookup/clipping.
    rect_abs: RectI,
    /// Rect normalized to src coordinates (clipped to the image bounds), used for display/selection.
    rect_src: Rect,
}

/// The list of top-level windows at capture start (front-to-back Z order).
pub(super) struct Snapshot {
    windows: Vec<SnapWindow>,
    origin: (i32, i32),
    img_w: i64,
    img_h: i64,
}

/// Working state passed to the `EnumWindows` callback during enumeration.
struct Collector {
    origin: (i32, i32),
    img_w: i64,
    img_h: i64,
    windows: Vec<SnapWindow>,
}

impl Snapshot {
    /// Enumerates currently visible top-level windows into a snapshot.
    /// Returns an empty snapshot if enumeration fails (snapping is simply
    /// disabled; existing behavior is preserved).
    ///
    /// Must be called **before** creating the overlay window (so it
    /// doesn't include itself as a candidate).
    pub(super) fn capture(origin: (i32, i32), img_size: (usize, usize)) -> Self {
        let (img_w, img_h) = (img_size.0 as i64, img_size.1 as i64);
        let mut collector = Collector {
            origin,
            img_w,
            img_h,
            windows: Vec::new(),
        };
        // SAFETY: EnumWindows just calls the callback synchronously for
        // each top-level window. `lparam` carries a raw pointer to
        // `collector`, dereferenced exclusively inside the callback (no
        // race, since enumeration runs synchronously on one thread).
        unsafe {
            let _ = EnumWindows(
                Some(enum_proc),
                LPARAM(&mut collector as *mut Collector as isize),
            );
        }
        Self {
            windows: collector.windows,
            origin,
            img_w,
            img_h,
        }
    }

    /// The rect (src coordinates) to snap to directly under `p_src` (src
    /// coordinates). If a top-level window is found, adopts whichever of
    /// its descendants containing the cursor has the smallest area.
    /// `None` if nothing is hit.
    pub(super) fn element_at(&self, p_src: (f64, f64)) -> Option<Rect> {
        let px = p_src.0.floor() as i64;
        let py = p_src.1.floor() as i64;
        let top = self.window_at_index(px, py)?;
        let top = &self.windows[top];

        let abs_px = px + self.origin.0 as i64;
        let abs_py = py + self.origin.1 as i64;
        // SAFETY: top.hwnd is a valid top-level window obtained at snapshot time.
        let descendants = unsafe { collect_descendants(HWND(top.hwnd as *mut c_void)) };
        let ideal_abs = smallest_containing(&descendants, abs_px, abs_py).unwrap_or(top.rect_abs);

        // Clip to the top-level rect before converting to src.
        let clipped = intersect(ideal_abs, top.rect_abs);
        abs_to_src_clipped(clipped, self.origin, self.img_w, self.img_h).or(Some(top.rect_src))
    }

    /// Top-level-only hit-testing (doesn't look at descendants); split out for testing.
    fn window_at_index(&self, px: i64, py: i64) -> Option<usize> {
        self.windows.iter().position(|w| {
            let (x0, y0, x1, y1) = (
                w.rect_src.x0 as i64,
                w.rect_src.y0 as i64,
                w.rect_src.x1 as i64,
                w.rect_src.y1 as i64,
            );
            px >= x0 && px < x1 && py >= y0 && py < y1
        })
    }
}

/// The `EnumWindows` callback. `lparam` is a raw pointer to `Collector`.
///
/// # Safety
/// Only called by `EnumWindows`, and `lparam` points to a valid `Collector`.
unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is a valid pointer to the Collector passed by
    // capture(). Enumeration is synchronous and single-threaded, so
    // exclusive dereferencing is safe.
    let collector = unsafe { &mut *(lparam.0 as *mut Collector) };
    if let Some(win) =
        unsafe { evaluate_window(hwnd, collector.origin, collector.img_w, collector.img_h) }
    {
        // Drops anything fully covered (i.e. invisible) by a window already in front.
        if !fully_covered(win.rect_src, &collector.windows) {
            collector.windows.push(win);
        }
    }
    TRUE // continue enumeration
}

/// Evaluates one top-level window, returning a `SnapWindow` if it should
/// be added as a candidate.
///
/// # Safety
/// `hwnd` is a valid top-level window passed by `EnumWindows`.
unsafe fn evaluate_window(
    hwnd: HWND,
    origin: (i32, i32),
    img_w: i64,
    img_h: i64,
) -> Option<SnapWindow> {
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return None;
    }

    let mut winfo = WINDOWINFO {
        cbSize: core::mem::size_of::<WINDOWINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetWindowInfo(hwnd, &mut winfo) }.is_err() {
        return None;
    }
    let style = winfo.dwStyle.0;
    let exstyle = winfo.dwExStyle.0;

    // Excludes minimized windows, and caption-less layered windows (transparent overlays, etc.).
    if style & WS_MINIMIZE.0 != 0 {
        return None;
    }
    if style & WS_CAPTION.0 == 0 && exstyle & WS_EX_LAYERED.0 != 0 {
        return None;
    }

    // Excludes DWM-cloaked windows (on another virtual desktop, a hidden UWP app, etc.).
    let mut cloaked: u32 = 0;
    if unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            core::mem::size_of::<u32>() as u32,
        )
    }
    .is_ok()
        && cloaked != 0
    {
        return None;
    }

    let abs = unsafe { true_bounds(hwnd, &winfo) };
    if (abs.2 - abs.0) < MIN_WINDOW_SIZE || (abs.3 - abs.1) < MIN_WINDOW_SIZE {
        return None;
    }

    if is_blacklisted_class(&unsafe { class_name(hwnd) }) {
        return None;
    }
    if unsafe { title_is_empty(hwnd) } {
        return None;
    }

    let rect_src = abs_to_src_clipped(abs, origin, img_w, img_h)?;
    Some(SnapWindow {
        hwnd: hwnd.0 as isize,
        rect_abs: abs,
        rect_src,
    })
}

/// Collects the absolute rects of every candidate-eligible descendant
/// window of `parent` (at any depth); see `is_adoptable_descendant`.
///
/// # Safety
/// `parent` must be a valid window handle.
unsafe fn collect_descendants(parent: HWND) -> Vec<RectI> {
    let mut out = Vec::new();
    // SAFETY: EnumChildWindows just calls the callback synchronously for
    // each descendant window. `lparam` carries a raw pointer to `out`,
    // dereferenced exclusively inside the callback (no race, since
    // enumeration runs synchronously on one thread).
    unsafe {
        let _ = EnumChildWindows(
            parent,
            Some(descendant_enum_proc),
            LPARAM(&mut out as *mut Vec<RectI> as isize),
        );
    }
    out
}

/// The `EnumChildWindows` callback. `lparam` is a raw pointer to `Vec<RectI>`.
///
/// # Safety
/// Only called by `EnumChildWindows`, and `lparam` points to a valid `Vec<RectI>`.
unsafe extern "system" fn descendant_enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is a valid pointer to the Vec passed by collect_descendants.
    let out = unsafe { &mut *(lparam.0 as *mut Vec<RectI>) };
    if unsafe { IsWindowVisible(hwnd) }.as_bool() {
        let exstyle = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
        let mut r = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut r) }.is_ok() {
            let rect = (r.left as i64, r.top as i64, r.right as i64, r.bottom as i64);
            if is_adoptable_descendant(rect, exstyle) {
                out.push(rect);
            }
        }
    }
    TRUE // continue enumeration (also traverses grandchildren and below)
}

/// The true bounding rect (absolute coordinates). Prefers DWM's extended
/// frame bounds (excludes the invisible drop-shadow border), falling back
/// to the plain window rect if unavailable.
///
/// # Safety
/// `hwnd` must be a valid window handle.
unsafe fn true_bounds(hwnd: HWND, winfo: &WINDOWINFO) -> RectI {
    let mut r = RECT::default();
    if unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut c_void,
            core::mem::size_of::<RECT>() as u32,
        )
    }
    .is_ok()
    {
        return (r.left as i64, r.top as i64, r.right as i64, r.bottom as i64);
    }
    let w = winfo.rcWindow;
    (w.left as i64, w.top as i64, w.right as i64, w.bottom as i64)
}

/// Gets a window's class name.
///
/// # Safety
/// `hwnd` must be a valid window handle.
unsafe fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Whether the title is empty (used to reject caption-less helper windows, etc.).
///
/// # Safety
/// `hwnd` must be a valid window handle.
unsafe fn title_is_empty(hwnd: HWND) -> bool {
    let mut buf = [0u16; 8];
    unsafe { GetWindowTextW(hwnd, &mut buf) <= 0 }
}

/// Whether this is a known non-window (desktop/shell overlay) class; starts minimal.
fn is_blacklisted_class(cls: &str) -> bool {
    matches!(
        cls,
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "SysListView32"
    )
}

/// Whether `cand` is fully contained by any one of `claimed` (the windows already in front).
fn fully_covered(cand: Rect, claimed: &[SnapWindow]) -> bool {
    claimed.iter().any(|w| {
        let r = w.rect_src;
        r.x0 <= cand.x0 && r.y0 <= cand.y0 && r.x1 >= cand.x1 && r.y1 >= cand.y1
    })
}

/// Whether a descendant window can be a snap candidate. Excludes tool
/// windows (floating toolbars, etc.) and anything too small.
///
/// `WS_EX_TRANSPARENT` is **not** excluded: a browser's content-area
/// window (a child HWND kept around for accessibility compatibility, e.g.
/// with a `*_RenderWidgetHostHWND`-style class name) is built with exactly
/// that transparent style, and rejecting it would break snapping to
/// browser content entirely. The intermediate GPU-compositing surface
/// (near full-window size) is also transparent, but it naturally loses to
/// the content area under `smallest_containing`'s smallest-area
/// preference, so it doesn't need separate exclusion.
fn is_adoptable_descendant(rect: RectI, exstyle: u32) -> bool {
    let toolwindow = exstyle & WS_EX_TOOLWINDOW.0 != 0;
    let too_small =
        (rect.2 - rect.0) < MIN_WINDOW_CHILD_SIZE || (rect.3 - rect.1) < MIN_WINDOW_CHILD_SIZE;
    !toolwindow && !too_small
}

/// Returns the smallest-area rect among `candidates` containing `(px,py)`
/// (`None` if none contain it). Ties keep whichever was found first.
fn smallest_containing(candidates: &[RectI], px: i64, py: i64) -> Option<RectI> {
    candidates
        .iter()
        .filter(|r| px >= r.0 && px < r.2 && py >= r.1 && py < r.3)
        .min_by_key(|r| (r.2 - r.0) * (r.3 - r.1))
        .copied()
}

/// The intersection of absolute rects `a` and `b`.
fn intersect(a: RectI, b: RectI) -> RectI {
    (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3))
}

/// Converts an absolute rect to src coordinates and clips it to the image
/// bounds `[0,img_w) x [0,img_h)`. `None` if the clipped result is empty.
fn abs_to_src_clipped(abs: RectI, origin: (i32, i32), img_w: i64, img_h: i64) -> Option<Rect> {
    let x0 = (abs.0 - origin.0 as i64).clamp(0, img_w);
    let y0 = (abs.1 - origin.1 as i64).clamp(0, img_h);
    let x1 = (abs.2 - origin.0 as i64).clamp(0, img_w);
    let y1 = (abs.3 - origin.1 as i64).clamp(0, img_h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(Rect {
        x0: x0 as usize,
        y0: y0 as usize,
        x1: x1 as usize,
        y1: y1 as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::WS_EX_TRANSPARENT;

    fn win(rect_src: Rect) -> SnapWindow {
        SnapWindow {
            hwnd: 0,
            rect_abs: (0, 0, 0, 0),
            rect_src,
        }
    }
    fn r(x0: usize, y0: usize, x1: usize, y1: usize) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    #[test]
    fn fully_covered_detects_containment() {
        let claimed = vec![win(r(0, 0, 100, 100))];
        // Fully contained -> true.
        assert!(fully_covered(r(10, 10, 50, 50), &claimed));
        // Identical rect -> true (boundary inclusive).
        assert!(fully_covered(r(0, 0, 100, 100), &claimed));
        // Partially spills out -> false.
        assert!(!fully_covered(r(50, 50, 150, 150), &claimed));
        // No covering candidate -> false.
        assert!(!fully_covered(r(10, 10, 20, 20), &[]));
    }

    #[test]
    fn window_at_index_prefers_front() {
        let snap = Snapshot {
            // The front window (index 0) is preferred over the larger one behind it.
            windows: vec![win(r(10, 10, 60, 60)), win(r(0, 0, 200, 200))],
            origin: (0, 0),
            img_w: 200,
            img_h: 200,
        };
        assert_eq!(snap.window_at_index(30, 30), Some(0));
        // Outside the front window, inside the back one -> the back one.
        assert_eq!(snap.window_at_index(100, 100), Some(1));
        // Outside both -> None.
        assert_eq!(snap.window_at_index(300, 300), None);
    }

    #[test]
    fn is_adoptable_descendant_accepts_normal_window() {
        assert!(is_adoptable_descendant((0, 0, 900, 900), 0));
    }

    #[test]
    fn is_adoptable_descendant_rejects_too_small() {
        assert!(!is_adoptable_descendant((0, 0, 100, 100), 0)); // Under 200px.
    }

    #[test]
    fn is_adoptable_descendant_rejects_toolwindow() {
        assert!(!is_adoptable_descendant(
            (0, 0, 900, 900),
            WS_EX_TOOLWINDOW.0
        ));
    }

    #[test]
    fn is_adoptable_descendant_accepts_transparent() {
        // A browser's content-area window is built with a transparent
        // style, so WS_EX_TRANSPARENT alone must not be grounds for exclusion.
        assert!(is_adoptable_descendant(
            (0, 0, 900, 900),
            WS_EX_TRANSPARENT.0
        ));
    }

    #[test]
    fn smallest_containing_prefers_smaller_over_larger() {
        let candidates = [
            (0, 0, 1000, 1000),   // Larger (e.g. a compositing surface near top-level size)
            (100, 100, 900, 800), // Smaller (e.g. the actual content area)
        ];
        assert_eq!(
            smallest_containing(&candidates, 500, 500),
            Some((100, 100, 900, 800))
        );
    }

    #[test]
    fn smallest_containing_ignores_non_containing_candidates() {
        let candidates = [(0, 0, 100, 100)];
        assert_eq!(smallest_containing(&candidates, 500, 500), None);
    }

    #[test]
    fn abs_to_src_clips_to_image_bounds() {
        // origin (-100,-50), image 200x200.
        let origin = (-100, -50);
        // A rect partially outside the image -> clipped to image bounds.
        let out = abs_to_src_clipped((-50, 0, 300, 300), origin, 200, 200).unwrap();
        assert_eq!(out, r(50, 50, 200, 200));
        // Entirely outside the image -> None.
        assert_eq!(
            abs_to_src_clipped((500, 500, 600, 600), origin, 200, 200),
            None
        );
    }
}
