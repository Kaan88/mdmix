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
| SBX Bass on/off + level | ✅ | ✅ | HID | as above; `9617` = 80.0 is probably the bass crossover (Hz), unconfirmed |
| SBX Smart Volume on/off + level + Night/Loud | ✅ | ✅ | HID | mode = `9606` (0 Normal, 1 Loud, 2 Night) |
| SBX Dialog+ on/off + level | ✅ | ✅ | HID | as above |
| SBX master switch | ❓ | ❌ | HID | per-effect switches read 0 with it off, but no own readback found; capture toggling only the master |
| Mic boost 0/10/20/30 dB | ✅ | ✅ | HID | `5a3c0201` → `5a3c0401 00 02 <dB>` |
| Voice clarity on/off, noise reduction, AEC, smart volume | ✅ | 🟡 | HID | SBC reads `5a110301 95<id>` at startup (ids 04, 05, 0a–1b, 2c, 13); capture A/B with recording settings changed |
| Mic EQ on/off + presets | ✅ | 🟡 | HID | probably `95 0b–0d` (400/1400/2000 = frequencies?) and `95 14–1b` (gains); same A/B capture |
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
| Graphic equalizer on/off, preamp, 10 bands | ❌ | ✅ | HID | read `96 09` / `96 0a` / `96 0b–14` (dB float32). Set: very likely `5a1207 0196 <id> <float>` like SBX; capture one band move to confirm before adding |
| EQ presets | – | – | – | app-side: a preset is just 10 band values |
| Scout Mode | ❌ | 🟡 | HID | SBC writes `5a2605 07…` (blocked from replay); `5a260308ffff` answer changed 01→02 with it on. Capture an on/off pair |
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
