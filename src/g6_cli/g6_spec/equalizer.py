"""
Graphic equalizer (payloads/raw/g6-sbc-session-c.pcap: EQ on/off, presets Rock and Hip Hop).

Same scheme as the SBX sliders: 5a 12 07 01 96 <id> <float32 LE> (data), then 5a 11 03 01 96 <id> (commit/read).
- 0x09: EQ on (1.0) / off (0.0)
- 0x0a: preamp, dB
- 0x0b..0x14: bands 31 Hz .. 16 kHz, dB
Presets are app-side: SoundBlaster Command just writes all 11 values.
"""
from __future__ import annotations

import struct

from g6_cli.g6_spec import DataFragmentMode, UsbHidDataFragment, DataFragmentStatic

EQ_ENABLE_ID = 0x09
EQ_PREAMP_ID = 0x0A
EQ_BAND_FREQS = ('31', '62', '125', '250', '500', '1k', '2k', '4k', '8k', '16k')  # as labelled in SBC
EQ_BAND_IDS = dict(zip(EQ_BAND_FREQS, range(0x0B, 0x15)))
EQ_DB_MIN, EQ_DB_MAX = -12.0, 12.0


def _param(param_id: int, value: float) -> list[UsbHidDataFragment]:
    common = dict(intermediate=DataFragmentStatic.PLAYBACK_INTERMEDIATE.value, audio_feature=bytes([param_id]))
    return [
        UsbHidDataFragment.empty_additional_payload(mode=DataFragmentMode.DATA.value, value=struct.pack('<f', value),
                                                    **common),
        UsbHidDataFragment.empty_additional_payload(mode=DataFragmentMode.COMMIT.value, value=bytes(4), **common),
    ]


def _check_db(db: float) -> float:
    if not EQ_DB_MIN <= db <= EQ_DB_MAX:
        raise ValueError(f'EQ gain must be between {EQ_DB_MIN:g} and {EQ_DB_MAX:g} dB, got {db:g}')
    return float(db)


def equalizer_enabled(enable: bool) -> list[UsbHidDataFragment]:
    return _param(EQ_ENABLE_ID, 1.0 if enable else 0.0)


def equalizer_preamp(db: float) -> list[UsbHidDataFragment]:
    return _param(EQ_PREAMP_ID, _check_db(db))


def equalizer_band(freq: str, db: float) -> list[UsbHidDataFragment]:
    if freq not in EQ_BAND_IDS:
        raise ValueError(f'unknown EQ band {freq!r}, expected one of {", ".join(EQ_BAND_FREQS)}')
    return _param(EQ_BAND_IDS[freq], _check_db(db))
