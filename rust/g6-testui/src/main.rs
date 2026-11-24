//! `g6-testui`: a THROWAWAY test harness for trying the Rust port on a real G6. It is not the real GUI: that one is
//! built on mdshared-ui (`doc/gui-plan.md`). Looks don't matter here; delete this crate once `g6-gui` exists.
//!
//! Every control writes to the device as soon as it's clicked (sliders on release). Each write goes through the
//! same path as the `g6` CLI: a `g6_proto::apply::Request` with one setting, `Request::actions`, `Action::batches`,
//! and `HidTransport::send_hid`. Nothing here builds command bytes. After each write the full status is read back.
//! Device I/O runs on a worker thread, one job at a time; the controls are disabled while it runs.

#![forbid(unsafe_code)]

use std::sync::mpsc::{Receiver, channel};

use eframe::egui;
use g6_cli::hid_transport::{HidTransport, USB_NOT_BUILT};
use g6_cli::status_fmt::format_status;
use g6_cli::transport::Transport;
use g6_proto::apply::{Host, Request, SbxSetting};
use g6_proto::eq_presets;
use g6_proto::packet::{Batch, ProtoError};
use g6_proto::status::DeviceStatus;
use g6_proto::types::{DecoderMode, Output, PlaybackFilter, SbxEffect};

/// Log lines kept.
const LOG_MAX: usize = 2000;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("g6-testui (throwaway test harness)")
            .with_inner_size([1100.0, 760.0]),
        ..Default::default()
    };
    eframe::run_native(
        "g6-testui",
        options,
        Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))),
    )
}

/// What a finished job hands back to the UI thread.
struct Done {
    transport: Option<HidTransport>,
    status: Option<DeviceStatus>,
    log: Vec<String>,
    error: Option<String>,
}

/// The output the EQ preset curve is for, and the output reported by the G6 for anything else.
struct UiHost {
    output: Output,
}

impl Host for UiHost {
    fn next_toggle_state(&mut self) -> Result<Output, ProtoError> {
        Err(ProtoError("the test UI has no toggle".into()))
    }

    fn current_output(&mut self) -> Output {
        self.output
    }
}

/// Sends one request the way `g6_cli::run::device_set_audio_effects` does (HID only), printing into `out`.
fn apply(
    t: &mut HidTransport,
    req: &Request,
    output: Output,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let e = |e: g6_cli::CliError| e.to_string();
    if !t.is_hid_available(out).map_err(e)? {
        return Err("The G6 HID interface should be available at this point!".into());
    }
    let mut host = UiHost { output };
    for action in req.actions() {
        for batch in action.batches(&mut host).map_err(|e| e.0)? {
            match batch {
                Batch::Hid(reports) => t.send_hid(&reports, out).map_err(e)?,
                Batch::Usb(_) => return Err(USB_NOT_BUILT.into()),
            }
        }
    }
    Ok(())
}

fn lines(buf: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(buf)
        .lines()
        .map(str::to_owned)
        .collect()
}

