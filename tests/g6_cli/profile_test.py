"""Settings profiles: status -> JSON -> CLI arguments."""
from __future__ import annotations

import json

import pytest

import g6_cli
from g6_cli import main as cli_main, parse_cli_args
from g6_cli.g6_profile import PROFILE_FLAGS, load_profile_argv, profile_from_status, profile_to_argv, save_profile
from g6_cli.g6_status import DeviceStatus


def _status(**over) -> DeviceStatus:
    s = DeviceStatus(
        firmware='2.1.250903.1324', output='Speakers', decoder_mode='Normal',
        dac_filter='SLOW_ROLL_OFF_MINIMUM_PHASE', direct_mode=False, spdif_direct=False,
        line_out_headphone_virtualization=True, rgb_enabled=False, ring_enabled=True,
        sbx={'surround': {'enabled': True, 'level': 67}, 'crystalizer': {'enabled': False, 'level': 50},
             'bass': {'enabled': True, 'level': None}, 'smart_volume': {'enabled': False, 'level': 74},
             'dialog_plus': {'enabled': None, 'level': 50}},
        smart_volume_mode='Loud', eq_enabled=True, eq_preamp_db=-1.5,
        eq_bands_db={'31': 0.0, '1k': 2.5, '16k': None}, mic_boost_db=20, sbx_enabled=True, scout_mode=False,
        voice_clarity=True,
        recording={'noise_reduction': True, 'noise_reduction_level': 40, 'acoustic_echo_cancellation': False,
                   'smart_volume': None, 'mic_eq': True},
        mic_eq_preset='PRESET_3', mixer={'playback': {'Speakers': {'level_percent': 50}}})
    for k, v in over.items():
        setattr(s, k, v)
    return s


def test_profile_from_status():
    p = profile_from_status(_status())
    assert p['g6_profile'] == 1
    s = p['settings']
    assert s['playback-direct-mode'] == 'Disabled' and s['decoder-mode'] == 'Normal'
    assert s['sbx-surround'] == 'Enabled' and s['sbx-surround-value'] == 67
    assert 'sbx-bass-value' not in s and 'sbx-dialog-plus' not in s       # not read: left out
    assert s['sbx-smart-volume-special-value'] == 'Loud'
    assert s['eq-band'] == [['31', 0.0], ['1k', 2.5]]
    assert s['recording-voice-clarity-noise-reduction'] == 40
    assert 'recording-voice-clarity-smart-volume' not in s
    assert s['recording-voice-clarity-mic-eq-preset'] == 'PRESET_3'
    assert s['lighting-disable'] is True and s['lighting-volume-ring'] == 'Enabled'
    assert set(s) <= set(PROFILE_FLAGS)
    assert not any('output' in k or 'mixer' in k or 'rgb' in k for k in s)  # no output switch, volumes or colour


def test_unreadable_values_skipped():
    s = profile_from_status(_status(smart_volume_mode='Normal', rgb_enabled=True, mic_eq_preset='[1, 2, 3]',
                                    decoder_mode='0x00000007', mic_boost_db=None))['settings']
    for flag in ('sbx-smart-volume-special-value', 'lighting-disable', 'recording-voice-clarity-mic-eq-preset',
                 'decoder-mode', 'recording-mic-boost-db'):
        assert flag not in s


def test_argv_round_trip_parses():
    argv = profile_to_argv(profile_from_status(_status()))
    assert argv[:2] == ['--playback-direct-mode', 'Disabled']
    assert '--lighting-disable' in argv and argv.count('--eq-band') == 2
    args = parse_cli_args(argv)
    assert args.sbx_surround_value == 67 and args.eq_preamp == -1.5 and args.lighting_disable
    assert args.eq_band == [['31', '0.0'], ['1k', '2.5']]


@pytest.mark.parametrize('profile,match', [
    ({}, 'not a G6 profile'),
    ({'g6_profile': 2, 'settings': {}}, 'not a G6 profile'),
    ({'g6_profile': 1, 'settings': {'reload-audio-services': True}}, 'not a profile setting'),
    ({'g6_profile': 1, 'settings': {'set-output': 'Headphones'}}, 'not a profile setting'),
    ({'g6_profile': 1, 'settings': {'eq-band': '1k 3'}}, 'eq-band'),
    ({'g6_profile': 1, 'settings': {'eq-band': [['1k']]}}, 'eq-band'),
    ({'g6_profile': 1, 'settings': {'lighting-disable': 'yes'}}, 'lighting-disable'),
    ({'g6_profile': 1, 'settings': {'sbx': None}}, 'unsupported'),
])
def test_rejects(profile, match):
    with pytest.raises(ValueError, match=match):
        profile_to_argv(profile)


def test_save_and_load_file(tmp_path):
    path = tmp_path / 'p.json'
    save_profile(str(path), _status())
    assert json.loads(path.read_text(encoding='utf-8'))['settings']['eq'] == 'Enabled'
    assert load_profile_argv(str(path)) == profile_to_argv(profile_from_status(_status()))
    with pytest.raises(ValueError, match='nothing to save'):
        save_profile(str(tmp_path / 'empty.json'), DeviceStatus())
    (tmp_path / 'bad.json').write_text('{nope', encoding='utf-8')
    with pytest.raises(ValueError, match='not valid JSON'):
        load_profile_argv(str(tmp_path / 'bad.json'))


def test_cli_save_only(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(g6_cli, 'read_status', lambda: _status())
    monkeypatch.setattr(g6_cli, 'G6Api', lambda **kw: pytest.fail('no device writes for a save'))
    cli_main(['--profile-save', str(tmp_path / 'p.json')])
    assert 'Saved' in capsys.readouterr().out and (tmp_path / 'p.json').exists()


def test_cli_load_then_command_line_wins(tmp_path, capsys):
    path = tmp_path / 'p.json'
    path.write_text(json.dumps({'g6_profile': 1, 'settings': {'sbx-bass-value': 30, 'eq-band': [['1k', 2.0]]}}),
                    encoding='utf-8')
    cli_main(['--dry-run', '--debug', f'--profile-load={path}', '--eq-band', '1k', '-3'])
    out = capsys.readouterr().out
    plus2, minus3 = '005a12070196100000' + '0040', '005a12070196100000' + '40c0'  # 1k band: 2.0 dB, then -3.0 dB
    assert out.index(plus2) < out.rindex(minus3)


def test_cli_load_bad_file(tmp_path):
    path = tmp_path / 'p.json'
    path.write_text(json.dumps({'g6_profile': 1, 'settings': {'toggle-output': True}}), encoding='utf-8')
    with pytest.raises(ValueError, match='not a profile setting'):
        cli_main(['--dry-run', '--profile-load', str(path)])
