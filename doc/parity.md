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
| DAC filter | ✅ | 🟡 | HID | reads `0x00` while Direct mode is on: capture with it off |
| Decoder mode (Normal/Full/Night) | ✅ | 🟡 | HID | same as DAC filter |
| SBX Surround on/off + level | ✅ | ❌ | HID | try reading with the commit form `5a1103 0196 <feature>`; snapshot/diff |
| SBX Crystalizer on/off + level | ✅ | ❌ | HID | as above |
| SBX Bass on/off + level | ✅ | ❌ | HID | as above |
| SBX Smart Volume on/off + level + Night/Loud | ✅ | ❌ | HID | as above |
| SBX Dialog+ on/off + level | ✅ | ❌ | HID | as above |
| Mic boost 0/10/20/30 dB | ✅ | 🟡 | HID | **`5a3c0201` → `5a3c0401 00 02 <dB>` seen in captures**; add to `g6_status` |
| Voice clarity on/off, noise reduction, AEC, smart volume | ✅ | ❌ | HID | recording family `0195`; snapshot/diff |
| Mic EQ on/off + presets | ✅ | ❌ | HID | |
| RGB light on/off | ✅ | ✅ | HID | |
| RGB static colour | ✅ | ❌ | HID | |
| RGB effects (breathing, cycle, …) | ❌ | ❌ | HID | ❓ capture each effect |
| Volume ring light | ✅ | ✅ | HID | |
| Playback volume L/R | ✅ | ❌ | UAC | feature unit 1, selector 0x02, 1/256 dB. **Readable with standard GET_CUR (0x81)**, no RE |
| Playback mute | ✅ | ❌ | UAC | feature unit, selector 0x01: GET_CUR |
| Mic recording volume / mute | ✅ | ❌ | UAC | GET_CUR on its feature unit |
| Mic monitoring volume / mute | ✅ | ❌ | UAC | GET_CUR |
| Mixer monitoring / recording sources | ✅ | ❌ | UAC | GET_CUR per source |
| Speaker layout (stereo / 5.1 / 7.1) | ✅ | ❌ | UAC | ❓ may live in Windows driver settings, not on the device |
| Graphic equalizer (bands, presets) | ❌ | ❌ | HID? | ❓ capture moving one band at a time; check the float encoding |
| Scout Mode | ❌ | ❌ | HID? | ❓ |
| Headphone gain / impedance | ❌ | ❌ | ? | ❓ may be the hardware switch only |
| Profiles (save / load) | ❌ | – | – | probably app-side: a bundle of the settings above, no own protocol |
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
