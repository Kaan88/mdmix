# Parity with SoundBlaster Command

Goal: everything SoundBlaster Command (SBC) can set or show on the G6, this project can too.

Key: ✅ done · 🟡 partly / evidence exists · ❌ missing · ❓ check whether SBC has it

Transport: **HID** = 64-byte reports on interface 4. **UAC** = standard USB Audio Class requests to the
AudioControl interface (needs the kernel driver detached on Linux).

## Checklist

| Feature | Set | Read back | Transport | Notes / next step |
|---|---|---|---|---|
| Output speakers / headphones | ✅ | ✅ | HID | |
| Direct mode | ✅ | ✅ | HID | flags byte A bit 0x20 |
| SPDIF-out direct mode | ✅ | ✅ | HID | flags byte B bit 0x20 |
| DAC filter | ✅ | ✅ | HID | confirmed Fast→Slow min. phase in g6-sbc-startup-a/b (reads `0x00` only while Direct mode is on) |
| Decoder mode (Normal/Full/Night) | ✅ | ✅ | HID | confirmed Normal→Night in g6-sbc-startup-a/b |
| SBX Surround on/off + level | ✅ | ✅ | HID | `5a110301 96<id>` float32, ids = `AudioFeature` |
| SBX Crystalizer on/off + level | ✅ | ✅ | HID | as above |
| SBX Bass on/off + level | ✅ | ✅ | HID | as above. `9617` = 80.0 is the bass crossover in Hz (confirmed): SBC's profiles store `"Bass": {…, "XOver": 80, "SubWooferGain": false}` (all 64 at 80), the G6 DLL has `XBassCrossOverFrequencyEffectParameterId` and custom min/max, and in g6-sbc-session-c SBC writes `5a1207 0196 17 0000a042` (80.0) when it reloads the headphone settings on an output switch. The crossover control only appears for speaker setups with a subwoofer, which the G6 doesn't have, so SBC never sends anything but 80; no setter (any other value would be a guess) |
| SBX Smart Volume on/off + level + Night/Loud | ✅ | ✅ | HID | mode = `9606` (0 Normal, 1 Loud, 2 Night) |
| SBX Dialog+ on/off + level | ✅ | ✅ | HID | as above |
| SBX master switch | ✅ | ✅ | HID | write `5a2605 07 01 00 <on>`; mode register `5a260308ffff` → bit 0x01 (g6-sbc-session-c). Excludes Scout Mode |
| Mic boost 0/10/20/30 dB | ✅ | ✅ | HID | `5a3c0201` → `5a3c0401 00 02 <dB>` |
| Voice clarity on/off, noise reduction, AEC, smart volume | ✅ | ✅ | HID | `95 04` NR (what `--recording-voice-clarity` writes), `95 05` NR level, `95 00` AEC, `95 2c` smart volume; Voice Clarity = mode bit 0x04, no own write (SBC turns it off with the last effect) |
| Mic EQ on/off + presets | ✅ | ✅ | HID | `95 13` switch, `95 14–1b` gains matched to `MicrophoneEqualizerPreset` (PRESET_1/2 band `17` fixed: 2 dB, was ~0) |
| RGB light on/off | ✅ | ✅ | HID | |
| RGB static colour | ✅ | – | HID | SBC never reads the colour back either: its startup queries (`5a3a01…`, `5a3a02…`, `5a3a03…`) don't contain it, SBC keeps it host-side |
| RGB effects pulsate, cycle | ✅ | – | HID | **host-side**, like SBC: `--lighting-effect {pulsate,cycle}` streams frames `5a3a020601`, `5a3a06040003010001`, `5a3a090a00030101 <brightness> <b> <g> <r>` (the byte `lighting_enable_set_rgb` fixed to `ff` is the brightness). Pulsate ramps brightness, cycle crossfades red→blue→green. Runs in the foreground at 30 frames/s (the HID handle stays open for the run: `HidInterface.held_open`). GUI: Lighting tab → Lighting Effects, kept running by the window |
| RGB effect music reactive | ✅ | – | HID | host-side, own design (SBC sent no frames in the capture, nothing was playing): `--lighting-effect music`, peak level of the G6 playback endpoint (Core Audio `IAudioMeterInformation`, Windows only) on a −45…0 dB scale → brightness, instant attack, 0.25 s decay; the `--lighting-rgb` colour or a 12 s colour cycle. Linux: ❌ (no meter yet) |
| RGB effect solo | ✅ | – | HID | = static colour (`--lighting-rgb`) |
| Volume ring light (SBC: LED indicator) | ✅ | ✅ | HID | write `5a3903 00 0e <00 on / 01 off>` |
| Playback volume L/R | ✅ | 🟡 | UAC | Windows: read through Core Audio (`g6_os_audio`), since libusb can't GET_CUR an interface the audio driver owns. Linux: `amixer -c <G6> scontents`, parser unit-tested only, the G6's ALSA control names are unconfirmed |
| Playback mute | ✅ | 🟡 | UAC | as above |
| Mic recording volume / mute | ✅ | 🟡 | UAC | as above (recording endpoints) |
| Mic monitoring volume / mute | ✅ | 🟡 | UAC | as above (playback-path topology parts: Microphone = External Mic) |
| Mixer monitoring / recording sources | ✅ | 🟡 | UAC | as above; Auxiliary = Line In; SPDIF In's level part is named "External Input Recording Mute" by the driver (unconfirmed) |
| Speaker layout (stereo / 5.1 / 7.1) | ✅ | 🟡 | UAC | Windows: the speaker setup of the playback endpoint (`PKEY_AudioEndpoint_PhysicalSpeakers` channel mask, `0x3` = Stereo); a driver setting, not a device register. Linux: ❌ |
| Speaker type (desktop / bookshelf) | – | – | – | app-side only: no write at all between the Bookshelf and Desktop clicks in g6-sbc-session-c (the next write is the Line Out checkbox), and SBC stores it in none of its files. It's a label in SBC's speaker dialog, nothing reaches the G6 |
| Headphone virtualization to Line Out | ✅ | ✅ | HID | `5a3903 00 0a <01/00>` (Direct-mode family); readback = flags byte B bit 0x04 (snapshot diff: `5a39050120400004` off, `…20440004` on) |
| Graphic equalizer on/off, preamp, 10 bands | ✅ | ✅ | HID | `5a1207 0196 <09 / 0a / 0b–14> <float32>` + commit, confirmed in g6-sbc-session-c |
| EQ presets | ✅ | via the bands | HID | app-side: `--eq-preset NAME` writes preamp + 10 bands from SBC's own preset files (`%ProgramData%/Creative/SBCommand/Product/G6/Presets/EQ`, 43 presets, separate speaker/headphone curves; Rock and Hip Hop match g6-sbc-session-c). The G6 stores no preset name |
| Scout Mode | ✅ | ✅ | HID | write `5a2605 07 02 00 <on>`; mode register bit 0x02 |
| Headphone gain / impedance | – | – | – | hardware switch only; SBC's headphone dialog has no gain setting |
| Profiles (save / load) | ✅ | – | HID | ours, not SBC's file format: `--profile-save FILE` stores the HID settings above as CLI flags in JSON, `--profile-load FILE` applies them (only known setting flags accepted). Not included: output, RGB colour (no readback), volumes, speaker layout. GUI: System tab → Profiles |
| Firmware version | – | ✅ | HID | |
| Firmware update | ✗ | – | – | **out of scope**: this is the one place you can brick the card |

