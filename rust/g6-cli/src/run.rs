//! Python's `main`: parse, save/load a profile, print the status, apply the changes, run a lighting effect.

use std::io::{self, Write};
use std::path::PathBuf;

use crate::args::{self, Cli, PROG, ParseError};
use crate::env::Env;
use crate::profile::{load_profile_argv, save_profile, settings_len};
use crate::pyval::decode_text_file;
use crate::status_fmt::format_status;
use crate::transport::Transport;
use crate::{CliError, Io, os_error_text};
use g6_proto::apply::Host;
use g6_proto::effects::{
    Effect, FRAME_INTERVAL_S, LevelFollower, MUSIC_FLOOR_DB, effect_frame, level_to_brightness,
};
use g6_proto::eq_presets::find_eq_preset;
use g6_proto::lighting::{lighting_enable_set_rgb, lighting_frame};
use g6_proto::packet::{Batch, ProtoError};
use g6_proto::pyfmt::py_repr;
use g6_proto::types::Output;

/// Python's `parse_cli_args`, printing where argparse prints.
pub fn parse_cli_args(argv: &[String], io: &mut Io) -> Result<Cli, CliError> {
    let mut cli = match args::parse(argv) {
        Ok(cli) => cli,
        Err(ParseError::Usage(m)) => return usage_error(&m, io),
        Err(ParseError::Clap(e)) => {
            // only help and version get this far: argparse has had its say on everything else
            let text = e.render().to_string();
            if e.use_stderr() {
                write!(io.err, "{text}")?;
            } else {
                write!(io.out, "{text}")?;
            }
            return Err(CliError::Exit(e.exit_code()));
        }
    };
    if cli.reload_audio_services_no_sudo {
        cli.reload_audio_services = true;
    }
    let refuse = |message: &str, io: &mut Io| -> Result<Cli, CliError> {
        writeln!(io.out, "{message}")?;
        write!(io.out, "{}", args::render_help())?;
        Err(CliError::msg(message))
    };
    if !cli.has_action() {
        return refuse("No meaningful argument has been specified!", io);
    } else if cli.toggle_output && cli.set_output.is_some() {
        return refuse(
            "Only one of the following CLI arguments may be specified: '--toggle-output', '--set-output'!",
            io,
        );
    }
    if let Some(name) = &cli.eq_preset {
        match find_eq_preset(name) {
            Ok(id) => cli.eq_preset = Some(id.to_string()),
            Err(e) => return usage_error(&e.to_string(), io),
        }
    }
    Ok(cli)
}

/// argparse's `parser.error()`: usage and message on stderr, exit code 2.
fn usage_error<T>(message: &str, io: &mut Io) -> Result<T, CliError> {
    writeln!(io.err, "usage: {PROG}\n{PROG}: error: {message}")?;
    Err(CliError::Exit(2))
}

/// Python's `_without_option`: `argv` without `option VALUE` and `option=VALUE` (the exact spelling only).
pub fn without_option(argv: &[String], option: &str) -> Vec<String> {
    let prefix = format!("{option}=");
    let mut out = Vec::new();
    let mut skip = false;
    for a in argv {
        if skip {
            skip = false;
        } else if a == option {
            skip = true;
        } else if !a.starts_with(&prefix) {
            out.push(a.clone());
        }
    }
    out
}

impl Cli {
    /// `--profile-save` and nothing else to do.
    pub fn only_profile_save(&self) -> bool {
        let mut c = self.clone();
        c.profile_save = None;
        !c.has_action()
    }

    /// `--status` and nothing else to do.
    pub fn only_status_requested(&self) -> bool {
        let mut c = self.clone();
        c.status = false;
        !c.has_action()
    }
}

