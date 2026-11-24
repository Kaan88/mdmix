"""
Read the G6's volume and mute state through the operating system's audio stack (read only).

The G6 exposes volume/mute through standard USB Audio Class feature units. The OS audio driver owns that interface,
so on Windows libusb cannot send GET_CUR to it. Windows' Core Audio API reads the same feature-unit values through
the driver:
- every G6 endpoint (Speakers, External Mic, Line In, SPDIF In, What U Hear) has an IAudioEndpointVolume
  -> SBC Mixer "Playback" and "Recording" sliders
- the monitoring paths (sources mixed into the output) are volume/mute parts in the playback endpoint's device
  topology -> SBC Mixer "Monitoring" sliders

Linux is not covered yet (amixer would be the equivalent); the readers return ``None`` there.
Nothing in this module sets a value.
"""
from __future__ import annotations

import sys
from dataclasses import dataclass, field
from functools import lru_cache
from typing import Optional

G6_NAME_MARKERS = ('BlasterX G6', 'Blaster X G6')


@dataclass
class EndpointVolume:
    name: str  # Windows endpoint name, e.g. 'Speakers (Sound BlasterX G6)'
    flow: str  # 'playback' | 'recording'
    level_db: float
    level_percent: int  # Windows' scalar volume, as shown in the Windows mixer and SBC
    muted: bool
    is_default: bool = False

    def to_dict(self) -> dict:
        return dict(self.__dict__)


@dataclass
class TopologyControl:
    """A volume and/or mute control inside the device topology, e.g. the 'Line In' monitoring path."""
    name: str
    level_db: list[float] = field(default_factory=list)  # per channel
    level_range_db: Optional[tuple[float, float]] = None
    muted: Optional[bool] = None


# driver part names on the playback path -> SBC Mixer "Monitoring" labels
MONITORING_LABELS = {'Microphone': 'External Mic', 'Auxiliary': 'Line In', 'SPDIF-In': 'SPDIF In'}
OUTPUT_PART = 'Speaker'  # the output itself; already reported as the playback endpoint


def group_monitoring(controls: list[TopologyControl]) -> dict:
    """
    Merge '<X> Volume' and '<X> Mute' parts into {label: {'level_db': float | None, 'muted': bool | None}}.
    Parts with unknown names keep the driver's name, so nothing is silently dropped.
    """
    out: dict[str, dict] = {}
    for ctl in controls:
        base = ctl.name
        for suffix in (' Volume', ' Mute'):
            if base.endswith(suffix):
                base = base[:-len(suffix)]
        if base == OUTPUT_PART:
            continue
        entry = out.setdefault(MONITORING_LABELS.get(base, base), {'level_db': None, 'muted': None})
        if ctl.level_db:
            entry['level_db'] = ctl.level_db[0]
        if ctl.muted is not None and ctl.name.endswith(' Mute'):
            entry['muted'] = ctl.muted
    return out


def endpoints_by_flow(endpoints: list[EndpointVolume]) -> dict:
    """{'playback': {'Speakers': {...}}, 'recording': {'External Mic': {...}, ...}}"""
    out: dict[str, dict] = {'playback': {}, 'recording': {}}
    for ep in endpoints:
        out[ep.flow][short_name(ep.name)] = {'level_percent': ep.level_percent, 'level_db': ep.level_db,
                                             'muted': ep.muted, 'default': ep.is_default}
    return out


def read_mixer() -> Optional[dict]:
    """Playback, recording and monitoring levels of the G6 as one dict, or None if not supported here."""
    endpoints = read_endpoint_volumes()
    if endpoints is None:
        return None
    mixer = endpoints_by_flow(endpoints)
    mixer['monitoring'] = group_monitoring(read_monitoring() or [])
    return mixer


def is_g6_endpoint(name: str) -> bool:
    return any(m.lower() in name.lower() for m in G6_NAME_MARKERS)


