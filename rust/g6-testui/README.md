# g6-testui: throwaway test harness

**This is not the real GUI.** It's a quick egui window for trying the Rust port on a real G6 by hand. The real
GUI will be built on mdshared-ui (`doc/gui-plan.md`). Delete this crate once `g6-gui` exists. It isn't meant to look
good.

**Every control writes to the G6 as soon as you click it.** Sliders write when you let go. There is no dry run.

## Run

From `rust/`:

```sh
cargo run --release -p g6-testui
```

The binary is `target/release/g6-testui(.exe)`.

## What it does

- On start, and when you press **Refresh**, it reads the full status over HID (the same queries as `g6 --status`).
  Fields go in a grid; the JSON (`g6 --status --json`, with `mixer` always `null`) is under a collapsible header.
- If no G6 is found it says so, and **Retry** looks again.
- Controls: output Speakers/Headphones, SBX, Scout Mode, each SBX effect's switch and level, EQ on/off, EQ preset
  (with the output whose curve it applies), Direct mode, DAC filter, decoder mode, and logo lighting (colour + on,
  off).
- Each control builds a `g6_proto::apply::Request` with one setting and sends it the way the `g6` CLI does:
  `Request::actions`, then `Action::batches`, then `HidTransport::send_hid`. No command bytes are made up here.
  After each write it reads the status back.
- The log at the bottom shows what the transport prints with `--debug`: each report as `00` + 64 bytes in hex, and
  the device's responses.
- Device I/O runs on a worker thread, one job at a time. The controls are disabled while a job runs.

HID only: none of these controls needs USB Audio Control, which isn't ported yet.
