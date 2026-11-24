# GUI implementation plan (Rust, on mdshared-ui)

This is the plan for building `doc/mockups/01-sbc-sidebar.html` as the real GUI. The mockup and the
"Usability pass" section of `doc/mockups/README.md` are the spec. It extends phase 6 of
`doc/rust-port.md`: the GUI is built on the user's `mdshared-ui` toolkit
(github.com/Kaan88/markdown-shared-rs), not on egui.

Written 2026-11-24. Steps G2, G3 and the G4 pages are built (see their notes under Order of work); the shared slider was
not needed as a new branch (the toolkit's `slider` at the pinned rev is used). The rest has not been built or run yet.

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

### G2: Run / Not verified (2026-11-24, Windows, cargo 1.99.1)

**Built:** `rust/g6-gui` (binary `g6-gui`, a workspace member, not a default member). The window, the title row, the
sidebar, the eight pages and the Settings dialog (Appearance, Device) on `mdshared-ui` at `0984eed`. The model is
a full status read by a worker thread through `g6-device` and `g6_proto::status::collect_status`; the UI thread never
waits for HID. **It writes nothing:** the worker's only command is `Read`, no device control has a hit region, and
the only settings that change are the app's own (theme, backdrop, corners, the page, the window's size, in
`%LOCALAPPDATA%\g6-gui\settings.ini`). Controls are drawn as the device has them, in a faded palette; a page says
"Device read only" in its header. A value the G6 didn't give is "—", a choice with no answer has nothing chosen.
The G6 away: a banner with Retry, a "Not connected" badge, the last status kept on screen.

**Round 2 (mixer and review fixes):** `f/g0-proto` is merged. The worker also calls `g6_osaudio::read_mixer()` (Core
Audio reads only; no Set\* call, no change of Windows' volume, mute or default device) after each HID read, on the
same thread, and `model::MixerView::from_os` maps it by name onto the strips: the master is the default playback
endpoint, recording strips use Windows' percent and mute, monitoring strips only have a level in dB (no fader
position), and Windows' speaker setup of the output in use selects Stereo, 5.1 or 7.1 (any other name is said in
the caption). They fill the sidebar volume and speaker setup, the Playback, Mixer and Microphone pages and the chain's
Recording, Monitoring and Mixer stages. No mixer (Windows gave none): "—" as before. The output meter is not opened.
A raw zero decoder mode or filter while Direct mode is on reads "Not reported in Direct mode". Review fixes: page
hit regions are clipped to the page's view and the title row's drag sits above them; ThemeChanged re-syncs the frost;
closing while minimised keeps the saved size; the dialog's Device page follows a read starting; the toast is drawn over
the dialog; a read carries whether a person asked, so its toast describes that read.

**Run:**

