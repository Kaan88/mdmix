//! How writes reach the G6: the dry run (which prints what Python's `G6Device` prints in a dry run), a recorder for
//! tests, and a fake that answers status queries from a transcript. The real device is
//! [`crate::hid_transport::HidTransport`].

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::CliError;
use g6_proto::packet::{HidReport, UsbRequest, hex};

/// The G6 as the CLI uses it. `out` takes the progress lines; a lighting effect passes a sink for its frames.
pub trait Transport {
    /// Whether the HID interface can be written to.
    fn is_hid_available(&mut self, out: &mut dyn Write) -> Result<bool, CliError>;
    /// Whether the USB Audio Control interface is claimed and usable.
    fn is_audio_available(&mut self) -> bool;
    fn claim_audio(&mut self, out: &mut dyn Write) -> Result<(), CliError>;
    fn release_audio(&mut self, out: &mut dyn Write) -> Result<(), CliError>;
    /// Write the 64-byte reports, in order (the report id 0 goes in front on the wire).
    fn send_hid(&mut self, reports: &[HidReport], out: &mut dyn Write) -> Result<(), CliError>;
    /// Send the SET_CUR requests, in order.
    fn send_usb(&mut self, requests: &[UsbRequest], out: &mut dyn Write) -> Result<(), CliError>;
    /// Keep the HID handle open across sends (a lighting effect streams 30 frames a second).
    fn hold_hid_open(&mut self) -> Result<(), CliError> {
        Ok(())
    }
    fn end_hid_hold(&mut self) {}
    /// One status query and its answer, padded to 64 bytes; `None` if there is none.
    fn query(&mut self, _payload: &[u8]) -> Option<HidReport> {
        None
    }
}

/// The device path Python shows for a dry run without a G6.
pub const DRY_RUN_DEVICE_PATH: &str = "dry-run";

/// Python's `G6Device` in a dry run: sends nothing, prints what it would do.
pub struct DryRunTransport {
    debug: bool,
    path: String,
    hid_checked_at: Option<Instant>,
    audio_claimed: bool,
}

impl DryRunTransport {
    pub fn new(debug: bool) -> Self {
        DryRunTransport {
            debug,
            path: DRY_RUN_DEVICE_PATH.into(),
            hid_checked_at: None,
            audio_claimed: false,
        }
    }
}

/// Python checks HID availability again only after this long.
const AVAILABILITY_CHECK_INTERVAL: Duration = Duration::from_millis(100);

impl Transport for DryRunTransport {
    fn is_hid_available(&mut self, out: &mut dyn Write) -> Result<bool, CliError> {
        let due = self
            .hid_checked_at
            .is_none_or(|t| t.elapsed() > AVAILABILITY_CHECK_INTERVAL);
        if due {
            self.hid_checked_at = Some(Instant::now());
            let p = &self.path;
            writeln!(
                out,
                "About to check availability of HID interface at path: {p} ..."
            )?;
            writeln!(
                out,
                "About to check availability of HID interface at path: {p}: This is a dry run. No availability \
                 check is performed!"
            )?;
        }
        Ok(true)
    }

    fn is_audio_available(&mut self) -> bool {
        self.audio_claimed
    }

    // From reading g6_core.AudioInterface.claim_interface; Python's dry run of it is never run here, as the
    // repo's rules ask (it cuts system audio when not a dry run).
    fn claim_audio(&mut self, out: &mut dyn Write) -> Result<(), CliError> {
        writeln!(
            out,
            "About to check if kernel driver is attached to G6 device ..."
        )?;
        writeln!(
            out,
            "About to check if kernel driver is attached to G6 device: This is a dry run - kernel driver is not \
             checked!"
        )?;
        writeln!(out, "About to claim the audio interface from G6 device ...")?;
        writeln!(
            out,
            "This is a dry run. No interface is claimed from the G6 device!"
        )?;
        self.audio_claimed = true;
        writeln!(
            out,
            "About to claim the audio interface from G6 device: ok."
        )?;
        writeln!(out, "About to configure USB device ...")?;
        writeln!(
            out,
            "This is a dry run. No configuration is applied to the G6 device!"
        )?;
        writeln!(out, "About to configure USB device: ok.")?;
        Ok(())
    }

    fn release_audio(&mut self, out: &mut dyn Write) -> Result<(), CliError> {
        writeln!(
            out,
            "This is a dry run. No interface is released and no kernel driver is attached to the G6 device!"
        )?;
        self.audio_claimed = false;
        Ok(())
    }

