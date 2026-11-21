"""
g6-probe: reverse-engineering helper for finding new G6 readback and commands.

  g6-probe events  CAPTURE.pcapng            decoded timeline (HID + USB Audio requests)
  g6-probe queries CAPTURE.pcapng -o FILE    distinct HID payloads SoundBlaster Command sent -> query file
  g6-probe snapshot [-q FILE] -o SNAP.json   replay read queries on the connected G6, save responses
  g6-probe diff A.json B.json                which response bytes changed between two snapshots

Typical loop (see doc/parity.md): capture SoundBlaster Command starting up -> `queries` -> review the file ->
snapshot -> change ONE setting -> snapshot -> diff.

Safety: `snapshot` only sends payloads from the query file (default: the known status queries) and refuses
anything matching a known write prefix. It never scans unknown commands.
"""
from __future__ import annotations

import argparse
import json
import math
import struct
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

from g6_cli import g6_status
from g6_cli.g6_capture import is_known_write, load_events, trim, unique_hid_queries


def default_queries() -> list[bytes]:
    qs = [g6_status.Q_FIRMWARE, g6_status.Q_OUTPUT, *g6_status.Q_MODE_FLAGS, g6_status.Q_DECODER,
          g6_status.Q_DAC_FILTER, *g6_status.Q_RGB, *g6_status.Q_RING]
    return list(dict.fromkeys(qs))


# --- query files --------------------------------------------------------------------------------------------------

def parse_query_file(text: str) -> list[bytes]:
    """One hex payload per line; '#' starts a comment. Raises ValueError on bad or unsafe lines."""
    queries: list[bytes] = []
    for n, raw in enumerate(text.splitlines(), 1):
        line = raw.split('#', 1)[0].strip().replace(' ', '')
        if not line:
            continue
        try:
            payload = bytes.fromhex(line)
        except ValueError:
            raise ValueError(f'line {n}: not hex: {raw!r}') from None
        check_query_safe(payload, where=f'line {n}')
        queries.append(payload)
    return list(dict.fromkeys(queries))


def check_query_safe(payload: bytes, where: str = '') -> None:
    prefix = f'{where}: ' if where else ''
    if not payload.startswith(b'\x5a'):
        raise ValueError(f'{prefix}{payload.hex()} does not start with 5a')
    if len(payload) > g6_status.HID_REPORT_SIZE:
        raise ValueError(f'{prefix}{payload.hex()} is longer than 64 bytes')
    if is_known_write(payload):
        raise ValueError(f'{prefix}{payload.hex()} matches a known WRITE command; refusing to replay it')


def format_query_file(queries, source: str) -> str:
    lines = [
        f'# HID payloads sent by the host in {source}',
        '# Review before use: only uncommented lines are sent by `g6-probe snapshot`.',
        '# Known writes are commented out and refused anyway. Payloads of the form 5a XX 01 01 are the',
        '# "commit"/read form seen throughout the protocol. Delete anything you are not sure about.',
        '',
    ]
    for q in queries:
        resp = q.first_response.hex() if q.first_response else '-'
        note = f'  # x{q.count}  -> {resp}'
        lines.append(f'# WRITE {q.payload.hex()}{note}' if q.is_write else f'{q.payload.hex()}{note}')
    return '\n'.join(lines) + '\n'


# --- snapshots ----------------------------------------------------------------------------------------------------

def take_snapshot(queries: list[bytes], query_fn=None, label: str = '') -> dict:
    for q in queries:
        check_query_safe(q)
    query_fn = query_fn or _device_query_fn()
    responses = {}
    for q in queries:
        r = query_fn(q)
        responses[q.hex()] = trim(r).hex() if r else None
    return {
        'label': label,
        'taken_at': datetime.now(timezone.utc).isoformat(timespec='seconds'),
        'responses': responses,
    }


def _device_query_fn():
    path = g6_status.find_hid_path()
    if path is None:
        raise IOError('SoundBlaster X G6 HID interface not found')

    def query(payload: bytes) -> Optional[bytes]:
        try:
            return g6_status.hid_query(payload, path=path)
        except (IOError, OSError):
            return None

    return query


