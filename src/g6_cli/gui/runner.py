"""
Runs g6_cli commands for the GUI, including the Linux "release the sound driver" dance.

Kept free of any UI code: logging goes through a callback and the sudo password comes from a provider callback,
so this can be driven (and tested) without a window.
"""
from __future__ import annotations

import subprocess
import sys
import threading
import time
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from typing import Optional

LogFn = Callable[[str, str], None]  # (text, kind) kind in: cmd | ok | err | warn | dim
PasswordProvider = Callable[[], Optional[str]]

AUDIO_SERVICES = ['wireplumber', 'pipewire-pulse', 'pipewire']
CLI_TIMEOUT_S = 15

# Do not flash a console window for every command when the GUI runs under pythonw on Windows.
_NO_WINDOW = getattr(subprocess, 'CREATE_NO_WINDOW', 0)


def cli_base_command() -> list[str]:
    """Run the CLI from the same interpreter/package as the GUI; no need for it to be on PATH."""
    return [sys.executable, '-m', 'g6_cli']


@dataclass
class RunnerOptions:
    dry_run: bool = False
    auto_reload: bool = True  # Linux: restore audio after a claimed (Audio) command
    use_modprobe: bool = True  # Linux: unload/reload snd_usb_audio instead of `alsa force-reload`
    use_sudo: bool = True  # Linux: use sudo for `--reload-audio-services`
    settle_ms: int = 600  # Linux: wait after the command before reloading the driver


