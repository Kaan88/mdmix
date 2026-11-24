"""
Read back the current state of the G6 over its HID interface.

The G6 answers 64-byte HID "read" queries on interface 4 with a 64-byte response. The queries and the meaning of
the response bytes below were worked out from USB captures of SoundBlaster Command (see doc/ and payloads/raw/).

Everything in this module except :func:`read_status` / :func:`hid_query` is pure and hardware independent, so the
parsers can be unit-tested against captured response bytes.

Confidence of each field (as of firmware 2.1.250903.1324):
- firmware, output, decoder_mode, dac_filter, rgb_enabled: matched against captures, reliable.
- direct_mode, spdif_direct, ring_enabled: decoded from a shared flags response; matched with on/off captures.
- sbx (per-effect switch + level), smart_volume_mode, eq_*: float32 parameters (5a110301 96 <id>), matched against
  SoundBlaster Command in two settings states (payloads/raw/g6-sbc-startup-a/-b) and live reads.
  The SBX *master* switch has no known readback yet.
- Speaker layout (stereo/5.1/7.1) has no known readback and is intentionally not reported.
"""
from __future__ import annotations

import dataclasses
import struct
from collections.abc import Callable
from dataclasses import dataclass
from typing import Optional

from g6_cli.g6_spec import AudioFeature, PlaybackFilter

G6_VENDOR_ID = 0x041E
G6_PRODUCT_ID = 0x3256
G6_HID_INTERFACE = 4
HID_REPORT_SIZE = 64
HID_READ_TIMEOUT_MS = 500

# A transport takes a query payload (<= 64 bytes) and returns the 64-byte response, or None on no answer.
Transport = Callable[[bytes], Optional[bytes]]

# --- Queries -------------------------------------------------------------------------------------------------------
# Some values can be queried in two different ways. The candidates are tried in order until one returns a response
# with the expected prefix.

Q_FIRMWARE = bytes.fromhex('5a070102')
Q_OUTPUT = bytes.fromhex('5a2c0101')
Q_MODE_FLAGS = (bytes.fromhex('5a020a398100'), bytes.fromhex('5a390101'))
Q_DECODER = bytes.fromhex('5a1103019702')
Q_DAC_FILTER = bytes.fromhex('5a6c0101')
Q_RGB = (bytes.fromhex('5a020a3a00060100'), bytes.fromhex('5a3a020701'))
Q_RING = (bytes.fromhex('5a020a3a000e0100'), bytes.fromhex('5a3a021001'))

R_MODE_FLAGS_PREFIXES = (bytes.fromhex('5a390501'), bytes.fromhex('5a390902'))
R_DECODER_PREFIX = bytes.fromhex('5a110801009702')
R_DAC_FILTER_PREFIX = bytes.fromhex('5a6c0301')
R_RGB_PREFIX = bytes.fromhex('5a3a0207')
R_RING_PREFIX = bytes.fromhex('5a3a0210')

OUTPUT_BY_CODE = {0x02: 'Speakers', 0x04: 'Headphones'}

# little-endian float32 values, same encoding as g6_spec.decoder.DecoderMode
DECODER_BY_RAW = {
    0x3F800000: 'Full',
    0x40000000: 'Normal',
    0x40400000: 'Night',
}

# response byte -> PlaybackFilter (the low byte of the enum value)
DAC_FILTER_BY_CODE = {pf.value[-1]: pf.name for pf in PlaybackFilter}

# Playback parameters: query 5a 11 03 01 96 <id>, answer 5a 11 08 01 00 96 <id> <float32 LE>.
# Same ids as the SBX write builders in g6_spec (AudioFeature).
PLAYBACK_PARAM = 0x96
SBX_EFFECTS = {  # name -> (switch id, level id); level is 0.0-1.0, reported as 0-100
    'surround': (AudioFeature.SURROUND_TOGGLE, AudioFeature.SURROUND_SLIDER),
    'crystalizer': (AudioFeature.CRYSTALIZER_TOGGLE, AudioFeature.CRYSTALIZER_SLIDER),
    'bass': (AudioFeature.BASS_TOGGLE, AudioFeature.BASS_SLIDER),
    'smart_volume': (AudioFeature.SMART_VOLUME_TOGGLE, AudioFeature.SMART_VOLUME_SLIDER),
    'dialog_plus': (AudioFeature.DIALOG_PLUS_TOGGLE, AudioFeature.DIALOG_PLUS_SLIDER),
}
SMART_VOLUME_MODE_BY_VALUE = {0.0: 'Normal', 1.0: 'Loud', 2.0: 'Night'}  # see SmartVolumeSpecialHex
EQ_ENABLE_ID = 0x09
EQ_PREAMP_ID = 0x0A
EQ_BAND_IDS = tuple(range(0x0B, 0x15))
EQ_BAND_FREQS = ('31', '62', '125', '250', '500', '1k', '2k', '4k', '8k', '16k')  # Hz, as labelled in SBC


