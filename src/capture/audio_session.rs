//! An audio session that lives for the whole recording, with desktop/mic
//! sources attachable and detachable on the fly.
//!
//! `windows-capture`'s `VideoEncoder` fixes the audio track layout once at
//! creation, so toggling audio during recording works by swapping the
//! track's content (real audio vs. silence) rather than the track itself.
//! The receiver returned here keeps producing PCM without gaps, paced to
//! the wall clock, regardless of which sources are enabled — so
//! `super::RecordHandler`/`super::gdi::run_mp4` can keep draining it exactly
//! as before, unmodified.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::audio::{AudioFormat, LoopbackCapture};
use super::mic::{self, MicCapture};
use super::mixer::{self, Resampler};

const TARGET_CHANNELS: u32 = 2;
/// Combiner polling interval (same as the existing mixer).
const TICK: Duration = Duration::from_millis(4);
/// A per-mille peak (same unit as `desktop_level`/`mic_level`) at or below
/// this is treated as "effectively silent" — an internal threshold that
/// tolerates WASAPI's noise floor/dither; not user-configurable.
const SILENCE_THRESHOLD_MILLI: u32 = 5;

/// A control message sent to the combiner thread to attach/detach a source
/// (`rate` is the source's negotiated sample rate).
enum CtrlMsg {
    SetDesktop(Option<(Receiver<Vec<u8>>, u32)>),
    SetMic(Option<(Receiver<Vec<u8>>, u32)>),
}

/// A running audio session, held on the main thread (it actually owns the
/// `LoopbackCapture`/`MicCapture`, so Drop reliably stops them —
/// `MicCapture`'s inner `cpal::Stream` is `!Send`, so it must live on the
/// main thread, per the note in `super::mic`).
pub(crate) struct AudioSession {
    ctrl_tx: Sender<CtrlMsg>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    desktop: Option<LoopbackCapture>,
    mic: Option<MicCapture>,
    /// Latest desktop/mic peak levels (per-mille, 0..=1000), for the control
    /// bar's volume indicator (updated by the combiner thread).
    desktop_level: Arc<AtomicU32>,
    mic_level: Arc<AtomicU32>,
    /// The largest peak (per-mille, 0..=1000) seen across desktop+mic since
    /// the session started — used to decide whether to strip the audio
    /// track afterward. Stays 0 if a source was never enabled, and also
    /// never exceeds the threshold if it was enabled but effectively
    /// silent, so both cases are handled by the same check.
    peak_ever: Arc<AtomicU32>,
}