- From `rust/`: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` pass (34 tests in `g6-gui`); `cargo build --release -p g6-gui` gives a 3.5 MB exe. No test
  opens a device.
- `cargo run --release -p g6-gui -- --shots target/g6-gui-shots` renders 50 PNGs without a window or a device: every
  page in both themes from the recorded G6 (`tests/vectors/status_live.json`, which a test re-parses to check
  `sample::live_status`), every page again from invented "busy" values (EQ on, SBX on, mixer values), the G6 away,
  Mica and Acrylic simulated over a sample wallpaper, the dialog's two pages, and the smallest window (760 x 600).
  `--shot <page> <light|dark> [--sample] [--backdrop] [--dialog] [--out] [--size] [--scale]` renders one. They are for
  review; `target/` is ignored by git.
- Round 2: the release window was started once more with `--page mixer` (no click, killed after 9 s): the master
  volume read 96 with Speakers and Stereo chosen, the recording strips 68, External Mic monitoring +8.1 dB muted. Those
  are Windows' values as the window showed them; they were not compared with Windows' own mixer by hand.
- The real window was started twice on this PC with the G6 connected (no click, no key, killed after 9 to 12 s):
  no error output, and a capture of the window showed the Equalizer page with the G6's own firmware
  (2.1.250903.1324), Speakers, EQ off, its ten bands and the preset they match (HipHop), in the system's dark theme.

**Not verified:**

- Everything that needs a hand on the window: the pointer (page switching, the settings button, Retry, Read again,
  the appearance choices), the keyboard, the wheel, dragging and resizing the frameless window, the caption buttons,
  double-click to maximise, the dialog opened from the title row. Only drawn pictures and the unit tests cover them.
- Backdrops on Windows 11 (Mica, Acrylic) and the focus-dependent frost: the shots only simulate the keyed chrome.
  `platform::set_backdrop` was never called in this step's runs. Rounded corners and the shadow likewise.
- The disconnected state live (unplugging the G6, Retry after it returns), and a read where the G6 answers only
  in part. The shots draw the banner from a fixed state.
- The Light theme and any display scale other than 1 in the running window; shots at scale 1.5 are only in a test.
- Linux and macOS: `Cargo.toml` has the toolkit's winit and softbuffer features for Linux, and nothing was built
  there. The blur path (`BlurSystem`) is written, not run.
- The mixer on other machines or with other G6 endpoint names (a name Windows spells differently is "—"), the
  speaker setup of the output that isn't in use (not read), and `read_mixer` while the G6 is away.
- The Live shots have no mixer: no real Core Audio read is kept in the repository. Only the Busy shots show mixer values,
  and those are invented.
- The look. Nothing here counts until the user opens it.

**Differences from the plan, so nobody is surprised:** the sidebar list is plain geometry (eight fixed entries and a
rule), not `rows::Rows`; the sidebar has no Profile dropdown yet; the Signal chain lanes wrap into more rows when the
page is narrow instead of becoming columns (that is G8, with the bypass over SBX drawn only when SBX and its
neighbours share a row); there are no tooltips with the CLI flag and no status line (G3); the pointer shape on the
resize bands is set, nothing else is hovered but the live buttons.

**Toolkit gaps met (built in `g6-gui` for now):** the switch (`settings::draw_toggle` is private), a closed
dropdown, a horizontal and a vertical slider, a button with hover look outside a dialog, badges; cards of rows in a
grid; a line (the EQ curve is stroked quads through `Painter::polygon`); a disabled look for `segments::draw_segments`
(done by passing a faded `Palette`); a read-only value row in the paged settings form (done with disabled text
rows, whose reason line holds the explanation); a shared `--shot` harness and a way to see the frosted chrome in a
shot (`shot.rs` keys the frame with `frost::key_out` over a sample wallpaper); and the window loop itself, as for
mdedit. `#![forbid(unsafe_code)]` holds: the platform calls (`style_window`, `set_backdrop`, the corners) are the
toolkit's, and the window handle comes from `raw-window-handle` without `unsafe`.

### G3/G4: Run / Not verified (2026-11-24, Windows, cargo 1.99.1)

**Built:** the write path and the pages that need only switches, segments, sliders and dropdowns. **Not run on the
G6:** no write in this step went to a device; everything below was checked against a fake device in the tests.

