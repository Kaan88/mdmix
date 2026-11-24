"""
Write builders and readback parsers decoded from payloads/raw/g6-sbc-session-c.pcap
(see payloads/raw/g6-sbc-session-c.txt). Expected bytes are the HID reports SoundBlaster Command sent,
with the trailing zero padding removed.
"""
from __future__ import annotations

import struct

import pytest

from g6_cli.g6_spec.equalizer import equalizer_band, equalizer_enabled, equalizer_preamp
from g6_cli.g6_spec.modes import ProcessingMode, processing_mode
from g6_cli.g6_spec.playback import enable_line_out_headphone_virtualization
from g6_cli.g6_spec.recording import MicrophoneEqualizerPreset, voice_clarity_mic_equalizer_preset
from g6_cli.g6_status import (
    MIC_EQ_PRESET_GAINS,
    RECORDING_PARAM,
    DeviceStatus,
    collect_status,
    match_mic_eq_preset,
    parse_playback_param,
    parse_processing_mode,
    playback_param_query,
)


def _report(fragment) -> str:
    return bytes.fromhex(str(fragment)).rstrip(b'\x00').hex()


@pytest.mark.parametrize('fragments,expected', [
    (processing_mode(ProcessingMode.SCOUT_MODE, False), '5a26050702'),
    (processing_mode(ProcessingMode.SCOUT_MODE, True), '5a260507020001'),
    (processing_mode(ProcessingMode.SBX, False), '5a26050701'),
    (processing_mode(ProcessingMode.SBX, True), '5a260507010001'),
    (equalizer_enabled(True), '5a12070196090000803f'),
    (equalizer_enabled(False), '5a1207019609'),
    (equalizer_preamp(0), '5a120701960a'),
    (equalizer_band('125', 2), '5a120701960d00000040'),
    (equalizer_band('4k', -1), '5a1207019612000080bf'),
    (equalizer_band('4k', -2.5), '5a1207019612000020c0'),
    (equalizer_band('16k', -1.5), '5a12070196140000c0bf'),
    (equalizer_band('16k', 3), '5a120701961400004040'),
    (enable_line_out_headphone_virtualization(True), '5a3903000a01'),
    (enable_line_out_headphone_virtualization(False), '5a3903000a'),
], ids=lambda v: v if isinstance(v, str) else None)
def test_first_report_matches_capture(fragments, expected):
    assert _report(fragments[0]) == expected


def test_processing_mode_rereads_mode_register():
    assert _report(processing_mode(ProcessingMode.SBX, True)[1]) == '5a260308ffff'


def test_equalizer_param_is_committed():
    assert _report(equalizer_band('1k', 1)[1]) == '5a1103019610'


@pytest.mark.parametrize('bad', [lambda: equalizer_band('3k', 0), lambda: equalizer_band('1k', 12.5),
                                 lambda: equalizer_preamp(-13)])
def test_equalizer_rejects_invalid(bad):
    with pytest.raises(ValueError):
        bad()


@pytest.mark.parametrize('answer,expected', [
    ('5a260b08ffff01', (True, False, False)),
    ('5a260b08ffff05', (True, False, True)),
    ('5a260b08ffff02', (False, True, False)),
    ('5a260b08ffff00', (False, False, False)),  # Direct mode
])
def test_parse_processing_mode(answer, expected):
    r = bytes.fromhex(answer).ljust(64, b'\x00')
    m = parse_processing_mode(r)
    assert (m['sbx_enabled'], m['scout_mode'], m['voice_clarity']) == expected


def test_parse_processing_mode_rejects_foreign_answer():
    assert parse_processing_mode(bytes.fromhex('5a020a26').ljust(64, b'\x00')) is None
    assert parse_processing_mode(None) is None


def test_recording_param_needs_recording_group():
    r = bytes.fromhex('5a1108010095050000003f').ljust(64, b'\x00')
    assert parse_playback_param(r, 0x05, RECORDING_PARAM) == 0.5
    assert parse_playback_param(r, 0x05) is None  # same id, playback group


def test_mic_eq_preset_gains_match_captured_writes():
    # bands 0x14..0x1b as SoundBlaster Command wrote them for preset 1 (payloads/raw/g6-recording-mic-eq-preset-1)
    assert MIC_EQ_PRESET_GAINS['PRESET_1'] == (-3.0, -4.0, 0.0, 2.0, 3.0, -3.0, 4.0, 5.0)
    assert MIC_EQ_PRESET_GAINS['PRESET_2'][3] == 2.0


@pytest.mark.parametrize('preset', list(MicrophoneEqualizerPreset))
def test_every_preset_is_recognised(preset):
    gains = tuple(struct.unpack('<f', bytes.fromhex(str(f))[6:10])[0]
                  for f in voice_clarity_mic_equalizer_preset(preset)[::2])
    assert match_mic_eq_preset(gains) == preset.name


def test_unknown_mic_eq_gains_are_shown():
    assert match_mic_eq_preset((1.0,) * 8) == '1 1 1 1 1 1 1 1'
    assert match_mic_eq_preset((1.0, None) + (0.0,) * 6) is None


def _fake_device(params: dict[tuple[int, int], float], mode_mask: int):
    def query(payload: bytes):
        if payload == bytes.fromhex('5a260308ffff'):
            return bytes.fromhex('5a260b08ffff') + bytes([mode_mask]) + bytes(57)
        if payload[:4] == bytes.fromhex('5a110301'):
            key = (payload[4], payload[5])
            if key in params:
                return bytes([0x5A, 0x11, 0x08, 0x01, 0x00, *key]) + struct.pack('<f', params[key]) + bytes(53)
        return None
    return query


def test_collect_status_reads_modes_and_recording():
    gains = MIC_EQ_PRESET_GAINS['PRESET_3']
    params = {(0x95, 0x04): 1.0, (0x95, 0x05): 0.6, (0x95, 0x00): 0.0, (0x95, 0x2C): 1.0, (0x95, 0x13): 1.0}
    params.update({(0x95, 0x14 + i): g for i, g in enumerate(gains)})
    s = collect_status(_fake_device(params, 0x05))
    assert (s.sbx_enabled, s.scout_mode, s.voice_clarity) == (True, False, True)
    assert s.recording == {'noise_reduction': True, 'noise_reduction_level': 60,
                           'acoustic_echo_cancellation': False, 'smart_volume': True, 'mic_eq': True}
    assert s.mic_eq_preset == 'PRESET_3'


def test_collect_status_without_answers_leaves_fields_empty():
    s = collect_status(lambda _: None)
    assert s.sbx_enabled is None and s.recording is None and s.mic_eq_preset is None
    assert s == DeviceStatus()


def test_playback_param_query_group():
    assert playback_param_query(0x13, RECORDING_PARAM) == bytes.fromhex('5a1103019513')


@pytest.mark.parametrize('answer,expected', [
    ('5a39050120400004', False),  # live snapshot, Line Out virtualization off
    ('5a39050120440004', True),   # same, on (2026-11-24)
])
def test_line_out_headphone_virtualization_readback(answer, expected):
    from g6_cli.g6_status import parse_mode_flags
    flags = parse_mode_flags(bytes.fromhex(answer).ljust(64, b'\x00'))
    assert flags['line_out_headphone_virtualization'] is expected
    assert flags['direct_mode'] is True and flags['ring_enabled'] is False
