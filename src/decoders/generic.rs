use super::{DecodeOutput, DecodedItem, Decoder};
use crate::sources::RawSignal;

/// Fallback inspector used when no specific protocol decoder applies: reports
/// basic signal statistics so you can see the tool is receiving *something*
/// even before a dedicated decoder exists for it.
pub struct GenericDecoder;

impl Decoder for GenericDecoder {
    fn name(&self) -> &'static str {
        "Generic signal inspector"
    }

    fn decode(&self, raw: &RawSignal) -> DecodeOutput {
        let items = match raw {
            RawSignal::Iq(samples) => {
                let n = samples.len().max(1);
                let power: f32 = samples.iter().map(|(i, q)| i * i + q * q).sum::<f32>() / n as f32;
                let peak = samples
                    .iter()
                    .map(|(i, q)| (i * i + q * q).sqrt())
                    .fold(0.0_f32, f32::max);
                vec![DecodedItem {
                    summary: format!(
                        "{} IQ samples, avg power {power:.4}, peak magnitude {peak:.4}",
                        samples.len()
                    ),
                    detail: "No specific protocol decoder matched this IQ capture. This is a raw \
                              signal-strength summary — add a protocol-specific decoder (e.g. FM \
                              broadcast, POCSAG pager, AIS) to interpret it further."
                        .into(),
                }]
            }
            RawSignal::Audio {
                samples,
                sample_rate,
            } => {
                vec![DecodedItem {
                    summary: format!(
                        "{} audio samples at {sample_rate} Hz ({:.1}s)",
                        samples.len(),
                        samples.len() as f32 / *sample_rate as f32
                    ),
                    detail: "No protocol-specific decoder matched. Try the APT decoder if this is a NOAA weather satellite pass.".into(),
                }]
            }
            RawSignal::Packets(frames) => frames
                .iter()
                .enumerate()
                .map(|(i, f)| DecodedItem {
                    summary: format!("Frame {i}: {} bytes", f.len()),
                    detail: f
                        .iter()
                        .map(|b| format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                })
                .collect(),
        };

        DecodeOutput {
            items,
            image_path: None,
        }
    }
}
