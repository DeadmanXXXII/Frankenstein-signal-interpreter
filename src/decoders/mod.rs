pub mod adsb;
pub mod apt;
pub mod generic;

use crate::sources::RawSignal;

#[derive(Debug, Clone)]
pub struct DecodedItem {
    pub summary: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct DecodeOutput {
    pub items: Vec<DecodedItem>,
    /// Set when a decoder produced an image file on disk (e.g. APT).
    pub image_path: Option<String>,
}

/// Something that turns a `RawSignal` into human/computer-readable output.
pub trait Decoder {
    fn name(&self) -> &'static str;
    fn decode(&self, raw: &RawSignal) -> DecodeOutput;
}
