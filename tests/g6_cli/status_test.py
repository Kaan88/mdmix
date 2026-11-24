"""
Status readback parsers, tested against responses captured from a real G6 (fw 2.1.250903.1324, Direct mode on,
RGB and ring off) plus hand-made variants for the other states.
"""
from __future__ import annotations

import json

import pytest

from g6_cli import g6_status as s
from g6_cli import format_status


def resp(hex_str: str) -> bytes:
    """64-byte HID response from its non-zero prefix."""
    return bytes.fromhex(hex_str).ljust(64, b'\0')


# query -> response, as captured on 2026-11-21
CAPTURED = {
    s.Q_FIRMWARE: resp('5a0710322e312e3235303930332e31333234'),
    s.Q_OUTPUT: resp('5a2c050102'),
    s.Q_MODE_FLAGS[0]: None,
    s.Q_MODE_FLAGS[1]: resp('5a39050120400004'),
    s.Q_DECODER: resp('5a110801009702'),
    s.Q_DAC_FILTER: resp('5a6c0301'),
    s.Q_RGB[0]: None,
    s.Q_RGB[1]: resp('5a3a0207'),
    s.Q_RING[0]: None,
    s.Q_RING[1]: resp('5a3a0210'),
}


def fake_transport(table: dict):
    calls = []

    def query(payload: bytes):
        calls.append(payload)
        return table.get(payload)

    query.calls = calls
    return query


def test_collect_status_from_captured_device():
    st = s.collect_status(fake_transport(CAPTURED))
    assert st.firmware == '2.1.250903.1324'
    assert st.output == 'Speakers'
    assert st.direct_mode is True
    assert st.spdif_direct is False
    assert st.ring_enabled is False  # flags_b bit 0x40 set -> ring off
    assert st.rgb_enabled is False
    assert st.mode_flags == (0x20, 0x40)
    # in Direct mode the device reports 0 for decoder and filter
    assert st.decoder_mode == '0x00000000'
    assert st.dac_filter == '0x00'


def test_ring_query_only_used_when_flags_missing():
    q = fake_transport(CAPTURED)
    s.collect_status(q)
    assert s.Q_RING[0] not in q.calls and s.Q_RING[1] not in q.calls

    table = dict(CAPTURED)
    table[s.Q_MODE_FLAGS[1]] = None
    table[s.Q_RING[1]] = resp('5a3a021001')
    st = s.collect_status(fake_transport(table))
    assert st.direct_mode is None
    assert st.ring_enabled is True


def test_no_device_answers_gives_empty_status():
    st = s.collect_status(lambda payload: None)
    assert st.is_empty()


def test_fallback_query_used_when_first_has_wrong_prefix():
    table = dict(CAPTURED)
    table[s.Q_RGB[0]] = resp('5a0000')  # garbage answer to first candidate
    table[s.Q_RGB[1]] = resp('5a3a020701')
    assert s.collect_status(fake_transport(table)).rgb_enabled is True


@pytest.mark.parametrize('code,expected', [(0x02, 'Speakers'), (0x04, 'Headphones'), (0x07, '0x07')])
def test_parse_output(code, expected):
    assert s.parse_output(resp(f'5a2c0501{code:02x}')) == expected


@pytest.mark.parametrize('raw,expected', [('0000803f', 'Full'), ('00000040', 'Normal'), ('00004040', 'Night')])
def test_parse_decoder(raw, expected):
    assert s.parse_decoder(resp('5a110801009702' + raw)) == expected


@pytest.mark.parametrize('code,expected', [
    (0x01, 'FAST_ROLL_OFF_MINIMUM_PHASE'),
    (0x02, 'SLOW_ROLL_OFF_MINIMUM_PHASE'),
    (0x04, 'FAST_ROLL_OFF_LINEAR_PHASE'),
    (0x05, 'SLOW_ROLL_OFF_LINEAR_PHASE'),
])
def test_parse_dac_filter(code, expected):
    assert s.parse_dac_filter(resp(f'5a6c0301{code:02x}')) == expected


@pytest.mark.parametrize('flags,direct,spdif,ring', [
    ('2040', True, False, False),
    ('0000', False, False, True),
    ('0020', False, True, True),
])
def test_parse_mode_flags(flags, direct, spdif, ring):
    parsed = s.parse_mode_flags(resp('5a390501' + flags))
    assert (parsed['direct_mode'], parsed['spdif_direct'], parsed['ring_enabled']) == (direct, spdif, ring)


@pytest.mark.parametrize('parser', [s.parse_firmware, s.parse_output, s.parse_mode_flags, s.parse_decoder,
                                    s.parse_dac_filter, s.parse_rgb, s.parse_ring])
@pytest.mark.parametrize('bad', [None, b'', b'\x5a', resp('ffffffff')])
def test_parsers_tolerate_bad_input(parser, bad):
    result = parser(bad)
    assert result is None or isinstance(result, str)  # parse_output reports unknown codes as hex


