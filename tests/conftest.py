"""
Test isolation: unless a test is marked ``@pytest.mark.hardware``, the real USB/HID devices are hidden.

This makes the default test run deterministic whether or not a G6 is plugged in (CI, other machines), and turns any
accidental hardware access into a loud failure instead of a write to the device.
"""
from __future__ import annotations

import tempfile

import pytest


class _NoHardwareHidDevice:
    def __getattr__(self, name):
        if name in ('open', 'open_path'):
            def refuse(*args, **kwargs):
                raise RuntimeError('Test tried to open a real HID device. Mark it @pytest.mark.hardware.')
            return refuse
        # unopened device in dry runs: everything else is a harmless no-op
        return lambda *args, **kwargs: None


@pytest.fixture(autouse=True)
def _isolate_from_hardware(request, monkeypatch, tmp_path):
    # the toggle-output state file lives in the temp dir; keep tests from sharing/overwriting the real one
    monkeypatch.setattr(tempfile, 'tempdir', str(tmp_path))

    if request.node.get_closest_marker('hardware'):
        return

    import hid
    import usb.core

    monkeypatch.setattr(hid, 'enumerate', lambda *args, **kwargs: [])
    monkeypatch.setattr(hid, 'device', _NoHardwareHidDevice)
    monkeypatch.setattr(usb.core, 'find', lambda *args, **kwargs: None)
