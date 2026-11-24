//! The real G6 over HID (`g6-device`), printing what Python's `G6Device` prints when it isn't a dry run.
//!
//! USB Audio Control isn't ported yet (phase 4 of `doc/rust-port.md`), so USB writes and the claim fail with
//! [`USB_NOT_BUILT`]. Nothing here builds command bytes: it sends the reports `g6-proto` built.

use std::ffi::CString;
use std::io::Write;
use std::time::{Duration, Instant};

use crate::CliError;
use crate::transport::Transport;
use g6_device::{DeviceError, Handle, Hid, Lookup, detect_error, py_bytes_repr};
use g6_proto::packet::{HidReport, UsbRequest, hex};
use g6_proto::status::DeviceStatus;

/// What a USB Audio Control write says until phase 4.
pub const USB_NOT_BUILT: &str = "USB Audio Control (volumes, mutes, speaker layout) isn't built yet in the Rust \
                                 port; only HID settings can be written";

/// Python's `UDEV_RULE`.
pub const UDEV_RULE: &str =
    r#"SUBSYSTEM=="usb", ATTRS{idVendor}=="041e", ATTRS{idProduct}=="3256", TAG+="uaccess""#;

/// Python checks HID availability again only after this long.
const AVAILABILITY_CHECK_INTERVAL: Duration = Duration::from_millis(100);

pub struct HidTransport {
    hid: Hid,
    path: CString,
    debug: bool,
    hid_checked_at: Option<Instant>,
    hid_available: bool,
    held: Option<Handle>,
}

fn device_error(e: DeviceError) -> CliError {
    CliError::Msg(e.0)
}

/// Python's `_detect_device_hid`: the path of interface 4, printed as Python prints it.
fn detect_hid(hid: &mut Hid, out: &mut dyn Write) -> Result<CString, CliError> {
    match hid.lookup().map_err(device_error)? {
        Lookup::Found(path) => {
            writeln!(
                out,
                "Device with HID interface detected at path: {}",
                py_bytes_repr(path.as_bytes())
            )?;
            Ok(path)
        }
        Lookup::NoInterface => Err(CliError::msg(detect_error(true))),
        Lookup::NoDevice => Err(CliError::msg(detect_error(false))),
    }
}

impl HidTransport {
    /// Finds the G6 (the HID half of Python's `detect_device`).
    pub fn detect(debug: bool, out: &mut dyn Write) -> Result<Self, CliError> {
        let mut hid = Hid::new().map_err(device_error)?;
        let path = detect_hid(&mut hid, out)?;
        Ok(HidTransport {
            hid,
            path,
            debug,
            hid_checked_at: None,
            hid_available: false,
            held: None,
        })
    }

    /// The path as Python prints it.
    pub fn path_repr(&self) -> String {
        py_bytes_repr(self.path.as_bytes())
    }

    /// Every status query, as `g6 --status` asks them.
    pub fn read_status(&self) -> DeviceStatus {
        self.hid.read_status(&self.path)
    }

    /// Python's `__write_lines`: each report, then its responses. As in Python, only a debug run reads past the
    /// first one.
    fn write_reports(
        h: &Handle,
        reports: &[HidReport],
        debug: bool,
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        writeln!(out, "Sending HID data to G6 ...")?;
        for r in reports {
            if debug {
                writeln!(out, "00{}", hex(r))?;
            }
            h.write_report(r).map_err(device_error)?;
            loop {
                let response = h.read().map_err(device_error)?;
                if response.is_empty() || !debug {
                    break;
                }
                let list: Vec<String> = response.iter().map(u8::to_string).collect();
                writeln!(out, "Response: [{}]", list.join(", "))?;
            }
        }
        writeln!(out, "Sending HID data to G6: ok.")?;
        Ok(())
    }

    fn print_open_failure(p: &str, e: &CliError, out: &mut dyn Write) -> Result<(), CliError> {
        writeln!(
            out,
            "Unable to open a connection to the device by path: {p}"
        )?;
        writeln!(out, "{e}")?;
        writeln!(out, "\nAre the udev rules set and used by the kernel?")?;
        writeln!(
            out,
            "Create a udev-rule file at `/etc/udev/rules.d/50-soundblaster-x-g6.rules` with the following content:"
        )?;
        writeln!(out, "{UDEV_RULE}")?;
        writeln!(
            out,
            "\nIf the file already exists, it might not be used by the kernel. Try to reload the configuration with:"
        )?;
        writeln!(out, "`sudo udevadm trigger`")?;
        Ok(())
    }
}

