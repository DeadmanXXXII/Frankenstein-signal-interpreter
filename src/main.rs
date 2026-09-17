mod decoders;
mod sources;

use decoders::adsb::AdsbDecoder;
use decoders::apt::AptDecoder;
use decoders::generic::GenericDecoder;
use decoders::{DecodeOutput, Decoder, DecodedItem};
use eframe::egui;
use sources::audio::AudioFileSource;
use sources::iq::IqFileSource;
use sources::live::{self, BandPreset, LiveCapture, LiveKind, PUBLIC_BAND_PRESETS};
use sources::live_demod::{AdsbLiveDemod, FmDemod};
use sources::packet::PacketStreamSource;
use sources::{RawSignal, SignalSource};

/// Frankenstein palette, pulled from the app logo: near-black background,
/// the "alive" neon green, and a rust-orange accent for the gears motif.
mod theme {
    use eframe::egui::Color32;
    pub const BG: Color32 = Color32::from_rgb(13, 15, 11);
    pub const PANEL: Color32 = Color32::from_rgb(20, 24, 18);
    pub const GREEN: Color32 = Color32::from_rgb(57, 255, 20);
    pub const GREEN_DIM: Color32 = Color32::from_rgb(30, 140, 20);
    pub const RUST: Color32 = Color32::from_rgb(193, 68, 14);
    pub const TEXT: Color32 = Color32::from_rgb(214, 222, 209);
}

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = theme::BG;
    visuals.window_fill = theme::PANEL;
    visuals.extreme_bg_color = theme::PANEL;
    visuals.override_text_color = Some(theme::TEXT);
    visuals.widgets.inactive.bg_fill = theme::PANEL;
    visuals.widgets.hovered.bg_fill = theme::GREEN_DIM;
    visuals.widgets.active.bg_fill = theme::GREEN_DIM;
    visuals.widgets.noninteractive.fg_stroke.color = theme::TEXT;
    visuals.selection.bg_fill = theme::GREEN_DIM;
    visuals.hyperlink_color = theme::GREEN;
    visuals.warn_fg_color = theme::RUST;
    ctx.set_visuals(visuals);
}

#[derive(PartialEq, Clone, Copy)]
enum SourceKind {
    Iq,
    Audio,
    Packet,
}

struct ChannelState {
    kind: SourceKind,
    enabled: bool,
    path: String,
    output: Option<DecodeOutput>,
    error: Option<String>,
}

impl ChannelState {
    fn new(kind: SourceKind, enabled: bool) -> Self {
        Self {
            kind,
            enabled,
            path: String::new(),
            output: None,
            error: None,
        }
    }

    fn label(&self) -> &'static str {
        match self.kind {
            SourceKind::Iq => "IQ / SDR raw file  (.cu8 / .cf32)",
            SourceKind::Audio => "Audio (WAV) file  — e.g. NOAA APT pass",
            SourceKind::Packet => "Packet / data stream  — hex frames, e.g. ADS-B",
        }
    }
}

/// State for the live-capture panel. `enabled` is the user-facing toggle;
/// the panel only ever offers the hardcoded public-band presets, never a
/// free-text frequency field, and `capture` only ever reads from the dongle.
struct LiveState {
    enabled: bool,
    preset_idx: usize,
    device_count: usize,
    capture: Option<LiveCapture>,
    log: Vec<DecodedItem>,
    status: String,
    // Per-protocol live demod state — reset each time Start is pressed.
    fm: FmDemod,
    apt_audio: Vec<f32>,
    apt_sample_rate: u32,
    apt_last_decoded_len: usize,
    apt_image_path: Option<String>,
    adsb: AdsbLiveDemod,
}

impl Default for LiveState {
    fn default() -> Self {
        Self {
            enabled: false,
            preset_idx: 0,
            device_count: live::device_count(),
            capture: None,
            log: Vec::new(),
            status: "Idle".into(),
            fm: FmDemod::new(),
            apt_audio: Vec::new(),
            apt_sample_rate: 0,
            apt_last_decoded_len: 0,
            apt_image_path: None,
            adsb: AdsbLiveDemod::new(),
        }
    }
}

struct SignalInterpreterApp {
    channels: Vec<ChannelState>,
    live: LiveState,
}