def playback_param_query(param_id: int) -> bytes:
    return bytes([0x5A, 0x11, 0x03, 0x01, PLAYBACK_PARAM, param_id])


DAC_FILTER_LABELS = {
    PlaybackFilter.FAST_ROLL_OFF_MINIMUM_PHASE.name: 'Fast Roll-off / Minimum Phase',
    PlaybackFilter.SLOW_ROLL_OFF_MINIMUM_PHASE.name: 'Slow Roll-off / Minimum Phase',
    PlaybackFilter.FAST_ROLL_OFF_LINEAR_PHASE.name: 'Fast Roll-off / Linear Phase',
    PlaybackFilter.SLOW_ROLL_OFF_LINEAR_PHASE.name: 'Slow Roll-off / Linear Phase',
}


@dataclass
class DeviceStatus:
    """Read-back state of the G6. ``None`` means the value could not be read."""
    firmware: Optional[str] = None
    output: Optional[str] = None  # 'Speakers' | 'Headphones' | '0x..' if unknown
    decoder_mode: Optional[str] = None  # 'Normal' | 'Full' | 'Night' | '0x........'
    dac_filter: Optional[str] = None  # PlaybackFilter name (same value as --playback-filter)
    direct_mode: Optional[bool] = None
    spdif_direct: Optional[bool] = None
    rgb_enabled: Optional[bool] = None
    ring_enabled: Optional[bool] = None
    mode_flags: Optional[tuple[int, int]] = None  # raw bytes behind direct/spdif/ring, for debugging
    sbx: Optional[dict] = None  # effect name -> {'enabled': bool | None, 'level': int 0-100 | None}
    smart_volume_mode: Optional[str] = None  # 'Normal' | 'Loud' | 'Night'
    eq_enabled: Optional[bool] = None
    eq_preamp_db: Optional[float] = None
    eq_bands_db: Optional[dict] = None  # band label (Hz) -> dB

    def to_dict(self) -> dict:
        d = dataclasses.asdict(self)
        if self.mode_flags is not None:
            d['mode_flags'] = [f'0x{b:02x}' for b in self.mode_flags]
        return d

    def is_empty(self) -> bool:
        return all(v is None for v in dataclasses.astuple(self))


# --- Parsers (pure) ------------------------------------------------------------------------------------------------

def parse_firmware(r: Optional[bytes]) -> Optional[str]:
    if not r or len(r) < 3 or r[2] == 0:
        return None
    try:
        return r[3:3 + r[2]].decode('ascii').rstrip('\x00') or None
    except UnicodeDecodeError:
        return None


def parse_output(r: Optional[bytes]) -> Optional[str]:
    if not r or len(r) < 5:
        return None
    return OUTPUT_BY_CODE.get(r[4], f'0x{r[4]:02x}')


def parse_mode_flags(r: Optional[bytes]) -> Optional[dict]:
    """
    Returns direct_mode / spdif_direct / ring_enabled decoded from the 0x39 flags response.
    Startup captures with the LEDs on/off show the ring bit is inverted: bit set = ring off.
    """
    if not r or len(r) < 6 or r[:4] not in R_MODE_FLAGS_PREFIXES:
        return None
    flags_a, flags_b = r[4], r[5]
    return {
        'mode_flags': (flags_a, flags_b),
        'direct_mode': bool(flags_a & 0x20),
        'spdif_direct': bool(flags_b & 0x20),
        'ring_enabled': not bool(flags_b & 0x40),
    }


def parse_decoder(r: Optional[bytes]) -> Optional[str]:
    if not r or len(r) < 11 or r[:7] != R_DECODER_PREFIX:
        return None
    raw = int.from_bytes(r[7:11], 'little')
    return DECODER_BY_RAW.get(raw, f'0x{raw:08x}')


def parse_dac_filter(r: Optional[bytes]) -> Optional[str]:
    if not r or len(r) < 5 or r[:4] != R_DAC_FILTER_PREFIX:
        return None
    return DAC_FILTER_BY_CODE.get(r[4], f'0x{r[4]:02x}')


def _parse_on_off(r: Optional[bytes], prefix: bytes) -> Optional[bool]:
    # Full startup captures show 0x01 = on, 0x00 = off for these status bytes.
    if not r or len(r) < 5 or r[:4] != prefix:
        return None
    return r[4] == 0x01


def parse_playback_param(r: Optional[bytes], param_id: int) -> Optional[float]:
    """float32 value of playback parameter ``param_id``; None if the answer is for another parameter."""
    prefix = bytes([0x5A, 0x11, 0x08, 0x01, 0x00, PLAYBACK_PARAM, param_id])
    if not r or len(r) < 11 or r[:7] != prefix:
        return None
    value = struct.unpack_from('<f', r, 7)[0]
    return None if value != value else round(value, 4)  # NaN -> None