impl HidTransport {
    fn check_hid_available(
        &mut self,
        out: &mut dyn Write,
        err: &mut dyn Write,
    ) -> Result<bool, CliError> {
        let due = self
            .hid_checked_at
            .is_none_or(|t| t.elapsed() > AVAILABILITY_CHECK_INTERVAL);
        if due {
            self.hid_checked_at = Some(Instant::now());
            let p = self.path_repr();
            writeln!(
                out,
                "About to check availability of HID interface at path: {p} ..."
            )?;
            match detect_hid(&mut self.hid, out) {
                Ok(found) => {
                    self.hid_available = found == self.path;
                    let state = if self.hid_available {
                        "available"
                    } else {
                        "unavailable"
                    };
                    writeln!(
                        out,
                        "About to check availability of HID interface at path: {p}: {state}"
                    )?;
                }
                Err(_) => {
                    writeln!(
                        err,
                        "Failed to check availability of HID interface at path: {p}"
                    )?;
                    self.hid_available = false;
                }
            }
        }
        Ok(self.hid_available)
    }
}

impl Transport for HidTransport {
    fn is_hid_available(&mut self, out: &mut dyn Write) -> Result<bool, CliError> {
        self.check_hid_available(out, &mut std::io::stderr())
    }

    fn is_hid_available_to(
        &mut self,
        out: &mut dyn Write,
        err: &mut dyn Write,
    ) -> Result<bool, CliError> {
        self.check_hid_available(out, err)
    }

    fn is_audio_available(&mut self) -> bool {
        false
    }

    fn claim_audio(&mut self, _out: &mut dyn Write) -> Result<(), CliError> {
        Err(CliError::msg(USB_NOT_BUILT))
    }

    fn release_audio(&mut self, _out: &mut dyn Write) -> Result<(), CliError> {
        Ok(())
    }

    fn send_hid(&mut self, reports: &[HidReport], out: &mut dyn Write) -> Result<(), CliError> {
        if let Some(h) = &self.held {
            return Self::write_reports(h, reports, self.debug, out);
        }
        let p = self.path_repr();
        writeln!(out, "Opening the device '{p}' ...")?;
        let debug = self.debug;
        let sent = (|| -> Result<(), CliError> {
            let h = self.hid.open(&self.path).map_err(device_error)?;
            writeln!(out, "Opening the device '{p}': ok.")?;
            let [manufacturer, product, serial] =
                h.strings().map(|s| s.unwrap_or_else(|| "None".into()));
            writeln!(out, "Manufacturer: '{manufacturer}'")?;
            writeln!(out, "Product: '{product}'")?;
            writeln!(out, "Serial No: '{serial}'")?;
            let written = h
                .set_nonblocking()
                .map_err(device_error)
                .and_then(|()| Self::write_reports(&h, reports, debug, out));
            writeln!(out, "Closing the device ...")?;
            drop(h);
            writeln!(out, "Closing the device: ok.")?;
            written
        })();
        match sent {
            Ok(()) => Ok(()),
            // Python prints this and carries on: its IOError handler doesn't re-raise
            Err(e) => Self::print_open_failure(&p, &e, out),
        }
    }

    fn send_usb(&mut self, _requests: &[UsbRequest], _out: &mut dyn Write) -> Result<(), CliError> {
        Err(CliError::msg(USB_NOT_BUILT))
    }

    fn hold_hid_open(&mut self) -> Result<(), CliError> {
        if self.held.is_none() {
            let h = self.hid.open(&self.path).map_err(device_error)?;
            h.set_nonblocking().map_err(device_error)?;
            self.held = Some(h);
        }
        Ok(())
    }

    fn end_hid_hold(&mut self) {
        self.held = None;
    }

    fn query(&mut self, payload: &[u8]) -> Option<HidReport> {
        self.hid.query(&self.path, payload).ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::UDEV_RULE;

    #[test]
    fn udev_rule_matches_python() {
        let py = include_str!("../../../src/g6_cli/g6_core.py");
        assert!(py.contains(&format!("UDEV_RULE = r'{UDEV_RULE}'")));
    }
}
