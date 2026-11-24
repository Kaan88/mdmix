//! The G6 over HID, through `hidapi` (Python: `g6_core.HidInterface` and `g6_status.hid_query`).
//!
//! The device is found the way Python finds it: enumerate vendor 0x041e / product 0x3256 and take the first entry on
//! USB interface 4 (data sent to interface 3 is ignored by the G6). Reports go out as the report id `0x00` followed
//! by the 64-byte payload. Nothing here builds command bytes: callers pass what `g6-proto` built.
//!
//! USB Audio Control (volumes, mutes, speaker layout) isn't here yet: phase 4 of `doc/rust-port.md`.

#![forbid(unsafe_code)]

use std::ffi::{CStr, CString};
use std::fmt;

use g6_proto::packet::{HidReport, REPORT_LEN};
use g6_proto::status::{DeviceStatus, collect_status};
use hidapi::{HidApi, HidDevice};

pub const VENDOR_ID: u16 = 0x041e;
pub const PRODUCT_ID: u16 = 0x3256;
/// The USB interface SoundBlaster Connect talks to.
pub const HID_INTERFACE: i32 = 4;
/// Python's `HID_READ_TIMEOUT_MS`: how long a status query waits for its answer.
pub const HID_READ_TIMEOUT_MS: i32 = 500;

/// Python's `read_status` / `hid_query` error when interface 4 isn't there.
pub const STATUS_NOT_FOUND: &str = "SoundBlaster X G6 HID interface not found. Is the device connected (and udev rule set on Linux)?";

/// Python's `_detect_device_hid` errors.
pub fn detect_error(g6_seen: bool) -> String {
    let ids = format!("vendor_id='{VENDOR_ID:#x}' and product_id='{PRODUCT_ID:#x}'");
    if g6_seen {
        format!(
            "The SoundBlaster X G6 device could be found having {ids}. But the required HID interface does not seem \
             to be available. Something is wrong here, and thus, the program execution is terminated!"
        )
    } else {
        format!(
            "No SoundBlaster X G6 device could be found by hidapi library having {ids}. Is the device connected to \
             your system? Are you allowed to access the device (missing udev-rules in linux)?"
        )
    }
}

/// A HID or lookup failure, as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceError(pub String);

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DeviceError {}

impl From<hidapi::HidError> for DeviceError {
    fn from(e: hidapi::HidError) -> Self {
        DeviceError(e.to_string())
    }
}

/// What enumerating the G6 found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// The path of interface 4.
    Found(CString),
    /// The G6 is there, but not its interface 4.
    NoInterface,
    /// No G6.
    NoDevice,
}

/// The hidapi context. Its device list only ever holds G6 entries.
pub struct Hid {
    api: HidApi,
}

impl Hid {
    pub fn new() -> Result<Hid, DeviceError> {
        let mut api = HidApi::new()?;
        api.reset_devices()?;
        Ok(Hid { api })
    }

    /// Enumerates the G6 again (Python's `hid.enumerate(G6_VENDOR_ID, G6_PRODUCT_ID)`) and takes the first entry
    /// on interface 4.
    pub fn lookup(&mut self) -> Result<Lookup, DeviceError> {
        self.api.reset_devices()?;
        self.api.add_devices(VENDOR_ID, PRODUCT_ID)?;
        let mut seen = false;
        for info in self.api.device_list() {
            if info.vendor_id() == VENDOR_ID && info.product_id() == PRODUCT_ID {
                seen = true;
                if info.interface_number() == HID_INTERFACE {
                    return Ok(Lookup::Found(info.path().to_owned()));
                }
            }
        }
        Ok(if seen {
            Lookup::NoInterface
        } else {
            Lookup::NoDevice
        })
    }

    /// Python's `find_hid_path`.
    pub fn find_path(&mut self) -> Result<Option<CString>, DeviceError> {
        Ok(match self.lookup()? {
            Lookup::Found(p) => Some(p),
            _ => None,
        })
    }

    pub fn open(&self, path: &CStr) -> Result<Handle, DeviceError> {
        Ok(Handle(self.api.open_path(path)?))
    }

