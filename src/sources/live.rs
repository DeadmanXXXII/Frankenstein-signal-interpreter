use super::RawSignal;
use rtlsdr_rs::RtlSdr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// A single legally-receivable band. This list is intentionally the ONLY
/// way the GUI can pick a frequency — there is no free-text frequency entry
/// anywhere in the live-capture path. Every entry here is a band where
/// receiving is legal for the general public in the vast majority of
/// jurisdictions (check your own local rules before transmitting anything —
/// this app never transmits, but a HAM license is still required to key up
/// on the amateur bands with separate radio gear).
///
/// Deliberately NOT included, and not going in later: trunked/encrypted
/// public-safety or municipal radio, cellular, satellite phone, or any band
/// whose traffic is not intended for public reception. This app is
/// receive-only and stays within public/amateur/ISM allocations.
/// Which live demodulation path a band uses. Kept separate from the label
/// string so routing logic never has to string-match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveKind {
    /// FM-demodulate to audio and run the existing APT image decoder.
    Apt,
    /// PPM-demodulate to Mode S frames and run the existing ADS-B decoder.
    Adsb,
    /// No protocol-specific live path yet — show signal-strength stats.
    Generic,
}

#[derive(Debug, Clone, Copy)]
pub struct BandPreset {
    pub label: &'static str,
    pub center_freq_hz: u32,
    pub sample_rate_hz: u32,
    pub note: &'static str,
    pub kind: LiveKind,
}

pub const PUBLIC_BAND_PRESETS: &[BandPreset] = &[
    BandPreset {
        label: "NOAA APT weather satellites",
        center_freq_hz: 137_500_000,
        sample_rate_hz: 2_048_000,
        note: "137.5 MHz — public weather-satellite downlink, decodes with the APT decoder.",
        kind: LiveKind::Apt,
    },
    BandPreset {
        label: "ADS-B aircraft transponders",
        center_freq_hz: 1_090_000_000,
        // Simplified PPM demod below assumes exactly 2 samples/bit, so this
        // is pinned to 2.0 Msps rather than the 2.4 Msps dump1090 typically
        // uses (which needs a fractional-sample correlator). Most RTL-SDR
        // dongles support 2,000,000 as a valid sample rate.
        sample_rate_hz: 2_000_000,
        note: "1090 MHz — public aircraft position broadcasts (same signal flight trackers use).",
        kind: LiveKind::Adsb,
    },
    BandPreset {
        label: "Marine AIS",
        center_freq_hz: 161_975_000,
        sample_rate_hz: 250_000,
        note: "AIS ch. A, 161.975 MHz — public vessel-tracking broadcasts.",
        kind: LiveKind::Generic,
    },
    BandPreset {
        label: "2 m amateur (HAM) band",
        center_freq_hz: 146_000_000,
        sample_rate_hz: 250_000,
        note: "144–148 MHz — general amateur-radio activity, open to any listener.",
        kind: LiveKind::Generic,
    },
    BandPreset {
        label: "70 cm amateur (HAM) band",
        center_freq_hz: 435_000_000,
        sample_rate_hz: 250_000,
        note: "420–450 MHz — amateur satellite / repeater segment.",
        kind: LiveKind::Generic,
    },
    BandPreset {
        label: "FM broadcast",
        center_freq_hz: 100_000_000,
        sample_rate_hz: 250_000,
        note: "88–108 MHz — commercial FM radio, useful for a quick smoke-test of the dongle.",
        kind: LiveKind::Generic,
    },
    BandPreset {
        label: "License-free ISM (433 MHz)",
        center_freq_hz: 433_920_000,
        sample_rate_hz: 250_000,
        note: "433.05–434.79 MHz — weather stations, doorbells, sensors: unlicensed by design.",
        kind: LiveKind::Generic,
    },
];

pub struct LiveCapture {
    stop_flag: Arc<AtomicBool>,
    pub rx: Receiver<RawSignal>,
}

#[derive(Debug)]
pub enum LiveCaptureError {
    NoDeviceFound,
    Driver(String),
}

impl std::fmt::Display for LiveCaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LiveCaptureError::NoDeviceFound => {
                write!(f, "No RTL-SDR dongle found. Plug one in and retry.")
            }
            LiveCaptureError::Driver(e) => write!(f, "SDR driver error: {e}"),
        }
    }
}

impl LiveCapture {
    /// Starts a background capture thread tuned to `preset`. Streams ~0.5s
    /// chunks of IQ samples back over the channel so the GUI can decode
    /// continuously. This function only ever calls receive-side driver
    /// calls (open, set_center_freq, set_sample_rate, read) — there is no
    /// TX/transmit call anywhere in this file or the crate it wraps.
    pub fn start(preset: BandPreset, device_index: usize) -> Result<Self, LiveCaptureError> {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let thread_stop = stop_flag.clone();
        let (tx, rx): (Sender<RawSignal>, Receiver<RawSignal>) = std::sync::mpsc::channel();

        let mut sdr = RtlSdr::open(device_index).map_err(|_| LiveCaptureError::NoDeviceFound)?;
        sdr.set_center_freq(preset.center_freq_hz)
            .map_err(|e| LiveCaptureError::Driver(format!("{e:?}")))?;
        sdr.set_sample_rate(preset.sample_rate_hz)
            .map_err(|e| LiveCaptureError::Driver(format!("{e:?}")))?;
        sdr.reset_buffer()
            .map_err(|e| LiveCaptureError::Driver(format!("{e:?}")))?;

        thread::spawn(move || {
            // 0.5s worth of interleaved u8 I/Q samples per read.
            let chunk_len = (preset.sample_rate_hz as usize / 2) * 2;
            let mut buf = vec![0u8; chunk_len];

            while !thread_stop.load(Ordering::Relaxed) {
                match sdr.read_sync(&mut buf) {
                    Ok(n) if n >= 2 => {
                        let samples: Vec<(f32, f32)> = buf[..n & !1]
                            .chunks_exact(2)
                            .map(|c| ((c[0] as f32 - 127.5) / 127.5, (c[1] as f32 - 127.5) / 127.5))
                            .collect();
                        if tx.send(RawSignal::Iq(samples)).is_err() {
                            break; // receiver (GUI) dropped, stop capturing
                        }
                    }
                    Ok(_) => thread::sleep(Duration::from_millis(20)),
                    Err(_) => break,
                }
            }
        });

        Ok(Self { stop_flag, rx })
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

/// Best-effort device count so the GUI can show "no dongle detected" instead
/// of a raw driver error.
pub fn device_count() -> usize {
    rtlsdr_rs::RtlSdr::device_count().unwrap_or(0)
}