/// Python's `main(argv)`.
pub fn main_with(argv: &[String], env: &mut dyn Env, io: &mut Io) -> Result<(), CliError> {
    let mut cli = parse_cli_args(argv, io)?;

    // save first, so "--profile-save old.json --profile-load new.json" keeps a backup; a loaded profile becomes
    // ordinary arguments ahead of the ones given on the command line
    if let Some(path) = &cli.profile_save {
        let profile = save_profile(path, &env.read_status()?)?;
        writeln!(
            io.out,
            "Saved {} settings to {path}",
            settings_len(&profile)
        )?;
    }
    if let Some(path) = &cli.profile_load {
        let rest = without_option(&without_option(argv, "--profile-load"), "--profile-save");
        let mut full = load_profile_argv(path)?;
        full.extend(rest);
        cli = parse_cli_args(&full, io)?;
    } else if cli.profile_save.is_some() && cli.only_profile_save() {
        return Ok(());
    }

    if cli.status {
        let status = env.read_status()?;
        writeln!(io.out, "{}", format_status(&status, cli.json))?;
        if cli.only_status_requested() {
            return Ok(());
        }
    }

    let mut device = env.open_device(cli.dry_run, cli.debug, io.out)?;
    device_set_audio_effects(device.as_mut(), &cli, env, io)?;

    if let Some(name) = &cli.lighting_effect {
        let effect = Effect::from_name(name)?;
        let default_rgb = if effect == Effect::Music {
            None
        } else {
            Some([255, 0, 0])
        };
        let rgb = cli
            .lighting_rgb
            .as_ref()
            .map(|v| [v[0], v[1], v[2]])
            .or(default_rgb);
        run_lighting_effect(
            device.as_mut(),
            env,
            effect,
            rgb,
            cli.lighting_effect_period,
            cli.lighting_effect_duration,
            cli.dry_run,
            !cli.debug,
            io,
        )?;
    }
    Ok(())
}

/// What applying needs from outside the request: the toggle state file and the current output.
struct CliHost<'a> {
    out: &'a mut dyn Write,
    env: &'a mut dyn Env,
    dry_run: bool,
}

impl CliHost<'_> {
    fn toggle(&mut self) -> Result<Output, String> {
        let path: PathBuf = self.env.toggle_state_path();
        let shown = path.to_string_lossy().into_owned();
        let io_err = |e: io::Error| os_error_text(&e, &shown);
        let next = if path.exists() {
            let current = decode_text_file(&std::fs::read(&path).map_err(io_err)?)?;
            let next = if current == "Headphones" {
                Output::Speakers
            } else {
                Output::Headphones
            };
            writeln!(self.out, "Toggle from {current} -> {}", next.name())
                .map_err(|e| e.to_string())?;
            next
        } else {
            writeln!(self.out, "Toggle to Speakers").map_err(|e| e.to_string())?;
            Output::Speakers
        };
        // recorded even in a dry run, as Python does
        std::fs::write(&path, next.name()).map_err(io_err)?;
        Ok(next)
    }
}

impl Host for CliHost<'_> {
    fn next_toggle_state(&mut self) -> Result<Output, ProtoError> {
        self.toggle().map_err(ProtoError)
    }

    fn current_output(&mut self) -> Output {
        if !self.dry_run
            && let Ok(status) = self.env.read_status()
            && let Some(output) = status.output.as_deref().and_then(Output::from_name)
        {
            return output;
        }
        Output::Headphones
    }
}

fn send(device: &mut dyn Transport, batch: &Batch, out: &mut dyn Write) -> Result<(), CliError> {
    match batch {
        Batch::Hid(reports) => device.send_hid(reports, out),
        Batch::Usb(requests) => device.send_usb(requests, out),
    }
}

/// Python's `device_set_audio_effects`: every requested change in order, then release and reload as asked, even
/// after an error. An error while releasing or reloading wins over an earlier one, as in Python's `finally`.
pub fn device_set_audio_effects(
    device: &mut dyn Transport,
    cli: &Cli,
    env: &mut dyn Env,
    io: &mut Io,
) -> Result<(), CliError> {
    if !device.is_hid_available(io.out)? {
        return Err(CliError::msg(
            "The G6 HID interface should be available at this point!",
        ));
    }
    // claim_and_release is implemented from reading the Python source only: it cuts system audio on Linux, so
    // it is never run, not even as a dry run, to compare (see doc/rust-port.md)
    if cli.claim_and_release {
        device.claim_audio(io.out)?;
        if !device.is_audio_available() {
            return Err(CliError::msg(
                "The G6 AudioControl interface should be available at this point! Claiming the interface failed.",
            ));
        }
    }

    let applied = (|| -> Result<(), CliError> {
        for action in cli.to_request().actions() {
            let batches = {
                let mut host = CliHost {
                    out: &mut *io.out,
                    env: &mut *env,
                    dry_run: cli.dry_run,
                };
                action.batches(&mut host)?
            };
            for batch in &batches {
                send(device, batch, io.out)?;
            }
        }
        Ok(())
    })();

    if cli.claim_and_release && device.is_audio_available() {
        device.release_audio(io.out)?;
    }
    if cli.reload_audio_services {
        env.reload_audio_services(!cli.reload_audio_services_no_sudo, cli.dry_run, io.out)?;
    }
    applied
}