## Tools

`g6-probe` (or `python -m g6_cli.g6_probe` with `src` on `PYTHONPATH`) works on Wireshark/USBPcap `.pcapng` files
directly. tshark is not needed.

```shell
g6-probe events  capture.pcapng [--hid-only]     # timeline: HID ->/<-, UAC SET_CUR/GET_CUR, writes flagged
g6-probe queries capture.pcapng -o queries.txt   # distinct host->device HID payloads, writes commented out
g6-probe snapshot -q queries.txt -o a.json -l "surround 40"   # replay reads on the G6, save responses
g6-probe diff a.json b.json                      # changed bytes + float32 guesses
```

**Safety:** `snapshot` only sends lines from your query file (default: the known status queries), and refuses
anything matching a known write prefix (`g6_capture.KNOWN_WRITE_PREFIXES`). Never add guessed command bytes to a
query file. Only use payloads SBC itself sent.

## Procedure for new readback

1. **Capture SBC starting up.** Close SBC, start a USBPcap capture on the G6's root hub, start SBC, wait until the
   UI is populated, then stop the capture. While starting, SBC reads (almost) the whole device state.
2. `g6-probe queries startup.pcapng -o doc/queries-startup.txt`, then **review the file**: keep the `5a XX 01 01 …`
   style lines (the read/commit form) and delete anything unclear.
3. With SBC closed, `g6-probe snapshot -q doc/queries-startup.txt -o s0.json -l baseline`.
4. Change **one** setting (with SBC or `g6`), close SBC again, snapshot `s1.json`, then `g6-probe diff s0.json s1.json`.
   The changed offset is where that setting lives. Repeat with 2–3 values to confirm the encoding (most values are
   little-endian float32 0..1, or small codes).
5. Add the query and parser to `g6_status.py`, with the captured response bytes as a test in
   `tests/g6_cli/status_test.py`.

## Procedure for a new command

1. Capture SBC while changing **one** control slowly through a few values (one capture per control, named like the
   existing `payloads/raw/` files).
2. `g6-probe events capture.pcapng`: the `[WRITE]`-flagged or new `HID ->` lines and `UAC SET_CUR` lines are the
   command. The device's `5a020a…` reply is an ACK.
3. Add a builder in `g6_spec/`, then an `G6Api` method, a CLI flag, a row in `gui/controls.py`, and tests. If the new
   command has a data prefix, add it to `KNOWN_WRITE_PREFIXES`.

## When to disassemble instead

Only when captures stall: an encoding that doesn't make sense after a few captures, or behaviour SBC only shows in
rare situations. Captures cover everything else.
