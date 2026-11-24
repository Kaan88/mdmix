"""
Settings profiles: save what the G6 reports (``read_status``) to a JSON file and apply it again later.

A profile is a plain map of CLI settings, so it can be read and edited by hand::

    {"g6_profile": 1, "settings": {"decoder-mode": "Normal", "sbx": "Enabled", "eq-band": [["1k", 2.5], ...], ...}}

Loading turns it back into the same command-line arguments, which go through the normal argument checks.
Only the settings listed in ``PROFILE_FLAGS`` are accepted, so a profile can't trigger anything else
(no output switch, driver reload, effects, ...).

Not in a profile:
- the output (Speakers/Headphones): a profile shouldn't flip where the sound goes;
- the RGB colour: the G6 has no colour readback, so only "lighting off" is saved;
- volumes and mutes (the OS mixer): they belong to Windows/ALSA, not to the G6's sound settings;
- the speaker layout (no HID readback).
"""
from __future__ import annotations

import json
from typing import Any

from g6_cli.g6_spec.playback import PlaybackFilter
from g6_cli.g6_spec.recording import MicrophoneEqualizerPreset
from g6_cli.g6_status import DeviceStatus

PROFILE_VERSION = 1
SBX_EFFECTS = ('surround', 'crystalizer', 'bass', 'smart_volume', 'dialog_plus')

# every setting a profile may hold, in the order it is applied
PROFILE_FLAGS = (
    'playback-direct-mode', 'playback-spdif-out-direct-mode', 'playback-filter',
    'playback-line-out-headphone-virtualization', 'decoder-mode',
    'sbx', 'scout-mode',
    *(f for e in SBX_EFFECTS for f in (f'sbx-{e.replace("_", "-")}', f'sbx-{e.replace("_", "-")}-value')),
    'sbx-smart-volume-special-value',
    'eq', 'eq-preamp', 'eq-band',
    'recording-mic-boost-db', 'recording-voice-clarity', 'recording-voice-clarity-noise-reduction',
    'recording-voice-clarity-aec', 'recording-voice-clarity-smart-volume', 'recording-voice-clarity-mic-eq',
    'recording-voice-clarity-mic-eq-preset',
    'lighting-disable', 'lighting-volume-ring',
)
_SWITCHES = {'lighting-disable'}  # flags without a value


def _ed(v: bool) -> str:
    return 'Enabled' if v else 'Disabled'


def profile_from_status(status: DeviceStatus) -> dict[str, Any]:
    """The profile of what the G6 reported; values it couldn't read are left out."""
    s: dict[str, Any] = {}

    def put_bool(flag: str, value):
        if value is not None:
            s[flag] = _ed(value)

    put_bool('playback-direct-mode', status.direct_mode)
    put_bool('playback-spdif-out-direct-mode', status.spdif_direct)
    if status.dac_filter in PlaybackFilter.__members__:
        s['playback-filter'] = status.dac_filter
    put_bool('playback-line-out-headphone-virtualization', status.line_out_headphone_virtualization)
    if status.decoder_mode in ('Normal', 'Full', 'Night'):
        s['decoder-mode'] = status.decoder_mode
    put_bool('sbx', status.sbx_enabled)
    put_bool('scout-mode', status.scout_mode)
    for effect, state in (status.sbx or {}).items():
        if effect not in SBX_EFFECTS:
            continue
        flag = f'sbx-{effect.replace("_", "-")}'
        put_bool(flag, state.get('enabled'))
        if state.get('level') is not None:
            s[f'{flag}-value'] = int(state['level'])
    if status.smart_volume_mode in ('Loud', 'Night'):
        s['sbx-smart-volume-special-value'] = status.smart_volume_mode
    put_bool('eq', status.eq_enabled)
    if status.eq_preamp_db is not None:
        s['eq-preamp'] = status.eq_preamp_db
    if status.eq_bands_db:
        s['eq-band'] = [[freq, db] for freq, db in status.eq_bands_db.items() if db is not None]
    if status.mic_boost_db in (0, 10, 20, 30):
        s['recording-mic-boost-db'] = status.mic_boost_db
    put_bool('recording-voice-clarity', status.voice_clarity)
    rec = status.recording or {}
    level = rec.get('noise_reduction_level')
    if level is not None and level in range(0, 101, 20):
        s['recording-voice-clarity-noise-reduction'] = level
    put_bool('recording-voice-clarity-aec', rec.get('acoustic_echo_cancellation'))
    put_bool('recording-voice-clarity-smart-volume', rec.get('smart_volume'))
    put_bool('recording-voice-clarity-mic-eq', rec.get('mic_eq'))
    if status.mic_eq_preset in MicrophoneEqualizerPreset.__members__:
        s['recording-voice-clarity-mic-eq-preset'] = status.mic_eq_preset
    if status.rgb_enabled is False:
        s['lighting-disable'] = True
    put_bool('lighting-volume-ring', status.ring_enabled)
    return {'g6_profile': PROFILE_VERSION, 'settings': s}


def profile_to_argv(profile: dict[str, Any]) -> list[str]:
    """CLI arguments that apply ``profile``; raises ValueError for anything that isn't a profile setting."""
    if not isinstance(profile, dict) or profile.get('g6_profile') != PROFILE_VERSION \
            or not isinstance(profile.get('settings'), dict):
        raise ValueError(f'not a G6 profile (expected {{"g6_profile": {PROFILE_VERSION}, "settings": {{...}}}})')
    settings = profile['settings']
    unknown = sorted(set(settings) - set(PROFILE_FLAGS))
    if unknown:
        raise ValueError(f'not a profile setting: {", ".join(unknown)}')
    argv: list[str] = []
    for flag in PROFILE_FLAGS:
        if flag not in settings:
            continue
        value = settings[flag]
        if flag in _SWITCHES:
            if value is True:
                argv.append(f'--{flag}')
            elif value is not False:
                raise ValueError(f'{flag}: expected true or false, got {value!r}')
        elif flag == 'eq-band':
            if not isinstance(value, list):
                raise ValueError(f'eq-band: expected a list of [band, dB] pairs, got {value!r}')
            for pair in value:
                if not (isinstance(pair, list) and len(pair) == 2):
                    raise ValueError(f'eq-band: expected [band, dB], got {pair!r}')
                argv += ['--eq-band', str(pair[0]), str(pair[1])]
        elif isinstance(value, (str, int, float)) and not isinstance(value, bool):
            argv += [f'--{flag}', str(value)]
        else:
            raise ValueError(f'{flag}: unsupported value {value!r}')
    return argv


def save_profile(path: str, status: DeviceStatus) -> dict[str, Any]:
    profile = profile_from_status(status)
    if not profile['settings']:
        raise ValueError('the G6 reported no settings, nothing to save')
    with open(path, 'w', encoding='utf-8') as f:
        json.dump(profile, f, indent=2)
        f.write('\n')
    return profile


def load_profile_argv(path: str) -> list[str]:
    with open(path, encoding='utf-8') as f:
        try:
            profile = json.load(f)
        except json.JSONDecodeError as e:
            raise ValueError(f'{path}: not valid JSON ({e})') from None
    return profile_to_argv(profile)
