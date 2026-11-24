//! Everything a run needs from outside the process besides stdout: the device, the status read, the toggle state
//! file, the clock and the output level meter. Tests swap in their own.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::CliError;
use crate::transport::{DryRunTransport, Transport};
use g6_proto::status::DeviceStatus;

/// Python's `TOGGLE_STATE_TEMP_FILE_NAME`.
pub const TOGGLE_STATE_FILE_NAME: &str = "g6-cli-toggle-state";

/// What G0 says wherever Python would open the G6.
pub const NOT_BUILT: &str =
    "device access isn't built yet in the Rust port (step G1); only --dry-run works";

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

/// The real system, as far as G0 goes: dry runs only.
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
            return Err(CliError::msg(NOT_BUILT));
        }
        // Python prints this when a dry run finds no G6; G0 never looks for one
        writeln!(
            out,
            "This is a dry run. G6 not detected (device access isn't built yet); using a placeholder device."
        )?;
        Ok(Box::new(DryRunTransport::new(debug)))
    }

    fn read_status(&mut self) -> Result<DeviceStatus, CliError> {
        Err(CliError::msg(NOT_BUILT))
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
        // unreachable in G0 (a run that isn't dry fails at open_device), but say so rather than pretend
        Err(CliError::msg(
            "reloading ALSA and PipeWire isn't built yet in the Rust port (step G1)",
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
        // the Windows meter comes with G1
        None
    }
}
