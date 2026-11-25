use crate::{
    Player,
    audio::{
        AudioTrack, OutputMode, OutputRequest, ResamplerQuality, replaygain::Mode as GainMode,
    },
    volume_slider,
};
use eframe::egui;
use std::time::{Duration, Instant};

impl Player {
    pub(super) fn output_request(&self) -> OutputRequest {
        OutputRequest {
            device: self.library.audio_device.clone(),
            options: crate::audio::OutputOptions {
                effects_requested: self.library.audio_dsp.active(),
                ..self.library.audio_output
            },
            after: self.audio_release.clone(),
            dsp: self.library.audio_dsp.clone(),
        }
    }

    pub(super) fn software_volume_available(&self) -> bool {
        self.playing
            .as_ref()
            .and_then(|playing| playing.sink.output.as_ref())
            .map_or(
                self.library.audio_output.mode == OutputMode::Shared
                    || self.library.audio_output.software_gain,
                |output| output.software_gain,
            )
    }

    pub(super) fn update_audio(&mut self, ctx: &egui::Context) {
        let changed = self
            .audio_devices
            .as_mut()
            .is_some_and(|monitor| monitor.refresh());
        let desired = self.audio_devices.as_ref().and_then(|monitor| {
            monitor
                .devices
                .selected(self.library.audio_device.as_deref())
        });
        let mut retry = false;
        let mut refresh_presence = false;
        if let Some(playing) = &mut self.playing {
            if let Some(error) = playing.sink.poll() {
                self.status = format!("Audio output: {error}");
                refresh_presence = playing.sink.failed;
            }
            retry = changed
                && desired.is_some_and(|id| {
                    playing.sink.failed
                        || playing
                            .sink
                            .output
                            .as_ref()
                            .is_some_and(|output| output.id != id)
                });
            if playing.sink.output_lost() && desired.is_some() {
                if self
                    .audio_retry_after
                    .is_none_or(|until| Instant::now() >= until)
                {
                    retry = true;
                } else {
                    ctx.request_repaint_after(Duration::from_millis(500));
                }
            }
        }
        self.apply_gapless_transition(ctx);
        self.observe_lastfm();
        if retry {
            self.reopen_audio(ctx, true);
        } else if refresh_presence {
            self.sync_presence();
        }
        self.record_play();
    }

    pub(super) fn reopen_audio(&mut self, ctx: &egui::Context, paused: bool) {
        let request = self.output_request();
        let volume = self.effective_volume();
        self.observe_lastfm();
        let Some(playing) = &mut self.playing else {
            return;
        };
        self.audio_retry_after = Some(Instant::now() + Duration::from_secs(2));
        playing.sink.pause();
        let Some(path) = self
            .library
            .track_path(playing.id)
            .map(|path| path.to_path_buf())
        else {
            return;
        };
        match AudioTrack::open(&path, request, playing.sink.get_pos(), volume, paused, ctx) {
            Ok(sink) => {
                self.audio_release = Some(sink.release_gate());
                playing.sink = sink;
                playing.lastfm.rebase(Duration::ZERO);
                self.status = if paused {
                    "Audio output changed. Playback paused.".into()
                } else {
                    String::new()
                };
            }
            Err(error) => self.status = format!("Could not reopen the audio output: {error}"),
        }
        self.sync_presence();
    }