- **What writes** (HID only, each control one `g6` flag, through `g6_proto::apply::Action::batches`, so the reports are
  the CLI's; `intent.rs`): SBX (`--sbx`), Scout Mode (`--scout-mode`), each effect's switch and level
  (`--sbx-surround`, `--sbx-surround-value`, and so for Crystalizer, Bass, Smart Volume, Dialog+), Smart Volume's Loud
  and Night (`--sbx-smart-volume-special-value`); the output (`--set-output`; sidebar, Playback, Mixer), Direct mode
  (`--playback-direct-mode`; Playback, Mixer), SPDIF-Out Direct (`--playback-spdif-out-direct-mode`), headphone
  virtualization to Line Out (`--playback-line-out-headphone-virtualization`), the decoder's dynamic range
  (`--decoder-mode`), the DAC filter (`--playback-filter`); EQ on/off (`--eq`), the preset for the output in use
  (`--eq-preset ID --eq-preset-output OUT`), the preamp (`--eq-preamp`) and the ten bands (`--eq-band FREQ DB`), in
  half-dB steps; mic boost (`--recording-mic-boost-db`; Microphone, Mixer), Voice Clarity, noise reduction (steps of
  20), echo cancellation, Smart Volume, Mic EQ and its preset (`--recording-voice-clarity*`); lighting off
  (`--lighting-disable`), a static colour from eight (`--lighting-rgb R G B`, which also turns the light on; the
  switch turns it on again in the colour set last), the volume ring (`--lighting-volume-ring`).
- **What stays read only, with the reason in the control's tip** (`kit::Why`): Windows' volumes and mutes, the
  master volume and every mixer strip (Core Audio stays read only, and the Rust port has no USB Audio Control writes);
  the speaker setup (how SBC writes it is unknown, step G9); the lighting effects (they need the effects thread, step
  G7); Smart Volume's Normal (the CLI has no write for it). With the G6 away or not read yet, every device control is
  inert and says so.
- **Design.** `intent::Setting` and `Val` are one user action. `desk::Desk` keeps the pending value per setting, drawn
  over the model until a read-back answers it (`Desk::shown`; the controls glide to it like a read). A dragged slider
  (the toolkit's `slider::Slider` does the press, drag and release arithmetic; the pointer is held until release, the
  look is the kit's) sends at most one write per 80 ms; the value in between waits and is sent when its time comes;
  the release always sends the last value. The worker (`worker.rs`) merges what waits in its channel (a newer write of
  a setting replaces the waiting one and goes last), sends the writes, then reads the whole status and hands back one
  result. Reconcile: read-back equal, the pending value clears with a toast ("SBX Surround set to 62 %. Ctrl+Z undoes
  it."); a refused write or another value read back drops it, so the control springs back, with an error toast. A
  change the G6 makes to another setting (the last SBX effect off turns SBX off) is just the read-back. Undo (Ctrl+Z,
  no toast button) re-sends the value from before the last confirmed change as a new write, one step, not for the
  output, Direct mode or a preset. A turn whose read fails drops every pending value and shows the away banner. The
  colour can't be read, so the one written is kept for the session and the toast says it wasn't read back.
- **Run:** `cargo fmt --all --check`, `cargo clippy --release --workspace --all-targets -- -D warnings`,
  `cargo test --release --workspace` (76 tests in `g6-gui`, 121 in the workspace), `cargo build -p g6-gui --release`,
  `python -m pytest -q` (1302 passed). `--shots` renders 56 pictures as before. The tests: every setting's reports equal
  `writes.json` for every case one setting stands for; a click on a drawn switch sends exactly the CLI's reports; a
  drag of 100 moves sends at most 6 writes over 400 ms and the release sends the last value; read-back clears the
  pending value; a refusal and a different read-back spring back with a toast; the G6 gone drops pending writes and
  shows it away; Undo re-sends the old value; the mixer strips, the speaker setup and the effects stay inert; nothing
  writes while the G6 is away or not read.
- **Not verified:** any write on the real G6, and so everything about the device's side: the read-back after each
  write (the worker reads the whole status, about 46 queries, after every turn; while dragging that may make the
  turns slower than 80 ms, which the merging absorbs), the 80 ms figure, what Direct mode off restores, an output
  switch's own EQ rewrite (the CLI's `--set-output` sends 28 to 30 reports). Nothing was clicked in a window: the
  pointer capture, the dropdown lists (the toolkit's `menu`, Esc, arrows, Enter, the wheel), Ctrl+Z and the timer that
  sends a waiting drag value were only exercised through their parts in tests. No keyboard nudge for sliders. The
  EQ curve's handles aren't draggable; the band sliders under it are.

## Motion (step G3, written 2026-11-24)

Built on the toolkit's `anim` module (`docs/ANIMATION.md` of markdown-shared-rs), pinned to its pushed `feat/anim`
(`6064dfb`, which carries the review's fixes; only `PageTransition::apply` needed changing here). **Run:** the user ran
the release build on Windows on 2026-11-24 and judged the motion good. The tests move a clock by hand and `--shots`
shows rest only. Reduced motion was not tried live.

- **The loop (`app.rs`):** an `Animator` on a `SystemClock` lives in `fx::Fx`. A frame is drawn between `begin_frame`
  and `end_frame`; the deadline that gives (the toast's timer joins it through `wake_at`) becomes `WaitUntil`, and
  `ResumeTimeReached` requests the redraw. With nothing moving, `end_frame` is `None` and the loop is `Wait`, as
  before: an idle window uses no CPU. The frame interval follows the monitor's refresh rate when winit reports it.
- **Timings (`anim::fluent`):** hover and press fades 83 ms linear; a control's change (knob, selection fill, slider
  glide, nav highlight) 167 ms point-to-point; the page 250 ms decelerating; the toast and the tooltip enter in 167 ms
  decelerating and leave in 167 ms and 83 ms.
- **Pages:** the page area (below the banner, not the sidebar or the title) slides up 20 px and fades in over 250 ms
  while the old page fades out (`PageEffect::SlideFade` from the bottom). Cross-fade was the alternative; SlideFade
  was kept because its edge repeat is invisible here (the page's top row is its own background). The choice was
  judged on frames rendered offline from a test, not in a window. A click during a switch starts the next from what is
  on screen. The scroll returns to the top on a switch, with no glide.
- **Sidebar:** the selected row's fill and accent mark slide between rows (kept relative to the list, so a sidebar that
  reflows does not set them sliding); hover fades in nav rows, the caption buttons (the close button's red and white
  glyph together) and the settings button; text and icon colour fade with the selection.
- **Controls (`kit.rs`):** the segmented choice (Theme, Backdrop, Corners, the output and the speaker setup) slides its
  fill and fades its hover; the switch slides its knob and fades its fill; sliders (horizontal, EQ and mixer faders,
  the master volume) glide to a new reading; Retry, Refresh and card buttons fade their fill. Widgets are named by
  page and draw order, so a value seen for the first time (start, a page's first draw, the first read) is drawn at
  rest with no motion. A layout that changes (a row appears) makes the widgets after it start fresh, which is never wrong.
- **Overlays:** the toast fades and rises 8 px in and fades out (`Transient`); the tip over a read-only control fades; the
  settings dialog fades in and out as one picture. The toolkit's `Form` draws its own dimming and has no animator, so
  the dialog does not rise and the dimming cannot be separated from the card.
- **Scrolling:** wheel and keys move a `SmoothScroll` target and the page glides on a spring. There is no thumb drag in
  the window yet; when there is, it uses `jump_to`.
- **Reduced motion:** `platform::motion_reduced()` (on Windows, Settings, Accessibility, Visual effects, "Animation
  effects") is read at start, on `ThemeChanged` and when the window gains focus. When on, every value is its target
  at once and no frame is asked for. `--shot` and the tests that draw a frame use `Fx::still()` (`Policy::Never`), so
  they do not depend on time; all 56 `--shots` came out byte-identical to the build before this step.
- **Left:** the dialog's `Form` and the scrollbar do not animate (no hover state for the scrollbar yet); the tooltip has
  no show delay; a toast showing when a page switches is part of the old page's picture and fades with it.

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
| Does turning an effect on while SBX is off bring SBX back on? | Show what the read-back says |

Answered on the real G6 (2026-11-24, test UI): turning off the last SBX effect makes the G6 turn SBX off itself,
and SBC shows the same. The SBX switch shows the read-back; this is an expected side effect, not a refusal, so it
must not spring back or show an error.
