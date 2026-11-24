"""Capture parsing (against the real captures in payloads/raw) and the probe snapshot/diff tooling."""
from __future__ import annotations

import json
import struct
from pathlib import Path

import pytest

from g6_cli import g6_probe as P
from g6_cli.g6_capture import is_known_write, load_events, unique_hid_queries

RAW = Path(__file__).resolve().parents[2] / 'payloads' / 'raw'


def test_lighting_capture_hid_out_and_responses():
    events = load_events(RAW / 'g6-lighting.pcapng')
    outs = [e for e in events if e.kind == 'hid_out']
    assert len(outs) == 9
    assert outs[0].payload[:5] == bytes.fromhex('5a3a020601')
    assert outs[0].response[:7] == bytes.fromhex('5a020a3a000601')  # device ACK
    assert all(e.is_write for e in outs)
    assert events[0].ts == 0.0


def test_playback_volume_capture_is_uac_set_cur():
    events = load_events(RAW / 'g6-playback-volume.pcapng')
    assert {e.kind for e in events} == {'uac_set_cur'}
    first = events[0]
    # feature unit 1, volume control selector, channel 1, -64 dB (0xc000 in 1/256 dB)
    assert (first.interface, first.w_index >> 8, first.w_value >> 8, first.w_value & 0xFF) == (0, 1, 0x02, 1)
    assert first.payload == bytes.fromhex('00c0')


def test_mic_boost_capture_contains_read_query():
    queries = unique_hid_queries(load_events(RAW / 'g6-recording-mic-boost-20db.pcapng'))
    by_payload = {q.payload.hex(): q for q in queries}
    read = by_payload['5a3c0201']
    assert not read.is_write
    assert read.first_response.hex() == '5a3c0401000214'  # 0x14 = 20 dB


def test_query_file_round_trip_comments_out_writes():
    queries = unique_hid_queries(load_events(RAW / 'g6-recording-mic-boost-20db.pcapng'))
    text = P.format_query_file(queries, 'x.pcapng')
    assert '# WRITE 5a3c04' in text
    parsed = P.parse_query_file(text)
    assert bytes.fromhex('5a3c0201') in parsed
    assert not any(is_known_write(q) for q in parsed)


@pytest.mark.parametrize('line', ['5a3a090a00030101ff0000ff', '5a1207019600', '0011', 'zz', '5a' + '00' * 64])
def test_unsafe_query_lines_rejected(line):
    with pytest.raises(ValueError):
        P.parse_query_file(line)


def test_default_queries_are_safe():
    for q in P.default_queries():
        P.check_query_safe(q)


def test_snapshot_with_fake_device_and_diff_finds_float():
    def device(value: float):
        def query(payload):
            if payload == bytes.fromhex('5a1103019702'):
                return (bytes.fromhex('5a110801009702') + struct.pack('<f', value)).ljust(64, b'\0')
            return None
        return query

    q = [bytes.fromhex('5a1103019702'), bytes.fromhex('5a2c0101')]
    a = P.take_snapshot(q, query_fn=device(0.4), label='40')
    b = P.take_snapshot(q, query_fn=device(0.6), label='60')
    assert a['responses']['5a2c0101'] is None
    changes = P.diff_snapshots(a, b)
    assert len(changes) == 1 and changes[0]['query'] == '5a1103019702'
    assert {'offset': 7, 'old': 0.4, 'new': 0.6} in changes[0]['float_guesses']
    assert 'float32 @ 7: 0.4 -> 0.6' in P.format_diff(changes)


def test_snapshot_refuses_write_even_if_passed_directly():
    with pytest.raises(ValueError):
        P.take_snapshot([bytes.fromhex('5a2c050102')], query_fn=lambda p: None)


def test_cli_queries_and_diff(tmp_path, capsys):
    out = tmp_path / 'q.txt'
    assert P.main(['queries', str(RAW / 'g6-lighting.pcapng'), '-o', str(out)]) == 0
    assert '# WRITE 5a3a09' in out.read_text(encoding='utf-8')

    a, b = tmp_path / 'a.json', tmp_path / 'b.json'
    a.write_text(json.dumps({'responses': {'5a2c0101': '5a2c050102'}}))
    b.write_text(json.dumps({'responses': {'5a2c0101': '5a2c050104'}}))
    assert P.main(['diff', str(a), str(b)]) == 0
    assert 'changed byte offsets: [4]' in capsys.readouterr().out


def test_cli_snapshot_without_device_fails_cleanly(tmp_path, capsys):
    # conftest hides the real device
    assert P.main(['snapshot', '-o', str(tmp_path / 's.json')]) == 1
    assert 'not found' in capsys.readouterr().err


def _pcapng_to_classic_pcap(src: Path, dst: Path) -> None:
    """Re-wrap the USBPcap frames of a pcapng capture as a libpcap file, like USBPcapCMD writes."""
    d = src.read_bytes()
    out = bytearray(struct.pack('<IHHiIII', 0xA1B2C3D4, 2, 4, 0, 0, 65535, 249))
    i = 0
    while i < len(d):
        btype, blen = struct.unpack_from('<II', d, i)
        if btype == 6:
            cap = struct.unpack_from('<I', d, i + 20)[0]
            frame = d[i + 28:i + 28 + cap]
            out += struct.pack('<IIII', 1, 0, len(frame), len(frame)) + frame
        i += blen
    dst.write_bytes(bytes(out))


def test_classic_pcap_decodes_like_pcapng(tmp_path):
    src = RAW / 'g6-recording-mic-boost-20db.pcapng'
    dst = tmp_path / 'capture.pcap'
    _pcapng_to_classic_pcap(src, dst)
    a = [(e.kind, e.payload, e.response) for e in load_events(src)]
    b = [(e.kind, e.payload, e.response) for e in load_events(dst)]
    assert a and a == b


def test_unknown_file_rejected(tmp_path):
    f = tmp_path / 'x.pcap'
    f.write_bytes(b'not a capture at all')
    with pytest.raises(ValueError):
        load_events(f)