    /// Python's `hid_query`: open, write the query (blocking mode), wait up to [`HID_READ_TIMEOUT_MS`] for one
    /// answer, close. `None` when nothing came back.
    pub fn query(&self, path: &CStr, payload: &[u8]) -> Result<Option<HidReport>, DeviceError> {
        if payload.len() > REPORT_LEN {
            return Err(DeviceError(format!(
                "HID query payload too long: {} > {REPORT_LEN}",
                payload.len()
            )));
        }
        let mut report = [0u8; REPORT_LEN];
        report[..payload.len()].copy_from_slice(payload);
        let h = self.open(path)?;
        h.0.set_blocking_mode(true)?;
        h.write_report(&report)?;
        let mut resp = [0u8; REPORT_LEN];
        let n = h.0.read_timeout(&mut resp, HID_READ_TIMEOUT_MS)?;
        Ok((n > 0).then_some(resp))
    }

    /// Python's `read_status` without the mixer: every status query, a failed one counting as no answer.
    pub fn read_status(&self, path: &CStr) -> DeviceStatus {
        collect_status(|p| self.query(path, p).ok().flatten())
    }
}

/// An open HID handle; closed when dropped.
pub struct Handle(HidDevice);

impl Handle {
    pub fn set_nonblocking(&self) -> Result<(), DeviceError> {
        Ok(self.0.set_blocking_mode(false)?)
    }

    /// Writes `00` followed by the 64-byte report.
    pub fn write_report(&self, report: &HidReport) -> Result<(), DeviceError> {
        let mut buf = [0u8; REPORT_LEN + 1];
        buf[1..].copy_from_slice(report);
        self.0.write(&buf)?;
        Ok(())
    }

    /// One `read(64)`: in non-blocking mode it returns at once, empty when nothing is waiting.
    pub fn read(&self) -> Result<Vec<u8>, DeviceError> {
        let mut buf = [0u8; REPORT_LEN];
        let n = self.0.read(&mut buf)?;
        Ok(buf[..n].to_vec())
    }

    /// Manufacturer, product and serial number, as Python prints them after opening.
    pub fn strings(&self) -> [Option<String>; 3] {
        [
            self.0.get_manufacturer_string().ok().flatten(),
            self.0.get_product_string().ok().flatten(),
            self.0.get_serial_number_string().ok().flatten(),
        ]
    }
}

/// Python's `repr()` of a `bytes` value; its messages print hidapi paths that way (`b'\\\\?\\hid#...'`).
pub fn py_bytes_repr(b: &[u8]) -> String {
    let quote = if b.contains(&b'\'') && !b.contains(&b'"') {
        b'"'
    } else {
        b'\''
    };
    let mut s = String::from("b");
    s.push(char::from(quote));
    for &c in b {
        match c {
            b'\\' => s.push_str("\\\\"),
            b'\t' => s.push_str("\\t"),
            b'\n' => s.push_str("\\n"),
            b'\r' => s.push_str("\\r"),
            c if c == quote => {
                s.push('\\');
                s.push(char::from(c));
            }
            0x20..=0x7e => s.push(char::from(c)),
            c => s.push_str(&format!("\\x{c:02x}")),
        }
    }
    s.push(char::from(quote));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_repr_matches_python() {
        assert_eq!(
            py_bytes_repr(br"\\?\hid#vid_041e&pid_3256&mi_04#7&1"),
            r"b'\\\\?\\hid#vid_041e&pid_3256&mi_04#7&1'"
        );
        // from Python 3.13: repr(b"a'b"), repr(b'a\'"\x01')
        assert_eq!(py_bytes_repr(b"a'b"), r#"b"a'b""#);
        assert_eq!(py_bytes_repr(b"a'\"\x01"), r#"b'a\'"\x01'"#);
        assert_eq!(py_bytes_repr(b"1-2.1:1.4"), "b'1-2.1:1.4'");
    }

    #[test]
    fn detect_errors_match_python() {
        assert!(detect_error(false).starts_with(
            "No SoundBlaster X G6 device could be found by hidapi library having vendor_id='0x41e' and \
             product_id='0x3256'."
        ));
        assert!(
            detect_error(true)
                .contains("But the required HID interface does not seem to be available.")
        );
    }
}
