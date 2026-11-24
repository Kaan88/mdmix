"""
Print a capture's G6 protocol events grouped by the steps in its .markers file (written by tools/mark.ps1).

    python tools/split_by_markers.py payloads/raw/g6-sbc-session-c.pcap [--all]

Without --all, plain read queries (non-writes) and their answers are hidden, to show what each step changed.
"""
from __future__ import annotations

import argparse
import bisect
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))

from g6_cli.g6_capture import decode, read_packets, trim  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument('capture')
    ap.add_argument('--all', action='store_true', help='also show reads')
    args = ap.parse_args()

    markers = []
    for line in Path(args.capture + '.markers').read_text(encoding='utf-8').splitlines():
        ts, label = line.split('\t', 1)
        markers.append((float(ts), label))
    starts = [m[0] for m in markers]

    packets = list(read_packets(args.capture))
    abs_ts = {p.index: p.ts for p in packets}
    events = decode(packets)

    current = None
    for ev in events:
        t = abs_ts[ev.index]
        seg = bisect.bisect_right(starts, t) - 1
        if seg != current:
            current = seg
            label = markers[seg][1] if seg >= 0 else '(before first marker)'
            print(f'\n=== [{seg}] {label}')
        if ev.kind == 'hid_in':
            continue
        if ev.kind == 'hid_out' and not ev.is_write and not args.all:
            continue
        rel = t - (starts[seg] if seg >= 0 else t)
        resp = f'  -> {trim(ev.response).hex()}' if ev.kind == 'hid_out' and ev.response else ''
        print(f'  +{rel:6.2f}s {ev.describe().split("  ", 1)[1].strip()}{resp}')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
