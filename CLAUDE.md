# Working on this repo

Controls a Creative SoundBlaster X G6 over USB (HID + USB Audio Control). Layout and feature flow: see doc/python.md
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
- `tests/vectors/` (for the Rust port, `doc/rust-port.md`) must match the Python code: after an intended change to
  what a command sends, run `python tools/export_vectors.py` and commit the JSON.  `--live` only reads status.
- New readback: add the query and parser in `g6_status.py`, plus a test using the captured response bytes.
- Every GUI control is a row in `gui/controls.py`; `tests/g6_cli/gui/controls_test.py` runs every flag through the
  real CLI parser.
- Version lives only in `g6_cli.VERSION`.

## Open items

Full feature checklist: doc/parity.md. Reverse-engineering helper: `python -m g6_cli.g6_probe` (snapshot only
replays reviewed read queries; never add guessed command bytes).


- Speaker layout (stereo/5.1/7.1): no readback known.
- Decoder mode and DAC filter read back as 0 while Direct mode is on. Unconfirmed whether that means "not
  applicable"; capture with Direct mode off to check.
- SBX, voice clarity, volume and mixer values: no readback yet.
- `--toggle-output` in the CLI toggles from a temp file, not from the device. The GUI already uses readback; the CLI
  could too.

## Using the shared toolkit (`Kaan88/markdown-shared-rs`)

This app is built on the owner's toolkit, pinned by commit in `rust/g6-gui/Cargo.toml`. Its widgets are pieces
that have to be wired together, and an audit of every app on it (2026-11-27) found each one had
wired some of them halfway: a text field with a caret but no drag, no triple click or no
right-click menu; a menu in English in a Turkish app; drags that stayed held after Alt+Tab.

1. **Look before writing.** Before writing input, menu, caret blink, timer, drag, scroll or
   dialog code, read the toolkit's `docs/CATALOGUE.md` at the pinned commit (its "I want to ..."
   index first). Use what is there. If it almost fits, change it in the toolkit, additively as
   its `AGENTS.md` says, rather than copy it here.
2. **Wire a text field whole.** Use `textfield::FieldInput`, or at a pin older than it
   `textfield::Pointer` with `TextField::context_menu`: every press, move and release, the right
   press, the keys, its deadline, and its menu drawn last. Never `TextField::press` alone: it
   places a caret and nothing else (the toolkit deprecates it from `FieldInput` on). The
   toolkit's settings forms, and at a pin that has them its palette and plan screen, have
   `pointer_press` routes that give their fields the same; use those.
3. **Every held press can be lost.** On Windows winit sends no button release when the window
   loses the mouse in the middle of a drag. On `Focus(false)` (or the host's `Event::PressLost`,
   at a pin that has it) call `cancel` (or `let_go`) on every widget that can hold a press. Do
   not fake it with a release far off screen: a release acts on what is under the pointer.
4. **The edit menu speaks the app's language.** Give it the app's words (`EditLabels`,
   `TextField::context_menu_labelled`, `set_edit_labels`).
5. **Move the pin on purpose.** When it moves, read the toolkit's commits since the old pin for
   deprecations and new opt-ins, act on each warning (do not allow them wholesale), run this
   repository's checks, and send the toolkit a pull request for the sibling revision table in its
   `docs/CATALOGUE.md`.
6. **A widget that is hard to wire right is the toolkit's bug.** Fix it there, or add a roadmap
   row there (`docs/ROADMAP.md`), and say so in this app's pull request.
