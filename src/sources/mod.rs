pub mod audio;
pub mod iq;
pub mod live;
pub mod live_demod;
pub mod packet;

/// A signal that has been loaded from some source, but not yet interpreted
/// by any protocol decoder.
#[derive(Debug, Clone)]
pub enum RawSignal {
    /// Complex I/Q samples, normalized roughly to the -1.0..1.0 range.
    Iq(Vec<(f32, f32)>),
    /// Mono PCM audio samples in the -1.0..1.0 range, plus their sample rate.
    Audio { samples: Vec<f32>, sample_rate: u32 },
    /// Discrete raw frames/packets, each a plain byte buffer.
    Packets(Vec<Vec<u8>>),
}

#[derive(Debug)]
pub enum SourceError {
    Io(std::io::Error),
    Format(String),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Io(e) => write!(f, "I/O error: {e}"),
            SourceError::Format(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for SourceError {}

impl From<std::io::Error> for SourceError {
    fn from(e: std::io::Error) -> Self {
        SourceError::Io(e)
    }
}

/// Something that can turn a file path into a `RawSignal`.
pub trait SignalSource {
    fn name(&self) -> &'static str;
    fn load(&self, path: &str) -> Result<RawSignal, SourceError>;
}