def diff_snapshots(a: dict, b: dict) -> list[dict]:
    """Per query whose response changed: offsets and old/new bytes, plus float32 guesses."""
    out = []
    ra, rb = a['responses'], b['responses']
    for q in ra.keys() | rb.keys():
        old = bytes.fromhex(ra[q]) if ra.get(q) else b''
        new = bytes.fromhex(rb[q]) if rb.get(q) else b''
        if old == new:
            continue
        n = max(len(old), len(new))
        old_p, new_p = old.ljust(n, b'\0'), new.ljust(n, b'\0')
        changed = [i for i in range(n) if old_p[i] != new_p[i]]
        out.append({
            'query': q,
            'old': old.hex() or None,
            'new': new.hex() or None,
            'changed_offsets': changed,
            'float_guesses': _float_guesses(old_p, new_p, changed),
        })
    return sorted(out, key=lambda d: d['query'])


def _float_guesses(old: bytes, new: bytes, changed: list[int]) -> list[dict]:
    """Little-endian float32 windows that cover the first change and decode to plausible numbers on both sides."""
    if not changed:
        return []
    guesses = []
    first = changed[0]
    for start in range(max(0, first - 3), first + 1):
        if start + 4 > len(old):
            continue
        fo = struct.unpack_from('<f', old, start)[0]
        fn = struct.unpack_from('<f', new, start)[0]
        if all(math.isfinite(v) and (v == 0 or 1e-4 < abs(v) < 1e5) for v in (fo, fn)):
            guesses.append({'offset': start, 'old': round(fo, 6), 'new': round(fn, 6)})
    return guesses


def format_diff(changes: list[dict]) -> str:
    if not changes:
        return 'No response changed.'
    lines = []
    for c in changes:
        lines.append(f'query {c["query"]}')
        lines.append(f'  old {c["old"]}')
        lines.append(f'  new {c["new"]}')
        lines.append(f'  changed byte offsets: {c["changed_offsets"]}')
        for g in c['float_guesses']:
            lines.append(f'  float32 @ {g["offset"]}: {g["old"]} -> {g["new"]}')
    return '\n'.join(lines)


# --- CLI ----------------------------------------------------------------------------------------------------------

def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(prog='g6-probe', description=__doc__.split('\n\n')[0])
    sub = parser.add_subparsers(dest='cmd', required=True)

    p = sub.add_parser('events', help='print the decoded timeline of a capture')
    p.add_argument('capture')
    p.add_argument('--device', type=int, help='USB address (default: auto-detect the G6)')
    p.add_argument('--hid-only', action='store_true')

    p = sub.add_parser('queries', help='extract distinct host->device HID payloads into a query file')
    p.add_argument('capture')
    p.add_argument('-o', '--output', help='query file to write (default: stdout)')
    p.add_argument('--device', type=int)

    p = sub.add_parser('snapshot', help='replay read queries on the connected G6 and save the responses')
    p.add_argument('-q', '--queries', help='query file (default: the known status queries)')
    p.add_argument('-o', '--output', required=True)
    p.add_argument('-l', '--label', default='', help='what the device was set to, e.g. "surround 40"')

    p = sub.add_parser('diff', help='show which response bytes changed between two snapshots')
    p.add_argument('a')
    p.add_argument('b')
    p.add_argument('--json', action='store_true')

    args = parser.parse_args(argv)
    try:
        if args.cmd == 'events':
            for ev in load_events(args.capture, device=args.device):
                if args.hid_only and not ev.kind.startswith('hid'):
                    continue
                print(ev.describe())
        elif args.cmd == 'queries':
            text = format_query_file(unique_hid_queries(load_events(args.capture, device=args.device)),
                                     Path(args.capture).name)
            if args.output:
                Path(args.output).write_text(text, encoding='utf-8')
                print(f'wrote {args.output}')
            else:
                print(text, end='')
        elif args.cmd == 'snapshot':
            queries = (parse_query_file(Path(args.queries).read_text(encoding='utf-8')) if args.queries
                       else default_queries())
            snap = take_snapshot(queries, label=args.label)
            Path(args.output).write_text(json.dumps(snap, indent=2), encoding='utf-8')
            answered = sum(1 for v in snap['responses'].values() if v)
            print(f'wrote {args.output}: {answered}/{len(queries)} queries answered')
        elif args.cmd == 'diff':
            a, b = (json.loads(Path(f).read_text(encoding='utf-8')) for f in (args.a, args.b))
            changes = diff_snapshots(a, b)
            print(json.dumps(changes, indent=2) if args.json else format_diff(changes))
    except (IOError, OSError, ValueError) as e:
        print(f'error: {e}', file=sys.stderr)
        return 1
    return 0


def cli_entry() -> None:
    sys.exit(main())


if __name__ == '__main__':
    cli_entry()