struct App {
    ctx: egui::Context,
    transport: Option<HidTransport>,
    busy: Option<Receiver<Done>>,
    connected: bool,
    error: Option<String>,
    status: Option<DeviceStatus>,
    log: Vec<String>,
    // control state that isn't a plain switch
    sbx_levels: [i64; 5],
    rgb: [u8; 3],
    eq_preset: usize,
    eq_preset_output: Output,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        let mut app = App {
            ctx: ctx.clone(),
            transport: None,
            busy: None,
            connected: false,
            error: None,
            status: None,
            log: Vec::new(),
            sbx_levels: [0; 5],
            rgb: [255, 0, 0],
            eq_preset: 0,
            eq_preset_output: Output::Headphones,
        };
        app.connect();
        app
    }

    /// Runs `job` on a worker thread with the transport (`None` if there is none yet).
    fn spawn(
        &mut self,
        title: String,
        job: impl FnOnce(Option<HidTransport>, &mut Vec<u8>) -> Done + Send + 'static,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.log.push(format!("=== {title}"));
        let transport = self.transport.take();
        let (tx, rx) = channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let mut done = job(transport, &mut out);
            let mut log = lines(&out);
            log.append(&mut done.log);
            done.log = log;
            let _ = tx.send(done);
            ctx.request_repaint();
        });
        self.busy = Some(rx);
    }

    /// Finds the G6 and reads its status.
    fn connect(&mut self) {
        self.spawn(
            "connect and read status".into(),
            |_, out| match HidTransport::detect(true, out) {
                Ok(t) => {
                    let status = t.read_status();
                    Done {
                        transport: Some(t),
                        status: Some(status),
                        log: vec!["status read".into()],
                        error: None,
                    }
                }
                Err(e) => Done {
                    transport: None,
                    status: None,
                    log: Vec::new(),
                    error: Some(e.to_string()),
                },
            },
        );
    }

    fn refresh(&mut self) {
        self.spawn("read status".into(), |t, _| {
            let status = t.as_ref().map(HidTransport::read_status);
            Done {
                transport: t,
                status,
                log: vec!["status read".into()],
                error: None,
            }
        });
    }

    /// Writes one setting, then reads the status back.
    fn send(&mut self, title: String, req: Request) {
        let output = self
            .status
            .as_ref()
            .and_then(|s| s.output.as_deref())
            .and_then(Output::from_name)
            .unwrap_or(Output::Headphones);
        self.spawn(title, move |t, out| {
            let Some(mut t) = t else {
                return Done {
                    transport: None,
                    status: None,
                    log: Vec::new(),
                    error: Some("no device".into()),
                };
            };
            let result = apply(&mut t, &req, output, out);
            let status = t.read_status();
            Done {
                transport: Some(t),
                status: Some(status),
                log: vec![match &result {
                    Ok(()) => "written; status read back".into(),
                    Err(e) => format!("FAILED: {e}"),
                }],
                error: result.err(),
            }
        });
    }

    fn poll(&mut self) {
        let Some(rx) = &self.busy else { return };
        let Ok(done) = rx.try_recv() else { return };
        self.busy = None;
        self.log.extend(done.log);
        if self.log.len() > LOG_MAX {
            let cut = self.log.len() - LOG_MAX;
            self.log.drain(..cut);
        }
        self.connected = done.transport.is_some();
        self.transport = done.transport;
        self.error = done.error;
        if let Some(st) = done.status {
            if st.firmware.is_none() && st.output.is_none() {
                self.error = Some("the G6 didn't answer the status queries (unplugged?)".into());
            }
            if let Some(sbx) = &st.sbx {
                for (level, s) in self.sbx_levels.iter_mut().zip(sbx) {
                    if let Some(l) = s.level {
                        *level = l;
                    }
                }
            }
            if let Some(o) = st.output.as_deref().and_then(Output::from_name) {
                self.eq_preset_output = o;
            }
            self.status = Some(st);
        }
    }

    fn status_panel(&self, ui: &mut egui::Ui) {
        let Some(st) = &self.status else {
            ui.label("No status read yet.");
            return;
        };
        egui::Grid::new("status")
            .striped(true)
            .num_columns(2)
            .show(ui, |ui| {
                for line in format_status(st, false).lines() {
                    ui.label(line.get(..13).unwrap_or(line).trim_end());
                    ui.monospace(line.get(14..).unwrap_or(""));
                    ui.end_row();
                }
            });
        ui.collapsing("Raw JSON (g6 --status --json)", |ui| {
            let mut json = format_status(st, true);
            ui.add(
                egui::TextEdit::multiline(&mut json)
                    .code_editor()
                    .desired_width(f32::INFINITY),
            );
        });
    }

    /// A checkbox showing a read-back switch; `None` shows unchecked with "(?)". Returns the new value on a click.
    fn switch(ui: &mut egui::Ui, label: &str, value: Option<bool>) -> Option<bool> {
        let mut v = value.unwrap_or(false);
        let text = if value.is_some() {
            label.to_string()
        } else {
            format!("{label} (?)")
        };
        ui.checkbox(&mut v, text).changed().then_some(v)
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        let st = self.status.clone().unwrap_or_default();
        let mut todo: Option<(String, Request)> = None;
        let mut want = |title: String, req: Request| todo = Some((title, req));

        ui.heading("Output");
        ui.horizontal(|ui| {
            for o in [Output::Speakers, Output::Headphones] {
                let current = st.output.as_deref() == Some(o.name());
                if ui.selectable_label(current, o.name()).clicked() {
                    want(
                        format!("--set-output {}", o.name()),
                        Request {
                            set_output: Some(o),
                            ..Request::default()
                        },
                    );
                }
            }
        });

        ui.separator();
        ui.heading("SBX");
        ui.horizontal(|ui| {
            if let Some(on) = Self::switch(ui, "SBX", st.sbx_enabled) {
                want(
                    format!("--sbx {}", enabled(on)),
                    Request {
                        sbx: Some(on),
                        ..Request::default()
                    },
                );
            }
            if let Some(on) = Self::switch(ui, "Scout Mode", st.scout_mode) {
                want(
                    format!("--scout-mode {}", enabled(on)),
                    Request {
                        scout_mode: Some(on),
                        ..Request::default()
                    },
                );
            }
        });
        egui::Grid::new("sbx").num_columns(2).show(ui, |ui| {
            for (i, effect) in SbxEffect::ALL.into_iter().enumerate() {
                let state = st.sbx_effect(effect).unwrap_or_default();
                let name = effect_label(effect);
                let flag = effect_flag(effect);
                if let Some(on) = Self::switch(ui, name, state.enabled) {
                    let mut req = Request::default();
                    req.sbx_effects[i] = SbxSetting {
                        enabled: Some(on),
                        level: None,
                    };
                    want(format!("--{flag} {}", enabled(on)), req);
                }
                let r = ui.add(egui::Slider::new(&mut self.sbx_levels[i], 0..=100).suffix(" %"));
                // drag or click: send on release; arrow keys: send at once
                if r.drag_stopped()
                    || r.clicked()
                    || (r.changed() && !r.is_pointer_button_down_on())
                {
                    let level = self.sbx_levels[i];
                    let mut req = Request::default();
                    req.sbx_effects[i] = SbxSetting {
                        enabled: None,
                        level: Some(level),
                    };
                    want(format!("--{flag}-value {level}"), req);
                }
                ui.end_row();
            }
        });

        ui.separator();
        ui.heading("Equalizer");
        if let Some(on) = Self::switch(ui, "EQ", st.eq_enabled) {
            want(
                format!("--eq {}", enabled(on)),
                Request {
                    eq: Some(on),
                    ..Request::default()
                },
            );
        }
        let ids: Vec<&str> = eq_presets::preset_ids().collect();
        ui.horizontal(|ui| {
            ui.label("Preset curve for");
            for o in [Output::Speakers, Output::Headphones] {
                ui.selectable_value(&mut self.eq_preset_output, o, o.name());
            }
        });
        ui.horizontal(|ui| {
            let before = self.eq_preset;
            egui::ComboBox::from_id_salt("eq_preset")
                .selected_text(ids.get(self.eq_preset).copied().unwrap_or("?"))
                .height(400.0)
                .show_ui(ui, |ui| {
                    for (i, id) in ids.iter().enumerate() {
                        ui.selectable_value(&mut self.eq_preset, i, *id);
                    }
                });
            let again = ui.button("Apply preset").clicked();
            if (self.eq_preset != before || again)
                && let Some(id) = ids.get(self.eq_preset)
            {
                let o = self.eq_preset_output;
                want(
                    format!("--eq-preset {id} --eq-preset-output {}", o.name()),
                    Request {
                        eq_preset: Some(((*id).to_string(), Some(o))),
                        ..Request::default()
                    },
                );
            }
        });

        ui.separator();
        ui.heading("Playback");
        if let Some(on) = Self::switch(ui, "Direct mode", st.direct_mode) {
            want(
                format!("--playback-direct-mode {}", enabled(on)),
                Request {
                    direct_mode: Some(on),
                    ..Request::default()
                },
            );
        }
        ui.horizontal(|ui| {
            ui.label("DAC filter");
            let current = st.dac_filter.clone().unwrap_or_else(|| "?".into());
            let mut chosen = None;
            egui::ComboBox::from_id_salt("dac_filter")
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    for f in PlaybackFilter::ALL {
                        if ui
                            .selectable_label(current == f.name(), f.label())
                            .clicked()
                        {
                            chosen = Some(f);
                        }
                    }
                });
            if let Some(f) = chosen {
                want(
                    format!("--playback-filter {}", f.name()),
                    Request {
                        playback_filter: Some(f),
                        ..Request::default()
                    },
                );
            }
        });
        ui.horizontal(|ui| {
            ui.label("Decoder mode");
            let current = st.decoder_mode.clone().unwrap_or_else(|| "?".into());
            let mut chosen = None;
            egui::ComboBox::from_id_salt("decoder")
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    for m in [DecoderMode::Normal, DecoderMode::Full, DecoderMode::Night] {
                        if ui.selectable_label(current == m.name(), m.name()).clicked() {
                            chosen = Some(m);
                        }
                    }
                });
            if let Some(m) = chosen {
                want(
                    format!("--decoder-mode {}", m.name()),
                    Request {
                        decoder_mode: Some(m),
                        ..Request::default()
                    },
                );
            }
        });

        ui.separator();
        ui.heading("Logo lighting");
        ui.horizontal(|ui| {
            let on = match st.rgb_enabled {
                Some(true) => "on",
                Some(false) => "off",
                None => "?",
            };
            ui.label(format!("Now: {on}"));
            ui.color_edit_button_srgb(&mut self.rgb);
            let [r, g, b] = self.rgb;
            if ui.button("On with this colour").clicked() {
                want(
                    format!("--lighting-rgb {r} {g} {b}"),
                    Request {
                        lighting_rgb: Some([r.into(), g.into(), b.into()]),
                        ..Request::default()
                    },
                );
            }
            if ui.button("Off").clicked() {
                want(
                    "--lighting-disable".into(),
                    Request {
                        lighting_disable: true,
                        ..Request::default()
                    },
                );
            }
        });

        if let Some((title, req)) = todo {
            self.send(title, req);
        }
    }
}