impl Default for SignalInterpreterApp {
    fn default() -> Self {
        Self {
            channels: vec![
                ChannelState::new(SourceKind::Iq, true),
                ChannelState::new(SourceKind::Audio, true),
                ChannelState::new(SourceKind::Packet, true),
            ],
            live: LiveState::default(),
        }
    }
}

impl SignalInterpreterApp {
    fn draw_live_panel(&mut self, ui: &mut egui::Ui) {
        let live = &mut self.live;

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut live.enabled, "Live capture (receive-only)");
                ui.label(
                    egui::RichText::new("public bands only \u{2014} no transmit, ever")
                        .italics()
                        .color(theme::RUST),
                );
            });

            if !live.enabled {
                if let Some(cap) = live.capture.take() {
                    cap.stop();
                    live.status = "Idle".into();
                }
                return;
            }

            if live.device_count == 0 {
                ui.colored_label(
                    theme::RUST,
                    "No RTL-SDR dongle detected. Plug one in, then reopen this panel.",
                );
                if ui.button("Re-scan for devices").clicked() {
                    live.device_count = live::device_count();
                }
                return;
            }

            egui::ComboBox::from_label("Band")
                .selected_text(PUBLIC_BAND_PRESETS[live.preset_idx].label)
                .show_ui(ui, |ui| {
                    for (i, preset) in PUBLIC_BAND_PRESETS.iter().enumerate() {
                        ui.selectable_value(&mut live.preset_idx, i, preset.label);
                    }
                });
            ui.label(
                egui::RichText::new(PUBLIC_BAND_PRESETS[live.preset_idx].note).weak(),
            );

            ui.horizontal(|ui| {
                let running = live.capture.is_some();
                if !running && ui.button("Start").clicked() {
                    let preset: BandPreset = PUBLIC_BAND_PRESETS[live.preset_idx];
                    match LiveCapture::start(preset, 0) {
                        Ok(cap) => {
                            live.capture = Some(cap);
                            live.status = format!("Receiving on {}", preset.label);
                            live.log.clear();
                            live.fm = FmDemod::new();
                            live.apt_audio.clear();
                            let decim = (preset.sample_rate_hz / 51_200).max(1);
                            live.apt_sample_rate = preset.sample_rate_hz / decim;
                            live.apt_last_decoded_len = 0;
                            live.apt_image_path = None;
                            live.adsb = AdsbLiveDemod::new();
                        }
                        Err(e) => live.status = format!("Failed to start: {e}"),
                    }
                }
                if running && ui.button("Stop").clicked() {
                    if let Some(cap) = live.capture.take() {
                        cap.stop();
                    }
                    live.status = "Idle".into();
                }
                ui.label(egui::RichText::new(&live.status).color(theme::GREEN));
            });

            // Drain whatever IQ chunks have arrived since the last frame and
            // route them to the matching live demodulator + the SAME
            // decoder the file-based channels above use.
            if live.capture.is_some() {
                let preset = PUBLIC_BAND_PRESETS[live.preset_idx];
                let mut chunks: Vec<RawSignal> = Vec::new();
                if let Some(cap) = &live.capture {
                    while let Ok(raw) = cap.rx.try_recv() {
                        chunks.push(raw);
                    }
                }

                for raw in chunks {
                    let RawSignal::Iq(iq) = &raw else { continue };
                    match preset.kind {
                        LiveKind::Generic => {
                            let mut out = GenericDecoder.decode(&raw);
                            live.log.append(&mut out.items);
                        }
                        LiveKind::Adsb => {
                            let frames = live.adsb.feed(iq);
                            if !frames.is_empty() {
                                let mut out =
                                    AdsbDecoder.decode(&RawSignal::Packets(frames));
                                live.log.append(&mut out.items);
                            }
                        }
                        LiveKind::Apt => {
                            let demod = live.fm.process(iq);
                            let decim = (preset.sample_rate_hz / 51_200).max(1) as usize;
                            live.apt_audio
                                .extend(sources::live_demod::decimate(&demod, decim));

                            // Re-run the APT decoder every ~4s of new audio
                            // rather than every frame — it's O(n) over the
                            // whole buffer, no point paying that 60x/sec.
                            let refresh_every = (live.apt_sample_rate as usize) * 4;
                            if live.apt_audio.len()
                                >= live.apt_last_decoded_len + refresh_every
                            {
                                live.apt_last_decoded_len = live.apt_audio.len();
                                let audio = RawSignal::Audio {
                                    samples: live.apt_audio.clone(),
                                    sample_rate: live.apt_sample_rate,
                                };
                                let out = AptDecoder.decode(&audio);
                                live.apt_image_path = out.image_path.clone();
                                live.log = out.items; // replace, not append: one running image
                            }
                        }
                    }
                }

                if live.log.len() > 200 {
                    let excess = live.log.len() - 200;
                    live.log.drain(0..excess);
                }
            }

            if let Some(img) = &live.apt_image_path {
                ui.label(format!("Live APT image (updating): {img}"));
            }

            if !live.log.is_empty() {
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("live_scroll")
                    .max_height(180.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for item in &live.log {
                            ui.monospace(item.summary.as_str());
                        }
                    });
            }
        });

        if live.enabled && live.capture.is_some() {
            ui.ctx().request_repaint();
        }
    }
}

