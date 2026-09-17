use super::{DecodeOutput, DecodedItem, Decoder};
use crate::sources::RawSignal;
use image::{GrayImage, Luma};

/// Decodes NOAA APT (Automatic Picture Transmission) weather satellite
/// imagery from demodulated FM audio — the standard hobbyist SDR/scanner
/// recording of a NOAA POES pass on ~137 MHz. NOAA transmits APT in the
/// clear specifically so it can be received by the public; this is a
/// well-established, legal amateur radio activity.
///
/// Feed this decoder a WAV recording of the audio output of an FM receiver
/// tuned to a NOAA APT downlink. It will not produce useful output on
/// arbitrary audio.
pub struct AptDecoder;

const APT_WORKING_RATE: u32 = 20800;
const APT_DECIMATION: usize = 5;
const APT_LINE_WIDTH: usize = 2080; // pixels per line, per the APT spec

impl Decoder for AptDecoder {
    fn name(&self) -> &'static str {
        "NOAA APT weather satellite image"
    }

    fn decode(&self, raw: &RawSignal) -> DecodeOutput {
        let (samples, sample_rate) = match raw {
            RawSignal::Audio {
                samples,
                sample_rate,
            } => (samples, *sample_rate),
            _ => {
                return DecodeOutput {
                    items: vec![DecodedItem {
                        summary: "APT decoder needs an audio source".into(),
                        detail: "This only works on FM-demodulated audio (WAV) captured from a NOAA APT pass.".into(),
                    }],
                    image_path: None,
                };
            }
        };

        let resampled = resample_linear(samples, sample_rate, APT_WORKING_RATE);
        let envelope = envelope_detect(&resampled);
        let pixels: Vec<f32> = envelope.iter().step_by(APT_DECIMATION).copied().collect();

        if pixels.len() < APT_LINE_WIDTH {
            return DecodeOutput {
                items: vec![DecodedItem {
                    summary: "Recording too short to decode a full APT line".into(),
                    detail: format!(
                        "Got {} pixel samples after decimation, need at least {}.",
                        pixels.len(),
                        APT_LINE_WIDTH
                    ),
                }],
                image_path: None,
            };
        }

        let min = pixels.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = pixels.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let range = (max - min).max(1e-6);

        let n_lines = pixels.len() / APT_LINE_WIDTH;
        let mut img = GrayImage::new(APT_LINE_WIDTH as u32, n_lines as u32);

        for y in 0..n_lines {
            for x in 0..APT_LINE_WIDTH {
                let v = pixels[y * APT_LINE_WIDTH + x];
                let norm = (((v - min) / range) * 255.0).clamp(0.0, 255.0) as u8;
                img.put_pixel(x as u32, y as u32, Luma([norm]));
            }
        }

        let out_path = "apt_decoded.png";
        let save_result = img.save(out_path);

        let mut items = vec![DecodedItem {
            summary: format!("Decoded {n_lines} image lines from APT audio"),
            detail: format!(
                "Input: {sample_rate} Hz, {} samples. Resampled to {APT_WORKING_RATE} Hz, envelope-detected, \
                 decimated {APT_DECIMATION}x to {APT_LINE_WIDTH} px/line.",
                samples.len()
            ),
        }];

        match &save_result {
            Ok(_) => {}
            Err(e) => items.push(DecodedItem {
                summary: "Failed to save decoded image".into(),
                detail: e.to_string(),
            }),
        }

        DecodeOutput {
            items,
            image_path: if save_result.is_ok() {
                Some(out_path.to_string())
            } else {
                None
            },
        }
    }
}

/// Simple linear-interpolation resampler — good enough for envelope-based
/// APT decoding. Swap in a proper polyphase resampler (e.g. the `rubato`
/// crate) if you need better fidelity.
fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let out_len = ((samples.len() as f64) * ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 / ratio;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;
        if idx + 1 < samples.len() {
            out.push(samples[idx] * (1.0 - frac) + samples[idx + 1] * frac);
        } else if idx < samples.len() {
            out.push(samples[idx]);
        }
    }
    out
}

/// Rectify + single-pole low-pass filter: a cheap approximation of AM
/// envelope detection, sufficient to pull the APT video signal off its
/// 2400 Hz subcarrier without a full Hilbert transform.
fn envelope_detect(samples: &[f32]) -> Vec<f32> {
    let alpha = 0.05_f32;
    let mut out = Vec::with_capacity(samples.len());
    let mut acc = 0.0_f32;
    for &s in samples {
        let rectified = s.abs();
        acc += alpha * (rectified - acc);
        out.push(acc);
    }
    out
}
