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
  - the current output reads as Headphones, so `--toggle-output` switches to Speakers;
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

The Python GUI is a tabbed form that runs the CLI (`CommandRunner`, `EffectProcess`), so the Rust GUI can work the
same way at first by spawning the `g6` binary. Two candidates:

- **`egui` / `eframe`.** Immediate mode, a single binary and little code, so it fits the slider-and-toggle layout.
  This is the recommended default.
- **`slint` or `tauri`.** A more native look, but more build machinery.

Later, the GUI can call `g6-device` directly instead of spawning a process. The lighting effects already run as
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
