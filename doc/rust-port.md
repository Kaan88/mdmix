# Rust port plan

This is the plan for rewriting `g6_cli` in Rust. The Python code stays the reference until the Rust binary passes
every vector in `tests/vectors/` and has been checked on a real G6 on Windows and Linux.

## Goals

- One static binary per platform for the CLI (`g6`) and one for the GUI. No Python, pip or venv for users.
- The same command-line flags, spelled the same way. Profiles (`--profile-save` / `--profile-load`) store CLI flag
  names, so a profile saved by the Python version must load in the Rust version.
- The same bytes on the wire. A command sends exactly what the Python version sends, in the same order (the vectors
  check this).
- The same hardware-safety rules (see `CLAUDE.md`): reads are free; writes only for the change the user asked for;
  no guessed command bytes; the Linux claim/release and audio-service reload stay explicit opt-ins.

Out of scope, as in the Python version: firmware updates, the Speaker Type and Bass Crossover controls
(app-only, see `doc/parity.md`), and replaying captured writes that aren't understood.

## Where it stands

Phases 1 and 2 are done (step G0 of `doc/gui-plan.md`): the `rust/` workspace has `g6-proto` and `g6-cli` (binary
`g6`), with no device I/O. Build, test and fixture commands are in `rust/README.md`.

- **`g6-proto`**: every packet builder, enum and value range, the EQ presets, the lighting effects, the status
  queries and parsers with a pure `collect_status`, and the order of writes in one place (`apply::Request::actions`).
  Python's number formatting (`repr`, `format(x, 'g')`, half-even `round`) is in `pyfmt`.
- **`g6-cli`**: Python's `main` step by step (parse, profile save/load, status, apply, lighting effect), behind a
  `Transport` trait (a recording transport and a dry-run one; tests answer status queries from transcripts) and an
  `Env` trait (status read, toggle state file, clock, output meter). `argparse.rs` reads the command line the way
  Python 3.13's argparse does (abbreviations, values that start with "-", `=`, `--`, glued `-h`, the order errors
  are found in, and their wording) and only then hands clap a spelled-out command line.
- **Without a device** the binary only completes dry runs. Anything that needs the G6, including `--status` and
  `--profile-save` (it reads the status first), says "device access isn't built yet in the Rust port (step G1)".
  `--profile-save` is tested through the fake transport instead.

What was run (2026-11-24, Windows, cargo 1.99.1, Python 3.13.17):

- `cargo test --release --workspace`: all pass. That covers:
  - every `writes.json` case through the real argument parser and the recording transport, compared exactly and
    in order;
  - `effects.json` (floats to 1e-9), `eq_presets.json` and `status_live.json`;
  - `rust/g6-cli/tests/fixtures/python_output.json` from `tools/rust_fixtures.py`: `--status` text and `--json`
    for five transcripts, dry-run output, lighting effect writes, every option's definition, profile save and
    load, float formatting, and 93 awkward command lines.
- `cargo fmt --all --check` and `cargo clippy --release --workspace --all-targets -- -D warnings`: clean.
- `python -m pytest -q -m "not hardware"`: 1290 passed. No Python behaviour was changed.

What wasn't run, on purpose: anything that touches a G6 or any USB/HID device (no `hidapi` or `rusb` yet), the
hardware tests, `tools/export_vectors.py --live`, and Python's `--claim-and-release` and `--reload-audio-services`,
not even as dry runs. The Rust claim/release/reload paths were written from reading the Python source and are
unverified.

Known differences from Python:

- **Help text** is clap's layout of the same options, headings and help strings; argparse's group descriptions are
  missing. Usage errors, `--version` and exit codes match.
- **Non-ASCII digits**: Python's `int()`, `float()` and argparse's negative-number test accept any Unicode digit;
  the port takes ASCII digits only.
- **Profiles** are read by a small JSON reader in `g6-cli::pyval` that copies `json.loads` (key order, a repeated key,
  `NaN`/`Infinity`, integers of any size, error messages), not by `serde_json`. A lone surrogate escape in a profile string
  becomes U+FFFD, where Python keeps the surrogate. Its `repr` in error messages escapes fewer non-printable
  characters than Python's.
- **`--status` mixer values** are always null: the Windows Core Audio reads come with `g6-osaudio`.
- **Temp directory** for `--toggle-output`: TMPDIR, TEMP, TMP, then the OS default, without Python's check that
  the directory is writable.
