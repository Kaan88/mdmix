# GUI implementation plan (Rust, on mdshared-ui)

This is the plan for building `doc/mockups/01-sbc-sidebar.html` as the real GUI. The mockup and the
"Usability pass" section of `doc/mockups/README.md` are the spec. It extends phase 6 of
`doc/rust-port.md`: the GUI is built on the user's `mdshared-ui` toolkit
(github.com/Kaan88/markdown-shared-rs), not on egui.

Written 2026-11-24. Nothing in it has been built or run yet.

## Rules this plan keeps

From this repository (`CLAUDE.md`, `doc/rust-port.md`):

- Every click and tap writes to the device at once, as SoundBlaster Command does. There is no dry run in the GUI.
  `--dry-run` stays a CLI flag.
- Only command bytes the Python code already sends, checked against `tests/vectors/`. No guessed bytes.
- Device writes during development happen only with the user's permission, one change at a time, read back and
  restored. Reads are always fine.
- The Linux claim/release and the audio-service reload never run on their own. The GUI's Linux USB path asks first.

From markdown-shared-rs (`docs/PLAN.md`, `docs/CI.md`, `docs/next/*`):

- **The G6 app pins a commit** (`rev = "…"`) like mdview and mdedit do. Work that touches both sides goes through a local
  `[patch]` in `.cargo/config.local.toml`, which git ignores.
