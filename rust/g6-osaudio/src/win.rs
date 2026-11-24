//! Windows Core Audio reads. Read only: nothing here calls a `Set*` method.
//!
//! COM rules kept here: every public entry point first joins the calling thread to the MTA ([`ComGuard`]) and drops
//! all its interface pointers before the guard goes; pointers are never handed to another thread (the types that
//! hold them are `!Send`).

#![deny(unsafe_op_in_unsafe_fn)]

use std::ffi::c_void;

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, EDataFlow, IAudioMute, IAudioVolumeLevel, IDeviceTopology, IMMDevice,
    IMMDeviceEnumerator, IPart, MMDeviceEnumerator, PKEY_AudioEndpoint_PhysicalSpeakers, eCapture,
    eConsole, eRender,
};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
use windows::Win32::System::Com::{
    CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize, STGM_READ,
};
use windows::Win32::System::Variant::{VT_LPWSTR, VT_UI4};
use windows::core::{Interface, PWSTR, Result};

use crate::{
    EndpointVolume, Flow, TopologyControl, build_mixer, is_g6_endpoint, round_percent, round2,
};

/// Joins the calling thread to the MTA and leaves it again on drop (only when this guard joined it).
struct ComGuard(bool);

impl ComGuard {
    fn new() -> Self {
        // SAFETY: plain call with no pointers; an error (e.g. the thread already is an STA) just means we don't
        // own an initialisation and must not call CoUninitialize.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        ComGuard(hr.is_ok())
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful CoInitializeEx of `new` on this same thread (the guard isn't Send);
            // every interface pointer was dropped before it (see the field order of `Meter`, locals elsewhere).
            unsafe { CoUninitialize() };
        }
    }
}

/// Copies a COM-allocated wide string and frees it.
fn take_string(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: `p` came from a Core Audio call that hands back a NUL-terminated string allocated with
    // CoTaskMemAlloc; we read it once and free it once.
    unsafe {
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const c_void));
        s
    }
}

/// Reads a string or u32 property and clears the variant.
enum Prop {
    Text(String),
    U32(u32),
    Other,
}

fn property(dev: &IMMDevice, key: &PROPERTYKEY) -> Result<Prop> {
    // SAFETY: COM calls on interface pointers we own; `GetValue` fills `pv`, which we read by its type tag
    // (`vt`) only, and then release with PropVariantClear.
    unsafe {
        let store = dev.OpenPropertyStore(STGM_READ)?;
        let mut pv: PROPVARIANT = store.GetValue(key)?;
        let inner = &pv.Anonymous.Anonymous;
        let prop = if inner.vt == VT_LPWSTR {
            let p = inner.Anonymous.pwszVal;
            if p.is_null() {
                Prop::Text(String::new())
            } else {
                Prop::Text(p.to_string().unwrap_or_default())
            }
        } else if inner.vt == VT_UI4 {
            Prop::U32(inner.Anonymous.ulVal)
        } else {
            Prop::Other
        };
        let _ = PropVariantClear(&mut pv);
        Ok(prop)
    }
}

fn enumerator() -> Result<IMMDeviceEnumerator> {
    // SAFETY: creates the system's device enumerator object; no pointers passed.
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER) }
}

/// `_g6_devices`: active endpoints of one direction whose name looks like a G6, in enumeration order.
fn g6_devices(en: &IMMDeviceEnumerator, flow: EDataFlow) -> Result<Vec<(String, IMMDevice)>> {
    let mut out = Vec::new();
    // SAFETY: COM calls on interface pointers we own; indexes stay below the count the collection reported.
    unsafe {
        let devices = en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        for i in 0..devices.GetCount()? {
            let dev = devices.Item(i)?;
            let name = match property(&dev, &PKEY_Device_FriendlyName)? {
                Prop::Text(t) => t,
                _ => String::new(),
            };
            if is_g6_endpoint(&name) {
                out.push((name, dev));
            }
        }
    }
    Ok(out)
}

fn device_id(dev: &IMMDevice) -> Result<String> {
    // SAFETY: GetId returns a CoTaskMem string that `take_string` frees.
    unsafe { dev.GetId().map(take_string) }
}

/// `_default_id`: the default (console role) endpoint of a direction, if there is one.
fn default_id(en: &IMMDeviceEnumerator, flow: EDataFlow) -> Option<String> {
    // SAFETY: plain COM calls on pointers we own.
    unsafe { en.GetDefaultAudioEndpoint(flow, eConsole).ok() }.and_then(|d| device_id(&d).ok())
}

fn endpoints(en: &IMMDeviceEnumerator) -> Result<Vec<EndpointVolume>> {
    let mut results = Vec::new();
    for (flow, kind) in [(eRender, Flow::Playback), (eCapture, Flow::Recording)] {
        let default = default_id(en, flow);
        for (name, dev) in g6_devices(en, flow)? {
            // SAFETY: Activate gives us our own reference to the endpoint's volume interface; the getters write
            // to the locals we pass.
            let (db, scalar, muted) = unsafe {
                let vol: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None)?;
                (
                    vol.GetMasterVolumeLevel()?,
                    vol.GetMasterVolumeLevelScalar()?,
                    vol.GetMute()?,
                )
            };
            results.push(EndpointVolume {
                name,
                flow: kind,
                level_db: round2(f64::from(db)),
                level_percent: round_percent(f64::from(scalar)),
                muted: muted.as_bool(),
                is_default: default.as_deref() == Some(device_id(&dev)?.as_str()),
            });
        }
    }
    Ok(results)
}

