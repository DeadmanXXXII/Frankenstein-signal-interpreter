use super::{RawSignal, SignalSource, SourceError};
use std::fs;

/// Loads a text file of hex-encoded frames, one per line. Understands plain
/// hex ("8D4840D6202CC371C32CE0576098") and dump1090-style raw framing
/// ("*8D4840D6202CC371C32CE0576098;"). Blank lines and lines starting with
/// `#` are ignored.
pub struct PacketStreamSource;

impl SignalSource for PacketStreamSource {
    fn name(&self) -> &'static str {
        "Packet / data stream (hex frames)"
    }

    fn load(&self, path: &str) -> Result<RawSignal, SourceError> {
        let text = fs::read_to_string(path)?;
        let mut frames = Vec::new();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let cleaned = line.trim_start_matches('*').trim_end_matches(';');
            let hex: String = cleaned.chars().filter(|c| c.is_ascii_hexdigit()).collect();
            if hex.len() < 2 {
                continue;
            }
            let hex = if hex.len() % 2 == 1 {
                hex[..hex.len() - 1].to_string()
            } else {
                hex
            };

            let mut bytes = Vec::with_capacity(hex.len() / 2);
            let mut ok = true;
            for i in (0..hex.len()).step_by(2) {
                match u8::from_str_radix(&hex[i..i + 2], 16) {
                    Ok(b) => bytes.push(b),
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && !bytes.is_empty() {
                frames.push(bytes);
            }
        }

        if frames.is_empty() {
            return Err(SourceError::Format(
                "no valid hex frames found (expected one hex-encoded frame per line)".into(),
            ));
        }

        Ok(RawSignal::Packets(frames))
    }
}
