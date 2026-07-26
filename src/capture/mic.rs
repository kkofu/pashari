//! Microphone recording.
//!
//! Captures from `cpal`'s default input device, converts to interleaved
//! stereo i16 (mono is duplicated), and sends it over mpsc. The PCM is
//! either mixed with desktop audio or passed to the encoder alone.
//! `cpal::Stream` is `!Send`, so it's held by the Recorder (main thread).

use std::sync::mpsc::{self, Receiver};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};

/// A running mic capture; dropping it stops the stream.
pub struct MicCapture {
    _stream: Stream,
}

/// Start result: sample rate, stereo i16 receiver, running handle.
type Started = (u32, Receiver<Vec<u8>>, MicCapture);

/// Input device names, for the settings GUI's dropdown.
pub fn input_device_names() -> Vec<String> {
    let Ok(devices) = cpal::default_host().input_devices() else {
        return Vec::new();
    };
    devices.filter_map(|d| d.name().ok()).collect()
}

/// Starts mic capture. If `device_name` is non-empty, looks for a device
/// with that name; falls back to the system default if not found (e.g. it
/// was unplugged).
pub fn start(device_name: &str) -> Result<Started, Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let by_name = if device_name.is_empty() {
        None
    } else {
        host.input_devices()
            .ok()
            .and_then(|mut devices| devices.find(|d| d.name().ok().as_deref() == Some(device_name)))
    };
    let device = by_name
        .or_else(|| host.default_input_device())
        .ok_or("マイク（入力デバイス）が見つかりません")?;
    let supported = device.default_input_config()?;
    let sample_format = supported.sample_format();
    let rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let config: cpal::StreamConfig = supported.into();

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let err_fn = |e| eprintln!("マイク入力エラー: {e}");

    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let out = to_stereo_i16(data, channels, |s: f32| {
                    (s.clamp(-1.0, 1.0) * 32767.0) as i16
                });
                let _ = tx.send(out);
            },
            err_fn,
            None,
        )?,
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                let out = to_stereo_i16(data, channels, |s: i16| s);
                let _ = tx.send(out);
            },
            err_fn,
            None,
        )?,
        SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _: &cpal::InputCallbackInfo| {
                let out = to_stereo_i16(data, channels, |s: u16| (s as i32 - 32768) as i16);
                let _ = tx.send(out);
            },
            err_fn,
            None,
        )?,
        other => return Err(format!("未対応のマイクサンプル形式: {other:?}").into()),
    };
    stream.play()?;
    Ok((rate, rx, MicCapture { _stream: stream }))
}

/// Converts interleaved input (`channels` channels) to stereo i16 LE bytes.
fn to_stereo_i16<T: Copy>(data: &[T], channels: usize, conv: impl Fn(T) -> i16) -> Vec<u8> {
    if channels == 0 {
        return Vec::new();
    }
    let frames = data.len() / channels;
    let mut out = Vec::with_capacity(frames * 4);
    for f in 0..frames {
        let base = f * channels;
        let l = conv(data[base]);
        let r = if channels >= 2 {
            conv(data[base + 1])
        } else {
            l
        };
        out.extend_from_slice(&l.to_le_bytes());
        out.extend_from_slice(&r.to_le_bytes());
    }
    out
}
