#!/bin/bash
# Run the CLI from this checkout (prefers ./venv if present). Usage: shell/g6.sh <cli args...>
root="$(cd "$(dirname "$0")/.." && pwd)"
python_bin="$root/venv/bin/python"
[ -x "$python_bin" ] || python_bin="${PYTHON:-python3}"
PYTHONPATH="$root/src${PYTHONPATH:+:$PYTHONPATH}" exec "$python_bin" -m g6_cli "$@"
