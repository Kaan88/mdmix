//! The two kinds of write the G6 takes, and the batch type the builders return.

use std::fmt;

/// HID reports are 64 bytes. On the wire a report id `0x00` goes in front.
pub const REPORT_LEN: usize = 64;

/// One HID write, without the report id.
pub type HidReport = [u8; REPORT_LEN];

/// Every HID write starts with this byte.
pub const PREFIX: u8 = 0x5a;

/// Builds `5a <mode:2> <intermediate:2> <feature> <value:4>` followed by zeros: the layout of almost every
/// report (Python's `UsbHidDataFragment` with an empty additional payload).
pub fn report(mode: [u8; 2], intermediate: [u8; 2], feature: u8, value: [u8; 4]) -> HidReport {
    report_with_payload(mode, intermediate, feature, value, &[])
}

/// As [`report`], with the bytes after the value given (the lighting colour report puts green and red there).
pub fn report_with_payload(
    mode: [u8; 2],
    intermediate: [u8; 2],
    feature: u8,
    value: [u8; 4],
    payload: &[u8],
) -> HidReport {
    let mut r = [0u8; REPORT_LEN];
    r[0] = PREFIX;
    r[1..3].copy_from_slice(&mode);
    r[3..5].copy_from_slice(&intermediate);
    r[5] = feature;
    r[6..10].copy_from_slice(&value);
    r[10..10 + payload.len()].copy_from_slice(payload);
    r
}

/// A USB Audio Control class request to the interface (host to device).
///
/// `w_value`, `w_index` and `w_length` hold the bytes in the order the Python builders store them, so they can be
/// compared with `tests/vectors/writes.json` as they are. A transport reads them as little-endian numbers.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UsbRequest {
    pub bm_request_type: u8,
    pub b_request: u8,
    pub w_value: [u8; 2],
    pub w_index: [u8; 2],
    pub w_length: [u8; 2],
    pub data: Vec<u8>,
}

/// Direction host-to-device, type class, recipient interface.
pub const BM_REQUEST_TYPE: u8 = 0x21;
/// SET_CUR.
pub const B_REQUEST: u8 = 0x01;

impl UsbRequest {
    /// A SET_CUR request; `wLength` follows from the data, as Python asserts.
    pub fn set_cur(w_value: [u8; 2], w_index: [u8; 2], data: &[u8]) -> Self {
        let len = u16::try_from(data.len()).expect("a control request carries at most 64 KiB");
        UsbRequest {
            bm_request_type: BM_REQUEST_TYPE,
            b_request: B_REQUEST,
            w_value,
            w_index,
            w_length: len.to_le_bytes(),
            data: data.to_vec(),
        }
    }

    pub fn w_value_le(&self) -> u16 {
        u16::from_le_bytes(self.w_value)
    }

    pub fn w_index_le(&self) -> u16 {
        u16::from_le_bytes(self.w_index)
    }

    pub fn w_length_le(&self) -> u16 {
        u16::from_le_bytes(self.w_length)
    }
}

/// What one Python API call sends: either HID reports or USB requests, never both. Transports get one batch at a
/// time, as Python's `send_hid_data_to_device` / `send_audio_data_to_device` do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Batch {
    Hid(Vec<HidReport>),
    Usb(Vec<UsbRequest>),
}

/// A builder refused its input. The text is Python's `ValueError` message.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProtoError(pub String);

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProtoError {}

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T, ProtoError> {
    Err(ProtoError(msg.into()))
}

/// Lower-case hex, as Python's `bytes.hex()`.
pub fn hex(bytes: &[u8]) -> String {
    use fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Parses an even-length hex string; `None` for anything else.
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// A response as the transcripts store it (trailing zeros removed) padded back to a full report.
pub fn pad_report(bytes: &[u8]) -> Option<HidReport> {
    if bytes.len() > REPORT_LEN {
        return None;
    }
    let mut r = [0u8; REPORT_LEN];
    r[..bytes.len()].copy_from_slice(bytes);
    Some(r)
}
