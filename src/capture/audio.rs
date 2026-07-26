//! Captures desktop (system) audio via WASAPI loopback.
//!
//! `windows-capture` doesn't capture audio, so this polls the default
//! render endpoint's loopback directly, converts to interleaved i16 PCM,
//! and sends it over mpsc. The recording handler passes the PCM to the
//! encoder's `send_audio_buffer` to mux into the MP4.

use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
    DEVICE_STATE_ACTIVE, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, STGM_READ,
};

/// The negotiated audio format, used to configure the encoder.
#[derive(Clone, Copy)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u32,
}

/// `device`'s display (friendly) name, or `None` if it can't be read.
///
/// SAFETY: the calling thread must have COM already initialized.
unsafe fn device_friendly_name(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let pwstr = PropVariantToStringAlloc(&value).ok()?;
        let name = pwstr.to_string().ok();
        CoTaskMemFree(Some(pwstr.0 as *const c_void));
        name
    }
}

/// Output (render) device names, for the settings GUI's dropdown.
pub fn output_device_names() -> Vec<String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let names = (|| -> windows::core::Result<Vec<String>> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let collection = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            let count = collection.GetCount()?;
            let mut names = Vec::with_capacity(count as usize);
            for i in 0..count {
                let device = collection.Item(i)?;
                if let Some(name) = device_friendly_name(&device) {
                    names.push(name);
                }
            }
            Ok(names)
        })()
        .unwrap_or_default();
        CoUninitialize();
        names
    }
}

/// Finds the output device whose display name matches `name`, or `Ok(None)`.
///
/// SAFETY: the calling thread must have COM already initialized.
unsafe fn find_output_device_by_name(
    enumerator: &IMMDeviceEnumerator,
    name: &str,
) -> windows::core::Result<Option<IMMDevice>> {
    unsafe {
        let collection = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        let count = collection.GetCount()?;
        for i in 0..count {
            let device = collection.Item(i)?;
            if device_friendly_name(&device).as_deref() == Some(name) {
                return Ok(Some(device));
            }
        }
        Ok(None)
    }
}

/// A running loopback capture; `stop()` stops its thread.
pub struct LoopbackCapture {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

/// Start result: negotiated format, PCM receiver, running handle.
type Started = (AudioFormat, Receiver<Vec<u8>>, LoopbackCapture);

impl LoopbackCapture {
    /// Starts loopback capture, returning the format and a PCM receiver. If
    /// `device_name` is non-empty, looks for a device with that name; falls
    /// back to the system default if not found (e.g. it was unplugged).
    pub fn start(device_name: &str) -> Result<Started, Box<dyn std::error::Error>> {
        let (audio_tx, audio_rx) = mpsc::channel::<Vec<u8>>();
        let (init_tx, init_rx) = mpsc::channel::<Result<AudioFormat, String>>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let device_name = device_name.to_string();

        let handle = thread::spawn(move || {
            run_loopback(&audio_tx, &init_tx, &stop_thread, &device_name);
        });

        match init_rx.recv() {
            Ok(Ok(format)) => Ok((
                format,
                audio_rx,
                LoopbackCapture {
                    stop,
                    handle: Some(handle),
                },
            )),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e.into())
            }
            Err(_) => {
                let _ = handle.join();
                Err("音声取得スレッドの初期化に失敗".into())
            }
        }
    }
}

impl Drop for LoopbackCapture {
    /// Stops the capture and joins its thread.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// WASAPI loopback setup and the polling loop itself.
///
/// SAFETY: this body calls Win32/COM directly. COM is initialized, used,
/// and released only on this thread; COM pointers never leave it.
fn run_loopback(
    audio_tx: &Sender<Vec<u8>>,
    init_tx: &Sender<Result<AudioFormat, String>>,
    stop: &AtomicBool,
    device_name: &str,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let setup =
            || -> windows::core::Result<(IAudioClient, IAudioCaptureClient, AudioFormat, bool)> {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                // Use the named device if found; otherwise (unspecified, or
                // it was unplugged) fall back to the system default render endpoint.
                let by_name = if device_name.is_empty() {
                    None
                } else {
                    find_output_device_by_name(&enumerator, device_name).unwrap_or(None)
                };
                let device = match by_name {
                    Some(d) => d,
                    None => enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?,
                };
                let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

                let pwfx = client.GetMixFormat()?;
                let wfx = *pwfx;
                let format = AudioFormat {
                    sample_rate: wfx.nSamplesPerSec,
                    channels: wfx.nChannels as u32,
                };
                let is_float = wfx.wBitsPerSample == 32;

                // Initialize loopback with a 500ms buffer.
                let init = client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK,
                    5_000_000,
                    0,
                    pwfx,
                    None,
                );
                CoTaskMemFree(Some(pwfx as *const c_void));
                init?;

                let capture: IAudioCaptureClient = client.GetService()?;
                client.Start()?;
                Ok((client, capture, format, is_float))
            };

        let (client, capture, format, is_float) = match setup() {
            Ok(v) => v,
            Err(e) => {
                let _ = init_tx.send(Err(format!("WASAPI ループバック初期化に失敗: {e}")));
                CoUninitialize();
                return;
            }
        };

        let channels = format.channels as usize;
        let rate = format.sample_rate as f64;
        let _ = init_tx.send(Ok(format));

        // Loopback stops producing packets once the audio engine goes idle
        // (true silence), which would make the timeline drift shorter than
        // real time. To keep up, pace against the wall clock and fill any
        // large gap (silence with no incoming packets) with zeros. A 50ms
        // threshold avoids filling for normal playback jitter.
        let start = Instant::now();
        let mut emitted: u64 = 0;
        let jitter_frames = (rate * 0.05) as u64;

        'outer: while !stop.load(Ordering::Relaxed) {
            let target = (start.elapsed().as_secs_f64() * rate) as u64;
            if target > emitted + jitter_frames {
                let missing = (target - emitted) as usize;
                if audio_tx.send(vec![0u8; missing * channels * 2]).is_err() {
                    break 'outer;
                }
                emitted = target;
            }

            thread::sleep(Duration::from_millis(8));
            loop {
                let packet = match capture.GetNextPacketSize() {
                    Ok(p) => p,
                    Err(_) => break 'outer,
                };
                if packet == 0 {
                    break;
                }

                let mut pdata: *mut u8 = ptr::null_mut();
                let mut num_frames: u32 = 0;
                let mut flags: u32 = 0;
                if capture
                    .GetBuffer(&mut pdata, &mut num_frames, &mut flags, None, None)
                    .is_err()
                {
                    break 'outer;
                }
                let frames = num_frames as usize;

                let mut out = Vec::with_capacity(frames * channels * 2);
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                    // Zero-fill silent stretches to keep the timeline continuous.
                    out.resize(frames * channels * 2, 0);
                } else if is_float {
                    let samples =
                        std::slice::from_raw_parts(pdata as *const f32, frames * channels);
                    for &s in samples {
                        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                } else {
                    // 16-bit PCM passes through unchanged.
                    let bytes = std::slice::from_raw_parts(pdata, frames * channels * 2);
                    out.extend_from_slice(bytes);
                }

                let _ = capture.ReleaseBuffer(num_frames);
                emitted += frames as u64;
                if audio_tx.send(out).is_err() {
                    break 'outer;
                }
            }
        }

        let _ = client.Stop();
        CoUninitialize();
    }
}