- **Every addition to the shared crates is opt-in.** Existing callers compile and behave as before, and nothing the
  G6 app needs is linked into `mdview.exe` (the viewer's size, memory and launch-time budget).
- **G6-specific code stays in the G6 repo.** That covers device widgets, pages and icons, among others. Only generic
  widgets go to the shared crates: a slider, and a press-and-hold repeat. Each goes on its own `f/…` branch with
  a `docs/next/<name>.md` entry in the house format: **Built**, **Public API added**, **What each app does**,
  **Run**, **Not verified**, **Release note**.
- **The code is our own:** our own drawings and code, MIT. No bundled fonts, no DLLs beyond the system's, `unsafe`
  only in a `platform` module (here: `g6-osaudio` and `g6-device`'s OS glue).
- **Checks:** `cargo fmt --all --check`, `cargo clippy --release --workspace --all-targets -- -D warnings`, and
  `cargo test --release --workspace` pass before a push, on toolchain 1.99.1. Dependencies are pinned with `=`, the
  way the toolkit pins winit 0.30.13.
- **Report results plainly**, including what was not run. Nothing counts as working on Windows until it has run on
  Windows. The user approves the interface by using it; screenshots and tests don't stand in for that.
- Linux comes after Windows.

## Shape

One Cargo workspace in this repository (`rust/`), next to the Python package:

| Crate | Holds | Depends on |
|---|---|---|
| `g6-proto` | Builders, parsers, EQ presets, lighting effects (phase 1 of `rust-port.md`) | nothing |
| `g6-device` | HID through `hidapi`, USB Audio Control through `rusb`, the status query loop | g6-proto |
| `g6-osaudio` | Windows Core Audio: volumes, mutes, meters, the speaker setup. Linux `amixer` later | `windows` |
| `g6-cli` | The `g6` binary | the three above |
| `g6-gui` | The window, the pages, the device worker | the four above, mdshared-ui, mdshared-app |

`g6-gui` uses mdshared-ui for its painter, palettes, chrome, frost, settings form, segments, toast, tooltip,
menu and glyph renderer. mdshared-app gives it the settings file and single-instance hand-over. The window
loop is the app's own, as the editor's is: the shared frame (step 4 of the shared `PLAN.md`) isn't built yet.
When it is, the G6 app is a third real user for its design.

## Architecture

```
 UI thread (winit loop, draws)          device worker thread             audio thread (COM MTA)
 ─────────────────────────────          ─────────────────────            ───────────────────────
 Model  ◄── Event::Readback ─────────── owns HID handle, rusb             owns IMMDevice*,
   │                                    applies Write, reads back         IAudioEndpointVolume,
   └─ Intent ─► Throttle ─► Write ────► (g6_status query for that key)    IAudioMeterInformation
                                        lighting effect loop (music)  ◄── meter levels (30 Hz)
```

- **`Model`** is the last state read from the device, field for field, the same struct as `DeviceStatus`.
  Next to it sits each control's `Pending` value, so the UI shows what was asked for until the read-back
  arrives. Drawing reads only the model.
- **`Intent`** is one user action: a typed `Setting` key and a value, for example `Setting::SbxSurround(Level(62))`.
  Every control maps to exactly one CLI flag. The flag is also the control's tooltip, as in the mockup.
- **Throttle, per control:** the newest value replaces a waiting one; at most one write per 80 ms (the mockup's
  figure, to tune on the device); the final value is always sent on release. A drag never queues more than one
  write.
- **The worker** turns a `Write` into reports through `g6-proto` and calls the same function the CLI calls, so the
  bytes are the ones `writes.json` checks. Write order for combined changes, such as a preset, comes from
  `device_set_audio_effects`' order, kept in one place in `g6-proto`. After each write it reads back what the write
  touched:
  - `g6_status` queries for HID settings;
  - Core Audio for volumes, mutes and the speaker setup;
  - nothing for RGB, which has no read-back: the model keeps the written value and the tooltip says so.
- **Reconcile:** a read-back equal to the pending value clears it. A refused or different value springs the
  control back, with an error in the status line, as in the mockup.
- **Undo** (button and Ctrl+Z) is one step per control: the value from before the write, sent as a new write. What
  Undo re-sends for a preset or an output switch is an open device question (below); until it's answered, Undo
  covers single settings only and is disabled with a reason for the rest.
- **Disconnected:**
  - The worker polls for the device every 2 s without opening it.
  - On loss it shows a banner with "Try again" and a "Not connected" badge, and disables the device controls
    (`Item::disabled` with the reason).
  - Pending writes are dropped, not queued.
  - On return it does one full status read.
- **The status line** is the toolkit's toast (`overlay::Bubble::toast`), worded as in the mockup ("SBX Surround
  set to 62 %"), with Undo.

## Mapping the mockup to the toolkit

| Mockup | Toolkit piece | New? |
|---|---|---|
| Title row, frost, window buttons | `chrome`, `frost`, `window`, `platform::set_backdrop` | No |
| Sidebar: device header and page list | `pane::draw_head`, `rows::Rows` for the list, `pane::draw_row_fill` | No |
| On/off switches, choices | `settings::draw_toggle`, `segments::draw_segments` | No |
| Cards of rows | `settings::Row` drawing, laid out in cards by g6-gui | Layout only |
| Settings dialog: Appearance (theme, backdrop, corners) and Device | `settings::Form` and `pages` with mdshared-app's `Backdrop` and settings layer | No |
| Dropdowns (profile, filter) | `menu` | Enter-to-commit, see below |
| Tooltips with the CLI flag, status line | `overlay::Bubble::tip`, `Bubble::toast` | No |
| Horizontal and vertical slider with a centre zero | **new**: `mdshared-ui::slider` (generic) | Shared, opt-in |
| Press beside the knob moves 10, hold repeats | **new**: part of `slider` | Shared |
| Channel strip (meter, fader, value, Mute) | g6-gui `widgets::strip` on top of the slider | G6 |
| L/R meters with peak hold | g6-gui `widgets::meter` | G6 |
| EQ curve with ten band handles | g6-gui `widgets::eq_curve` | G6 |
| Colour swatches, lighting ring | g6-gui `widgets::lighting` | G6 |
| Signal chain stages, wires, bypass arc | g6-gui `widgets::chain` | G6 |
| Lit badges ("Direct", "SBX", "Not connected") | g6-gui `widgets::badge` | G6 |
| Nine icons (speaker, muted, mic, bulb, wave, chip, mixer, chain, spark) | g6-gui `icons.rs`, drawn new in `glyph`'s format (16 grid, 1.25 stroke, strokes only) | G6 |

The slider goes to the shared crate because both apps could use one: mdedit for zoom, the settings form for a
number. If the user would rather keep the toolkit untouched for now, it lives in g6-gui first and moves later.

Toolkit issues the usability pass found. They're proposals for markdown-shared-rs, each its own opt-in change, and
none is needed to start:

1. **Dropdown should commit on Enter.** Proposed as `menu`'s keyboard path, which is opt-in.
2. **Dark + Acrylic over a bright wallpaper measures 3.5–4.3:1 for secondary text.** Proposed as a darker
   veil for Acrylic, or a secondary-text colour per backdrop. This changes how mdview and mdedit look, so the user
   decides, and it would be off by default.

## Device-side work that's new

1. **Speaker setup (Stereo / 5.1 / 7.1) on Windows.** It's in scope per the user. Reading it already works:
   `PKEY_AudioEndpoint_PhysicalSpeakers`. Writing it needs research before any code:
   - How does SoundBlaster Command change it? Find out with a read-only trace on the user's PC, or by watching
     the property store while the user changes it in SBC.
   - Candidate: the endpoint's property store through `IPolicyConfig`. That's undocumented, used by the
     Sound control panel, and may need admin rights, which would mean a UAC prompt the user approves.
   - Candidate: the driver's own interface.

   Nothing is written until the method is known and the user approves a test. Linux: not covered, the control
   shows disabled with the reason.
2. **Per-source meters for the Mixer.** Today there's only the playback meter (`IAudioMeterInformation`). For
   the recording sources, try the capture endpoints' meters, and the `IDeviceTopology` parts if the driver exposes
   them. Windows only at first. A source without a meter shows a flat, labelled "no meter" strip rather than a
   fake one.
3. **The music lighting effect** becomes a thread fed by the audio thread's level. It's a loop in the worker that
   owns the HID handle, so stopping it is clean and the static colour needn't be rewritten after a kill.
   `effects.json` checks the frames.
4. **Remember the last page.** Settings go through mdshared-app's settings file: page, window place, appearance.

## Order of work

Each step ends with the checks above and a short "Run / Not verified" note in this file.

| Step | Work | Done when | Needs the device? |
|---|---|---|---|
| G0 | `rust-port.md` phases 1–2: `g6-proto` and `g6-cli` with a recording transport | All of `writes.json`, `effects.json`, `eq_presets.json` and `status_live.json` pass | No |
| G1 | `g6-device` on Windows (HID) and `g6-osaudio` reads: volumes, mutes, speaker setup, playback meter | `g6 --status --json` matches Python's on the real G6 | Read only |
| G2 | g6-gui skeleton: window, chrome, frost, sidebar, page switching, settings dialog (Appearance, Device), the model fed by a full status read. No writes yet: every control shows the device's state | The user opens it and sees their device's real state on every page | Read only |
| G3 | The write path: intents, throttle, worker, read-back, reconcile, spring-back, toast, Undo, disconnected banner. Tested against a fake device that records writes and can refuse | UI-state tests pass (below). Then one write the user picks, read back and undone | One write, with permission |
| G4 | Shared `slider` (on `f/slider` in markdown-shared-rs with its `docs/next` entry), then the pages that need only toggles, segments and sliders: SBX, Playback and decoder, Microphone, Settings | Each page drives the fake device; the user tries each page on the G6 | Writes, with permission |
| G5 | Equalizer: EQ curve, presets (43 from `eq_presets.json`), preamp, band handles | Same | Same |
| G6 | Mixer: channel strips, meters, the lower row, speaker setup read-only first | Same; meters move with real audio | Same |
| G7 | Lighting: swatches, ring, effects thread, music effect | Same; the effect stops cleanly | Same |
| G8 | Signal chain page: lanes, stage switches, focus hand-over both ways, column layout below 730 px | Same | Same |
| G9 | Speaker setup writing, once the method is known | One change the user asks for, read back, restored | With permission, maybe UAC |
| G10 | Linux: amixer reads, USB Audio Control behind an explicit opt-in, HID permission (udev rule) | Runs on the user's Linux machine | The user present |
| G11 | Release builds (MSVC), then retire `src/g6_cli/gui/` or keep it until parity is agreed | The user approves | — |

G2 and G3 can overlap with G4's shared slider branch, since they don't touch the same files.

## Tests

- **Protocol:** the vectors, as in `rust-port.md`.
- **UI state, no window:**
  - Model and intent tests against a fake device: the throttle (a drag of 100 moves sends at most one write per
    interval, plus the last one), spring-back on refusal, Undo, and dropping writes on disconnect.
  - Every control maps to one flag, and each flag appears once.
- **Drawing:** a `--shot <page> <theme> <backdrop>` mode that renders a page to PNG, as mdview has. Each page has
  shots in both themes, plus Acrylic. They're for review and regression; byte-compare them only within one build.
  The user approves the look in the running app.
- **Device:** only reads run unattended. Each write test is one change the user agreed to, read back and
  restored. `tests/conftest.py`'s rule carries over: no test touches a real device unless it's marked as a
  hardware test.

## Open device questions (from the mockup README)

Answering each needs one read-back test with the user's permission. Until then the UI does the safe thing noted.

| Question | Until answered |
|---|---|
| What does Direct mode off restore? | Read everything back after the toggle |
| Does Direct mode bypass the decoder and the DAC filter, or only SBX? | The signal chain draws the bypass over SBX only |
| What should Undo re-send for a preset, an output switch or Direct mode? | Undo is disabled for these, with the reason |
| What does "Mute mix" (`--mixer-playback-mute`) do, compared with `--playback-mute`? | Both are shown, labelled with their flags |
| Can an output switch fade inside the write? | No fade; switch at once, as SBC does |
| How does SBC write the speaker setup? | Read-only control |
