"""
tests/vectors/ (the Rust port's test vectors, see doc/rust-port.md) must match what the Python code does.
After an intended change to a packet builder, regenerate them: python tools/export_vectors.py
"""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
VECTORS = ROOT / 'tests' / 'vectors'


def _exporter():
    spec = importlib.util.spec_from_file_location('export_vectors', ROOT / 'tools' / 'export_vectors.py')
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


EXPORTER = _exporter()


@pytest.mark.parametrize('name', sorted(EXPORTER.OFFLINE))
def test_vectors_up_to_date(name):
    committed = json.loads((VECTORS / name).read_text(encoding='utf-8'))
    current = json.loads(json.dumps(EXPORTER.OFFLINE[name]()))
    assert committed == current, f'tests/vectors/{name} is stale: run python tools/export_vectors.py'


def test_hid_reports_fit():
    for case in json.loads((VECTORS / 'writes.json').read_text(encoding='utf-8'))['cases']:
        for report in case['hid']:
            assert report.startswith('5a') and len(bytes.fromhex(report)) <= 64, case['argv']


def test_status_transcript_parses_the_same():
    path = VECTORS / 'status_live.json'
    if not path.exists():
        pytest.skip('no live transcript')
    from g6_cli.g6_status import collect_status
    data = json.loads(path.read_text(encoding='utf-8'))
    answers = {}
    for ex in data['exchanges']:
        answers.setdefault(ex['query'], []).append(ex['response'])

    def query(payload: bytes):
        r = answers[payload.hex()].pop(0)
        return None if r is None else bytes.fromhex(r).ljust(64, b'\0')

    d = collect_status(query).to_dict()
    d.pop('mixer', None)
    assert json.loads(json.dumps(d)) == data['parsed']
