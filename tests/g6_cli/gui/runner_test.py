from __future__ import annotations

import subprocess

import pytest

from g6_cli.gui import runner as R


class Recorder:
    def __init__(self, returncode=0):
        self.calls: list[list[str]] = []
        self.returncode = returncode

    def __call__(self, cmd, **kwargs):
        self.calls.append(list(cmd))
        return subprocess.CompletedProcess(cmd, self.returncode, stdout='out\n', stderr='')


def run_sync(runner: R.CommandRunner, steps, options, claim=False) -> bool:
    return runner.run(steps, options, claim=claim)


@pytest.fixture()
def rec(monkeypatch):
    r = Recorder()
    monkeypatch.setattr(R.subprocess, 'run', r)
    monkeypatch.setattr(R.time, 'sleep', lambda s: None)
    return r


def make(is_linux, logs=None, password='pw'):
    logs = logs if logs is not None else []
    return R.CommandRunner(log=lambda t, k: logs.append((k, t)), ask_password=lambda: password, is_linux=is_linux)


def cli_args(call):
    return call[3:] if call[1:3] == ['-m', 'g6_cli'] else None


def test_windows_never_claims(rec):
    assert run_sync(make(False), [['--playback-mute', 'Enabled']], R.RunnerOptions(), claim=True)
    assert [cli_args(c) for c in rec.calls] == [['--playback-mute', 'Enabled']]


def test_dry_run_flag_passed(rec):
    assert run_sync(make(False), [['--sbx-bass', 'Enabled']], R.RunnerOptions(dry_run=True))
    assert cli_args(rec.calls[0]) == ['--dry-run', '--sbx-bass', 'Enabled']


def test_linux_claim_with_modprobe_sequence(rec):
    assert run_sync(make(True), [['--playback-volume', '50']], R.RunnerOptions(), claim=True)
    flat = [' '.join(c) for c in rec.calls]
    assert flat[0].startswith('systemctl --user stop')
    assert flat[1] == 'sudo -S modprobe -r snd_usb_audio'
    assert cli_args(rec.calls[2]) == ['--claim-and-release', '--playback-volume', '50']
    assert flat[3] == 'sudo -S modprobe snd_usb_audio'
    assert flat[4].startswith('systemctl --user start')


def test_linux_claim_without_modprobe_uses_cli_reload(rec):
    opts = R.RunnerOptions(use_modprobe=False, use_sudo=False)
    assert run_sync(make(True), [['--playback-mute', 'Enabled']], opts, claim=True)
    assert cli_args(rec.calls[-1]) == ['--reload-audio-services-no-sudo']


def test_cancelled_password_aborts(rec):
    assert not run_sync(make(True, password=None), [['--playback-mute', 'Enabled']], R.RunnerOptions(), claim=True)
    assert rec.calls == []


def test_steps_stop_at_first_failure(monkeypatch):
    r = Recorder(returncode=1)
    monkeypatch.setattr(R.subprocess, 'run', r)
    assert not run_sync(make(False), [['--a'], ['--b']], R.RunnerOptions())
    assert len(r.calls) == 1