/// Python's `run_lighting_effect`: stream frames until the duration runs out, then set the static colour (red
/// without one). A dry run sends one period's frames without waiting. Returns the number of frames sent.
#[allow(clippy::too_many_arguments)] // the same parameters as the Python function
pub fn run_lighting_effect(
    device: &mut dyn Transport,
    env: &mut dyn Env,
    effect: Effect,
    rgb: Option<[i64; 3]>,
    period: Option<f64>,
    duration: Option<f64>,
    dry_run: bool,
    quiet: bool,
    io: &mut Io,
) -> Result<u64, CliError> {
    // `period or default`: 0.0 means the default too, NaN doesn't
    let period = period
        .filter(|p| *p != 0.0)
        .unwrap_or(effect.default_period());
    if period <= 0.0 {
        return Err(CliError::msg(format!(
            "--lighting-effect-period must be > 0, got {}",
            py_repr(period)
        )));
    }
    let mut meter: Option<Box<dyn FnMut() -> f64>> = None;
    let mut follower = None;
    if effect == Effect::Music {
        meter = if dry_run {
            Some(Box::new(|| 0.0))
        } else {
            env.output_meter()
        };
        if meter.is_none() {
            return Err(CliError::msg(
                "--lighting-effect music needs the G6 output level, which is only readable on Windows",
            ));
        }
        follower = Some(LevelFollower::default());
    }
    let duration = if dry_run {
        // Python's min(duration, period) keeps a NaN duration, and the loop then never ends (a Python bug,
        // reported in doc/rust-port.md; copied, not fixed)
        Some(duration.map_or(period, |d| if period < d { period } else { d }))
    } else {
        if duration.is_none() {
            writeln!(
                io.err,
                "Lighting effect {} running, Ctrl+C to stop.",
                effect.name()
            )?;
        }
        duration
    };
    // the colour has been sent already by device_set_audio_effects, so it is in range here
    let frame_rgb = match rgb {
        Some([r, g, b]) => Some((byte(r)?, byte(g)?, byte(b)?)),
        None => None,
    };

    let mut frames: u64 = 0;
    let start = env.monotonic();
    let streamed = (|| -> Result<(), CliError> {
        device.hold_hid_open()?;
        let result = (|| -> Result<(), CliError> {
            loop {
                let t = if dry_run {
                    frames as f64 * FRAME_INTERVAL_S
                } else {
                    env.monotonic() - start
                };
                if duration.is_some_and(|d| t >= d) {
                    return Ok(());
                }
                let level = match (&mut follower, &mut meter) {
                    (Some(f), Some(m)) => {
                        Some(f.update(level_to_brightness(m(), MUSIC_FLOOR_DB), FRAME_INTERVAL_S))
                    }
                    _ => None,
                };
                let (r, g, b, brightness) = effect_frame(effect, t, period, frame_rgb, level)?;
                let reports = lighting_frame(r.into(), g.into(), b.into(), brightness.into())?;
                if quiet {
                    device.send_hid(&reports, &mut io::sink())?;
                } else {
                    device.send_hid(&reports, io.out)?;
                }
                frames += 1;
                if !dry_run {
                    let wait = start + frames as f64 * FRAME_INTERVAL_S - env.monotonic();
                    env.sleep(wait.max(0.0));
                }
            }
        })();
        device.end_hid_hold();
        result
    })();

    let [r, g, b] = rgb.unwrap_or([255, 0, 0]);
    device.send_hid(&lighting_enable_set_rgb(r, g, b)?, io.out)?;
    streamed?;
    Ok(frames)
}

fn byte(v: i64) -> Result<u8, CliError> {
    u8::try_from(v).map_err(|_| CliError::msg(format!("colour value {v} is out of range 0..255")))
}
