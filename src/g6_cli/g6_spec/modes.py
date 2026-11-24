"""
Processing modes: the SBX master switch and Scout Mode (payloads/raw/g6-sbc-session-c.pcap).

Write: 5a 26 05 07 <mode id> 00 <01 on | 00 off>, then SoundBlaster Command re-reads the mode register
(5a 26 03 08 ff ff -> 5a 26 0b 08 ff ff <bitmask>), which is sent here too.
SBX and Scout Mode exclude each other in SBC; Direct mode turns both off.
"""
from __future__ import annotations

from enum import Enum

from g6_cli.g6_spec import UsbHidDataFragment


class ProcessingMode(Enum):
    SBX = 0x01
    SCOUT_MODE = 0x02


# bits of the mode register (5a260308ffff answer, byte 6)
MODE_BIT_SBX = 0x01
MODE_BIT_SCOUT = 0x02
MODE_BIT_VOICE_CLARITY = 0x04  # follows the recording effects; there is no own write for it

MODE_QUERY = bytes.fromhex('5a260308ffff')


def processing_mode(mode: ProcessingMode, enable: bool) -> list[UsbHidDataFragment]:
    return [
        UsbHidDataFragment.empty_additional_payload(mode=bytes.fromhex('2605'), intermediate=bytes([0x07, mode.value]),
                                                    audio_feature=bytes.fromhex('00'),
                                                    value=bytes.fromhex('01000000' if enable else '00000000')),
        UsbHidDataFragment.empty_additional_payload(mode=bytes.fromhex('2603'), intermediate=bytes.fromhex('08ff'),
                                                    audio_feature=bytes.fromhex('ff'),
                                                    value=bytes.fromhex('00000000')),
    ]
