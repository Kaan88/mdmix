//! The device worker: a thread that owns the HID handle, sends writes and reads the whole status back.
//!
//! A status read is up to 46 queries, and each one waits up to 500 ms for an answer, so it never runs on the UI
//! thread. Commands that pile up while one runs are merged ([`Queue`]): reads are one read, and a newer write for a
//! setting replaces the one still waiting. Every turn of the loop sends the waiting writes in order, then reads the
//! whole status (and the mixer from Windows, `g6_osaudio::read_mixer`, Core Audio reads only: no Set* call), and
//! hands back one [`ReadResult`]. The mixer read stays on this thread because its COM apartment does.
//!
//! A write goes out as the reports `Action::batches` builds, the ones the CLI sends for the same flag, written the way
//! the CLI's HID transport writes them (open, non-blocking, each report then one read of its answer, close). A batch
//! for USB Audio Control is refused: the Rust port has no USB Audio Control writes, and the window never asks for one.

use std::sync::mpsc::{self, Sender};
use std::thread;

use g6_device::{DeviceError, Hid, Lookup, STATUS_NOT_FOUND};
use g6_proto::apply::{Action, Host};
use g6_proto::packet::{Batch, HidReport, ProtoError};
use g6_proto::status::DeviceStatus;
use g6_proto::types::Output;

use crate::intent::{Setting, Val};
use crate::model::MixerView;

/// One write: the setting, its value and the CLI action that writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Write {
    pub setting: Setting,
    pub val: Val,
    pub action: Action,
}

/// What the UI asks of the worker.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    /// Look for the G6 and read the whole status. `user` says a person asked for it.
    Read { user: bool },
    /// Send this, then read the status back.
    Write(Write),
}

/// How one write went: sent, or why not.
#[derive(Clone, Debug, PartialEq)]
pub struct WriteOutcome {
    pub setting: Setting,
    pub val: Val,
    pub result: Result<(), String>,
}

/// What a turn of the worker gave: the writes it sent, then the status read after them (or why there isn't one), and
/// the mixer when Windows gave one.
#[derive(Debug)]
pub struct ReadResult {
    /// A person asked for this read (or for one that was merged into it), so the result is announced.
    pub user: bool,
    pub status: Result<DeviceStatus, String>,
    pub mixer: Option<MixerView>,
    pub writes: Vec<WriteOutcome>,
}

/// The commands of one turn, merged.
#[derive(Debug, Default, PartialEq)]
pub struct Queue {
    pub writes: Vec<Write>,
    pub user: bool,
}

impl Queue {
    /// A newer write for a setting replaces the waiting one and goes to the end, so the writes keep the order the user
    /// made them in (SBX and Scout Mode switch each other off: the last one asked for wins).
    pub fn add(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Read { user } => self.user |= user,
            Cmd::Write(w) => {
                self.writes.retain(|x| x.setting != w.setting);
                self.writes.push(w);
            }
        }
    }
}

/// The G6 as the worker uses it; the real one is HID, the tests' a fake that records.
pub trait Device {
    /// Writes the reports of `w`, in order.
    fn send(&mut self, w: &Write, reports: &[HidReport]) -> Result<(), String>;
    fn read(&mut self) -> Result<DeviceStatus, String>;
    fn mixer(&mut self) -> Option<MixerView>;
}

/// The window has no toggle and names the output of every preset, so neither question is asked.
struct NoHost;

impl Host for NoHost {
    fn next_toggle_state(&mut self) -> Result<Output, ProtoError> {
        Err(ProtoError("The window has no output toggle.".into()))
    }
    fn current_output(&mut self) -> Output {
        Output::Headphones
    }
}

/// The reports of a write, as the CLI builds them. Only HID: there is no USB Audio Control write in the Rust port.
pub fn reports_of(action: &Action) -> Result<Vec<HidReport>, String> {
    let mut out = Vec::new();
    for b in action.batches(&mut NoHost).map_err(|e| e.0)? {
        match b {
            Batch::Hid(r) => out.extend(r),
            Batch::Usb(_) => {
                return Err(
                    "USB Audio Control isn't built in the Rust port; only HID settings can be written."
                        .into(),
                );
            }
        }
    }
    Ok(out)
}

