//! Everything a run needs from outside the process besides stdout: the device, the status read, the toggle state
//! file, the clock and the output level meter. Tests swap in their own.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::CliError;
use crate::hid_transport::HidTransport;
use crate::transport::{DryRunTransport, Transport};
use g6_device::{Hid, STATUS_NOT_FOUND};
use g6_osaudio::Mixer;
use g6_proto::status::DeviceStatus;

/// Python's `TOGGLE_STATE_TEMP_FILE_NAME`.
pub const TOGGLE_STATE_FILE_NAME: &str = "g6-cli-toggle-state";

pub trait Env {
    /// Python's `detect_device`: the G6, or in a dry run without one, a placeholder.
    fn open_device(
        &mut self,
        dry_run: bool,
        debug: bool,
        out: &mut dyn Write,
    ) -> Result<Box<dyn Transport>, CliError>;
    /// Python's `read_status()` (HID reads only; it ignores `--dry-run` there too).
    fn read_status(&mut self) -> Result<DeviceStatus, CliError>;
    /// Python's `read_mixer_safely()`: volumes from the OS audio stack, `None` when unavailable. Called for
    /// `--status` only.
    fn read_mixer(&mut self) -> Option<Mixer> {
        None
    }
    /// Where `--toggle-output` remembers the last state.
    fn toggle_state_path(&self) -> PathBuf;
    /// `G6Api.reload_alsa_and_pipewire`.
    fn reload_audio_services(
        &mut self,
        sudo: bool,
        dry_run: bool,
        out: &mut dyn Write,
    ) -> Result<(), CliError>;
    /// Seconds on a monotonic clock.
    fn monotonic(&mut self) -> f64;
    fn sleep(&mut self, seconds: f64);
    /// The G6's output peak meter (0..1), where the OS has one.
    fn output_meter(&mut self) -> Option<Box<dyn FnMut() -> f64>>;
}

/// `tempfile.gettempdir()`: Python looks at TMPDIR, TEMP and TMP first, in that order.
pub fn python_temp_dir() -> PathBuf {
    ["TMPDIR", "TEMP", "TMP"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .find(|p| p.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

/// The real system: the G6 over HID. The mixer readback and the output meter come from `g6-osaudio`.
pub struct SystemEnv {
    start: Instant,
}

impl Default for SystemEnv {
    fn default() -> Self {
        SystemEnv {
            start: Instant::now(),
        }
    }
}

impl Env for SystemEnv {
    fn open_device(
        &mut self,
        dry_run: bool,
        debug: bool,
        out: &mut dyn Write,
    ) -> Result<Box<dyn Transport>, CliError> {
        if !dry_run {
            return Ok(Box::new(HidTransport::detect(debug, out)?));
        }
        // Python looks for the G6 in a dry run too; the port doesn't, so a dry run never opens or enumerates
        // anything (on Windows Python's USB lookup fails without libusb and ends up here as well)
        writeln!(
            out,
            "This is a dry run. G6 not detected (the Rust port doesn't look for it in a dry run); using a placeholder device."
        )?;
        Ok(Box::new(DryRunTransport::new(debug)))
    }

    fn read_status(&mut self) -> Result<DeviceStatus, CliError> {
        let mut hid = Hid::new().map_err(|e| CliError::Msg(e.0))?;
        match hid.find_path().map_err(|e| CliError::Msg(e.0))? {
            Some(path) => Ok(hid.read_status(&path)),
            None => Err(CliError::msg(STATUS_NOT_FOUND)),
        }
    }

    fn read_mixer(&mut self) -> Option<Mixer> {
        g6_osaudio::read_mixer()
    }

    fn toggle_state_path(&self) -> PathBuf {
        python_temp_dir().join(TOGGLE_STATE_FILE_NAME)
    }

    fn reload_audio_services(
        &mut self,
        _sudo: bool,
        dry_run: bool,
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        if dry_run {
            writeln!(
                out,
                "This is a dry run. ALSA and PipeWire will not be reloaded."
            )?;
            return Ok(());
        }
        Err(CliError::msg(
            "reloading ALSA and PipeWire isn't built yet in the Rust port (phase 4)",
        ))
    }

    fn monotonic(&mut self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn sleep(&mut self, seconds: f64) {
        if seconds > 0.0 && seconds.is_finite() {
            std::thread::sleep(Duration::from_secs_f64(seconds));
        }
    }

    fn output_meter(&mut self) -> Option<Box<dyn FnMut() -> f64>> {
        let meter = g6_osaudio::open_output_meter()?;
        Some(Box::new(move || meter.peak()))
    }
}
