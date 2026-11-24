"""
Read-only checks against a connected G6. Not part of the default run:

    python -m pytest -m hardware tests/hardware_test.py

Nothing here writes to the device.
"""
from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

from g6_cli.g6_status import read_status

pytestmark = pytest.mark.hardware


def _env() -> dict:
    import g6_cli
    return dict(os.environ, PYTHONPATH=str(Path(g6_cli.__file__).resolve().parents[1]))


def test_read_status_returns_firmware_and_output():
    status = read_status()
    assert status.firmware, 'no firmware string read back'
    assert status.output in ('Speakers', 'Headphones')
    assert status.mode_flags is not None


def test_cli_status_json_exits_zero():
    r = subprocess.run([sys.executable, '-m', 'g6_cli', '--status', '--json'], capture_output=True, text=True, env=_env(),
                       timeout=30)
    assert r.returncode == 0, r.stderr
    assert '"firmware"' in r.stdout
