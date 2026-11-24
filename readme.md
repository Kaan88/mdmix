# SoundBlaster X G6 control

Control **and read back** a Creative SoundBlaster X G6 from a CLI or a GUI, on Linux and Windows, without
SoundBlaster Command. The USB protocol was worked out from Wireshark/USBPcap captures (see [doc/](doc/) and
[payloads/raw/](payloads/raw/)).

> **Use at your own risk.** Sending bad data to a USB device can, in theory, break it. Everything here is tested only
> on firmware **2.1.250903.1324**.

Based on the original [soundblaster-x-g6-cli](https://github.com/nils-skowasch/soundblaster-x-g6-cli) by Nils
Skowasch (GPL-2.0). This fork adds the GUI, device status readback, volume-ring control, and a hardware-free test
suite.

## What works

| Area | Set | Read back | Transport |
|---|---|---|---|
| Output (speakers / headphones) | yes | yes | HID |
| Direct mode, SPDIF-out direct mode | yes | yes | HID |
| DAC filter, decoder mode | yes | yes (reads 0 while Direct mode is on) | HID |
| SBX master switch, Scout Mode | yes | yes | HID |
| SBX effects (surround, crystalizer, bass, smart volume, dialog+) | yes | yes | HID |
| Equalizer (on/off, preamp, 10 bands) | yes | yes | HID |
| EQ presets (SBC's 43, speaker and headphone curves) | yes (`--eq-preset`) | as the band values | HID |
| Mic boost | yes | yes | HID |
| Voice clarity, noise reduction, AEC, mic smart volume, mic EQ + preset | yes | yes | HID |
| Headphone virtualization to Line Out | yes | yes | HID |
| RGB light on/off, volume-ring light | yes | yes | HID |
| RGB colour | yes | – (SBC can't either) | HID |
| RGB effects pulsate, cycle, music reactive | yes, host-side at 30 fps (`--lighting-effect`, GUI); music: Windows | – | HID |
| Profiles (save / load the sound settings) | yes (`--profile-save`, `--profile-load`, GUI) | – | HID |
| Firmware version | – | yes | HID |
| Volumes / mutes / mixer | yes | Windows (Core Audio), Linux (amixer, untested on hardware) | USB Audio Control (needs claim on Linux) |
| Speaker layout | yes | Windows (speaker setup) | USB Audio Control |

**HID** commands just work. **Audio Control** commands need the kernel sound driver detached on Linux
(`--claim-and-release`, then audio is restored). See [Linux audio commands](#linux-audio-commands).

## Install

Python 3.12+.

```shell
git clone <this repo> && cd soundblaster-x-g6-cli
python -m venv venv
# Linux: . venv/bin/activate     Windows: venv\Scripts\activate
pip install -e ".[gui,test]"
```

This installs three commands: `g6` (short for `soundblaster-x-g6-cli`), `soundblaster-x-g6-cli`, and
`soundblaster-x-g6-gui`. Without installing, use `python -m g6_cli` with `src` on `PYTHONPATH`. You can also
double-click `g6_gui.pyw`, or run `shell/g6.sh` on Linux.

### Linux setup

```shell
# USB access for your user
echo 'SUBSYSTEM=="usb", ATTRS{idVendor}=="041e", ATTRS{idProduct}=="3256", TAG+="uaccess"' \
  | sudo tee /etc/udev/rules.d/50-soundblaster-x-g6.rules
sudo udevadm trigger

# libusb
sudo apt-get install -y libusb-1.0-0

# optional: lets --reload-audio-services run without a password prompt
echo "$USER ALL=(ALL:ALL) NOPASSWD: /usr/sbin/alsa force-reload" | sudo tee /etc/sudoers.d/50-soundblaster-x-g6
```

### Windows setup

HID commands and status readback work out of the box. Audio Control commands need `libusb-1.0.dll` on `PATH`; it is
in the `libusb` package on PyPI, under `src/libusb/_platform/_windows/x64/`.

## CLI

```shell
g6 --status                 # read the device state (read-only)
g6 --status --json          # same, as JSON (for scripts)
g6 --set-output Headphones
g6 --toggle-output
g6 --sbx-surround Enabled --sbx-surround-value 40
g6 --scout-mode Disabled --sbx Enabled
g6 --eq Enabled --eq-preamp -2 --eq-band 1k 1.5 --eq-band 16k -3
g6 --eq Enabled --eq-preset "hip hop"   # SBC's preset, curve for the current output
g6 --lighting-volume-ring Disabled
g6 --lighting-effect pulsate --lighting-rgb 0 0 255   # animate from this PC until Ctrl+C (the G6 stores no effect)
g6 --lighting-effect music                           # brightness follows what the G6 plays (Windows)
g6 --profile-save gaming.json                        # current sound settings to a file
g6 --profile-load gaming.json                        # and back
g6 --dry-run --debug --playback-filter SLOW_ROLL_OFF_LINEAR_PHASE   # print the packets, send nothing
g6 --help                   # every option
```

Exit code is `0` on success and `1` on errors such as a missing device or a bad argument, with a one-line message
instead of a traceback. `--dry-run` works without a device connected.

### Linux audio commands

Volume, mute, mixer and speaker-layout commands go through the USB Audio Control interface, which
`snd_usb_audio` owns:

```shell
g6 --claim-and-release --playback-volume 40 --reload-audio-services
```

The GUI does this automatically. With "modprobe (recommended)" checked it runs: stop PipeWire →
`modprobe -r snd_usb_audio` → command → `modprobe snd_usb_audio` → start PipeWire.

## GUI

`soundblaster-x-g6-gui`, or double-click `g6_gui.pyw`. It reads the device state on start (Refresh to re-read).
Changes run through the CLI in a subprocess, and every command shows up in the output console. Use **Dry Run** to see
what would be sent without sending it.

## Development

```
src/g6_cli/
  __init__.py      CLI (argparse) -> G6Api
  g6_api/          one method per feature
  g6_spec/         packet builders (pure, no I/O)
  g6_core.py       USB/HID transport (pyusb + hidapi)
  g6_status.py     status queries + response parsers
  g6_capture.py    pcapng/USBPcap reader -> protocol events
  g6_probe.py      g6-probe: capture -> queries -> snapshot -> diff (reverse-engineering helper)
  gui/
    controls.py    declarative tables of GUI controls -> CLI flags
    runner.py      runs CLI commands (+ Linux driver unload/reload), no UI code
    app.py         CustomTkinter window
tests/             pytest; real devices are hidden from every test unless it is marked `hardware`
legacy/            pre-restructure scripts, kept for reference (not packaged)
doc/, payloads/    protocol notes and raw captures
```

```shell
python -m pytest                                  # full suite, no hardware needed (CI runs this)
python -m pytest -m hardware tests/hardware_test.py   # read-only checks against a connected G6
```

`tests/g6_cli/g6_api/api_real_test.py` is also marked `hardware`. It claims the interface and reloads ALSA, so run it
by hand only.

Parity with SoundBlaster Command, and the capture/snapshot/diff workflow: [doc/parity.md](doc/parity.md).

Adding a feature: capture it in Wireshark → add a builder in `g6_spec/` → an `G6Api` method → a CLI flag → a row in
`gui/controls.py` (the controls test checks the flag exists) → tests. For readback, add a query and parser to
`g6_status.py` with the captured response as a test fixture.

## License

GPL-2.0-only, see [LICENSE](LICENSE).