/// One turn: the writes in order, then the status read.
pub fn serve(dev: &mut dyn Device, q: &Queue) -> ReadResult {
    let writes = q
        .writes
        .iter()
        .map(|w| WriteOutcome {
            setting: w.setting,
            val: w.val,
            result: reports_of(&w.action).and_then(|r| dev.send(w, &r)),
        })
        .collect();
    ReadResult {
        user: q.user,
        status: dev.read(),
        mixer: dev.mixer(),
        writes,
    }
}

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
                let mut dev = RealDevice { hid: None };
                while let Ok(first) = rx.recv() {
                    let mut q = Queue::default();
                    q.add(first);
                    while let Ok(more) = rx.try_recv() {
                        q.add(more);
                    }
                    deliver(serve(&mut dev, &q));
                }
            });
        if let Err(e) = spawned {
            eprintln!("g6-gui: the device worker did not start: {e}");
        }
        Worker { tx }
    }

    /// Hands a command to the worker. False when the worker is gone.
    pub fn ask(&self, cmd: Cmd) -> bool {
        self.tx.send(cmd).is_ok()
    }
}

/// The G6 through `g6-device`.
struct RealDevice {
    hid: Option<Hid>,
}

impl RealDevice {
    fn hid(&mut self) -> Result<&mut Hid, String> {
        if self.hid.is_none() {
            self.hid = Some(Hid::new().map_err(describe)?);
        }
        self.hid
            .as_mut()
            .ok_or_else(|| "HID is not available.".into())
    }
}

impl Device for RealDevice {
    fn send(&mut self, _w: &Write, reports: &[HidReport]) -> Result<(), String> {
        let h = self.hid()?;
        let path = match h.lookup().map_err(describe)? {
            Lookup::Found(path) => path,
            Lookup::NoInterface => return Err(STATUS_NOT_FOUND.into()),
            Lookup::NoDevice => return Err(NO_DEVICE.into()),
        };
        let fail = |e: DeviceError| format!("The G6 did not take the write: {e}");
        let d = h.open(&path).map_err(fail)?;
        d.set_nonblocking().map_err(fail)?;
        for r in reports {
            d.write_report(r).map_err(fail)?;
            // As the CLI does: one read after each report, so an answer doesn't wait in the queue.
            d.read().map_err(fail)?;
        }
        Ok(())
    }

    fn read(&mut self) -> Result<DeviceStatus, String> {
        let h = self.hid()?;
        match h.lookup().map_err(describe)? {
            Lookup::Found(path) => status_of(h.read_status(&path)),
            Lookup::NoInterface => Err(STATUS_NOT_FOUND.into()),
            Lookup::NoDevice => Err(NO_DEVICE.into()),
        }
    }

    fn mixer(&mut self) -> Option<MixerView> {
        g6_osaudio::read_mixer().as_ref().map(MixerView::from_os)
    }
}

const NO_DEVICE: &str = "No G6 could be found. Is it connected to this computer?";

/// A read where nothing answered is not a status: the G6 was found but said nothing.
pub fn status_of(s: DeviceStatus) -> Result<DeviceStatus, String> {
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

    fn w(setting: Setting, val: Val) -> Cmd {
        Cmd::Write(Write {
            setting,
            val,
            action: Action::Eq(true),
        })
    }

    #[test]
    fn a_newer_write_replaces_the_waiting_one_and_reads_merge() {
        let mut q = Queue::default();
        q.add(Cmd::Read { user: false });
        q.add(w(Setting::Sbx, Val::On(true)));
        q.add(w(Setting::ScoutMode, Val::On(true)));
        q.add(w(Setting::Sbx, Val::On(true)));
        q.add(Cmd::Read { user: true });
        let order: Vec<Setting> = q.writes.iter().map(|w| w.setting).collect();
        assert_eq!(order, [Setting::ScoutMode, Setting::Sbx]);
        assert!(q.user);
    }

    #[test]
    fn a_usb_write_is_refused() {
        use g6_proto::apply::Volume;
        use g6_proto::types::Channels;
        let usb = Action::PlaybackVolume(Volume {
            percent: 50,
            channels: Channels::default(),
        });
        assert!(reports_of(&usb).is_err());
        assert_eq!(reports_of(&Action::Eq(true)).unwrap().len(), 2);
    }
}