def _switch(value: Optional[float]) -> Optional[bool]:
    return None if value is None else value != 0.0


def _percent(value: Optional[float]) -> Optional[int]:
    return None if value is None else round(value * 100)


def parse_rgb(r: Optional[bytes]) -> Optional[bool]:
    return _parse_on_off(r, R_RGB_PREFIX)


def parse_ring(r: Optional[bytes]) -> Optional[bool]:
    return _parse_on_off(r, R_RING_PREFIX)


# --- Querying ------------------------------------------------------------------------------------------------------

def _first_match(query: Transport, candidates: tuple[bytes, ...], parser: Callable):
    for payload in candidates:
        parsed = parser(query(payload))
        if parsed is not None:
            return parsed
    return None


def collect_status(query: Transport) -> DeviceStatus:
    """Run all status queries through ``query`` and decode the answers."""
    status = DeviceStatus()
    status.firmware = parse_firmware(query(Q_FIRMWARE))
    status.output = parse_output(query(Q_OUTPUT))

    flags = _first_match(query, Q_MODE_FLAGS, parse_mode_flags)
    if flags:
        status.mode_flags = flags['mode_flags']
        status.direct_mode = flags['direct_mode']
        status.spdif_direct = flags['spdif_direct']
        status.ring_enabled = flags['ring_enabled']

    status.decoder_mode = parse_decoder(query(Q_DECODER))
    status.dac_filter = parse_dac_filter(query(Q_DAC_FILTER))
    status.rgb_enabled = _first_match(query, Q_RGB, parse_rgb)

    # The dedicated ring query was unreliable in captures; only use it when the flags response was missing.
    if status.ring_enabled is None:
        status.ring_enabled = _first_match(query, Q_RING, parse_ring)

    def param(param_id: int) -> Optional[float]:
        return parse_playback_param(query(playback_param_query(param_id)), param_id)

    sbx = {}
    for name, (switch, level) in SBX_EFFECTS.items():
        sbx[name] = {'enabled': _switch(param(switch.value[0])), 'level': _percent(param(level.value[0]))}
    if any(v is not None for effect in sbx.values() for v in effect.values()):
        status.sbx = sbx
    mode = param(AudioFeature.SMART_VOLUME_SPECIAL.value[0])
    if mode is not None:
        status.smart_volume_mode = SMART_VOLUME_MODE_BY_VALUE.get(mode, str(mode))

    status.eq_enabled = _switch(param(EQ_ENABLE_ID))
    status.eq_preamp_db = param(EQ_PREAMP_ID)
    bands = {freq: param(i) for freq, i in zip(EQ_BAND_FREQS, EQ_BAND_IDS)}
    if any(v is not None for v in bands.values()):
        status.eq_bands_db = bands
    return status


def find_hid_path(hid_module=None) -> Optional[bytes]:
    hid_module = hid_module or _import_hid()
    for info in hid_module.enumerate(G6_VENDOR_ID, G6_PRODUCT_ID):
        if info.get('interface_number') == G6_HID_INTERFACE:
            return info['path']
    return None


def hid_query(payload: bytes, path: Optional[bytes] = None, hid_module=None) -> Optional[bytes]:
    """
    Send one read query to the G6 and return the 64-byte response (or None).
    The device is opened and closed for every query, which is how the queries were validated on hardware.
    Raises IOError if the G6 HID interface cannot be found or opened.
    """
    if len(payload) > HID_REPORT_SIZE:
        raise ValueError(f'HID query payload too long: {len(payload)} > {HID_REPORT_SIZE}')
    hid_module = hid_module or _import_hid()
    path = path or find_hid_path(hid_module)
    if path is None:
        raise IOError('SoundBlaster X G6 HID interface not found. Is the device connected (and udev rule set on Linux)?')

    buf = bytearray(HID_REPORT_SIZE)
    buf[:len(payload)] = payload
    dev = hid_module.device()
    dev.open_path(path)
    try:
        dev.set_nonblocking(0)
        # leading 0x00 is the report id; without it the first payload byte would be swallowed
        dev.write(b'\x00' + bytes(buf))
        resp = dev.read(HID_REPORT_SIZE, timeout_ms=HID_READ_TIMEOUT_MS)
        return bytes(resp) if resp else None
    finally:
        dev.close()


def read_status(hid_module=None) -> DeviceStatus:
    """Read the full status from a connected G6. Raises IOError when no device is found."""
    hid_module = hid_module or _import_hid()
    path = find_hid_path(hid_module)
    if path is None:
        raise IOError('SoundBlaster X G6 HID interface not found. Is the device connected (and udev rule set on Linux)?')

    def query(payload: bytes) -> Optional[bytes]:
        try:
            return hid_query(payload, path=path, hid_module=hid_module)
        except (IOError, OSError, ValueError):
            return None

    return collect_status(query)


def _import_hid():
    import hid
    return hid