def test_hid_query_with_fake_hid_module():
    written = []

    class Dev:
        def open_path(self, path): assert path == b'p4'
        def set_nonblocking(self, v): pass
        def write(self, data): written.append(bytes(data))
        def read(self, n, timeout_ms=0): return list(CAPTURED[s.Q_OUTPUT])
        def close(self): pass

    class FakeHid:
        @staticmethod
        def enumerate(vid, pid):
            return [{'interface_number': 3, 'path': b'p3'}, {'interface_number': 4, 'path': b'p4'}]

        device = Dev

    r = s.hid_query(s.Q_OUTPUT, hid_module=FakeHid)
    assert r == CAPTURED[s.Q_OUTPUT]
    assert written[0][0] == 0 and written[0][1:5] == s.Q_OUTPUT and len(written[0]) == 65


def test_read_status_raises_without_device():
    class NoHid:
        @staticmethod
        def enumerate(vid, pid):
            return []

    with pytest.raises(IOError):
        s.read_status(hid_module=NoHid)


# Playback parameter answers read live on 2026-11-24 (SBC state of payloads/raw/g6-sbc-startup-b.txt):
# SBX switches off except smart volume, levels 71/11/91/90/6, EQ off with the "Hip Hop" curve.
PLAYBACK_PARAMS = {pid: resp(f'5a1108010096{pid:02x}{raw}') for pid, raw in {
    0x00: '00000000', 0x01: '8fc2353f', 0x02: '00000000', 0x03: '8fc2753d', 0x04: '0000803f', 0x05: '6666663f',
    0x06: '00000000', 0x07: '00000000', 0x08: 'ae47e13d', 0x18: '00000000', 0x19: 'c3f5683f', 0x09: '00000000',
    0x0a: '00000000', 0x0b: '00000000', 0x0c: '00000000', 0x0d: '00000040', 0x0e: '00002040', 0x0f: '0000c03f',
    0x10: '00002040', 0x11: '00000000', 0x12: '000020c0', 0x13: '000080bf', 0x14: '0000c0bf',
}.items()}
CAPTURED_WITH_PARAMS = {**CAPTURED, **{s.playback_param_query(pid): r for pid, r in PLAYBACK_PARAMS.items()}}


def test_collect_sbx_and_eq():
    st = s.collect_status(fake_transport(CAPTURED_WITH_PARAMS))
    assert st.sbx == {
        'surround': {'enabled': False, 'level': 71},
        'crystalizer': {'enabled': False, 'level': 11},
        'bass': {'enabled': False, 'level': 91},
        'smart_volume': {'enabled': True, 'level': 90},
        'dialog_plus': {'enabled': False, 'level': 6},
    }
    assert st.smart_volume_mode == 'Normal'
    assert st.eq_enabled is False and st.eq_preamp_db == 0.0
    assert list(st.eq_bands_db.values()) == [0, 0, 2, 2.5, 1.5, 2.5, 0, -2.5, -1, -1.5]
    assert list(st.eq_bands_db) == list(s.EQ_BAND_FREQS)


def test_sbx_levels_from_startup_capture_a():
    # payloads/raw/g6-sbc-startup-a: Surround 63, Crystalizer 20, Bass 38, Smart Volume 77, Dialog+ 24, all on
    a = {0x01: 'ae47213f', 0x08: 'cdcc4c3e', 0x19: '5c8fc23e', 0x05: 'b81e453f', 0x03: '8fc2753e'}
    got = {pid: s.parse_playback_param(resp(f'5a1108010096{pid:02x}{raw}'), pid) for pid, raw in a.items()}
    assert [round(v * 100) for v in got.values()] == [63, 20, 38, 77, 24]
    assert s.parse_playback_param(resp('5a1108010096000000803f'), 0x00) == 1.0


def test_playback_param_rejects_answer_for_other_id():
    # answers can arrive out of order; a reply for 0x05 must not be taken as 0x03
    assert s.parse_playback_param(PLAYBACK_PARAMS[0x05], 0x03) is None
    assert s.parse_playback_param(resp('5a110801009603ffffffff'), 0x03) is None  # NaN


def test_sbx_missing_when_device_does_not_answer_params():
    st = s.collect_status(fake_transport(CAPTURED))
    assert st.sbx is None and st.eq_bands_db is None and st.eq_enabled is None


def test_playback_param_queries_are_reads():
    from g6_cli.g6_capture import is_known_write
    for pid in range(0x00, 0x20):
        assert not is_known_write(s.playback_param_query(pid))


def test_format_status_shows_sbx_and_eq():
    text = format_status(s.collect_status(fake_transport(CAPTURED_WITH_PARAMS)), as_json=False)
    assert 'Surround      Off, 71' in text
    assert 'Smart volume  On, 90, Normal' in text
    assert 'EQ (dB)       preamp 0 | 31:0 62:0 125:2 250:2.5' in text
    data = json.loads(format_status(s.collect_status(fake_transport(CAPTURED_WITH_PARAMS)), as_json=True))
    assert data['sbx']['bass'] == {'enabled': False, 'level': 91} and data['eq_bands_db']['16k'] == -1.5


def test_format_status_text_and_json():
    st = s.collect_status(fake_transport(CAPTURED))
    text = format_status(st, as_json=False)
    assert 'Firmware      2.1.250903.1324' in text
    assert 'Direct mode   On' in text
    data = json.loads(format_status(st, as_json=True))
    assert data['output'] == 'Speakers' and data['mode_flags'] == ['0x20', '0x40']
