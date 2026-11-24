# Rust port

A port of the Python `soundblaster-x-g6-cli` (`src/g6_cli/`), following `doc/rust-port.md`. So far it covers
phases 1 and 2 (the protocol and the command line) and the HID half of phase 3 (device access on Windows).

- `g6-proto`: packet builders, value ranges, EQ presets, lighting effects, status parsers. No I/O.
- `g6-device`: the G6 over HID through `hidapi`. It finds interface 4 the way Python does, sends `00` + 64-byte
  reports, and runs the status queries (500 ms read timeout, as in Python). USB Audio Control isn't in it yet.
- `g6-osaudio`: read-only volumes, mutes, speaker setup and output peak meter from Windows Core Audio (`None` on
  other platforms). The only crate with `unsafe`.
- `g6-cli`: the `g6` binary. Same flags as the Python CLI. Without `--dry-run` it uses the G6 over HID; settings
  that go over USB Audio Control (volumes, mutes, speaker layout, `--claim-and-release`) stop with an error.
- `g6-gui`: the real GUI, drawn with the user's `mdshared-ui` toolkit (step G2 of `doc/gui-plan.md`). It reads the G6 (HID and the Windows mixer) on
  a worker thread and shows every page; it writes nothing yet. `cargo run --release -p g6-gui`, or
  `-- --shots target/g6-gui-shots` for pictures of every page without a window or a device.
- `g6-testui`: a **throwaway** egui test harness for trying the port on a real G6 by hand. Every control writes at
  once. It is not the real GUI; see `g6-testui/README.md`.

## Build and test

From this directory:

```sh
cargo build --release                 # target/release/g6(.exe)
cargo build --release -p g6-testui    # target/release/g6-testui(.exe)
cargo test --release --workspace
cargo fmt --all --check
cargo clippy --release --workspace --all-targets -- -D warnings
```

The tests need no hardware and never open a device: they run on recording and transcript transports, never
`SystemEnv` or `HidTransport`. They compare the port with the Python code through two sets of files:

- `../tests/vectors/*.json`: the bytes every command sends, lighting effect frames, EQ presets, a status
  transcript. They're shared with the Python test suite. After an intended change to what the Python code sends,
  regenerate them from the repository root with `python tools/export_vectors.py` (not `--live`, which reads a real
  G6) and commit the JSON.
- `g6-cli/tests/fixtures/python_output.json`: what the Python CLI prints (`--status` text and JSON, dry runs with stdout, stderr and exit code,
  profiles, option definitions, float formatting, how argparse reads awkward command lines). Regenerate it from
  the repository root with:

  ```sh
  python tools/rust_fixtures.py
  ```

  It runs the Python code with stub HID/USB libraries and transcript answers, so it never touches a device.
  Re-run it after changing the Python CLI's output or options, then run the Rust tests.

## Try it

```sh
cargo run --release -- --dry-run --debug --sbx Enabled --eq-band 1k -2.5
```

A dry run looks for the G6 (read only) but never writes to it. Without `--dry-run` the command writes to the device. Reading is always
safe:

```sh
cargo run --release -- --status          # or --status --json
```

On Windows `--status` also shows the mixer (volumes, monitoring, speaker setup) from `g6-osaudio`; elsewhere `mixer` is `null`.
