# Rust port

A port of the Python `soundblaster-x-g6-cli` (`src/g6_cli/`), following `doc/rust-port.md`. So far it covers
phases 1 and 2: the protocol and the command line, without device access.

- `g6-proto`: packet builders, value ranges, EQ presets, lighting effects, status parsers. No I/O.
- `g6-cli`: the `g6` binary. Same flags as the Python CLI. Without a device layer it only completes `--dry-run`;
  anything that needs the G6 (including `--status` and `--profile-save`) says device access isn't built yet.

## Build and test

From this directory:

```sh
cargo build --release                 # target/release/g6(.exe)
cargo test --release --workspace
cargo fmt --all --check
cargo clippy --release --workspace --all-targets -- -D warnings
```

The tests need no hardware. They compare the port with the Python code through two sets of files:

- `../tests/vectors/*.json`: the bytes every command sends, lighting effect frames, EQ presets, a status
  transcript. They're shared with the Python test suite. After an intended change to what the Python code sends,
  regenerate them from the repository root with `python tools/export_vectors.py` (not `--live`, which reads a real
  G6) and commit the JSON.
- `g6-cli/tests/fixtures/python_output.json`: what the Python CLI prints (`--status` text and JSON, dry runs,
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

Leave out `--dry-run` and it stops with the "isn't built yet" message, without looking for a device.
