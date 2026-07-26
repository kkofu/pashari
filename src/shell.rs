//! Opens the output file in Explorer after saving, shown selected.
//!
//! `SHOpenFolderAndSelectItems` (the standard "open file location"
//! implementation) reuses an Explorer window already showing the target
//! folder if one exists (unlike `explorer.exe /select,`, it doesn't
//! unconditionally open a new window).

use std::path::Path;

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::Shell::{
    ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW, SW_SHOWNORMAL};
use windows::core::HSTRING;

/// Opens `path`'s file in Explorer, shown selected. Saving itself is
/// already done, so failure here is just logged and otherwise ignored.
pub fn reveal_and_select(path: &Path) {
    // SAFETY: pidl is used only within this function, right after
    // ILCreateFromPathW returns it, and is always released with ILFree at the end.
    unsafe {
        // Ignores the result, since it's fine if it's already initialized in a different mode.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let wide = HSTRING::from(path);
        let pidl = ILCreateFromPathW(&wide);
        if pidl.is_null() {
            eprintln!(
                "フォルダを開けませんでした: パスの解決に失敗 ({})",
                path.display()
            );
            return;
        }
        if let Err(e) = SHOpenFolderAndSelectItems(pidl, None, 0) {
            eprintln!("フォルダを開けませんでした: {e}");
        }
        ILFree(Some(pidl));
    }
}

/// Shows an error to the user via a modal dialog. `eprintln!` is
/// invisible when launched without a console (the tray app, or a
/// fire-and-forget editor process), so failures that need to reach the
/// user go through this instead.
pub fn show_error_dialog(message: &str) {
    // SAFETY: just passes a valid HSTRING; this is a modal call that
    // blocks until the user closes it. No other shared state involved.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(message),
            &HSTRING::from("pashari"),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// Opens `url` in the default browser (used to open the release page
/// from the update check). Failure is just logged and otherwise ignored.
pub fn open_url(url: &str) {
    // SAFETY: just passes a valid HSTRING; the return value (an
    // HINSTANCE-like result code) is only used to check for errors. No other shared state involved.
    unsafe {
        let result = ShellExecuteW(
            None,
            &HSTRING::from("open"),
            &HSTRING::from(url),
            None,
            None,
            SW_SHOWNORMAL,
        );
        // ShellExecuteW returns a value greater than 32 on success (an HINSTANCE-shaped value).
        if result.0 as isize <= 32 {
            eprintln!("URL を開けませんでした: {url}");
        }
    }
}
