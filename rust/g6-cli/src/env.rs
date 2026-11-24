//! Everything a run needs from outside the process besides stdout: the device, the status read, the toggle state
//! file, the clock and the output level meter. Tests swap in their own.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::CliError;
use crate::hid_transport::HidTransport;
use crate::transport::{DryRunTransport, Transport};
use g6_device::{
    Hid, Lookup, PRODUCT_ID, STATUS_NOT_FOUND, VENDOR_ID, detect_error, py_bytes_repr,
};
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
    /// From here on Ctrl+C is reported by [`Env::interrupted`] instead of ending the process (Python's
    /// `KeyboardInterrupt` in a lighting effect). Called before a lighting effect starts streaming.
    fn watch_interrupt(&mut self) -> Result<(), CliError> {
        Ok(())
    }
    /// Whether Ctrl+C has been pressed since [`Env::watch_interrupt`].
    fn interrupted(&mut self) -> bool {
        false
    }
}

/// The text of Python's `IOError` for a G6 that pyusb doesn't find (its lookup comes before the HID one).
pub fn usb_not_found_text() -> String {
    format!(
        "No SoundBlaster X G6 device could be found by pyusb library having vendor_id='{VENDOR_ID:#x}' and \
         product_id='{PRODUCT_ID:#x}'. Is the device connected to your system? Are you allowed to access the \
         device (missing udev-rules in linux)?"
    )
}

/// The device lookup of Python's `detect_device` in a dry run, from what the HID enumeration found: the G6's
/// interface 4 (then the dry run goes by its real path), or the placeholder device with the reason.
/// Python looks for the USB Audio Control interface first (pyusb); the port has no USB lookup, so a G6 that
/// HID finds counts as found, and a G6 that HID doesn't find is reported as pyusb not finding it.
pub fn open_dry_run_device(
    lookup: Result<Lookup, CliError>,
    debug: bool,
    out: &mut dyn Write,
) -> Result<Box<dyn Transport>, CliError> {
    let reason = match lookup {
        Ok(Lookup::Found(path)) => {
            let shown = py_bytes_repr(path.as_bytes());
            writeln!(out, "Device with HID interface detected at path: {shown}")?;
            return Ok(Box::new(DryRunTransport::with_path(&shown, debug)));
        }
        Ok(Lookup::NoInterface) => detect_error(true),
        Ok(Lookup::NoDevice) => usb_not_found_text(),
        Err(e) => e.to_string(),
    };
    writeln!(
        out,
        "This is a dry run. G6 not detected ({reason}); using a placeholder device."
    )?;
    Ok(Box::new(DryRunTransport::new(debug)))
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

/// The real system: the G6 over HID. The mixer readback and the output meter come with `g6-osaudio`.
pub struct SystemEnv {
    start: Instant,
    interrupted: Arc<AtomicBool>,
    watching: bool,
}

impl Default for SystemEnv {
    fn default() -> Self {
        SystemEnv {
            start: Instant::now(),
            interrupted: Arc::new(AtomicBool::new(false)),
            watching: false,
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
        // Python looks for the G6 in a dry run too (reading only); a dry run works without one
        let lookup = Hid::new()
            .and_then(|mut hid| hid.lookup())
            .map_err(|e| CliError::Msg(e.0));
        open_dry_run_device(lookup, debug, out)
    }

    fn read_status(&mut self) -> Result<DeviceStatus, CliError> {
        let mut hid = Hid::new().map_err(|e| CliError::Msg(e.0))?;
        match hid.find_path().map_err(|e| CliError::Msg(e.0))? {
            Some(path) => Ok(hid.read_status(&path)),
            None => Err(CliError::msg(STATUS_NOT_FOUND)),
        }
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

    /// Sleeps in short steps, so that Ctrl+C is noticed at once.
    fn sleep(&mut self, seconds: f64) {
        if !(seconds > 0.0 && seconds.is_finite()) {
            return;
        }
        let end = Instant::now() + Duration::from_secs_f64(seconds);
        while !self.interrupted.load(Ordering::SeqCst) {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            std::thread::sleep(left.min(Duration::from_millis(20)));
        }
    }

    fn watch_interrupt(&mut self) -> Result<(), CliError> {
        if !self.watching {
            let flag = Arc::clone(&self.interrupted);
            ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst))
                .map_err(|e| CliError::msg(format!("cannot watch for Ctrl+C: {e}")))?;
            self.watching = true;
        }
        Ok(())
    }

    fn interrupted(&mut self) -> bool {
        self.interrupted.load(Ordering::SeqCst)
    }

    fn output_meter(&mut self) -> Option<Box<dyn FnMut() -> f64>> {
        // the Windows meter comes with g6-osaudio
        None
    }
}