- **Lighting effects** without `--dry-run` need the device; Ctrl+C handling and the music meter come with G1.

Python bugs found during the port, fixed on 2026-11-24 in both versions (tests added in Python and Rust):

- `device_toggle_output` sent the opposite of what it printed ("Toggle to Speakers" sent `toggle_to_headphones`).
  It now sends the state it prints and remembers.
- `--lighting-effect-duration nan` never ended. `--lighting-effect-duration` and `--lighting-effect-period` now
  refuse NaN and the infinities ("must be a finite number of seconds").
- `_percent(inf)` in `g6_status` raised `OverflowError`, so a status answer that decoded to infinity ended in a
  traceback. A non-finite reading is now unknown (`null`).

A port bug found and fixed on the way: clap appended a repeated `--lighting-rgb` (`1 2 3 --lighting-rgb 4 5 6`),
where argparse keeps the last colour. The argv fixture pins it.

Left for G1 (`doc/gui-plan.md`): `g6-device` on Windows HID (`hidapi`, interface 4, report id 0), the status
query loop on the real device (`g6 --status --json` must match Python's), Ctrl+C for lighting effects, and the
Core Audio reads and output meter in `g6-osaudio`.

## Test vectors (`tests/vectors/`)

They are generated from the Python code by `tools/export_vectors.py`, and `tests/g6_cli/vectors_test.py` fails when
they no longer match it. After an intended change to a packet builder, regenerate them with
`python tools/export_vectors.py`, and commit the JSON alongside the change.

| File | What it holds | How the Rust tests use it |
|---|---|---|
| `writes.json` | 235 CLI cases: `argv`, the HID reports sent (`hid`), and USB Audio Control requests (`usb`) | Run each `argv` against a recording transport; compare the sequences exactly |
| `effects.json` | Lighting effect frames over time, `level_to_brightness`, and a `LevelFollower` sequence | Compare floats to 1e-9 and frames exactly |
| `eq_presets.json` | The 43 SBC EQ presets (speaker and headphone curves) | Either generate `eq_presets.rs` from it, or check the Rust table against it |
| `status_live.json` | A read-only status transcript from a real G6 (46 queries and their responses, `null` = no answer) and the parsed result | Feed the responses to the Rust parsers in order; compare with `parsed` |

Formats:

- **HID reports** are the 64-byte payload without the leading report id `0x00`, with trailing zero bytes removed.
  Pad them back to 64 bytes before comparing. On the wire the report is `00` followed by the 64 bytes.
- **USB requests** give every field as a hex string. `wValue`, `wIndex` and `wLength` are in the byte order the
  Python builders store them in: `wLength` `0100` is 1, little-endian. Rust should compare its encoded setup packet
  fields byte for byte.
- **Fixed conditions** in `writes.json`:
  - the toggle state reads as Speakers set last, so `--toggle-output` switches to Headphones;
  - `--eq-preset` without `--eq-preset-output` uses the Headphones curve;
  - `--dry-run` doesn't change what is recorded: the recorder replaces the device.
- **Coverage gaps:** `status_live.json` is one device state (output Speakers, Direct mode on, effects off). Add more
  transcripts (Headphones, effects on, EQ on) with `--live` while the device is in those states; the drift test
  re-parses every recorded transcript.

## Module map

| Python | Rust (workspace crate / module) | Notes |
|---|---|---|
| `g6_spec/*` (packet builders, enums, value ranges) | `g6-proto` (no I/O, `no_std`-friendly) | Pure functions `fn(..) -> Vec<Report>`. Most of the port, and fully covered by `writes.json` |
| `g6_spec/eq_presets.py` | `g6-proto::eq_presets` | Generated from `eq_presets.json` by a build script or checked in |
| `g6_spec/lighting_effects.py` | `g6-proto::effects` | `effect_frame`, `level_to_brightness`, `LevelFollower` |
| `g6_status.py` (queries, parsers, `DeviceStatus`) | `g6-proto::status` (parsers) + `g6-device::status` (query loop) | Keep `collect_status(query: impl FnMut(&[u8]) -> Option<[u8; 64]>)` pure, as in Python |
| `g6_core.py` (device detection, HID / USB transport) | `g6-device` | `hidapi` crate for HID; `rusb` for USB Audio Control transfers |
| `g6_api/__init__.py` (`G6Api`) | `g6-device::Api` | Thin: builder → transport |
| `g6_os_audio.py` (Core Audio / amixer mixer reads, output meter) | `g6-osaudio` | Windows: `windows` crate (`IMMDeviceEnumerator`, `IAudioEndpointVolume`, `IAudioMeterInformation`, `IDeviceTopology`, `PKEY_AudioEndpoint_PhysicalSpeakers`). Linux: run `amixer`, or use the `alsa` crate later |
| `g6_profile.py` | `g6-cli::profile` | `serde_json`; same `{"g6_profile": 1, "settings": {...}}` format and the same allowlist |
| `__init__.py` (argparse, `main`) | `g6-cli` (bin `g6`) | `clap` derive. Flag names, value names and the order actions run in must match |
| `gui/` (customtkinter) | `g6-gui` (bin) | See the GUI section |
| `g6_capture.py`, `g6_probe.py`, `tools/*` | Stay in Python | Reverse-engineering tools, not shipped to users |

Crates to start with: `hidapi`, `rusb`, `clap`, `serde` / `serde_json`, `anyhow` / `thiserror`, and `windows`
(behind a `cfg(windows)` gate).

## Phases

1. **`g6-proto`.** Port the builders and parsers, then make `writes.json`, `effects.json` and the status transcripts
   pass. No hardware is needed for this phase.
2. **`g6-cli` with a recording transport.** Port `clap` and `main`'s order of actions, then replay all of
   `writes.json` through the real CLI code path. Add `--dry-run`, `--debug` (print the hex as Python does) and
   `--status` (text and `--json` output identical to Python's, comparable with a diff).
3. **`g6-device` on Windows.** Use HID only. `--status` on the real device must match Python's `--status --json`.
   Then do one write the user asks for, read it back, and restore.
4. **USB Audio Control and Linux.** On Linux, use `rusb` with kernel-driver detach/claim/release. This phase needs
   the user at a Linux machine, because claiming the interface cuts system audio. Port `--reload-audio-services`
   last, and keep it opt-in.
5. **`g6-osaudio`.** Port the mixer readback (Windows COM) and the output meter for the music effect.
6. **GUI.** Do it after the CLI is at parity.
7. **Packaging.** Release builds for Windows (MSVC) and Linux, plus a udev rule for HID access. Then retire the
   Python package or keep it as the reference.

## GUI options

Decided: the GUI is built on the user's `mdshared-ui` toolkit (markdown-shared-rs), from the mockup in
`doc/mockups/01-sbc-sidebar.html`. Every control writes at once, as SoundBlaster Command does. The plan is in
`doc/gui-plan.md`. It calls `g6-device` directly from a worker thread instead of spawning the `g6` binary. The lighting effects already run as
their own long-lived process in Python. In Rust they become a thread that owns the HID handle and stops cleanly,
so the static colour no longer has to be rewritten after a kill.

## Risks and things to keep

- **Order of writes.** Python applies arguments in the order of `device_set_audio_effects`, not the command-line
  order: `--scout-mode Enabled --sbx Disabled` sends the SBX write first. Combined-flag cases in `writes.json` pin
  this; keep the order in one place in Rust.
- **Float encoding and rounding.** EQ values and parameters are `f32` little-endian. Python's `round()` rounds
  halves to even (`round(2.5) == 2`) and the lighting effects use it; Rust's `f64::round` rounds halves away from
  zero, so use `round_ties_even` to match `effects.json`.
- **Channel order.** Left/right volume writes go left first (`in_channel_order`). Python used to iterate a set
  of enums here, so the order changed from run to run; Rust should send left first too.
- **HID on Windows.** The G6 exposes several HID collections. Python picks interface 4 by path, so match the same
  usage page and interface number in `hidapi`. Reports are written with report id `0` prepended. After a write,
  responses are read until none is left.
- **USB Audio Control on Windows.** The driver owns that interface. Python's USB path only works on Linux, or when
  WinUSB is installed. Keep Windows mute and volume read-only through Core Audio.
- **Linux claim/release** detaches `snd-usb-audio`, which cuts audio. It must never run unattended or in tests.
- **COM threading.** Initialise COM per thread (MTA) in `g6-osaudio`, and don't share interface pointers between
  threads.
- **Profiles and flag names.** A renamed flag breaks saved profiles. If a rename is ever needed, keep an alias
  table.
- **Status JSON** is used by scripts. Keep its key names and value spellings, such as `SLOW_ROLL_OFF_MINIMUM_PHASE`
  and `0x20`.