impl AudioSession {
    /// Starts the combiner thread and returns the fixed format plus an
    /// output receiver. `desktop_device`/`mic_device` are the device names
    /// to use (empty = system default).
    pub(crate) fn start(
        initial_desktop: bool,
        initial_mic: bool,
        desktop_device: &str,
        mic_device: &str,
        target_rate: u32,
    ) -> (Self, AudioFormat, Receiver<Vec<u8>>) {
        let (ctrl_tx, ctrl_rx) = mpsc::channel();
        let (out_tx, out_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_th = stop.clone();
        let desktop_level = Arc::new(AtomicU32::new(0));
        let mic_level = Arc::new(AtomicU32::new(0));
        let peak_ever = Arc::new(AtomicU32::new(0));
        let (desktop_level_th, mic_level_th, peak_ever_th) =
            (desktop_level.clone(), mic_level.clone(), peak_ever.clone());

        let handle = thread::spawn(move || {
            run(
                ctrl_rx,
                &out_tx,
                &stop_th,
                &desktop_level_th,
                &mic_level_th,
                &peak_ever_th,
                target_rate,
            )
        });

        let mut session = Self {
            ctrl_tx,
            stop,
            handle: Some(handle),
            desktop: None,
            mic: None,
            desktop_level,
            mic_level,
            peak_ever,
        };
        if initial_desktop {
            session.set_desktop(true, desktop_device);
        }
        if initial_mic {
            session.set_mic(true, mic_device);
        }

        let format = AudioFormat {
            sample_rate: target_rate,
            channels: TARGET_CHANNELS,
        };
        (session, format, out_rx)
    }

    /// Turns desktop audio on/off. On actually starts loopback; off
    /// actually drops it (so the recording indicator follows suit).
    /// `device` is the device name to use (empty = system default; ignored
    /// when turning off).
    pub(crate) fn set_desktop(&mut self, on: bool, device: &str) {
        if on {
            if self.desktop.is_some() {
                return;
            }
            match LoopbackCapture::start(device) {
                Ok((fmt, rx, cap)) => {
                    self.desktop = Some(cap);
                    let _ = self
                        .ctrl_tx
                        .send(CtrlMsg::SetDesktop(Some((rx, fmt.sample_rate))));
                }
                Err(e) => eprintln!("デスクトップ音声の取得に失敗（無効化）: {e}"),
            }
        } else {
            self.desktop = None; // Drop actually stops it.
            let _ = self.ctrl_tx.send(CtrlMsg::SetDesktop(None));
        }
    }

    /// Turns the mic on/off. On actually starts it; off actually drops it.
    /// `device` is the device name to use (empty = system default; ignored
    /// when turning off).
    pub(crate) fn set_mic(&mut self, on: bool, device: &str) {
        if on {
            if self.mic.is_some() {
                return;
            }
            match mic::start(device) {
                Ok((rate, rx, cap)) => {
                    self.mic = Some(cap);
                    let _ = self.ctrl_tx.send(CtrlMsg::SetMic(Some((rx, rate))));
                }
                Err(e) => eprintln!("マイクの取得に失敗（無効化）: {e}"),
            }
        } else {
            self.mic = None;
            let _ = self.ctrl_tx.send(CtrlMsg::SetMic(None));
        }
    }

    /// Whether both desktop and mic audio have stayed effectively silent
    /// since the session started (true whether a source was never enabled,
    /// or was enabled but stayed silent).
    pub(crate) fn was_silent(&self) -> bool {
        self.peak_ever.load(Ordering::Relaxed) <= SILENCE_THRESHOLD_MILLI
    }

    /// Latest desktop/mic volume levels (0.0..=1.0), for the control bar's
    /// indicator.
    pub(crate) fn levels(&self) -> (f32, f32) {
        (
            self.desktop_level.load(Ordering::Relaxed) as f32 / 1000.0,
            self.mic_level.load(Ordering::Relaxed) as f32 / 1000.0,
        )
    }

    /// Stops the captures first (cutting off new data), then signals the
    /// combiner to stop and joins it.
    pub(crate) fn stop(mut self) {
        self.desktop = None;
        self.mic = None;
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Per-source slot state.
struct Slot {
    rx: Receiver<Vec<u8>>,
    resampler: Resampler,
    queue: VecDeque<i16>,
}

fn run(
    ctrl_rx: Receiver<CtrlMsg>,
    out_tx: &Sender<Vec<u8>>,
    stop: &AtomicBool,
    desktop_level: &AtomicU32,
    mic_level: &AtomicU32,
    peak_ever: &AtomicU32,
    target_rate: u32,
) {
    let mut desktop: Option<Slot> = None;
    let mut mic: Option<Slot> = None;
    let start = Instant::now();
    let mut emitted: u64 = 0;
    let mut tmp: Vec<i16> = Vec::new();

    loop {
        while let Ok(msg) = ctrl_rx.try_recv() {
            match msg {
                CtrlMsg::SetDesktop(Some((rx, rate))) => desktop = make_slot(rx, rate, target_rate),
                CtrlMsg::SetDesktop(None) => desktop = None,
                CtrlMsg::SetMic(Some((rx, rate))) => mic = make_slot(rx, rate, target_rate),
                CtrlMsg::SetMic(None) => mic = None,
            }
        }

        pull(&mut desktop, &mut tmp, desktop_level, peak_ever);
        pull(&mut mic, &mut tmp, mic_level, peak_ever);

        // Emit only what's needed to catch up to the wall clock. Silence
        // keeps filling even while a source is absent/disconnected, so
        // turning it on later doesn't shift the timeline.
        let target = (start.elapsed().as_secs_f64() * target_rate as f64) as u64;
        if target > emitted {
            let n = (target - emitted) as usize;
            let mut out = Vec::with_capacity(n * 4);
            for _ in 0..n {
                let (dl, dr) = pop_frame(desktop.as_mut());
                let (ml, mr) = pop_frame(mic.as_mut());
                let l = mixer::soft_clip(dl as f32 + ml as f32);
                let r = mixer::soft_clip(dr as f32 + mr as f32);
                out.extend_from_slice(&l.to_le_bytes());
                out.extend_from_slice(&r.to_le_bytes());
            }
            emitted = target;
            if out_tx.send(out).is_err() {
                return;
            }
        }

        if stop.load(Ordering::Relaxed) {
            return;
        }
        thread::sleep(TICK);
    }
}

fn make_slot(rx: Receiver<Vec<u8>>, rate: u32, target_rate: u32) -> Option<Slot> {
    match Resampler::new(rate, target_rate) {
        Ok(resampler) => Some(Slot {
            rx,
            resampler,
            queue: VecDeque::new(),
        }),
        Err(e) => {
            eprintln!("リサンプラの初期化に失敗（このソースは無音扱い）: {e}");
            None
        }
    }
}

/// Drains everything available from the slot, resamples to the target
/// rate, and queues it. Also updates `level` (per-mille peak for the
/// volume indicator): immediately 0 if there's no source; otherwise
/// reflects that call's peak only when new data actually arrived (the
/// previous value is kept otherwise — good enough since PCM arriving less
/// often than TICK essentially never happens in practice).
fn pull(slot: &mut Option<Slot>, tmp: &mut Vec<i16>, level: &AtomicU32, peak_ever: &AtomicU32) {
    let Some(s) = slot else {
        level.store(0, Ordering::Relaxed);
        return;
    };
    let mut peak: u32 = 0;
    let mut got_any = false;
    while let Ok(bytes) = s.rx.try_recv() {
        tmp.clear();
        let samples = mixer::bytes_to_i16(&bytes);
        s.resampler.process(&samples, tmp);
        for &v in tmp.iter() {
            peak = peak.max(v.unsigned_abs() as u32);
        }
        s.queue.extend(tmp.iter().copied());
        got_any = true;
    }
    if got_any {
        let milli = (peak * 1000 / 32768).min(1000);
        level.store(milli, Ordering::Relaxed);
        peak_ever.fetch_max(milli, Ordering::Relaxed);
    }
    mixer::cap(&mut s.queue);
}

/// Pops one frame (L, R) from the slot; silence if there's none.
fn pop_frame(slot: Option<&mut Slot>) -> (i16, i16) {
    match slot {
        Some(s) => {
            let l = s.queue.pop_front().unwrap_or(0);
            let r = s.queue.pop_front().unwrap_or(0);
            (l, r)
        }
        None => (0, 0),
    }
}
