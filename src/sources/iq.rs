use super::{RawSignal, SignalSource, SourceError};
use std::fs;

/// Loads raw I/Q captures as produced by SDR tools (e.g. `rtl_sdr`, GNU Radio
/// file sinks). Two common raw formats are supported, chosen by file
/// extension:
///
/// - `.cf32` / `.cfile` / `.fc32`: interleaved 32-bit float I/Q (GNU Radio style)
/// - anything else: interleaved 8-bit unsigned I/Q (`rtl_sdr` `.cu8` style)
pub struct IqFileSource;

impl SignalSource for IqFileSource {
    fn name(&self) -> &'static str {
        "IQ / SDR raw file"
    }

    fn load(&self, path: &str) -> Result<RawSignal, SourceError> {
        let bytes = fs::read(path)?;
        let lower = path.to_lowercase();

        let samples: Vec<(f32, f32)> =
            if lower.ends_with(".cf32") || lower.ends_with(".cfile") || lower.ends_with(".fc32") {
                if bytes.len() % 8 != 0 {
                    return Err(SourceError::Format(
                        "cf32 file length is not a multiple of 8 bytes (float32 I/Q pairs)".into(),
                    ));
                }
                bytes
                    .chunks_exact(8)
                    .map(|c| {
                        let i = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                        let q = f32::from_le_bytes([c[4], c[5], c[6], c[7]]);
                        (i, q)
                    })
                    .collect()
            } else {
                if bytes.len() % 2 != 0 {
                    return Err(SourceError::Format(
                        "cu8 file length is not a multiple of 2 bytes (byte I/Q pairs)".into(),
                    ));
                }
                bytes
                    .chunks_exact(2)
                    .map(|c| {
                        let i = (c[0] as f32 - 127.5) / 127.5;
                        let q = (c[1] as f32 - 127.5) / 127.5;
                        (i, q)
                    })
                    .collect()
            };

        if samples.is_empty() {
            return Err(SourceError::Format("no IQ samples found in file".into()));
        }

        Ok(RawSignal::Iq(samples))
    }
}