/// `_part_activate`: a control interface of a topology part, or `None` when the part doesn't have it.
fn part_activate<T: Interface>(part: &IPart) -> Option<T> {
    let mut raw: *mut c_void = std::ptr::null_mut();
    // SAFETY: Activate writes an owned interface pointer for the IID we ask for into `raw` on success; we wrap it
    // as that same interface type exactly once.
    unsafe {
        part.Activate(CLSCTX_ALL.0, &T::IID, Some(&mut raw)).ok()?;
        if raw.is_null() {
            None
        } else {
            Some(T::from_raw(raw))
        }
    }
}

fn read_part(part: &IPart) -> Result<Option<TopologyControl>> {
    // `IAudioVolumeLevel` is Python's `IPerChannelDbLevel` read through the IAudioVolumeLevel IID
    let level = part_activate::<IAudioVolumeLevel>(part);
    let mute = part_activate::<IAudioMute>(part);
    if level.is_none() && mute.is_none() {
        return Ok(None);
    }
    // SAFETY: COM calls on pointers we own; channel indexes stay below the reported channel count.
    unsafe {
        let mut ctl = TopologyControl {
            name: take_string(part.GetName()?),
            ..Default::default()
        };
        if let Some(level) = level {
            for ch in 0..level.GetChannelCount()? {
                ctl.level_db.push(round2(f64::from(level.GetLevel(ch)?)));
            }
            let (mut lo, mut hi, mut step) = (0.0f32, 0.0f32, 0.0f32);
            level.GetLevelRange(0, &mut lo, &mut hi, &mut step)?;
        }
        if let Some(mute) = mute {
            ctl.muted = Some(mute.GetMute()?.as_bool());
        }
        Ok(Some(ctl))
    }
}

/// `monitoring`: walk upstream from the playback endpoint's connector and collect every volume/mute part.
fn monitoring(en: &IMMDeviceEnumerator) -> Result<Vec<TopologyControl>> {
    let mut controls: Vec<(String, TopologyControl)> = Vec::new();
    for (_name, dev) in g6_devices(en, eRender)? {
        // SAFETY: COM calls on pointers we own; part indexes stay below the count the list reported.
        unsafe {
            let topo: IDeviceTopology = dev.Activate(CLSCTX_ALL, None)?;
            let device_side = topo.GetConnector(0)?.GetConnectedTo()?;
            let start: IPart = device_side.cast()?;
            let mut seen: Vec<String> = Vec::new();
            let mut stack = vec![start];
            while let Some(part) = stack.pop() {
                let pid = take_string(part.GetGlobalId()?);
                if seen.contains(&pid) {
                    continue;
                }
                seen.push(pid);
                if let Some(ctl) = read_part(&part)? {
                    let mut key = ctl.name.clone();
                    while controls.iter().any(|(k, _)| *k == key) {
                        key.push('\'');
                    }
                    controls.push((key, ctl));
                }
                let Ok(incoming) = part.EnumPartsIncoming() else {
                    continue;
                };
                for i in 0..incoming.GetCount()? {
                    stack.push(incoming.GetPart(i)?);
                }
            }
        }
    }
    Ok(controls.into_iter().map(|(_, c)| c).collect())
}

/// `speaker_mask`: PKEY_AudioEndpoint_PhysicalSpeakers of the first G6 playback endpoint that has one.
fn speaker_mask(en: &IMMDeviceEnumerator) -> Result<Option<u32>> {
    for (_name, dev) in g6_devices(en, eRender)? {
        if let Prop::U32(mask) = property(&dev, &PKEY_AudioEndpoint_PhysicalSpeakers)? {
            return Ok(Some(mask));
        }
    }
    Ok(None)
}

/// `read_mixer` on Windows: `None` on any Core Audio error, like Python's `read_mixer_safely`.
pub fn read_mixer() -> Option<crate::Mixer> {
    let _com = ComGuard::new();
    fn all() -> Result<crate::Mixer> {
        let en = enumerator()?;
        let eps = endpoints(&en)?;
        let mon = monitoring(&en)?;
        let mask = speaker_mask(&en)?;
        Ok(build_mixer(&eps, &mon, mask))
    }
    all().ok()
}

/// IAudioMeterInformation of the G6 playback endpoint. Field order matters: the interface is released before the
/// COM guard leaves the apartment.
pub struct Meter {
    meter: IAudioMeterInformation,
    _com: ComGuard,
}

impl Meter {
    pub fn peak(&self) -> f64 {
        // SAFETY: COM call on a pointer this struct owns, used only on the thread that created it (`Meter` is
        // `!Send` because the interface pointer is).
        unsafe { self.meter.GetPeakValue() }.map_or(0.0, f64::from)
    }
}

pub fn open_meter() -> Option<Meter> {
    let com = ComGuard::new();
    let en = enumerator().ok()?;
    let (_name, dev) = g6_devices(&en, eRender).ok()?.into_iter().next()?;
    // SAFETY: Activate gives us our own reference to the meter interface.
    let meter: IAudioMeterInformation = unsafe { dev.Activate(CLSCTX_ALL, None) }.ok()?;
    Some(Meter { meter, _com: com })
}
