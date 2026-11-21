# Working on this repo

Controls a Creative SoundBlaster X G6 over USB (HID + USB Audio Control). Layout and feature flow: see readme.md
"Development".

## Commands

- Tests (no hardware, what CI runs): `python -m pytest`
- Read-only hardware check (G6 connected): `python -m pytest -m hardware tests/hardware_test.py`
- CLI without installing: `PYTHONPATH=src python -m g6_cli --help`
- Dev install: `pip install -e ".[gui,test]"`

## Hardware safety (a real G6 may be plugged into this machine)

- Reading is always fine: `--status`, `g6_status.read_status()`, `tests/hardware_test.py`.
- Anything that writes to the device needs `--dry-run` unless the user asked for that exact change.
- Never run `--claim-and-release`, `--reload-audio-services`, `modprobe`, or `tests/g6_cli/g6_api/api_real_test.py`
  unattended: on Linux they cut system audio.
- `tests/conftest.py` hides real USB/HID devices from every test not marked `@pytest.mark.hardware`. Keep it that way.

## Conventions

- Packet builders in `g6_spec/` stay pure (bytes in, bytes out) and get unit tests.
- New readback: add the query and parser in `g6_status.py`, plus a test using the captured response bytes.
- Every GUI control is a row in `gui/controls.py`; `tests/g6_cli/gui/controls_test.py` runs every flag through the
  real CLI parser.
- Version lives only in `g6_cli.VERSION`.

## Open items

- Speaker layout (stereo/5.1/7.1): no readback known.
- Decoder mode and DAC filter read back as 0 while Direct mode is on. Unconfirmed whether that means "not
  applicable"; capture with Direct mode off to check.
- SBX, voice clarity, volume and mixer values: no readback yet.
- `--toggle-output` in the CLI toggles from a temp file, not from the device. The GUI already uses readback; the CLI
  could too.