    fn send_hid(&mut self, reports: &[HidReport], out: &mut dyn Write) -> Result<(), CliError> {
        // a dry run never holds the handle, so every send "opens" it
        let p = &self.path;
        writeln!(out, "Opening the device '{p}' ...")?;
        writeln!(
            out,
            "Opening the device '{p}': This is a dry run. Device has not been opened."
        )?;
        writeln!(out, "Sending HID data to G6 ...")?;
        for r in reports {
            if self.debug {
                writeln!(out, "00{}", hex(r))?;
            }
            writeln!(out, "This is a dry run. No data has been sent to the G6!")?;
        }
        writeln!(out, "Sending HID data to G6: ok.")?;
        Ok(())
    }

    fn send_usb(&mut self, requests: &[UsbRequest], out: &mut dyn Write) -> Result<(), CliError> {
        writeln!(out, "Sending audio data to G6 ...")?;
        for r in requests {
            if self.debug {
                writeln!(out, "bmRequestType: {:02x}", r.bm_request_type)?;
                writeln!(out, "bRequest: {:02x}", r.b_request)?;
                writeln!(out, "wValue: {}", hex(&r.w_value))?;
                writeln!(out, "wIndex: {}", hex(&r.w_index))?;
                writeln!(out, "wLength: {}", hex(&r.w_length))?;
                writeln!(out, "data_fragment: {}", hex(&r.data))?;
            }
            writeln!(out, "This is a dry run. No data has been sent to the G6!")?;
        }
        writeln!(out, "Sending audio data to G6: ok.")?;
        Ok(())
    }
}

/// What a [`RecordingTransport`] saw.
#[derive(Default, Debug)]
pub struct Recorded {
    pub hid: Vec<HidReport>,
    pub usb: Vec<UsbRequest>,
}

/// Records every write and prints nothing, like the recorder behind `tests/vectors/writes.json`. Both interfaces
/// count as available.
#[derive(Clone, Default)]
pub struct RecordingTransport {
    pub log: Rc<RefCell<Recorded>>,
}

impl Transport for RecordingTransport {
    fn is_hid_available(&mut self, _out: &mut dyn Write) -> Result<bool, CliError> {
        Ok(true)
    }

    fn is_audio_available(&mut self) -> bool {
        true
    }

    fn claim_audio(&mut self, _out: &mut dyn Write) -> Result<(), CliError> {
        Ok(())
    }

    fn release_audio(&mut self, _out: &mut dyn Write) -> Result<(), CliError> {
        Ok(())
    }

    fn send_hid(&mut self, reports: &[HidReport], _out: &mut dyn Write) -> Result<(), CliError> {
        self.log.borrow_mut().hid.extend_from_slice(reports);
        Ok(())
    }

    fn send_usb(&mut self, requests: &[UsbRequest], _out: &mut dyn Write) -> Result<(), CliError> {
        self.log.borrow_mut().usb.extend_from_slice(requests);
        Ok(())
    }
}

/// Answers status queries from a transcript of (query, answer) pairs: each query takes the next answer recorded
/// for it, `None` meaning no answer. Writes are recorded.
pub struct FakeTransport {
    answers: HashMap<Vec<u8>, VecDeque<Option<HidReport>>>,
    pub writes: RecordingTransport,
}

impl FakeTransport {
    pub fn new(exchanges: impl IntoIterator<Item = (Vec<u8>, Option<HidReport>)>) -> Self {
        let mut answers: HashMap<Vec<u8>, VecDeque<Option<HidReport>>> = HashMap::new();
        for (q, a) in exchanges {
            answers.entry(q).or_default().push_back(a);
        }
        FakeTransport {
            answers,
            writes: RecordingTransport::default(),
        }
    }
}

impl Transport for FakeTransport {
    fn is_hid_available(&mut self, out: &mut dyn Write) -> Result<bool, CliError> {
        self.writes.is_hid_available(out)
    }

    fn is_audio_available(&mut self) -> bool {
        self.writes.is_audio_available()
    }

    fn claim_audio(&mut self, out: &mut dyn Write) -> Result<(), CliError> {
        self.writes.claim_audio(out)
    }

    fn release_audio(&mut self, out: &mut dyn Write) -> Result<(), CliError> {
        self.writes.release_audio(out)
    }

    fn send_hid(&mut self, reports: &[HidReport], out: &mut dyn Write) -> Result<(), CliError> {
        self.writes.send_hid(reports, out)
    }

    fn send_usb(&mut self, requests: &[UsbRequest], out: &mut dyn Write) -> Result<(), CliError> {
        self.writes.send_usb(requests, out)
    }

    fn query(&mut self, payload: &[u8]) -> Option<HidReport> {
        self.answers.get_mut(payload)?.pop_front().flatten()
    }
}
