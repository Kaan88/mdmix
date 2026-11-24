//! The device worker: a thread that reads the whole status through `g6-device` (HID) and hands it to the window.
//!
//! A status read is up to 46 queries, and each one waits up to 500 ms for an answer, so it never runs on the UI
//! thread. In step G2 the worker only reads. It has no way to write: its one command is `Read`.

use std::sync::mpsc::{self, Sender};
use std::thread;

use g6_device::{DeviceError, Hid, Lookup, STATUS_NOT_FOUND};
use g6_proto::status::DeviceStatus;

/// What the UI asks of the worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// Look for the G6 and read the whole status.
    Read,
}

/// What a read gave: the status, or why there isn't one.
pub type ReadResult = Result<DeviceStatus, String>;

/// The UI end of the worker. The function given to `start` is called with every result, on the worker's thread; the
/// window uses it to wake its loop.
pub struct Worker {
    tx: Sender<Cmd>,
}

impl Worker {
    pub fn start(mut deliver: impl FnMut(ReadResult) + Send + 'static) -> Worker {
        let (tx, rx) = mpsc::channel::<Cmd>();
        let spawned = thread::Builder::new()
            .name("g6-device".into())
            .spawn(move || {
                // `hidapi` is set up on this thread and stays here.
                let mut hid = None;
                while let Ok(Cmd::Read) = rx.recv() {
                    // Reads that piled up while one ran are one read.
                    while rx.try_recv().is_ok() {}
                    deliver(read(&mut hid));
                }
            });
        if let Err(e) = spawned {
            eprintln!("g6-gui: the device worker did not start: {e}");
        }
        Worker { tx }
    }

    /// Asks for a read. False when the worker is gone.
    pub fn ask(&self, cmd: Cmd) -> bool {
        self.tx.send(cmd).is_ok()
    }
}

fn read(hid: &mut Option<Hid>) -> ReadResult {
    if hid.is_none() {
        *hid = Some(Hid::new().map_err(describe)?);
    }
    let Some(h) = hid.as_mut() else {
        return Err("HID is not available.".into());
    };
    match h.lookup().map_err(describe)? {
        Lookup::Found(path) => status_of(h.read_status(&path)),
        Lookup::NoInterface => Err(STATUS_NOT_FOUND.into()),
        Lookup::NoDevice => Err("No G6 could be found. Is it connected to this computer?".into()),
    }
}

/// A read where nothing answered is not a status: the G6 was found but said nothing.
pub fn status_of(s: DeviceStatus) -> ReadResult {
    if s.firmware.is_none() && s.output.is_none() && s.sbx.is_none() {
        Err("The G6 was found but did not answer.".into())
    } else {
        Ok(s)
    }
}

fn describe(e: DeviceError) -> String {
    format!("The G6 could not be read: {e}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample;

    #[test]
    fn a_read_with_no_answers_is_not_a_status() {
        assert!(status_of(DeviceStatus::default()).is_err());
        assert!(status_of(sample::live_status()).is_ok());
        let only_firmware = DeviceStatus {
            firmware: Some("1".into()),
            ..DeviceStatus::default()
        };
        assert!(status_of(only_firmware).is_ok());
    }
}
