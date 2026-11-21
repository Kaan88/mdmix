"""CLI behaviour added in the fork: --status/--json, exit codes, no-sudo reload, dry runs without a device."""
from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path
from unittest.mock import MagicMock

import pytest

import g6_cli
from g6_cli import has_action, parse_cli_args
from g6_cli.g6_status import DeviceStatus


def _env() -> dict:
    import g6_cli
    return dict(os.environ, PYTHONPATH=str(Path(g6_cli.__file__).resolve().parents[1]))


def test_status_only_does_not_open_usb(monkeypatch, capsys):
    monkeypatch.setattr(g6_cli, 'read_status', MagicMock(return_value=DeviceStatus(firmware='x', output='Speakers')))
    api = MagicMock()
    monkeypatch.setattr(g6_cli, 'G6Api', api)
    g6_cli.main(['--status', '--json'])
    assert '"output": "Speakers"' in capsys.readouterr().out
    api.assert_not_called()


def test_status_plus_action_runs_both(monkeypatch):
    monkeypatch.setattr(g6_cli, 'read_status', MagicMock(return_value=DeviceStatus()))
    api = MagicMock()
    monkeypatch.setattr(g6_cli, 'G6Api', api)
    g6_cli.main(['--status', '--dry-run', '--lighting-disable'])
    api.return_value.lighting_disable.assert_called_once()


def test_no_sudo_alone_implies_reload():
    args = parse_cli_args(['--reload-audio-services-no-sudo'])
    assert args.reload_audio_services is True


@pytest.mark.parametrize('argv', [['--dry-run'], ['--debug', '--json'], ['--playback-volume-channels', 'Left']])
def test_modifier_only_args_are_rejected(argv):
    with pytest.raises(ValueError):
        parse_cli_args(argv)


def test_has_action():
    assert has_action(parse_cli_args(['--claim-and-release']))
    assert has_action(parse_cli_args(['--status']))


def test_dry_run_works_without_device(capsys):
    # conftest hides all USB/HID devices: the dry run must fall back to a placeholder device
    g6_cli.main(['--dry-run', '--lighting-volume-ring', 'Enabled', '--claim-and-release', '--playback-mute',
                 'Enabled'])
    out = capsys.readouterr().out
    assert 'placeholder device' in out
    assert 'No interface is released' in out


def test_cli_entry_exit_code_on_error():
    # subprocess: no device is hidden here, so use an argument error that never reaches the device
    r = subprocess.run([sys.executable, '-m', 'g6_cli', '--dry-run'], capture_output=True, text=True, env=_env(), timeout=30)
    assert r.returncode == 1
    assert 'error: No meaningful argument' in r.stderr
    assert 'Traceback' not in r.stderr
