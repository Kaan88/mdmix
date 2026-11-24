"""
SoundBlaster Command's EQ presets. Rock and Hip Hop are pinned to the band writes SBC sent in
payloads/raw/g6-sbc-session-c.pcap; the rest come from SBC's installed preset files.
"""
from __future__ import annotations

import struct

import pytest

import g6_cli
from g6_cli import main as cli_main
from g6_cli.g6_spec.equalizer import EQ_BAND_FREQS, EQ_DB_MAX, EQ_DB_MIN
from g6_cli.g6_spec.eq_presets import EQ_PRESETS, eq_preset_values, find_eq_preset


def test_all_presets_in_range():
    assert len(EQ_PRESETS) == 43
    for pid, curves in EQ_PRESETS.items():
        for values in curves:
            assert len(values) == 11, pid
            assert all(EQ_DB_MIN <= v <= EQ_DB_MAX for v in values), pid


@pytest.mark.parametrize('name,expected', [('Rock', 'Rock'), ('hip hop', 'HipHop'), ('HIP-HOP', 'HipHop'),
                                           ('rnb', 'RnB'), ('fps', 'FPS')])
def test_find_preset(name, expected):
    assert find_eq_preset(name) == expected


def test_unknown_preset():
    with pytest.raises(ValueError, match='unknown EQ preset'):
        find_eq_preset('Strobe')


def test_capture_values():
    # session C: SBC switched Flat -> Rock -> Hip Hop and wrote these gains (bands it left at 0 were not sent)
    assert eq_preset_values('Rock') == (0, dict(zip(EQ_BAND_FREQS, (0, 0, 2, 1, 0, 3, 1, -1, 3, 3))))
    assert eq_preset_values('HipHop')[1] == dict(zip(EQ_BAND_FREQS, (0, 0, 2, 2.5, 1.5, 2.5, 0, -2.5, -1, -1.5)))


def test_speaker_and_headphone_curves():
    assert set(eq_preset_values('FPS', 'Speakers')[1].values()) != set(eq_preset_values('FPS', 'Headphones')[1].values())
    assert eq_preset_values('Rock', 'Speakers') == eq_preset_values('Rock', 'Headphones')
    with pytest.raises(ValueError):
        eq_preset_values('Rock', 'Line Out')


def _band_writes(out: str) -> dict[int, float]:
    writes = {}
    for line in out.splitlines():
        if line.startswith('005a12070196'):
            param = int(line[12:14], 16)
            writes[param] = round(struct.unpack('<f', bytes.fromhex(line[14:22]))[0], 2)
    return writes


def test_cli_preset_then_override(capsys):
    cli_main(['--dry-run', '--debug', '--eq-preset', 'hip hop', '--eq-preset-output', 'Speakers',
              '--eq-band', '16k', '4'])
    out = capsys.readouterr().out
    assert '005a1207019613000080bf' in out              # 8k: -1.0 dB
    assert '005a12070196140000c0bf' in out             # 16k: the preset's -1.5 dB first ...
    assert out.rindex('005a120701961400008040') > out.rindex('005a12070196140000c0bf')  # ... then --eq-band 4 dB
    writes = _band_writes(out)
    assert writes[0x0a] == 0 and writes[0x0e] == 2.5


def test_cli_preset_uses_current_output(capsys, monkeypatch):
    monkeypatch.setattr(g6_cli, '_current_output', lambda dry_run: 'Speakers')
    cli_main(['--dry-run', '--debug', '--eq-preset', 'Dota2'])
    assert set(_band_writes(capsys.readouterr().out).values()) == {0}  # Dota 2 is flat on speakers


def test_cli_unknown_preset():
    with pytest.raises(SystemExit):
        cli_main(['--dry-run', '--eq-preset', 'nope'])
