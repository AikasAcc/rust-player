use crate::{
    Player,
    audio::{
        AudioTrack, OutputMode, OutputOptions, OutputRequest,
        setup::{self, DeviceCheck},
    },
};
use eframe::egui;
use std::{sync::mpsc, time::Duration};

pub(super) struct Setup {
    pub visible: bool,
    selected: Option<String>,
    mode: OutputMode,
    check: Option<DeviceCheck>,
    check_rx: Option<mpsc::Receiver<Result<DeviceCheck, String>>>,
    pcm_rx: Option<mpsc::Receiver<Result<String, String>>>,
    tone: Option<(AudioTrack, tempfile::TempDir)>,
    release: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    status: String,
}

impl Setup {
    pub fn new(visible: bool) -> Self {
        Self {
            visible,
            selected: None,
            mode: OutputMode::Shared,
            check: None,
            check_rx: None,
            pcm_rx: None,
            tone: None,
            release: None,
            status: String::new(),
        }
    }

    pub fn open(&mut self, selected: Option<String>, mode: OutputMode) {
        self.visible = true;
        self.selected = selected;
        self.mode = mode;
        self.check = None;
        self.check_rx = None;
        self.status.clear();
    }

    fn probe(&mut self, ctx: &egui::Context) {
        let device = self.selected.clone();
        let ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        self.check_rx = Some(rx);
        self.status.clear();
        std::thread::spawn(move || {
            let _ = tx.send(DeviceCheck::run(device.as_deref()));
            ctx.request_repaint();
        });
    }

    fn check_pcm(&mut self, rate: u32, ctx: &egui::Context) {
        let ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        self.pcm_rx = Some(rx);
        self.status = "Checking the software PCM path…".into();
        std::thread::spawn(move || {
            let _ = tx.send(setup::check_pcm(rate));
            ctx.request_repaint();
        });
    }

    fn start_tone(
        &mut self,
        rate: u32,
        request: OutputRequest,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let path = directory.path().join("test-tone.wav");
        setup::tone(&path, rate).map_err(|error| error.to_string())?;
        let sink = AudioTrack::open(&path, request, Duration::ZERO, 1.0, false, ctx)?;
        self.release = Some(sink.release_gate());
        self.tone = Some((sink, directory));
        self.status = "Playing a quiet test tone…".into();
        Ok(())
    }

    pub fn stop_tone(&mut self) {
        self.tone = None;
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.check_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.check_rx = None;
            match result {
                Ok(check) => self.check = Some(check),
                Err(error) => self.status = error,
            }
        }
        if let Some(result) = self.pcm_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pcm_rx = None;
            self.status = match result {
                Ok(hash) => format!("Software PCM check passed · MD5 {hash}"),
                Err(error) => error,
            };
        }
        if let Some((sink, _)) = &mut self.tone {
            if let Some(message) = sink.poll() {
                self.status = message;
            }
            if sink.failed || sink.empty() {
                if !sink.failed {
                    self.status = "Test tone finished.".into();
                }
                self.stop_tone();
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
    }
}

impl Player {
    pub(super) fn audio_setup_dialog(&mut self, ctx: &egui::Context) {
        if !self.setup.visible {
            return;
        }
        let mut open = true;
        let mut apply = false;
        let before = self.setup.selected.clone();
        egui::Window::new("Audio setup")
            .open(&mut open)
            .collapsible(false)
            .vscroll(true)
            .show(ctx, |ui| {
                if let Some(monitor) = &self.audio_devices {
                    let label = monitor.devices.output.iter()
                        .find(|device| Some(&device.id) == self.setup.selected.as_ref())
                        .map_or("Windows default", |device| device.label.as_str());
                    egui::ComboBox::from_label("Output device").selected_text(label).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.setup.selected, None, "Windows default");
                        for device in &monitor.devices.output {
                            ui.selectable_value(&mut self.setup.selected, Some(device.id.clone()), &device.label);
                        }
                    });
                }
                if self.setup.selected != before {
                    self.setup.check = None;
                    self.setup.probe(ctx);
                }
                if self.setup.check_rx.is_none() && self.setup.check.is_none() && self.setup.status.is_empty() {
                    self.setup.probe(ctx);
                }
                let mut rate = None;
                if let Some(check) = &self.setup.check {
                    ui.label(format!("Shared mix format · {}", check.shared_format));
                    if check.exclusive.is_empty() {
                        ui.weak("The driver reported no supported stereo Exclusive PCM formats.");
                    } else {
                        egui::CollapsingHeader::new("Driver-reported Exclusive stereo formats").show(ui, |ui| {
                            for (rate, depths) in &check.exclusive {
                                let depths = depths.iter().map(u16::to_string).collect::<Vec<_>>().join(", ");
                                ui.label(format!("{rate} Hz · {depths} bit"));
                            }
                        });
                    }
                    if let Some(warning) = &check.probe_warning {
                        ui.weak(warning);
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.selectable_value(&mut self.setup.mode, OutputMode::Shared, "WASAPI Shared");
                        ui.add_enabled_ui(check.tone_rate(OutputMode::Exclusive).is_some(), |ui| {
                            ui.selectable_value(&mut self.setup.mode, OutputMode::Exclusive, "WASAPI Exclusive");
                        });
                    });
                    if ui.button(match check.recommended() {
                        OutputMode::Shared => "Use recommended Shared mode",
                        OutputMode::Exclusive => "Use recommended Exclusive mode",
                    }).clicked() {
                        self.setup.mode = check.recommended();
                    }
                    rate = check.tone_rate(self.setup.mode);
                } else if self.setup.check_rx.is_some() {
                    ui.weak("Checking Windows output formats…");
                } else if ui.button("Check formats again").clicked() {
                    self.setup.probe(ctx);
                }
                if let Some(rate) = rate {
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(self.playing.is_none() && self.setup.tone.is_none(), egui::Button::new("Play quiet test tone"))
                            .on_disabled_hover_text("Stop playback before testing the output.").clicked()
                        {
                            let request = OutputRequest {
                                device: self.setup.selected.clone(),
                                options: OutputOptions { mode: self.setup.mode, ..OutputOptions::default() },
                                after: self.setup.release.clone().or_else(|| self.audio_release.clone()),
                                dsp: Default::default(),
                            };
                            if let Err(error) = self.setup.start_tone(rate, request, ctx) {
                                self.setup.status = error;
                            } else {
                                self.audio_release.clone_from(&self.setup.release);
                            }
                        }
                        if ui.add_enabled(self.setup.pcm_rx.is_none(), egui::Button::new("Verify software PCM")).clicked() {
                            self.setup.check_pcm(rate, ctx);
                        }
                    });
                    ui.weak("A hardware bit-perfect check requires a digital loopback capture and a PCM comparison.");
                }
                if !self.setup.status.is_empty() {
                    ui.label(&self.setup.status);
                }
                ui.horizontal(|ui| {
                    apply = ui.add_enabled(rate.is_some(), egui::Button::new("Apply output settings")).clicked();
                    if ui.button("Close").clicked() {
                        self.setup.visible = false;
                    }
                });
            });
        self.setup.visible &= open && !apply;
        if self.setup.visible {
            return;
        }
        self.setup.stop_tone();
        self.library.audio_setup_done = true;
        if apply {
            self.library.audio_device.clone_from(&self.setup.selected);
            self.library.audio_output.mode = self.setup.mode;
        }
        self.save();
        if apply {
            self.reopen_audio(ctx, true);
        }
    }
}
