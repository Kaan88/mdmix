# mdmix

**An open-source control panel for the Creative Sound BlasterX G6 USB DAC and headphone amp, for Windows and
Linux.** It's an alternative to Sound Blaster Command: one small native app, no account, no background
services, no telemetry.

> **Status: in development.** The Rust app described below has no release yet. What works today is the Python
> CLI and GUI in this repository ([Use it today](#use-it-today)). mdmix is unofficial and not affiliated with,
> endorsed by or supported by Creative Technology Ltd.

## Use it today

The Python version can already set and read back nearly everything listed below, from a command line or a simple GUI.
It runs on Windows and Linux and needs Python 3.12+:

```shell
git clone https://github.com/Kaan88/mdmix && cd mdmix
python -m venv venv
pip install -e ".[gui]"
g6 --status                 # read everything back from the G6
g6 --eq Enabled --eq-preset Acoustic   # an SBC preset, for the current output
soundblaster-x-g6-gui
```

Activate the venv before `pip install`: `venv\Scripts\activate` on Windows, `. venv/bin/activate` on Linux.
Linux needs a udev rule first. The full guide, with every flag and what each one does, is in
[doc/python.md](doc/python.md).

## What it does

mdmix talks to the G6 over USB directly, the way Sound Blaster Command does. Every click takes effect at once.
Then mdmix reads the setting back from the device, so what you see is what the G6 is really doing. If the G6
refuses a change, the control springs back and tells you why.

| Page | Controls |
|---|---|
| **SBX Acoustic Engine** | SBX on/off, Surround, Crystalizer, Bass, Smart Volume, Dialog+, each with its level; Scout Mode |
| **Equalizer** | EQ on/off, preamp, 10 bands (31 Hz to 16 kHz), the 43 Sound Blaster Command presets with their speaker and headphone curves |
| **Playback and decoder** | Output (speakers / headphones), Direct mode, SPDIF-out direct mode, DAC filter, decoder mode, headphone virtualization to Line Out, speaker setup (Stereo, 5.1, 7.1) |
| **Mixer** | Live meters and faders for playback, monitoring and recording: Line In, External Mic, SPDIF In, What U Hear; mutes, mic boost |
| **Microphone** | Voice Clarity, noise reduction, echo cancellation, mic Smart Volume, mic EQ and its presets |
| **Lighting** | Logo light on/off and colour, volume-ring light, pulsate / colour cycle / music-reactive effects |
| **Signal chain** | The whole audio path on one page, from PC audio through decoder, SBX, EQ and DAC to the output, plus the mic path. Each stage switches on or off and opens its own page |
| **Settings** | Profiles (save and load all sound settings), firmware version, appearance (light / dark theme, Mica or Acrylic window backdrop, window corners) |

Ctrl+Z undoes the last change. Unplug the G6 and mdmix says so; plug it back in and it reads the device again.

## Platforms

- **Windows 10 and 11.** The main target. Volumes, mutes, meters and the speaker setup go through Windows Core
  Audio; everything else goes over HID to the device.
- **Linux.** Planned. HID settings will work without root, using a udev rule. Mixer volumes go through USB Audio
  Control, which means briefly detaching the kernel sound driver and cutting audio. mdmix will never do that
  without asking.

Tested on G6 firmware **2.1.250903.1324**. Firmware updates are out of scope: use Sound Blaster Command for those.

## Plan

| Step | Work |
|---|---|
| 1 | Protocol library (`g6-proto`), checked byte for byte against test vectors exported from the working Python version |
| 2 | `g6` command-line tool with the same flags and profile format as the Python CLI |
| 3 | Device and Windows audio access, read-only first |
| 4 | The window: every page shows the device's real state |
| 5 | Writing: immediate apply, read-back, undo, disconnect handling |
| 6 | Pages one by one: SBX, playback, microphone, EQ, mixer, lighting, signal chain |
| 7 | Speaker setup writing on Windows, then Linux, then release builds |

The interface is drawn with [mdshared-ui](https://github.com/Kaan88/markdown-shared-rs), the toolkit behind
[mdview](https://github.com/Kaan88/markdown-viewer-rs) and [mdedit](https://github.com/Kaan88/markdown-editor-rs).
It gives one small executable with no bundled runtime.

## Safety

Sending bad data to a USB device can, in theory, break it. mdmix only sends commands that were captured from
Sound Blaster Command and understood. It never sends guessed bytes, and it never touches firmware. Use it at your own
risk.

## Where it comes from

- The USB protocol was reverse-engineered by Nils Skowasch in
  [soundblaster-x-g6-cli](https://github.com/nils-skowasch/soundblaster-x-g6-cli). It was worked out from
  Wireshark / USBPcap captures.
- This repository continues that project, with its full history. It adds the GUI, status read-back, EQ presets,
  lighting effects, profiles and a hardware-free test suite. That Python code (`src/g6_cli`) is the reference the
  Rust version is tested against: `tests/vectors/` records exactly what it sends.
- Design and plans: the [GUI mockup](doc/mockups/01-sbc-sidebar.html), the [GUI plan](doc/gui-plan.md), the
  [Rust port plan](doc/rust-port.md) and the [USB protocol notes](doc/usb-spec.md).

## Licence

GPL-2.0-only, the same as the project it derives from. See [LICENSE](LICENSE).

*Sound Blaster, Sound BlasterX, SBX and Sound Blaster Command are trademarks of Creative Technology Ltd. They're used
here only to say which device this software works with.*