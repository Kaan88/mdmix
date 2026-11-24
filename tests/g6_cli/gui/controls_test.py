"""
Every command the GUI can send must be accepted by the real CLI parser.
(This is the test that would have caught the GUI sending the non-existent --set-bass-value.)
"""
from __future__ import annotations

import pytest

from g6_cli import parse_cli_args
from g6_cli.gui import controls as C

FIXED_COMMANDS = [
    *(['--set-output', o] for o in C.OUTPUTS),
    ['--toggle-output'],
    *(['--decoder-mode', m] for m in C.DECODER_MODES),
    *(['--playback-filter', v] for v in C.DAC_FILTERS.values()),
    *([flag] for flag in C.SPEAKER_LAYOUT_FLAGS.values()),
    *(['--playback-direct-mode', v] for v in C.ENABLED_DISABLED),
    *(['--playback-spdif-out-direct-mode', v] for v in C.ENABLED_DISABLED),
    ['--lighting-disable'],
    ['--lighting-rgb', '255', '0', '0'],
    *(['--lighting-volume-ring', v] for v in C.ENABLED_DISABLED),
    ['--reload-audio-services'],
    ['--reload-audio-services-no-sudo'],
    ['--claim-and-release'],
]

ALL_COMMANDS = C.every_command() + FIXED_COMMANDS


@pytest.mark.parametrize('cmd', ALL_COMMANDS, ids=[' '.join(c) for c in ALL_COMMANDS])
def test_gui_command_is_valid_cli(cmd):
    args = parse_cli_args(['--dry-run'] + cmd)
    assert args.dry_run


def test_sliders_with_channels_send_channels():
    cmd = C.command_for(C.PLAYBACK_VOLUME.controls[1], 40, 'Left')
    assert cmd == ['--playback-volume', '40', '--playback-volume-channels', 'Left']


def test_tables_cover_all_mixer_sources():
    flags = {c.flag for s in C.all_sections() for c in s.controls}
    for src in ('line-in', 'external-mic', 'spdif-in'):
        assert f'mixer-monitoring-{src}-volume' in flags
    for src in ('line-in', 'external-mic', 'spdif-in', 'what-u-hear'):
        assert f'mixer-recording-{src}-mute' in flags
