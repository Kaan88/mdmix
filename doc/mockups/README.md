# GUI mockups for the Rust rewrite

One static mockup of the G6 control app in the look of `mdshared-ui`
([markdown-shared-rs](https://github.com/Kaan88/markdown-shared-rs)): `01-sbc-sidebar.html`. `index.html` introduces it.

It is one app with the Sound Blaster Command layout: a device header (name, output, master volume, speaker setup) over
a page list, and the chosen page at the right. Pages: SBX Acoustic Engine, Equalizer, Playback and decoder, Mixer (a live
console of channel strips), Microphone, Lighting, Signal chain (the sound's path as stages) and Settings.

- The file is self-contained (inline CSS and JS, system fonts, no network). Nothing talks to a real device.
- Sample state: firmware 2.1.250903.1324, Speakers, Direct mode on, EQ preset Rock. With Direct mode on, SBX and Scout
  Mode are shown bypassed and disabled; turn Direct mode off to try them.
- A link can set the look and the page: `01-sbc-sidebar.html#theme=Light&backdrop=Acrylic&corners=Small&wall=0&page=mixer`.

## Design rule: every change applies at once

The app works like Sound Blaster Command. There is no preview mode and no apply step anywhere.

- Switches, segmented choices, presets, colour picks and profiles take effect on the click.
- Sliders and EQ points take effect while they are dragged, throttled, and send the final value on release.
- Feedback is a small status line in the window: what changed, in words ("Output: Headphones, volume 34",
  "Direct mode on · SBX off, SPDIF-Out Direct off"), and an **Undo** button (also Ctrl+Z). A drag speaks once, on
  release. The matching CLI flag is the line's tooltip. It is not a command preview.
- Undo sends the settings as they were before the change, at once. One step per control: a drag, held arrow keys or a
  run through a dropdown go back to where they started.
- A volume never jumps: its knob is dragged from where it is, and a press beside the knob moves one step (10) towards
  the pointer. The End key does not send a volume to 100. Other sliders (effect levels, EQ) still go to the pointer.
- A refused write: the control springs back to the device's value and a short error shows ("The G6 did not take the
  change. Nothing was changed."). In the mockup: switch on "Writes fail" in the bottom bar, then change anything.
- The G6 away: a banner with "Try again" over the page, a "Not connected" badge in the title bar, the device controls
  disabled (not only dimmed), the meters stopped. Pages, the settings dialog and the appearance settings keep working.
  In the mockup: switch off "G6 connected" in the bottom bar.

For the Rust build this needs two things:

- **A throttle per control.** A drag produces far more values than the device should get. Each control keeps its
  latest value and sends at most one write per interval (the mockup uses 80 ms for the status; Sound Blaster Command's
  own rate is worth measuring from a capture), then always sends the last value on release. Writes for one control
  must not queue up behind each other; the newest value replaces a waiting one.
- **A read-back to reconcile the UI.** After a write (and after a failed one) the app reads the setting back
  (`g6_status` for HID settings, Core Audio for volumes, mutes and the speaker setup) and shows what the device
  reports. That is what makes the spring-back honest, and it also catches changes that pull others with them
  (Direct mode turning SBX and Scout Mode off) and changes made on the device itself (the volume knob, the SBX button).
  Settings without a read-back (RGB colour) can only show the last value sent.

## Speaker setup

Stereo / 5.1 / 7.1, kept per output (Speakers and Headphones) as in the Python GUI's Speaker Layout section, and
applied on the click like everything else.

- A compact control under the master volume, for the current output. The same on the Mixer page's lower row.
- A "Speaker setup" card with one row per output on the Playback and decoder page.
- The output stage of the Signal chain shows the current setup.
- Windows sets it through the speaker setup of the G6's playback endpoint (what Sound Blaster Command changes; the
  Python app reads it from `PKEY_AudioEndpoint_PhysicalSpeakers` in `src/g6_cli/g6_os_audio.py`). Linux is not covered.

## The Mixer page

Three groups of vertical strips, each strip with a level meter, a fader, its value and a Mute button. The groups wrap
under each other when the window is narrow.

- Monitoring: Line In, External Mic, SPDIF In, Mic monitoring.
- Recording sources: Line In, External Mic, SPDIF In, What U Hear, Mic recording.
- Playback: the master strip for the current output with left and right meters, and the "Mute mix" switch.
- A lower row with what belongs beside the faders: output, speaker setup, mic boost, Direct mode.

A muted strip shows no level. The meters in the mockup are sample movement scaled by the fader. The Python app has
no per-source metering (only the playback peak for the music lighting effect, through Core Audio's
`IAudioMeterInformation`), so real meters are new work and Windows-only at first.

## The Signal chain page

A page of the same app, in the sidebar above Settings. It draws where the sound goes and what each stage is doing now.

- Playback: PC audio → Decoder → SBX → Equalizer → DAC → the current output. Direct mode is a bypass
  drawn over the SBX stage; its button switches Direct mode.
- Microphone and inputs: Mic in → Mic boost → Voice Clarity → Recording and Monitoring → Mixer.
- Device: Lighting and the G6 itself (firmware).
- Each stage shows its state in a line (preset name, "Bypassed", "Muted", volume and speaker setup) and a lamp.
- **A stage opens its page**: Decoder, DAC and the output open Playback and decoder; SBX, Equalizer,
  Lighting open their own pages; Mic boost and Voice Clarity open Microphone; Recording, Monitoring and Mixer open
  Mixer; G6 opens Settings. A page opened this way shows a "Signal chain" button in its header to go back; opening a
  page from the sidebar does not.
- Stages that have one switch carry it (SBX, Equalizer, Voice Clarity, Lighting). They and the Direct mode button
  apply at once, like every other control. The chain holds no settings of its own.
- Wide, a lane runs from left to right and its stages share the width. When the page area is narrower than 730 px,
  each lane is a column from top to bottom and the bypass runs down the right of the SBX stage. Nothing scrolls sideways.
- The keyboard follows: opening a stage puts the focus on the page's "Signal chain" button, and that button puts it
  back on the stage.

## The transparency settings of the toolkit

The toolkit has **one** transparency setting. There is no opacity slider, no blur amount and no per-surface option;
the mockups add none.

| Setting | Choices | Where it lives in markdown-shared-rs |
|---|---|---|
| **Window backdrop** | Off · Mica · Acrylic (default Off) | `crates/mdshared-app/src/backdrop.rs`: `Backdrop`, `TITLE`, `LABELS`, `HELP` ("Windows 11. Behind the bars and the side pane."), settings-file values `off` / `mica` / `acrylic` |
| Theme | System · Light · Dark | `crates/mdshared-ui/src/theme.rs` (`DARK`, `LIGHT`); the row `Row::choice(.., "Theme", &["System", "Light", "Dark"], ..)` in `settings.rs` and `specimen.rs` |
| Window corners | Square · Small · Round (0 / 4 / 8 px) | `crates/mdshared-ui/src/corners.rs`: `Corners`, `SETTING_LABEL`, `LABELS` |

How the backdrop works there, and how the mockups imitate it:

- `crates/mdshared-ui/src/frost.rs`: the chrome (title row, bars, side panes) is drawn in its flat colours, then one
  key colour is taken out again and becomes transparency (`key_colour`: the darkest of title, bar and pane in the dark
  palette, the lightest in the light one). The other two chrome colours stay as a faint lightening or darkening.
  `veil()` keeps part of the key colour over the backdrop: Off 255, Mica 0, Acrylic 150 of 255.
- Only chrome is frosted. Pages, cards, menus and dialogs stay opaque. Which regions are chrome is the application's
  choice: here the title row and the sidebar.
- The chrome is frosted only while the window has the keyboard (`Frost::sync`); otherwise it is flat.
- The system calls are `platform::set_backdrop` on Windows 11 and `blur.rs` (`org_kde_kwin_blur`) on KDE Plasma
  Wayland, where Mica and Acrylic are the same blur.

In the mockups (`_src/base.css`): the window gets `backdrop-filter` and the key colour with alpha; `.c-title`,
`.c-bar` and `.c-pane` get the leftover tints (dark: white at 3.7 %, 1.4 %, 0; light: black at 4.9 %, 0, 0.8 %,
worked out from the palette as `key_out` does). Mica is `blur(90px)` under 80 % of the key colour, Acrylic
`blur(28px)` under 62 %. These two alphas are estimates for the system's own tint plus the veil, not measured values.

The mockup has the three settings twice: in the app (the sliders button in the title bar opens the toolkit's
settings dialog; they are also on the Settings page) and in the bar at the bottom.
The bar also has four mockup-only switches: "Window focused" (to show the flat state), "G6 connected" (off: the app
without the device), "Writes fail" (on: a refused write) and "Wallpaper".

## Toolkit elements reused

| Element | Values | Source |
|---|---|---|
| Colours | Both palettes: page, title, bar, pane, text, muted, faint, hover, press, on, line, accent, on_accent, field, popup, popup_line, thumb, danger, warn, warn_bg | `theme.rs` |
| Fonts | `Segoe UI Variable Text, Segoe UI, Inter, Noto Sans, Cantarell, DejaVu Sans`; mono `Cascadia Mono, Consolas, Courier New` | `gfx.rs` |
| Title row | 40 high, caption buttons 46 wide with 10 px icons, close hover in `danger`; bars 44 high | `chrome.rs` |
| Window | corner radius 0 / 4 / 8; shadow extent 36, sigma 12, drop 6, peak 0.35 | `corners.rs`, `shadow.rs` |
| Toggle | 40 × 20 pill, knob 12 (13 hover, 14 pressed), accent when on | `settings.rs` `draw_toggle` |
| Segmented choice | 28 high, frame radius 6 in `field` / `line`, chosen segment radius 4 in accent, semibold | `settings/pages.rs`, `segments.rs` |
| Dropdown, stepper | 28 high, radius 6, `field` / `line` | `settings/pages.rs` |
| Buttons | radius 5; popup mixed 4 % with text, `popup_line` edge | `overlay.rs` `draw_dialog_button` |
| Rows | 34 high, 40 with a help line; label 13.5, help 11.5, control text 13; child indent 16 | `settings/pages.rs` |
| Side list | items 34 high, radius 6, `on` fill, accent mark 3 × 16 at 4 px | `settings/pages.rs` |
| Cards and dialogs | `popup` fill, `popup_line` edge, radius 10, header 40 with a ×, window dimmed 45 % dark / 28 % light | `settings/pages.rs`, `overlay.rs` |
| Focus ring | accent, 3 px out, 1.5 px of the ground between | `settings.rs` `ring` |
| Pane | head 40 with an 11 px semibold muted title | `pane.rs` |
| Notice bar, status line | warn colours; popup fill with `popup_line` edge | `notice.rs`, `overlay.rs` (toast) |
| Icons | path data on a 16 grid, stroke 1.25, round ends: close, minus, plus, right, warn, info, check, reload, save, folder, sliders, window buttons | `glyph.rs` |

Not in the toolkit, drawn new in its style (to be written for the Rust build):

- **Slider**, horizontal and vertical, with a centre zero for ±12 dB. `mdshared-ui` has a stepper but no slider.
  Drawn like the toggle: `field` track with the toggle's edge, accent fill, accent knob with a ring of the ground.
- **Channel strip** (vertical slider, level meter, mute button) and the strip groups of the Mixer page.
- **EQ curve** with draggable points, **colour swatches**, the **lighting ring** preview, the **stage boxes and
  wires** of the Signal chain page.
- Nine icons: speaker, muted, mic, bulb, wave, chip, mixer, chain, spark.
- Lit badges (SBX, SCOUT, DIRECT, "Not connected").

## Mapping to mdshared-ui for the Rust build

| Part | mdshared-ui |
|---|---|
| Window and chrome | `chrome` title row with `draw_win_button`; side pane (`pane::draw_head`, list rows as in `settings/pages.rs`); frost areas: title row and pane |
| Settings pages | Lists of `settings::Row`s grouped in cards: `Row::Toggle`, `Row::Choice` (segmented or dropdown), stepper, plus the new slider row. `Item::child` and `Item::disabled` (with the reason in the help slot) fit the "Direct mode is on" case |
| Mixer page | The new strip widget in three cards |
| Signal chain page | Stage boxes are cards used as buttons, wires are 2 px fills; a stage's click sets the page of the side list. No state of its own: it draws from the same values as the pages |

The settings dialog is `settings/pages.rs` (`Form`, `Page`, `Item`) with an Appearance and a Device page.
The status line is the toolkit's toast, kept to one short line.

## Mock behaviour that is not confirmed device behaviour

- Turning Direct mode off restores SBX to its earlier state. What the G6 does must be checked.
- Direct mode is drawn as bypassing only the SBX stage (Signal chain page). `doc/parity.md` leaves open whether the decoder mode and
  DAC filter apply while it is on.
- Profile names, mixer levels and the meters are invented samples.
- Picking a profile from the list loads it at once. "Load…" and "Save…" stand for the file dialogs.
- Direct mode and SPDIF-Out Direct turn each other off, as the Python GUI does it (`_set_direct_mode` in `gui/app.py`).
- Undo restores every device setting of the mockup's state table. The Rust build has to decide what it re-sends: the
  one changed setting and the ones it pulled with it.
- "Try again" in the banner always fails here.

## Editing

`01-sbc-sidebar.html` is generated. Sources are in `_src/`: `base.css` and `base.js` (look, state, controls and the
content of the pages) and `01-sbc-sidebar.html` (the window, the sidebar and the Signal chain page), listed in
`VARIANTS` of `build.py`. Rebuild with

```shell
python doc/mockups/_src/build.py
```

The script reads the 43 EQ presets from `src/g6_cli/g6_spec/eq_presets.py` by parsing the file; it imports and runs
nothing of `g6_cli`.

## Usability pass

A review of the mockup as a user would meet it: headless Chrome at 1280 and 380 px, light and dark, each backdrop,
every page clicked through, plus scripted checks of contrast (sampled from the rendered pixels), names, tab order and
hit areas. Findings are ranked by how much they would hurt in use.

### Found and changed

| # | Finding | Change |
|---|---|---|
| 1 | **No way back from a change.** Every click writes to the device, and nothing undid one. "Flat" or a preset wiped a hand-made EQ curve; a profile changed many settings in one click. | The status line has **Undo** (also Ctrl+Z). It restores the settings from before the change, one step per control. |
| 2 | **A volume could jump.** A press anywhere on a volume track sent that level (34 to 100 in one click), on the master and on every fader; End did the same from the keyboard. | A volume's knob is dragged from where it is; a press beside it moves one step of 10. End is not bound on volumes. Effect levels and EQ bands keep press-to-set. |
| 3 | **The master volume was locked while muted**, so it could not be turned down before unmuting. | It stays adjustable; the slider is drawn quiet and the value reads "Muted". |
| 4 | **A caption acted as a button.** In the Mixer's lower row a click on the word "Output" switched the output to Speakers (a `<label>` round a group of buttons activates the first one). | Captions of groups are plain text. Only a switch keeps a clickable label. |
| 5 | **Side effects were silent.** Direct mode turned SBX and Scout Mode off without a word; Scout Mode and SBX replaced each other the same way. Direct mode and SPDIF-Out Direct were not modelled as excluding each other. | The status line names them ("Direct mode on · SBX off, SPDIF-Out Direct off"), the help lines say it beforehand, and the exclusion is modelled. The output switch says what it now plays at ("Output: Headphones, volume 34"). |
| 6 | **The G6 away looked like the G6 present.** Controls stayed live, each try sprang back after 350 ms, and the only standing sign was a dot turning red. | A banner with "Try again", "Not connected" in words in the header, device controls disabled. A refused write is its own case with its own text ("Writes fail" in the bottom bar). |
| 7 | **Disabled was only a look.** Dimmed controls kept their place in the tab order (13 on the SBX page under Direct mode) and were not announced as unavailable. With SBX merely off, the five effect cards were dimmed with no reason given. | Controls under a switched-off parent are really disabled. A second notice explains "SBX is off" (or Scout Mode being on) with a "Turn on SBX" button, next to the Direct mode one. Period and "Music: cycle colours" say what they need. |
| 8 | **The Signal chain scrolled sideways** as soon as the page area was narrower than the playback lane, and at 380 px showed two and a half stages. | Stages share the lane's width; under 730 px of page width the lanes turn into columns. Checked at 380, 600, 1000 and 1280 px. |
| 9 | **Nothing had a name for a screen reader.** No slider, switch, dropdown or group of segments had one (24 unnamed controls on the Mixer page); mute buttons were the letter "M"; the SBX / SCOUT / DIRECT badges showed their state by colour only. | Every control is named from its row, card or strip ("Line In monitoring volume", "Mute mic recording"). Sliders have `aria-orientation` and `aria-valuetext` ("+3 dB"); badges read "SBX on"; meters, lamps and icons are hidden from it; the page list uses `aria-current="page"`; errors are an alert, confirmations a polite status. |
| 10 | **Narrow layout.** At 380 px: six of the eight pages were off-screen in a strip that scrolled sideways with no hint; the EQ preset row squeezed its label to "P…"; the EQ curve's labels were scaled to about 5 px; the mixer scrolled sideways. | The page list is a grid of all eight pages under short names (the full name stays the accessible one). Wide controls go under their label. The EQ curve is drawn at its real width. The mixer's groups wrap. No sideways scrolling is left at 380 px. |
| 11 | **Contrast.** EQ axis labels 3.1:1 (`faint` used for text). Over a backdrop the sidebar and title-bar text in `muted` fell to 1.8–3.4:1. | Text uses `muted` at least (5.5:1 dark, 6.5:1 light on cards). Over a backdrop the chrome writes its secondary text in the full text colour. All text passes 4.5:1 with the backdrop Off and with Mica in both themes, and with Acrylic in light. One case is left, see below. |
| 12 | **Small targets.** Switches were 40 × 20, small segments 20 high, swatches 22, the Direct mode button on the chain 20. | The drawings keep the toolkit's sizes; the areas that take the pointer are at least 24 × 24. With a coarse pointer (touch) rows and controls grow to 32–44 px. |
| 13 | **Focus.** The ring was a `box-shadow`, so it replaced a control's own shadow (the swatches lost their edge). The settings dialog did not hold the focus: Tab went on into the window behind it. Opening a page from a stage dropped the focus. | The ring is an outline of the same geometry; on a slider it is on the knob. The dialog takes the focus, keeps it, closes on Escape and gives it back. A stage hands the focus to the page's back button and gets it back. |
| 14 | **Sliders by keyboard** had arrows and Page keys only. | Arrows one step, Page keys a tenth of the range, Home the minimum, End the maximum (not on volumes). The ten EQ sliders are the keyboard's way to the curve; the curve's points stay pointer-only. |
| 15 | **The status line was noisy and stood outside the window.** It fired "Applying" up to 12 times a second during a drag, into a live region; it read "Applied --flag value" rather than what changed. | One line per change, in words, inside the window. Only its Undo button takes the pointer, so it never blocks a control under it. |
| 16 | **Names.** The page was "Acoustic Engine", its switch "SBX Acoustic Engine", its badge "SBX", its stage "Acoustic Engine". On Playback and decoder the volume sat inside the "Speaker setup" card. | "SBX Acoustic Engine" for the page, "SBX" for the switch, badge and stage. Playback and decoder opens with an Output card: output, volume and mute, then the speaker setup per output with the playing one marked "In use". "M" is "Mute". Line Out virtualization says which output it concerns. The EQ says that a preset has one curve per output and which one is shown. |

### Left as it is

- **Which page opens first: SBX Acoustic Engine.** Output, volume and speaker setup are in the header on every page,
  SBX is the first page and the EQ preset one click away, as in Sound Blaster Command. The Signal chain explains the
  state (why SBX is bypassed, where the sound goes) but puts every setting one click further away, so it is the
  better second stop than the first. Worth doing in the Rust build: reopen the page that was open last.
- **No confirmation before an output switch and no ramp of the volume.** Both would add a step between the click and
  the device, against the rule that every change applies at once. Undo and the "volume 34" in the status line are
  the guard. A short fade inside the write (down, switch, up) would not be a step for the user; that is a decision
  for the Rust build and needs a test on the device.
- **Dark theme with Acrylic over a bright wallpaper: 3.5–4.3:1** for the title-bar and sidebar text, under AA, even
  in the full text colour. It comes from the backdrop (62 % of the key colour here, an estimate), and the backdrop
  switches stay as the toolkit has them. To be measured on Windows 11; if it holds, the fix belongs in the toolkit
  (a stronger veil for the dark palette).
- **Edges of unselected controls** (segment frame 1.2:1, slider track 2.3:1 against a card) are toolkit colours.
  State is carried by the accent fill and the knob, which pass 3:1.
- **The focus ring is 1.5 px**, the toolkit's. Visible in both themes; thinner than the 2 px of WCAG 2.4.13 (AAA).
- **Segments are buttons, not a radio group.** Arrow keys in a radio group select as they move, which here would
  write every step to the device. Tab and Enter or Space make each write a deliberate one.
- **Two ways to the appearance settings** (the title-bar button's dialog and the Settings page). The dialog is the
  toolkit's; the button's tooltip now says what it opens.
- **"Microphone"** stays the page's name although Sound Blaster Command calls it Recording: the plain word is easier
  to find, and "Recording" is already a group of the Mixer.
- **Dropdowns apply each item passed with the arrow keys** (the browser's `<select>`), so walking the Profile list
  loads profiles on the way. Undo goes back to the start of the walk. The toolkit's dropdown opens a menu and
  commits on Enter; check that it stays so.
- **Two playback mutes on the Mixer** ("Mute mix" for `--mixer-playback-mute`, Mute on the strip for
  `--playback-mute`). Both flags exist; what the first does differently is not written down anywhere in the repo.
- **The Profile dropdown is not shown under 760 px**; the Settings page has the list.
- **Speaker setup** stays Windows-only, **Direct mode** stays drawn as bypassing only SBX, and the sample state still
  opens with Direct mode on, so the first page starts dimmed with its notice.

## Checked

Headless Chrome 1280 × 800 and 380 × 760, driven over the DevTools protocol with real mouse and key events.

- **Click-through, no JavaScript error:** every enabled button, switch, segment, dropdown, slider (drag and keys) and
  EQ point of every page, twice over (291 controls at 1280 px, 311 at 380 px). All 8 pages from the page list; each of
  the 12 stages opens its page, the focus goes to the back button and returns to the stage.
- **Behaviour, 260 checks, all pass:** a change applies at once; the status line names it and its side effects; Undo
  and Ctrl+Z restore (output, Direct mode with SPDIF-Out Direct and SBX together, a drag to its start, "Flat" back to
  the custom curve); a press on a volume track moves 34 to 44, not to the pointer; End leaves a volume alone; a
  click on a caption changes nothing; the volume stays adjustable while muted; the notices and the disabled state on
  the SBX page under Direct mode, SBX off and Scout Mode; a refused write springs back with its error; with the G6
  away the device controls are disabled and the appearance settings still work; the dialog keeps the focus and
  returns it; all values of Theme, Window backdrop and Window corners reach the page; Tab lands only on enabled
  controls, each with a ring; the window fits and nothing scrolls sideways.
- **Audits:** no control without a name, no pointer area under 24 × 24, no disabled control in the tab order, on any
  page at either width. Text contrast from the rendered pixels in both themes with Off, Mica and Acrylic over the
  three wallpapers: all at 4.5:1 or more except dark with Acrylic over the bright wallpaper (above).
- **Screenshots looked at:** all pages at both widths, light and dark, the three backdrops, the Signal chain at 380,
  600, 1000 and 1280 px, the G6 away, a refused write, the status line with Undo, a focused slider, and the touch
  sizes under touch emulation.

Not checked: a real screen reader (only the roles, names and states it would read), Windows high-contrast mode,
200 % text size, real touch input, Firefox and Safari, and the real Mica and Acrylic of Windows 11 (the mockup
imitates them in CSS with estimated tints).
