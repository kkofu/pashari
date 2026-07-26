//! Enforces a single instance. Detects a duplicate launch via a named
//! Mutex, and signals the existing instance to "show Settings" via a named Event.

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, FALSE, GetLastError, TRUE};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, INFINITE, SetEvent, WaitForSingleObject,
};
use windows::core::w;

/// Attempts to launch. If another process is already running, sends it a
/// signal to show Settings and returns `true` (the caller should exit
/// immediately). Returns `false` if this is the first instance (fine to keep launching).
///
/// The Mutex handle is deliberately never closed (kept alive until the
/// process exits; Windows automatically closes open handles when a process ends).
pub fn acquire_or_notify() -> bool {
    // SAFETY: a direct Win32 API call that just creates/opens a fixed-name Mutex, touching no other state.
    let already_running = unsafe {
        match CreateMutexW(None, TRUE, w!("pashari-9c2f1b6e-single-instance")) {
            Ok(_handle) => GetLastError() == ERROR_ALREADY_EXISTS,
            // If creating the Mutex itself fails, gives up on the
            // single-instance check and continues launching.
            Err(_) => false,
        }
    };
    if already_running {
        notify_running_instance();
    }
    already_running
}

/// Signals the existing instance to show Settings.
fn notify_running_instance() {
    // SAFETY: a direct Win32 API call that just creates/opens a fixed-name Event and calls SetEvent.
    unsafe {
        if let Ok(event) = CreateEventW(None, FALSE, FALSE, w!("pashari-9c2f1b6e-show-settings")) {
            let _ = SetEvent(event);
        }
    }
}

/// Spawns a background thread that waits for a launch request from
/// another process, while this is the sole instance. Calls `on_signal` every time a signal arrives.
pub fn watch(on_signal: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        // SAFETY: a direct Win32 API call; just an event-wait loop, touching no other state.
        unsafe {
            let Ok(event) = CreateEventW(None, FALSE, FALSE, w!("pashari-9c2f1b6e-show-settings"))
            else {
                return;
            };
            loop {
                WaitForSingleObject(event, INFINITE);
                on_signal();
            }
        }
    });
}