class CommandRunner:
    def __init__(self, log: LogFn, ask_password: PasswordProvider, is_linux: bool):
        self._log = log
        self._ask_password = ask_password
        self.is_linux = is_linux
        self._lock = threading.Lock()
        self._sudo_pw: Optional[str] = None

    # --- public ------------------------------------------------------------------------------------------------

    @property
    def busy(self) -> bool:
        return self._lock.locked()

    def clear_password(self) -> None:
        self._sudo_pw = None

    def run_async(self, steps: Sequence[Sequence[str]], options: RunnerOptions, claim: bool = False,
                  on_success: Optional[Callable[[], None]] = None) -> None:
        """
        Run one or more CLI invocations in order on a worker thread. Stops at the first failing step.
        ``on_success`` is called (on the worker thread) only if all steps succeed.
        """

        def work():
            if self.run(steps, options, claim) and on_success:
                on_success()

        threading.Thread(target=work, daemon=True).start()

    def run(self, steps: Sequence[Sequence[str]], options: RunnerOptions, claim: bool = False) -> bool:
        """Blocking version of run_async. Returns False if busy, cancelled or any step failed."""
        if not self._lock.acquire(blocking=False):
            self._log('⏳  A command is already running — please wait.', 'warn')
            return False
        try:
            return all(self._run_one(list(step), options, claim) for step in steps)
        finally:
            self._lock.release()

    def run_in_background(self, fn: Callable[[], None]) -> None:
        threading.Thread(target=fn, daemon=True).start()

    def systemctl_user(self, action: str) -> bool:
        if not self.is_linux:
            self._log('Audio service control is Linux-only', 'dim')
            return False
        cmd = ['systemctl', '--user', action] + AUDIO_SERVICES
        self._log('$ ' + ' '.join(cmd), 'cmd')
        try:
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
        except (OSError, subprocess.TimeoutExpired) as e:
            self._log(f'✗  {e}', 'err')
            return False
        self._log_output(r.stdout + r.stderr, 'ok' if r.returncode == 0 else 'err')
        return r.returncode == 0

    def sudo(self, args: list[str]) -> bool:
        pw = self._get_sudo_pw()
        if pw is None:
            self._log('✗  Cancelled (no password)', 'err')
            return False
        self._log('$ sudo ' + ' '.join(args), 'cmd')
        ok, out = self._sudo_run(args, pw)
        if out:
            self._log(out, 'ok' if ok else 'err')
        if ok:
            self._log('✓  Done', 'ok')
        return ok

    # --- internals ---------------------------------------------------------------------------------------------

    def _run_one(self, args: list[str], options: RunnerOptions, claim: bool) -> bool:
        claim = claim and self.is_linux  # claim/release is only needed (and supported) on Linux
        dry = options.dry_run
        do_modprobe = claim and not dry and options.auto_reload and options.use_modprobe

        if do_modprobe and not self._unload_driver():
            return False

        cmd = cli_base_command()
        if dry:
            cmd.append('--dry-run')
        if claim:
            cmd.append('--claim-and-release')
        cmd.extend(str(a) for a in args)
        ok = self._exec_cli(cmd)

        if do_modprobe:
            self._reload_driver(options.settle_ms)
        elif claim and not dry and options.auto_reload:
            flag = '--reload-audio-services' if options.use_sudo else '--reload-audio-services-no-sudo'
            self._exec_cli(cli_base_command() + [flag])
        return ok

    def _unload_driver(self) -> bool:
        pw = self._get_sudo_pw()
        if pw is None:
            self._log('✗  Cancelled (no password)', 'err')
            return False
        self._log('⏹  Stopping audio services…', 'warn')
        self.systemctl_user('stop')
        time.sleep(0.4)
        self._log('↓  sudo modprobe -r snd_usb_audio', 'warn')
        ok, out = self._sudo_run(['modprobe', '-r', 'snd_usb_audio'], pw)
        if out:
            self._log(out, 'dim' if ok else 'err')
        if not ok:
            self._log('✗  modprobe -r failed — restarting services and aborting', 'err')
            self.systemctl_user('start')
        return ok

    def _reload_driver(self, settle_ms: int) -> None:
        if settle_ms > 0:
            self._log(f'⏳  Settling {settle_ms} ms…', 'warn')
            time.sleep(settle_ms / 1000)
        pw = self._get_sudo_pw()
        if pw:
            self._log('↑  sudo modprobe snd_usb_audio', 'warn')
            ok, out = self._sudo_run(['modprobe', 'snd_usb_audio'], pw)
            if out:
                self._log(out, 'ok' if ok else 'err')
        self._log('▶  Starting audio services…', 'warn')
        self.systemctl_user('start')
        self._log('✓  Done', 'ok')

    def _exec_cli(self, cmd: list[str]) -> bool:
        self._log('$ g6_cli ' + ' '.join(cmd[3:]), 'cmd')
        try:
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=CLI_TIMEOUT_S,
                               creationflags=_NO_WINDOW)
        except FileNotFoundError as e:
            self._log(f'✗  Could not start the CLI: {e}', 'err')
            return False
        except subprocess.TimeoutExpired:
            self._log('✗  Command timed out', 'err')
            return False
        self._log_output(r.stdout + r.stderr, 'ok' if r.returncode == 0 else 'err')
        if r.returncode != 0:
            self._log(f'✗  exit {r.returncode}', 'err')
            return False
        return True

    def _get_sudo_pw(self) -> Optional[str]:
        if self._sudo_pw is None:
            self._sudo_pw = self._ask_password()
        return self._sudo_pw

    def _sudo_run(self, args: list[str], pw: str, timeout: int = 15) -> tuple[bool, str]:
        try:
            r = subprocess.run(['sudo', '-S'] + args, input=pw + '\n', capture_output=True, text=True,
                               timeout=timeout)
        except subprocess.TimeoutExpired:
            return False, 'timed out'
        except FileNotFoundError:
            return False, 'sudo not found'
        out = (r.stdout + r.stderr).strip()
        if r.returncode != 0:
            if 'incorrect password' in out.lower() or 'try again' in out.lower():
                self._sudo_pw = None
            return False, out
        return True, out

    def _log_output(self, text: str, kind: str) -> None:
        for line in text.strip().splitlines():
            self._log(line, kind)


class EffectProcess:
    """
    A long-running CLI process (a host-side lighting effect) next to the one-shot CommandRunner: it does not take
    the runner's lock or its timeout. ``stop`` terminates it; the caller then writes the static colour again, since
    a terminated CLI can't run its own cleanup.
    """

    def __init__(self, log: LogFn, popen=subprocess.Popen):
        self._log = log
        self._popen = popen
        self._proc: Optional[subprocess.Popen] = None

    @property
    def running(self) -> bool:
        return self._proc is not None and self._proc.poll() is None

    def start(self, args: Sequence[str], dry_run: bool = False) -> None:
        self.stop()
        cmd = cli_base_command() + (['--dry-run'] if dry_run else []) + [str(a) for a in args]
        self._log('$ ' + ' '.join(cmd[2:]) + '   (runs until stopped)', 'cmd')
        self._proc = self._popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                 creationflags=_NO_WINDOW)

    def stop(self) -> bool:
        """Terminate the effect. Returns True if one was running."""
        proc, self._proc = self._proc, None
        if proc is None or proc.poll() is not None:
            return False
        proc.terminate()
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()
        self._log('■  Lighting effect stopped', 'dim')
        return True