impl eframe::App for SignalInterpreterApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(egui::RichText::new("Frankenstein Signal Interpreter").color(theme::GREEN));
            ui.label("Toggle a channel, point it at a file, and decode. All channels run independently.");
            ui.separator();

            self.draw_live_panel(ui);
            ui.separator();

            for i in 0..self.channels.len() {
                let (label, kind) = {
                    let c = &self.channels[i];
                    (c.label(), c.kind)
                };

                ui.group(|ui| {
                    ui.checkbox(&mut self.channels[i].enabled, label);

                    if self.channels[i].enabled {
                        ui.horizontal(|ui| {
                            ui.label("File path:");
                            ui.text_edit_singleline(&mut self.channels[i].path);
                        });

                        if ui.button("Load & Decode").clicked() {
                            let path = self.channels[i].path.clone();
                            match load_and_decode(kind, &path) {
                                Ok(output) => {
                                    self.channels[i].output = Some(output);
                                    self.channels[i].error = None;
                                }
                                Err(e) => {
                                    self.channels[i].output = None;
                                    self.channels[i].error = Some(e);
                                }
                            }
                        }

                        if let Some(err) = &self.channels[i].error {
                            ui.colored_label(egui::Color32::RED, err.as_str());
                        }

                        if let Some(output) = &self.channels[i].output {
                            ui.separator();
                            egui::ScrollArea::vertical()
                                .id_salt(format!("scroll_{i}"))
                                .max_height(240.0)
                                .show(ui, |ui| {
                                    for item in &output.items {
                                        ui.strong(item.summary.as_str());
                                        ui.monospace(item.detail.as_str());
                                        ui.add_space(4.0);
                                    }
                                });
                            if let Some(img_path) = &output.image_path {
                                ui.label(format!("Saved decoded image to: {img_path}"));
                            }
                        }
                    }
                });
            }
        });
    }
}

fn load_and_decode(kind: SourceKind, path: &str) -> Result<DecodeOutput, String> {
    if path.trim().is_empty() {
        return Err("Enter a file path first.".to_string());
    }

    match kind {
        SourceKind::Iq => {
            let raw = IqFileSource.load(path).map_err(|e| e.to_string())?;
            Ok(GenericDecoder.decode(&raw))
        }
        SourceKind::Audio => {
            let raw = AudioFileSource.load(path).map_err(|e| e.to_string())?;
            let apt_out = AptDecoder.decode(&raw);
            if apt_out.image_path.is_some() {
                Ok(apt_out)
            } else {
                Ok(GenericDecoder.decode(&raw))
            }
        }
        SourceKind::Packet => {
            let raw = PacketStreamSource.load(path).map_err(|e| e.to_string())?;
            Ok(AdsbDecoder.decode(&raw))
        }
    }
}

/// The app logo, embedded at compile time so no external file path is
/// needed at runtime — decoded with the `image` crate into the raw RGBA
/// format `egui::IconData` expects.
fn load_icon() -> egui::IconData {
    let bytes = include_bytes!("../assets/logo.png");
    let image = image::load_from_memory(bytes)
        .expect("embedded assets/logo.png should be a valid image")
        .into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(load_icon())
            .with_title("Frankenstein — Signal Analysis & Decoding"),
        ..Default::default()
    };
    eframe::run_native(
        "Frankenstein",
        options,
        Box::new(|cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(SignalInterpreterApp::default()))
        }),
    )
}
