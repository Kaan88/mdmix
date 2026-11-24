"""OS mixer readback: the pure grouping/formatting parts (the COM calls themselves need Windows + a G6)."""
from __future__ import annotations

import pytest

from g6_cli import format_mixer_rows
from g6_cli import g6_os_audio as oa
from g6_cli import g6_status as s

# what Core Audio returned on 2026-11-24 for the playback path of a G6 (SBC Mixer: External Mic monitoring muted)
TOPOLOGY = [
    oa.TopologyControl('Speaker Mute', muted=False),
    oa.TopologyControl('Speaker Volume', [-0.61, -0.61], (-64.0, 0.0)),
    oa.TopologyControl('SPDIF-In Mute', muted=False),
    oa.TopologyControl('External Input Recording Mute', [0.0, 0.0], (-64.0, 9.0)),
    oa.TopologyControl('Microphone Mute', muted=True),
    oa.TopologyControl('Microphone Volume', [8.08, 8.08], (-64.0, 9.0)),
    oa.TopologyControl('Auxiliary Mute', muted=False),
    oa.TopologyControl('Auxiliary Volume', [0.0, 0.0], (-64.0, 9.0)),
]

ENDPOINTS = [
    oa.EndpointVolume('Speakers (Sound BlasterX G6)', 'playback', -0.61, 96, False, True),
    oa.EndpointVolume('External Mic (Sound BlasterX G6)', 'recording', 0.0, 68, False, True),
    oa.EndpointVolume('Line In (Sound BlasterX G6)', 'recording', 0.0, 68, True),
]


def test_group_monitoring_uses_sbc_labels_and_skips_output():
    m = oa.group_monitoring(TOPOLOGY)
    assert m['External Mic'] == {'level_db': 8.08, 'muted': True}
    assert m['Line In'] == {'level_db': 0.0, 'muted': False}
    assert m['SPDIF In'] == {'level_db': None, 'muted': False}
    assert 'Speaker' not in m and 'Speakers' not in m
    # oddly named part (level control called "... Mute") is kept under its own name, not dropped or misread
    assert m['External Input Recording'] == {'level_db': 0.0, 'muted': None}


def test_endpoints_by_flow():
    m = oa.endpoints_by_flow(ENDPOINTS)
    assert m['playback'] == {'Speakers': {'level_percent': 96, 'level_db': -0.61, 'muted': False, 'default': True}}
    assert list(m['recording']) == ['External Mic', 'Line In']
    assert m['recording']['Line In']['muted'] is True


@pytest.mark.parametrize('name,ok', [('Speakers (Sound BlasterX G6)', True), ('Line In (Sound Blaster X G6)', True),
                                     ('Speakers (Realtek(R) Audio)', False)])
def test_is_g6_endpoint(name, ok):
    assert oa.is_g6_endpoint(name) is ok


def test_unsupported_platform_returns_none(monkeypatch):
    monkeypatch.setattr(oa.sys, 'platform', 'linux')
    assert oa.read_endpoint_volumes() is None and oa.read_monitoring() is None and oa.read_mixer() is None


def test_read_mixer_safely_swallows_errors(monkeypatch):
    def boom():
        raise OSError('COM went away')
    monkeypatch.setattr(oa, 'read_mixer', boom)
    assert s.read_mixer_safely() is None


def test_format_mixer_rows():
    mixer = oa.endpoints_by_flow(ENDPOINTS)
    mixer['monitoring'] = oa.group_monitoring(TOPOLOGY)
    text = '\n'.join(f'{a:<13} {b}' for a, b in format_mixer_rows(mixer))
    assert 'Playback vol  Speakers: 96% -0.6 dB' in text
    assert '              Line In: 68% +0.0 dB (muted)' in text
    assert 'Monitoring    SPDIF In: not muted' in text
    assert 'External Mic: +8.1 dB (muted)' in text
    assert format_mixer_rows(None) == []