def short_name(endpoint_name: str) -> str:
    """'External Mic (Sound BlasterX G6)' -> 'External Mic'"""
    return endpoint_name.split(' (', 1)[0]


def read_endpoint_volumes() -> Optional[list[EndpointVolume]]:
    """All active G6 endpoints with their volume, or None when the platform is not supported."""
    if sys.platform != 'win32':
        return None
    return _Windows().endpoints()


def read_monitoring() -> Optional[list[TopologyControl]]:
    """Volume/mute controls on the G6 playback path (monitoring sources and output), or None if unsupported."""
    if sys.platform != 'win32':
        return None
    return _Windows().monitoring()


# --- Windows Core Audio (comtypes) ------------------------------------------------------------------------------------

@lru_cache(maxsize=1)
def _com():
    """COM interface definitions, built on first use so importing this module works on every platform."""
    import ctypes
    from ctypes import HRESULT, POINTER, c_float, c_uint, c_void_p
    from ctypes.wintypes import BOOL, DWORD, LPWSTR

    from comtypes import COMMETHOD, GUID, IUnknown

    class PROPERTYKEY(ctypes.Structure):
        _fields_ = [('fmtid', GUID), ('pid', DWORD)]

    class PROPVARIANT(ctypes.Structure):
        _fields_ = [('vt', ctypes.c_ushort), ('r1', ctypes.c_ushort), ('r2', ctypes.c_ushort),
                    ('r3', ctypes.c_ushort), ('pwszVal', c_void_p), ('pad', c_void_p)]

    class IPropertyStore(IUnknown):
        _iid_ = GUID('{886d8eeb-8cf2-4446-8d02-cdba1dbdcf99}')
        _methods_ = [
            COMMETHOD([], HRESULT, 'GetCount', (['out'], POINTER(DWORD), 'c')),
            COMMETHOD([], HRESULT, 'GetAt', (['in'], DWORD, 'i'), (['out'], POINTER(PROPERTYKEY), 'k')),
            COMMETHOD([], HRESULT, 'GetValue', (['in'], POINTER(PROPERTYKEY), 'k'),
                      (['out'], POINTER(PROPVARIANT), 'v')),
        ]

    class IAudioEndpointVolume(IUnknown):
        _iid_ = GUID('{5CDF2C82-841E-4546-9722-0CF74078229A}')
        _methods_ = [
            COMMETHOD([], HRESULT, 'RegisterControlChangeNotify', (['in'], c_void_p, 'n')),
            COMMETHOD([], HRESULT, 'UnregisterControlChangeNotify', (['in'], c_void_p, 'n')),
            COMMETHOD([], HRESULT, 'GetChannelCount', (['out'], POINTER(c_uint), 'c')),
            COMMETHOD([], HRESULT, 'SetMasterVolumeLevel', (['in'], c_float, 'l'), (['in'], c_void_p, 'g')),
            COMMETHOD([], HRESULT, 'SetMasterVolumeLevelScalar', (['in'], c_float, 'l'), (['in'], c_void_p, 'g')),
            COMMETHOD([], HRESULT, 'GetMasterVolumeLevel', (['out'], POINTER(c_float), 'l')),
            COMMETHOD([], HRESULT, 'GetMasterVolumeLevelScalar', (['out'], POINTER(c_float), 'l')),
            COMMETHOD([], HRESULT, 'SetChannelVolumeLevel', (['in'], c_uint, 'c'), (['in'], c_float, 'l'),
                      (['in'], c_void_p, 'g')),
            COMMETHOD([], HRESULT, 'SetChannelVolumeLevelScalar', (['in'], c_uint, 'c'), (['in'], c_float, 'l'),
                      (['in'], c_void_p, 'g')),
            COMMETHOD([], HRESULT, 'GetChannelVolumeLevel', (['in'], c_uint, 'c'), (['out'], POINTER(c_float), 'l')),
            COMMETHOD([], HRESULT, 'GetChannelVolumeLevelScalar', (['in'], c_uint, 'c'),
                      (['out'], POINTER(c_float), 'l')),
            COMMETHOD([], HRESULT, 'SetMute', (['in'], BOOL, 'm'), (['in'], c_void_p, 'g')),
            COMMETHOD([], HRESULT, 'GetMute', (['out'], POINTER(BOOL), 'm')),
        ]

    class IPerChannelDbLevel(IUnknown):  # IAudioVolumeLevel has the same layout
        _iid_ = GUID('{7FB7B48F-531D-44A2-BCB3-5AD5A134B3DC}')  # IID_IAudioVolumeLevel
        _methods_ = [
            COMMETHOD([], HRESULT, 'GetChannelCount', (['out'], POINTER(c_uint), 'c')),
            COMMETHOD([], HRESULT, 'GetLevelRange', (['in'], c_uint, 'ch'), (['out'], POINTER(c_float), 'min'),
                      (['out'], POINTER(c_float), 'max'), (['out'], POINTER(c_float), 'step')),
            COMMETHOD([], HRESULT, 'GetLevel', (['in'], c_uint, 'ch'), (['out'], POINTER(c_float), 'l')),
        ]

    class IAudioMute(IUnknown):
        _iid_ = GUID('{DF45AEEA-B74A-4B6B-AFAD-2366B6AA012E}')
        _methods_ = [
            COMMETHOD([], HRESULT, 'SetMute', (['in'], BOOL, 'm'), (['in'], c_void_p, 'g')),
            COMMETHOD([], HRESULT, 'GetMute', (['out'], POINTER(BOOL), 'm')),
        ]

    class IPart(IUnknown):
        _iid_ = GUID('{AE2DE0E4-5BCA-4F2D-AA46-5D13F8FDB3A9}')

    class IPartsList(IUnknown):
        _iid_ = GUID('{6DAA848C-5EB0-45CC-AEA5-998A2CDA1FFB}')
        _methods_ = [
            COMMETHOD([], HRESULT, 'GetCount', (['out'], POINTER(c_uint), 'c')),
            COMMETHOD([], HRESULT, 'GetPart', (['in'], c_uint, 'i'), (['out'], POINTER(POINTER(IPart)), 'p')),
        ]

    IPart._methods_ = [
        COMMETHOD([], HRESULT, 'GetName', (['out'], POINTER(LPWSTR), 'n')),
        COMMETHOD([], HRESULT, 'GetLocalId', (['out'], POINTER(c_uint), 'i')),
        COMMETHOD([], HRESULT, 'GetGlobalId', (['out'], POINTER(LPWSTR), 'g')),
        COMMETHOD([], HRESULT, 'GetPartType', (['out'], POINTER(ctypes.c_int), 't')),
        COMMETHOD([], HRESULT, 'GetSubType', (['out'], POINTER(GUID), 's')),
        COMMETHOD([], HRESULT, 'GetControlInterfaceCount', (['out'], POINTER(c_uint), 'c')),
        COMMETHOD([], HRESULT, 'GetControlInterface', (['in'], c_uint, 'i'), (['out'], POINTER(c_void_p), 'ci')),
        COMMETHOD([], HRESULT, 'EnumPartsIncoming', (['out'], POINTER(POINTER(IPartsList)), 'l')),
        COMMETHOD([], HRESULT, 'EnumPartsOutgoing', (['out'], POINTER(POINTER(IPartsList)), 'l')),
        COMMETHOD([], HRESULT, 'GetTopologyObject', (['out'], POINTER(c_void_p), 't')),
        COMMETHOD([], HRESULT, 'Activate', (['in'], DWORD, 'ctx'), (['in'], POINTER(GUID), 'iid'),
                  (['out'], POINTER(c_void_p), 'o')),
    ]

    class IConnector(IUnknown):
        _iid_ = GUID('{9c2c4058-23f5-41de-877a-df3af236a09e}')

    IConnector._methods_ = [
        COMMETHOD([], HRESULT, 'GetType', (['out'], POINTER(ctypes.c_int), 't')),
        COMMETHOD([], HRESULT, 'GetDataFlow', (['out'], POINTER(ctypes.c_int), 'f')),
        COMMETHOD([], HRESULT, 'ConnectTo', (['in'], POINTER(IConnector), 'c')),
        COMMETHOD([], HRESULT, 'Disconnect'),
        COMMETHOD([], HRESULT, 'IsConnected', (['out'], POINTER(BOOL), 'b')),
        COMMETHOD([], HRESULT, 'GetConnectedTo', (['out'], POINTER(POINTER(IConnector)), 'c')),
    ]

    class IDeviceTopology(IUnknown):
        _iid_ = GUID('{2A07407E-6497-4A18-9787-32F79BD0D98F}')
        _methods_ = [
            COMMETHOD([], HRESULT, 'GetConnectorCount', (['out'], POINTER(c_uint), 'c')),
            COMMETHOD([], HRESULT, 'GetConnector', (['in'], c_uint, 'i'),
                      (['out'], POINTER(POINTER(IConnector)), 'c')),
        ]

    class IMMDevice(IUnknown):
        _iid_ = GUID('{D666063F-1587-4E43-81F1-B948E807363F}')
        _methods_ = [
            COMMETHOD([], HRESULT, 'Activate', (['in'], POINTER(GUID), 'iid'), (['in'], DWORD, 'ctx'),
                      (['in'], c_void_p, 'p'), (['out'], POINTER(c_void_p), 'i')),
            COMMETHOD([], HRESULT, 'OpenPropertyStore', (['in'], DWORD, 'a'),
                      (['out'], POINTER(POINTER(IPropertyStore)), 's')),
            COMMETHOD([], HRESULT, 'GetId', (['out'], POINTER(LPWSTR), 'id')),
            COMMETHOD([], HRESULT, 'GetState', (['out'], POINTER(DWORD), 's')),
        ]

    class IMMDeviceCollection(IUnknown):
        _iid_ = GUID('{0BD7A1BE-7A1A-44DB-8397-CC5392387B5E}')
        _methods_ = [
            COMMETHOD([], HRESULT, 'GetCount', (['out'], POINTER(c_uint), 'c')),
            COMMETHOD([], HRESULT, 'Item', (['in'], c_uint, 'i'), (['out'], POINTER(POINTER(IMMDevice)), 'd')),
        ]

    class IMMDeviceEnumerator(IUnknown):
        _iid_ = GUID('{A95664D2-9614-4F35-A746-DE8DB63617E6}')
        _methods_ = [
            COMMETHOD([], HRESULT, 'EnumAudioEndpoints', (['in'], DWORD, 'flow'), (['in'], DWORD, 'mask'),
                      (['out'], POINTER(POINTER(IMMDeviceCollection)), 'c')),
            COMMETHOD([], HRESULT, 'GetDefaultAudioEndpoint', (['in'], DWORD, 'flow'), (['in'], DWORD, 'role'),
                      (['out'], POINTER(POINTER(IMMDevice)), 'd')),
        ]

    return dict(locals())