    pub(super) fn audio_settings(&mut self, ui: &mut egui::Ui) {
        if ui.button("Audio setup and PCM check").clicked() {
            self.setup.open(
                self.library.audio_device.clone(),
                self.library.audio_output.mode,
            );
        }
        let mut selected = self.library.audio_device.clone();
        let mut options = self.library.audio_output;
        if let Some(monitor) = &self.audio_devices {
            let options = &monitor.devices;
            let active_label = options
                .output
                .iter()
                .find(|device| Some(device.id.as_str()) == selected.as_deref())
                .map(|device| device.label.as_str());
            let label = active_label.unwrap_or(if selected.is_some() {
                "Saved device unavailable"
            } else {
                "Windows default"
            });
            egui::ComboBox::from_label("Output device")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut selected, None, "Windows default");
                    for device in &options.output {
                        ui.selectable_value(&mut selected, Some(device.id.clone()), &device.label)
                            .on_hover_text(&device.id);
                    }
                });
            if options.output.is_empty() {
                ui.weak("No Windows output devices currently available.");
            }
        }
        egui::ComboBox::from_label("Output mode")
            .selected_text(mode_label(options.mode))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut options.mode,
                    OutputMode::Shared,
                    mode_label(OutputMode::Shared),
                );
                ui.selectable_value(
                    &mut options.mode,
                    OutputMode::Exclusive,
                    mode_label(OutputMode::Exclusive),
                );
            });
        if options.mode == OutputMode::Exclusive {
            ui.checkbox(&mut options.convert_exclusive, "Allow Exclusive format conversion")
                .on_hover_text("Try a supported device format if the original rate, channels, or sample format cannot be opened.");
            ui.checkbox(
                &mut options.fallback_shared,
                "Fall back to WASAPI Shared on an Exclusive error",
            );
            ui.checkbox(
                &mut options.software_gain,
                "Enable software volume in Exclusive",
            );
            ui.checkbox(
                &mut options.transition_fades,
                "Fade out on pause, seek, and stop",
            );
        }
        egui::ComboBox::from_label("ReplayGain")
            .selected_text(match options.replay_gain.mode {
                GainMode::Off => "Off",
                GainMode::Track => "Track",
                GainMode::Album => "Album",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut options.replay_gain.mode, GainMode::Off, "Off");
                ui.selectable_value(&mut options.replay_gain.mode, GainMode::Track, "Track");
                ui.selectable_value(&mut options.replay_gain.mode, GainMode::Album, "Album");
            });
        if options.replay_gain.enabled() {
            ui.add(
                egui::Slider::new(&mut options.replay_gain.preamp_db, -24..=12)
                    .suffix(" dB")
                    .text("Preamp"),
            );
        }
        let mut downmix = !options.reject_downmix;
        ui.checkbox(
            &mut downmix,
            "Allow downmix when the output has fewer channels",
        );
        options.reject_downmix = !downmix;
        ui.horizontal(|ui| {
            ui.label("Resampler");
            egui::ComboBox::from_id_salt("resampler-quality")
                .selected_text(options.resampler.label())
                .show_ui(ui, |ui| {
                    for quality in [
                        ResamplerQuality::Fast,
                        ResamplerQuality::Balanced,
                        ResamplerQuality::High,
                        ResamplerQuality::Maximum,
                    ] {
                        ui.selectable_value(&mut options.resampler, quality, quality.label());
                    }
                });
        })
        .response
        .on_hover_text("Used only when the source rate differs from the output rate.");
        ui.checkbox(&mut options.verify_flac, "Verify FLAC MD5 during playback")
            .on_hover_text("Verification runs when playing a complete FLAC from the beginning.");
        let mut dsp = self.library.audio_dsp.clone();
        egui::CollapsingHeader::new("DSP").show(ui, |ui| {
            ui.checkbox(&mut dsp.eq, "Parametric EQ");
            if ui.button("Import AutoEQ profile…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("AutoEQ preset", &["txt"])
                    .pick_file()
                && let Err(error) = dsp.import_eq(&path)
            {
                self.status = format!("Could not import the EQ profile: {error}");
            }
            if dsp.eq {
                ui.add(
                    egui::Slider::new(&mut dsp.preamp, -40.0..=20.0)
                        .suffix(" dB")
                        .text("EQ preamp"),
                );
                ui.weak(format!("{} imported bands", dsp.bands.len()));
            }
            ui.checkbox(&mut dsp.fir, "FIR convolution");
            if ui.button("Choose WAV impulse…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("WAV", &["wav"])
                    .pick_file()
            {
                dsp.impulse = Some(path);
                dsp.fir = true;
            }
            if let Some(path) = &dsp.impulse {
                ui.label(
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy(),
                );
            }
            ui.checkbox(&mut dsp.crossfeed, "Stereo crossfeed");
            ui.weak("Active DSP modifies the PCM samples.");
        });
        if selected != self.library.audio_device
            || options != self.library.audio_output
            || dsp != self.library.audio_dsp
        {
            self.library.audio_device = selected;
            self.library.audio_output = options;
            self.library.audio_dsp = dsp;
            self.save();
            self.reopen_audio(ui.ctx(), true);
        }
        if ui
            .add_enabled_ui(self.software_volume_available(), |ui| {
                volume_slider(ui, &mut self.library.volume)
            })
            .inner
            .changed()
        {
            self.library.muted = false;
            self.volume_adjusted();
        }
        if !self.software_volume_available() {
            ui.weak("Use the volume control on your audio device.");
        }
        ui.separator();
        ui.heading("Signal path");
        let Some(playing) = &self.playing else {
            ui.weak("Load a track to inspect the signal path.");
            return;
        };
        let sink = &playing.sink;
        let source = &sink.source;
        let decoded = pcm_format(&source.kind.label(), source.rate, source.channels);
        let Some(output) = &sink.output else {
            ui.label(format!("Decoder: {} · {decoded}", source.codec));
            ui.weak(if sink.failed {
                self.status.as_str()
            } else {
                "Opening Windows audio output…"
            });
            return;
        };

        let summary = if sink.failed || sink.output_lost() {
            self.status.as_str()
        } else if sink.bit_perfect_now() {
            "Bit-perfect PCM at the Windows driver input"
        } else if output.mode == OutputMode::Shared {
            "WASAPI Shared uses the Windows mixer"
        } else if output.direct.is_none() {
            "Exclusive playback with PCM processing"
        } else if output.transition_fades {
            "Direct PCM with transition fades enabled"
        } else if sink.underruns() > 0 {
            "Direct PCM interrupted by an underrun"
        } else {
            "Direct PCM format negotiated"
        };
        ui.label(summary);

        let processing = if output.direct.is_some() {
            decoded.clone()
        } else {
            pcm_format("32-bit float PCM", source.rate, source.channels)
        };
        let mapped = pcm_format("32-bit float PCM", source.rate, output.channels);
        let processed = pcm_format("32-bit float PCM", output.rate, output.channels);
        let device_pcm = pcm_format(&output.sample, output.rate, output.channels);
        let bypass_format = if output.direct.is_some() {
            &decoded
        } else {
            &processed
        };
        let volume = self.effective_volume();
        let replay_gain = match source.gain {
            Some(gain) => format!("{:+.1} dB", 20.0 * gain.log10()),
            None if self.library.audio_output.replay_gain.enabled() => "Bypassed · no tag".into(),
            None => "Bypassed".into(),
        };
        let channel_conversion = match source.channels.cmp(&output.channels) {
            std::cmp::Ordering::Greater => "Downmix",
            std::cmp::Ordering::Less => "Upmix",
            std::cmp::Ordering::Equal => "Bypassed",
        };
        let software_volume = if !output.software_gain {
            "Bypassed".into()
        } else if volume == 1.0 {
            "Unity gain".into()
        } else {
            format!("{:.0}%", volume * 100.0)
        };
        let dither = if !output.dither {
            "Bypassed"
        } else if output.software_gain && volume == 0.0 {
            "Bypassed · muted"
        } else {
            "Active"
        };

        egui::Grid::new("audio_signal_path")
            .num_columns(3)
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Stage");
                ui.strong("Status");
                ui.strong("PCM format");
                ui.end_row();
                for (stage, status, format) in [
                    ("Decoder", source.codec, decoded.as_str()),
                    ("ReplayGain", replay_gain.as_str(), processing.as_str()),
                    (
                        "Channel mapping",
                        channel_conversion,
                        if source.channels == output.channels {
                            processing.as_str()
                        } else {
                            mapped.as_str()
                        },
                    ),
                    (
                        "Sinc resampler",
                        if source.rate != output.rate {
                            "Active"
                        } else {
                            "Bypassed"
                        },
                        bypass_format.as_str(),
                    ),
                    (
                        "Equalizer",
                        effect_status(output.effects.eq),
                        bypass_format.as_str(),
                    ),
                    (
                        "FIR convolution",
                        effect_status(output.effects.fir),
                        bypass_format.as_str(),
                    ),
                    (
                        "Crossfeed",
                        effect_status(output.effects.crossfeed),
                        bypass_format.as_str(),
                    ),
                    (
                        "Software volume",
                        software_volume.as_str(),
                        bypass_format.as_str(),
                    ),
                    ("TPDF dither", dither, device_pcm.as_str()),
                    ("WASAPI", mode_label(output.mode), device_pcm.as_str()),
                    ("Output device", output.device.as_str(), "Windows endpoint"),
                ] {
                    ui.label(stage);
                    ui.label(status);
                    ui.label(format);
                    ui.end_row();
                }
            });

        ui.horizontal_wrapped(|ui| {
            if let Some(frames) = output.buffer_frames {
                ui.label(format!(
                    "Device buffer: {frames} frames ({:.1} ms)",
                    f64::from(frames) * 1_000.0 / f64::from(output.rate)
                ));
            }
            ui.label(format!("Underruns: {}", sink.underruns()));
        });
        if sink.priority_denied() {
            ui.weak("Windows denied real-time thread priority.");
        }
        if self
            .library
            .audio_device
            .as_ref()
            .is_some_and(|id| id != &output.id)
        {
            ui.weak("Using the Windows default while the saved device is unavailable.");
        }
    }
}

fn pcm_format(sample: &str, rate: u32, channels: u16) -> String {
    format!("{rate} Hz · {channels} ch · {sample}")
}

fn effect_status(active: bool) -> &'static str {
    if active { "Active" } else { "Bypassed" }
}

fn mode_label(mode: OutputMode) -> &'static str {
    match mode {
        OutputMode::Shared => "WASAPI Shared",
        OutputMode::Exclusive => "WASAPI Exclusive",
    }
}
