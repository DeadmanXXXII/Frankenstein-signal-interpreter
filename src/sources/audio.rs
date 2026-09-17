use super::{RawSignal, SignalSource, SourceError};

/// Loads mono/stereo PCM or float WAV files, downmixing to mono f32 samples
/// in the -1.0..1.0 range.
pub struct AudioFileSource;

impl SignalSource for AudioFileSource {
    fn name(&self) -> &'static str {
        "Audio (WAV) file"
    }

    fn load(&self, path: &str) -> Result<RawSignal, SourceError> {
        let mut reader = hound::WavReader::open(path)
            .map_err(|e| SourceError::Format(format!("failed to open WAV file: {e}")))?;
        let spec = reader.spec();
        let channels = spec.channels.max(1) as usize;
        let sample_rate = spec.sample_rate;

        let raw_samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| SourceError::Format(format!("error reading float samples: {e}")))?,
            hound::SampleFormat::Int => {
                let max_val = (1i64 << (spec.bits_per_sample.max(1) - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| v as f32 / max_val))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| SourceError::Format(format!("error reading int samples: {e}")))?
            }
        };

        if raw_samples.is_empty() {
            return Err(SourceError::Format("WAV file contains no samples".into()));
        }

        let samples: Vec<f32> = if channels > 1 {
            raw_samples
                .chunks(channels)
                .map(|c| c.iter().sum::<f32>() / channels as f32)
                .collect()
        } else {
            raw_samples
        };

        Ok(RawSignal::Audio {
            samples,
            sample_rate,
        })
    }
}
