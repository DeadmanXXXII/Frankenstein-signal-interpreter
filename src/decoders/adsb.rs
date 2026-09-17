use super::{DecodeOutput, DecodedItem, Decoder};
use crate::sources::RawSignal;

/// Decodes ADS-B extended squitter (DF17/18) Mode S frames: the public,
/// unencrypted broadcast that civil aircraft transmit for traffic
/// surveillance (the same data feeds sites like flightradar24 and apps like
/// FlightAware). Feed it hex frames from an SDR tool such as `dump1090
/// --raw`.
///
/// This does NOT decode Mode A/C/S secondary-surveillance-radar interrogation
/// or reply traffic beyond the header, and does not target military,
/// encrypted, or private communications links.
pub struct AdsbDecoder;

const CALLSIGN_CHARSET: &[u8] =
    b"?ABCDEFGHIJKLMNOPQRSTUVWXYZ????? ???????????????0123456789??????";
const CRC_GENERATOR: u32 = 0xFFF409;

impl Decoder for AdsbDecoder {
    fn name(&self) -> &'static str {
        "ADS-B / Mode S (aircraft transponder)"
    }

    fn decode(&self, raw: &RawSignal) -> DecodeOutput {
        let frames = match raw {
            RawSignal::Packets(frames) => frames,
            _ => {
                return DecodeOutput {
                    items: vec![DecodedItem {
                        summary: "ADS-B decoder needs a packet/data stream source".into(),
                        detail: "Feed it hex-encoded Mode S frames, one per line (e.g. dump1090 --raw output).".into(),
                    }],
                    image_path: None,
                };
            }
        };

        DecodeOutput {
            items: frames.iter().map(|f| decode_frame(f)).collect(),
            image_path: None,
        }
    }
}

fn decode_frame(frame: &[u8]) -> DecodedItem {
    if frame.len() != 14 && frame.len() != 7 {
        return DecodedItem {
            summary: format!("Unrecognized frame length ({} bytes)", frame.len()),
            detail: "Expected 7 bytes (56-bit short Mode S) or 14 bytes (112-bit long Mode S / ADS-B extended squitter)."
                .into(),
        };
    }

    let df = frame[0] >> 3;
    let ca = frame[0] & 0x07;

    if frame.len() == 7 {
        return DecodedItem {
            summary: format!("Short Mode S frame, DF={df}"),
            detail: "56-bit short frames (altitude/identity replies) aren't decoded in detail yet — only extended squitter (DF17/18) is.".into(),
        };
    }

    let icao = ((frame[1] as u32) << 16) | ((frame[2] as u32) << 8) | frame[3] as u32;
    let crc_ok = modes_crc_remainder(frame) == 0;

    if df != 17 && df != 18 {
        return DecodedItem {
            summary: format!(
                "DF={df} frame, ICAO {icao:06X}, CRC {}",
                if crc_ok { "OK" } else { "FAIL" }
            ),
            detail: "Only DF17/18 (ADS-B extended squitter) is decoded beyond the header right now.".into(),
        };
    }

    let me = &frame[4..11]; // 56-bit ME field
    let tc = me[0] >> 3;

    let mut detail_lines = vec![
        format!(
            "DF={df} CA={ca} ICAO={icao:06X} CRC={}",
            if crc_ok { "OK" } else { "FAIL" }
        ),
        format!("Type code (TC) = {tc}"),
    ];

    let summary = match tc {
        1..=4 => {
            let callsign = decode_callsign(me);
            detail_lines.push(format!("Callsign: {callsign}"));
            format!("ICAO {icao:06X}: identification, callsign \"{callsign}\"")
        }
        9..=18 => match decode_altitude(me) {
            Some(alt) => {
                detail_lines.push(format!("Barometric altitude: {alt} ft"));
                format!("ICAO {icao:06X}: airborne position, altitude {alt} ft")
            }
            None => {
                detail_lines.push("Altitude uses Gillham/Mode-C coding (not decoded here).".into());
                format!("ICAO {icao:06X}: airborne position (altitude coding not decoded)")
            }
        },
        19 => {
            detail_lines.push("Airborne velocity message (subtype decode not implemented yet).".into());
            format!("ICAO {icao:06X}: airborne velocity message")
        }
        5..=8 => format!("ICAO {icao:06X}: surface position (not decoded in detail)"),
        20..=22 => format!("ICAO {icao:06X}: airborne position with GNSS height (not decoded in detail)"),
        28 => format!("ICAO {icao:06X}: aircraft status message"),
        _ => format!("ICAO {icao:06X}: type code {tc} (not decoded)"),
    };

    detail_lines.push(format!(
        "Raw ME field: {}",
        me.iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    ));

    DecodedItem {
        summary,
        detail: detail_lines.join("\n"),
    }
}

fn decode_callsign(me: &[u8]) -> String {
    // 8 characters x 6 bits, packed across me[1..7] (48 bits total)
    let mut bits: u64 = 0;
    for &b in &me[1..7] {
        bits = (bits << 8) | b as u64;
    }
    let mut out = String::with_capacity(8);
    for i in (0..8).rev() {
        let idx = ((bits >> (i * 6)) & 0x3F) as usize;
        out.push(CALLSIGN_CHARSET[idx] as char);
    }
    out.trim_end_matches([' ', '?']).to_string()
}

fn decode_altitude(me: &[u8]) -> Option<i32> {
    // AC12 field = ME bits 9..20 (1-indexed within ME) = me[1] (8 bits)
    // followed by the top 4 bits of me[2].
    let ac12 = ((me[1] as u16) << 4) | ((me[2] as u16) >> 4);
    let q_bit = (ac12 >> 4) & 1;
    if q_bit == 0 {
        return None; // Gillham-coded (Mode C style), not handled here
    }
    let n = ((ac12 >> 5) << 4) | (ac12 & 0x0F);
    Some(n as i32 * 25 - 1000)
}

/// Mode S 24-bit CRC check, exposed so the live PPM demodulator can use it
/// to reject false preamble hits before a candidate frame ever reaches the
/// decoder above.
pub(crate) fn modes_crc_ok(frame: &[u8]) -> bool {
    modes_crc_remainder(frame) == 0
}

/// Mode S / ADS-B 24-bit CRC (generator polynomial 0xFFF409, no reflection,
/// no XOR-out). For an intact DF17/18 message — data plus its own appended
/// parity, with no interrogator-code XOR applied — dividing the *entire*
/// frame by the generator yields zero.
fn modes_crc_remainder(msg: &[u8]) -> u32 {
    let mut reg: u32 = 0;
    for &byte in msg {
        for bit in (0..8).rev() {
            let bit_in = ((byte >> bit) & 1) as u32;
            let top_bit = (reg >> 23) & 1;
            reg = ((reg << 1) | bit_in) & 0x00FF_FFFF;
            if top_bit == 1 {
                reg ^= CRC_GENERATOR;
            }
        }
    }
    reg
}
