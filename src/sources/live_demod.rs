//! Turns live raw IQ into the same shapes the existing file-based decoders
//! already consume (`RawSignal::Audio` for APT, `RawSignal::Packets` for
//! ADS-B), so the actual protocol decoding logic in `decoders::apt` and
//! `decoders::adsb` is reused unchanged rather than duplicated.
//!
//! These are real, working demodulators, not stubs — but they are
//! deliberately simplified versions of what a production tool (dump1090,
//! wxtoimg) does, traded for something that fits in a first pass. See the
//! comments on each piece for exactly what's cut down and why it's still
//! correct within that simplification.

use crate::decoders::adsb::modes_crc_ok;

/// Streaming quadrature FM discriminator. Keeps one sample of phase state
/// across calls so a signal isn't corrupted at chunk boundaries.
pub struct FmDemod {
    prev_i: f32,
    prev_q: f32,
    have_prev: bool,
}

impl FmDemod {
    pub fn new() -> Self {
        Self {
            prev_i: 0.0,
            prev_q: 0.0,
            have_prev: false,
        }
    }

    /// One demodulated sample per input IQ sample, proportional to
    /// instantaneous frequency deviation — i.e. standard FM audio.
    pub fn process(&mut self, iq: &[(f32, f32)]) -> Vec<f32> {
        let mut out = Vec::with_capacity(iq.len());
        for &(i, q) in iq {
            if self.have_prev {
                // angle(z[n] * conj(z[n-1]))
                let re = i * self.prev_i + q * self.prev_q;
                let im = q * self.prev_i - i * self.prev_q;
                out.push(im.atan2(re));
            } else {
                out.push(0.0);
            }
            self.prev_i = i;
            self.prev_q = q;
            self.have_prev = true;
        }
        out
    }
}

/// Naive integer decimation (pick every Nth sample, no anti-alias filter).
/// Fine here: the signal of interest (APT's 2.4 kHz subcarrier) sits far
/// below the Nyquist rate left after decimating a >1 MHz IQ stream down to
/// tens of kHz, so aliasing from anything above the new Nyquist is not
/// going to land on it in practice. A proper low-pass first would be more
/// correct on a noisy band; this is the "first pass, verify on real
/// hardware" tradeoff called out in CHANGES.md.
pub fn decimate(samples: &[f32], factor: usize) -> Vec<f32> {
    if factor <= 1 {
        return samples.to_vec();
    }
    samples.iter().step_by(factor).copied().collect()
}

/// Simplified Mode S / ADS-B PPM demodulator, in the same spirit as
/// dump1090's classic (pre-oversampling) decoder: correlate a short
/// magnitude pattern for the preamble, then read each bit as "first
/// half-bit chip louder than the second" (=1) or vice versa (=0).
///
/// Simplifications versus a production decoder:
/// - assumes exactly 2 samples per bit (i.e. sample rate pinned to
///   2.0 Msps — see the ADS-B `BandPreset`), not dump1090's 2.4 Msps
///   oversampled/interpolated approach
/// - only attempts to decode 112-bit / 14-byte long frames (DF17/18
///   extended squitter), not the 56-bit short replies
/// - every candidate is CRC-checked before being reported, which is what
///   keeps false preamble hits from producing garbage output — a failed
///   CRC is silently dropped rather than shown as a "decoded" frame
pub struct AdsbLiveDemod {
    carry: Vec<f32>,
}

const PREAMBLE_LEN: usize = 16; // 8 microseconds at 2 samples/us
const FRAME_BITS: usize = 112;
const FRAME_BYTES: usize = 14;

impl AdsbLiveDemod {
    pub fn new() -> Self {
        Self { carry: Vec::new() }
    }

    /// Feed a chunk of raw IQ (at 2.0 Msps); returns any complete, CRC-valid
    /// 14-byte frames found.
    pub fn feed(&mut self, iq: &[(f32, f32)]) -> Vec<Vec<u8>> {
        let mut mag: Vec<f32> = std::mem::take(&mut self.carry);
        mag.extend(iq.iter().map(|(i, q)| (i * i + q * q).sqrt()));

        let frame_span = PREAMBLE_LEN + FRAME_BITS * 2;
        let mut frames = Vec::new();
        let mut pos = 0;

        while pos + frame_span <= mag.len() {
            if is_preamble(&mag[pos..pos + PREAMBLE_LEN]) {
                if let Some(bytes) = decode_bits(&mag[pos + PREAMBLE_LEN..pos + frame_span]) {
                    frames.push(bytes);
                    pos += frame_span; // don't re-scan inside a frame we just used
                    continue;
                }
            }
            pos += 1;
        }

        // Keep a tail long enough that a frame straddling this chunk
        // boundary gets a full chance to match on the next call.
        let keep = frame_span.min(mag.len());
        self.carry = mag[mag.len() - keep..].to_vec();
        frames
    }
}

fn is_preamble(m: &[f32]) -> bool {
    // Mode S preamble: pulses at half-microsecond slots 0, 2, 7, 9 out of 16.
    m[0] > m[1] && m[2] > m[1] && m[2] > m[3] && m[7] > m[6] && m[7] > m[8] && m[9] > m[8]
}

fn decode_bits(bit_mags: &[f32]) -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; FRAME_BYTES];
    for bit_i in 0..FRAME_BITS {
        let a = bit_mags[2 * bit_i];
        let b = bit_mags[2 * bit_i + 1];
        if a > b {
            bytes[bit_i / 8] |= 1 << (7 - (bit_i % 8));
        }
    }
    if modes_crc_ok(&bytes) {
        Some(bytes)
    } else {
        None
    }
}
