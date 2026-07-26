//! Audio-mixing DSP primitives: resampling, additive mixing, PCM conversion.
//!
//! The actual orchestration — combining sources and pacing output to the
//! wall clock, so desktop audio/mic can be attached or detached mid-recording
//! — lives in [`super::audio_session`]. This module only has pure conversion
//! functions usable by either source.

use std::collections::VecDeque;

use rubato::{
    Resampler as _, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

/// Input buffer cap in stereo i16 samples; drops the oldest once past ~1s @48kHz.
pub(crate) const BUF_CAP: usize = 48_000 * 2;
/// Fixed input frame count per call into rubato.
const RESAMPLE_CHUNK: usize = 1024;
/// Soft-limiter threshold: passed through unchanged below it, eased toward
/// 32767 above it.
const LIMIT_THRESH: f32 = 24_000.0;

/// Resamples a source to a target rate; bypassed if rates already match.
pub(crate) enum Resampler {
    Passthrough,
    Resample {
        inner: SincFixedIn<f32>,
        inl: Vec<f32>,
        inr: Vec<f32>,
    },
}

impl Resampler {
    pub(crate) fn new(in_rate: u32, out_rate: u32) -> Result<Self, Box<dyn std::error::Error>> {
        if in_rate == out_rate {
            return Ok(Resampler::Passthrough);
        }
        let params = SincInterpolationParameters {
            sinc_len: 128,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 128,
            window: WindowFunction::BlackmanHarris2,
        };
        let inner = SincFixedIn::<f32>::new(
            out_rate as f64 / in_rate as f64,
            2.0,
            params,
            RESAMPLE_CHUNK,
            2,
        )?;
        Ok(Resampler::Resample {
            inner,
            inl: Vec::new(),
            inr: Vec::new(),
        })
    }

    /// Feeds in interleaved stereo i16 and appends resampled stereo i16 to `out`.
    pub(crate) fn process(&mut self, input: &[i16], out: &mut Vec<i16>) {
        match self {
            Resampler::Passthrough => out.extend_from_slice(input),
            Resampler::Resample { inner, inl, inr } => {
                for f in input.chunks_exact(2) {
                    inl.push(f[0] as f32 / 32768.0);
                    inr.push(f[1] as f32 / 32768.0);
                }
                while inl.len() >= RESAMPLE_CHUNK {
                    let l: Vec<f32> = inl.drain(0..RESAMPLE_CHUNK).collect();
                    let r: Vec<f32> = inr.drain(0..RESAMPLE_CHUNK).collect();
                    if let Ok(o) = inner.process(&[l, r], None) {
                        let (ol, or) = (&o[0], &o[1]);
                        for i in 0..ol.len().min(or.len()) {
                            out.push((ol[i].clamp(-1.0, 1.0) * 32767.0) as i16);
                            out.push((or[i].clamp(-1.0, 1.0) * 32767.0) as i16);
                        }
                    }
                }
            }
        }
    }
}

/// Smoothly caps summed amplitude (less distortion than a hard clip).
pub(crate) fn soft_clip(x: f32) -> i16 {
    let a = x.abs();
    let y = if a <= LIMIT_THRESH {
        a
    } else {
        let range = 32_767.0 - LIMIT_THRESH;
        LIMIT_THRESH + range * (1.0 - (-(a - LIMIT_THRESH) / range).exp())
    };
    (x.signum() * y) as i16
}

/// Converts stereo i16 LE bytes to `Vec<i16>`.
pub(crate) fn bytes_to_i16(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|p| i16::from_le_bytes([p[0], p[1]]))
        .collect()
}

/// Trims `q` to at most `BUF_CAP` (absorbs drift).
pub(crate) fn cap(q: &mut VecDeque<i16>) {
    while q.len() > BUF_CAP {
        q.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_passthrough_when_rates_equal() {
        let mut r = Resampler::new(48_000, 48_000).unwrap();
        let input = vec![1i16, 2, 3, 4];
        let mut out = Vec::new();
        r.process(&input, &mut out);
        assert_eq!(out, input);
    }

    #[test]
    fn resampler_downsamples_roughly_half() {
        let mut r = Resampler::new(48_000, 24_000).unwrap();
        // 8192 frames (stereo).
        let input: Vec<i16> = (0..8192 * 2).map(|i| (i % 100) as i16).collect();
        let mut out = Vec::new();
        r.process(&input, &mut out);
        let frames = out.len() / 2;
        assert!((3000..5000).contains(&frames), "frames={frames}");
    }

    #[test]
    fn soft_clip_passes_small_and_limits_large() {
        assert_eq!(soft_clip(1000.0), 1000);
        let hi = soft_clip(60_000.0);
        assert!(hi > 0 && hi < 32_767);
        assert!(soft_clip(-60_000.0) > -32_767);
    }
}