fn enabled(on: bool) -> &'static str {
    if on { "Enabled" } else { "Disabled" }
}

fn effect_label(e: SbxEffect) -> &'static str {
    match e {
        SbxEffect::Surround => "Surround",
        SbxEffect::Crystalizer => "Crystalizer",
        SbxEffect::Bass => "Bass",
        SbxEffect::SmartVolume => "Smart Volume",
        SbxEffect::DialogPlus => "Dialog+",
    }
}

/// The CLI flag for the effect's switch; its level flag adds `-value`. Only shown in the log.
fn effect_flag(e: SbxEffect) -> &'static str {
    match e {
        SbxEffect::Surround => "sbx-surround",
        SbxEffect::Crystalizer => "sbx-crystalizer",
        SbxEffect::Bass => "sbx-bass",
        SbxEffect::SmartVolume => "sbx-smart-volume",
        SbxEffect::DialogPlus => "sbx-dialog-plus",
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        let busy = self.busy.is_some();

        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("THROWAWAY TEST HARNESS: every control writes to the G6 at once.");
                if busy {
                    ui.spinner();
                    ui.label("talking to the G6 ...");
                }
            });
            ui.horizontal(|ui| {
                if self.connected {
                    ui.label("G6 connected.");
                    if ui
                        .add_enabled(!busy, egui::Button::new("Refresh"))
                        .clicked()
                    {
                        self.refresh();
                    }
                } else {
                    ui.colored_label(egui::Color32::RED, "G6 not connected.");
                    if ui.add_enabled(!busy, egui::Button::new("Retry")).clicked() {
                        self.connect();
                    }
                }
                if let Some(e) = &self.error {
                    ui.colored_label(egui::Color32::RED, e);
                }
            });
        });

        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(200.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Log (reports sent, as `g6 --debug` prints them)");
                    if ui.button("Clear").clicked() {
                        self.log.clear();
                    }
                });
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        for line in &self.log {
                            ui.monospace(line);
                        }
                    });
            });

        egui::Panel::left("status")
            .resizable(true)
            .default_size(470.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.status_panel(ui));
            });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_enabled_ui(self.connected && !busy, |ui| self.controls(ui));
            });
        });
    }
}