class _Windows:
    E_RENDER, E_CAPTURE, DEVICE_STATE_ACTIVE, CLSCTX_ALL, STGM_READ = 0, 1, 1, 23, 0

    def __init__(self):
        import comtypes
        self.ctypes = __import__('ctypes')
        self.comtypes = comtypes
        self.c = _com()
        comtypes.CoInitialize()
        self.enum = comtypes.CoCreateInstance(comtypes.GUID('{BCDE0395-E52F-467C-8E3D-C4579291692E}'),
                                              self.c['IMMDeviceEnumerator'], comtypes.CLSCTX_INPROC_SERVER)
        self.pkey_name = self.c['PROPERTYKEY'](comtypes.GUID('{a45c254e-df1c-4efd-8020-67d146a850e0}'), 14)

    def _activate(self, dev, iface):
        ptr = dev.Activate(self.ctypes.byref(iface._iid_), self.CLSCTX_ALL, None)
        return self.ctypes.cast(ptr, self.ctypes.POINTER(iface))

    def _g6_devices(self, flow: int):
        devices = self.enum.EnumAudioEndpoints(flow, self.DEVICE_STATE_ACTIVE)
        for i in range(devices.GetCount()):
            dev = devices.Item(i)
            pv = dev.OpenPropertyStore(self.STGM_READ).GetValue(self.ctypes.byref(self.pkey_name))
            name = self.ctypes.wstring_at(pv.pwszVal) if pv.pwszVal else ''
            if is_g6_endpoint(name):
                yield name, dev

    def _default_id(self, flow: int) -> Optional[str]:
        try:
            return self.enum.GetDefaultAudioEndpoint(flow, 0).GetId()
        except self.comtypes.COMError:
            return None

    def endpoints(self) -> list[EndpointVolume]:
        results = []
        for flow, label in ((self.E_RENDER, 'playback'), (self.E_CAPTURE, 'recording')):
            default = self._default_id(flow)
            for name, dev in self._g6_devices(flow):
                vol = self._activate(dev, self.c['IAudioEndpointVolume'])
                results.append(EndpointVolume(
                    name=name, flow=label,
                    level_db=round(vol.GetMasterVolumeLevel(), 2),
                    level_percent=round(vol.GetMasterVolumeLevelScalar() * 100),
                    muted=bool(vol.GetMute()),
                    is_default=dev.GetId() == default,
                ))
        return results

    def _part_activate(self, part, iface):
        try:
            ptr = part.Activate(self.CLSCTX_ALL, self.ctypes.byref(iface._iid_))
        except self.comtypes.COMError:
            return None
        return self.ctypes.cast(ptr, self.ctypes.POINTER(iface))

    def _read_part(self, part) -> Optional[TopologyControl]:
        level = self._part_activate(part, self.c['IPerChannelDbLevel'])
        mute = self._part_activate(part, self.c['IAudioMute'])
        if level is None and mute is None:
            return None
        ctl = TopologyControl(name=part.GetName() or '')
        if level is not None:
            n = level.GetChannelCount()
            ctl.level_db = [round(level.GetLevel(ch), 2) for ch in range(n)]
            lo, hi, _step = level.GetLevelRange(0)
            ctl.level_range_db = (round(lo, 2), round(hi, 2))
        if mute is not None:
            ctl.muted = bool(mute.GetMute())
        return ctl

    def monitoring(self) -> list[TopologyControl]:
        """Walk upstream from the playback endpoint's connector and collect every volume/mute part."""
        controls: dict[str, TopologyControl] = {}
        for _name, dev in self._g6_devices(self.E_RENDER):
            topo = self._activate(dev, self.c['IDeviceTopology'])
            device_side = topo.GetConnector(0).GetConnectedTo()
            start = device_side.QueryInterface(self.c['IPart'])
            seen, stack = set(), [start]
            while stack:
                part = stack.pop()
                pid = part.GetGlobalId()
                if pid in seen:
                    continue
                seen.add(pid)
                ctl = self._read_part(part)
                if ctl is not None:
                    key = ctl.name
                    while key in controls:  # same names on several paths: keep them apart
                        key += "'"
                    controls[key] = ctl
                try:
                    incoming = part.EnumPartsIncoming()
                except self.comtypes.COMError:
                    continue
                stack.extend(incoming.GetPart(i) for i in range(incoming.GetCount()))
        return list(controls.values())


if __name__ == '__main__':
    for ep in read_endpoint_volumes() or []:
        print(ep)
    for ctl in read_monitoring() or []:
        print(ctl)
    print(read_mixer())
