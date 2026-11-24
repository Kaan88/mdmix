"""
Record what the Python CLI prints, as fixtures for the Rust port's output tests (rust/g6-cli/tests/fixtures/).

    python tools/rust_fixtures.py

Writes rust/g6-cli/tests/fixtures/python_output.json with:
- ``status``: for each HID transcript, what ``g6 --status`` and ``g6 --status --json`` print, the profile
  ``--profile-save`` would write, and the arguments ``--profile-load`` turns that profile into. The first
  transcript is tests/vectors/status_live.json; the others are made-up answers that reach the formatting paths the
  live one doesn't (no answers, unknown codes, rounding ties, custom mic EQ).
- ``dry_run``: stdout, stderr and exit code of ``g6 --dry-run [--debug] ...`` for a set of commands, no G6 found.
- ``dry_run_detect``: the same for a G6 that is found, not found, or lacks interface 4 (the lookup's own lines).
- ``writes_dry_run``: the same for every argv in tests/vectors/writes.json.
- ``effect_writes``: the HID reports ``--dry-run --lighting-effect ...`` sends, recorded like tests/vectors/writes.json.
- ``options``: every argparse option (flags, metavar, choices, default, nargs, type, group) and whether giving it
  alone counts as something to do (``has_action``).
- ``pyfmt``: ``repr``, ``format(x, 'g')`` and ``round`` of a spread of floats, as the status output uses them.
- ``profile_load``: profile JSON texts and the arguments ``profile_to_argv`` makes of them, or its error.
- ``argv``: how argparse reads awkward command lines (values starting with "-", abbreviations, "=", "--"): every
  parsed value as ``repr``, or the exit code, stdout and stderr of the error, help or version. ``prog`` is pinned
  to the console-script name, as argparse otherwise takes it from ``sys.argv[0]``.

Nothing here touches a device (hid.enumerate and usb.core.find are stubs). The status
answers come from the transcripts, not from the G6. hidapi / pyusb are swapped for stubs that refuse to open
anything, as in tests/conftest.py.
"""
from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import sys
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'src'))

import g6_cli  # noqa: E402
import g6_cli.g6_api as g6_api  # noqa: E402
import g6_cli.g6_core as g6_core  # noqa: E402
from g6_cli.g6_profile import profile_from_status, profile_to_argv  # noqa: E402
from g6_cli.g6_status import collect_status  # noqa: E402

OUT = ROOT / 'rust' / 'g6-cli' / 'tests' / 'fixtures' / 'python_output.json'
LIVE = ROOT / 'tests' / 'vectors' / 'status_live.json'


class _RefuseHid:
    """hid.device stand-in: an unopened handle is fine (dry runs), opening one is an error."""

    def __getattr__(self, name):
        if name in ('open', 'open_path'):
            def refuse(*args, **kwargs):
                raise RuntimeError('tools/rust_fixtures.py tried to open a HID device')
            return refuse
        return lambda *args, **kwargs: None


def _refuse(*args, **kwargs):
    raise RuntimeError('tools/rust_fixtures.py tried to look for a device')


@contextlib.contextmanager
def _no_hardware():
    with mock.patch.object(g6_core.hid, 'device', _RefuseHid), \
            mock.patch.object(g6_core.hid, 'enumerate', _refuse), \
            mock.patch.object(g6_core.usb.core, 'find', _refuse), \
            mock.patch.object(g6_cli, 'read_status', side_effect=AssertionError('no status read here')), \
            mock.patch.object(g6_cli, 'determine_toggle_state', side_effect=AssertionError('no toggle here')), \
            mock.patch.object(g6_api.G6Api, 'reload_alsa_and_pipewire',
                              side_effect=AssertionError('never reload audio services here')):
        yield


def _transcript_query(exchanges):
    answers = {}
    for ex in exchanges:
        answers.setdefault(ex['query'], []).append(ex['response'])

    def query(payload: bytes):
        queue = answers.get(payload.hex())
        if not queue:
            return None
        r = queue.pop(0)
        return None if r is None else bytes.fromhex(r).ljust(64, b'\0')

    return query


def _variant(exchanges, changes: dict) -> list:
    """A copy of ``exchanges`` with some responses replaced (query hex -> response hex or None)."""
    out = []
    for ex in exchanges:
        ex = dict(ex)
        if ex['query'] in changes:
            ex['response'] = changes.pop(ex['query'])
        out.append(ex)
    assert not changes, f'queries not in the transcript: {changes}'
    return out


def _f32(value: float) -> str:
    import struct
    return struct.pack('<f', value).hex()


def _param_answer(group: int, param_id: int, value_hex: str) -> tuple[str, str]:
    query = bytes([0x5A, 0x11, 0x03, 0x01, group, param_id]).hex()
    return query, bytes([0x5A, 0x11, 0x08, 0x01, 0x00, group, param_id]).hex() + value_hex


def _transcripts() -> list[dict]:
    live = json.loads(LIVE.read_text(encoding='utf-8'))['exchanges']
    nothing = [{'query': ex['query'], 'response': None} for ex in live]
    odd = dict([
        ('5a070102', '5a0705' + b'1.2\x01'.hex()),  # firmware with a control character
        ('5a2c0101', '5a2c050104'),  # Headphones
        ('5a390101', '5a39050100a4'),  # flags: ring off, line out virtualization on
        ('5a1103019702', '5a110801009702' + '00002041'),  # decoder: unknown float (10.0)
        ('5a6c0101', '5a6c030107'),  # unknown DAC filter code
        ('5a3a020701', '5a3a020701'),  # RGB on
        _param_answer(0x96, 0x00, _f32(1.0)),
        _param_answer(0x96, 0x01, _f32(0.745)),  # 74.5 -> 74 (ties to even)
        _param_answer(0x96, 0x07, _f32(-0.0)),
        _param_answer(0x96, 0x08, _f32(0.125)),  # 12.5 -> 12
        _param_answer(0x96, 0x18, 'ffffffff'),  # NaN -> unknown
        _param_answer(0x96, 0x19, _f32(0.135)),  # 13.5 -> 14
        _param_answer(0x96, 0x06, _f32(0.5)),  # smart volume mode no preset has
        _param_answer(0x96, 0x09, _f32(1.0)),
        _param_answer(0x96, 0x0A, _f32(-3.14159)),
        _param_answer(0x96, 0x0B, _f32(0.123456)),
        _param_answer(0x96, 0x0C, _f32(-12.0)),
        _param_answer(0x96, 0x0D, _f32(0.00005)),  # rounds to 0.0001
        _param_answer(0x96, 0x0E, _f32(0.00004)),  # rounds to 0.0
        _param_answer(0x96, 0x0F, _f32(-0.00004)),  # rounds to -0.0
        _param_answer(0x96, 0x10, _f32(123456.75)),
        _param_answer(0x96, 0x11, _f32(1e20)),
        _param_answer(0x96, 0x12, _f32(0.03125)),
        _param_answer(0x96, 0x13, _f32(11.99995)),
        ('5a3c0201', '5a3c0401000214'),  # mic boost 20 dB
        ('5a260308ffff', '5a260b08ffff05'),  # SBX + voice clarity
        _param_answer(0x95, 0x04, _f32(1.0)),
        _param_answer(0x95, 0x05, _f32(0.4)),
        _param_answer(0x95, 0x14, _f32(1.5)),  # mic EQ gains no preset has
        _param_answer(0x95, 0x15, _f32(-2.25)),
        _param_answer(0x95, 0x16, _f32(0.0)),
        _param_answer(0x95, 0x17, _f32(3.333)),
    ])
    loud = dict([
        ('5a070102', '5a07ff' + b'9.9'.hex()),  # length byte past the end of the report
        ('5a2c0101', '5a2c0501'),  # output code 0
        ('5a390101', '5a39090200ff'),  # the other flags prefix
        ('5a3a020701', '5a3a0207ff'),
        _param_answer(0x96, 0x06, _f32(1.0)),  # Loud
        ('5a3c0201', '5a3c040100020f'),  # mic boost SBC can't set -> unknown
        ('5a260308ffff', '5a260b08ffff02'),  # Scout Mode
    ])
    night = dict([
        ('5a070102', '5a0703' + bytes([0x31, 0xE9, 0x32]).hex()),  # not ASCII -> unknown
        ('5a390101', None),  # no flags: ring from its own query
        ('5a1103019702', '5a110801009702' + '00004040'),  # Night
        _param_answer(0x96, 0x06, _f32(2.0)),  # Night
        ('5a3c0201', None),
    ])
    return [
        {'name': 'live', 'exchanges': live},
        {'name': 'no_answers', 'exchanges': nothing},
        {'name': 'odd_values', 'exchanges': _variant(live, odd)},
        {'name': 'loud_scout', 'exchanges': _variant(live, loud)},
        {'name': 'night_no_flags', 'exchanges': _variant(live, night) + [
            {'query': '5a020a3a000e0100', 'response': None},
            {'query': '5a3a021001', 'response': '5a3a021001'},
        ]},
    ]


def _run_main(argv: list[str], status=None) -> str:
    out = io.StringIO()
    with _no_hardware(), contextlib.redirect_stdout(out):
        if status is not None:
            with mock.patch.object(g6_cli, 'read_status', return_value=status):
                g6_cli.main(argv)
        else:
            g6_cli.main(argv)
    return out.getvalue()


def build_status() -> list[dict]:
    entries = []
    for t in _transcripts():
        def fresh():
            return collect_status(_transcript_query(t['exchanges']))

        profile = profile_from_status(fresh())
        entries.append({
            'name': t['name'],
            'exchanges': t['exchanges'],
            'text': _run_main(['--status'], fresh()),
            'json': _run_main(['--status', '--json'], fresh()),
            # what save_profile writes (json.dump with indent=2, then a newline)
            'profile': json.dumps(profile, indent=2) + '\n' if profile['settings'] else None,
            'profile_argv': profile_to_argv(profile),
        })
    return entries


DRY_RUN_CASES = [
    # runs that fail: an error from main (exit 1, "error: ..." on stderr) and usage errors (exit 2)
    ['--profile-load', 'does-not-exist.json'],
    ['--lighting-effect', 'pulsate', '--lighting-effect-period', '-1'],
    ['--eq-preset', 'nope'],
    ['--sbx', 'Bogus'],
    ['--debug', '--set-output', 'Speakers'],
    ['--debug', '--playback-volume', '50'],
    ['--debug', '--playback-volume', '30', '--playback-volume-channels', 'Right'],
    ['--debug', '--playback-mute', 'Enabled', '--recording-mic-boost-db', '20'],
    ['--debug', '--lighting-rgb', '12', '34', '56', '--lighting-volume-ring', 'Disabled'],
    ['--debug', '--eq-preset', 'Rock', '--eq-preset-output', 'Speakers'],
    ['--debug', '--eq-preset', 'hip hop', '--eq-band', '1k', '-2.5'],
    ['--debug', '--scout-mode', 'Enabled', '--sbx', 'Disabled'],
    ['--debug', '--eq-band', '1k', '2.5', '--eq-band', '31', '-12', '--eq-preamp', '3.5'],
    ['--debug', '--mixer-recording-what-u-hear-volume', '70', '--mixer-recording-what-u-hear-mute', 'Enabled'],
    ['--debug', '--recording-voice-clarity-mic-eq-preset', 'PRESET_DM_1'],
    ['--debug', '--lighting-effect', 'pulsate', '--lighting-effect-duration', '0.2'],
    ['--debug', '--lighting-effect', 'music', '--lighting-effect-duration', '0.1'],
    ['--lighting-effect', 'cycle', '--lighting-rgb', '0', '255', '0', '--lighting-effect-duration', '0.1'],
    ['--decoder-mode', 'Night', '--playback-filter', 'FAST_ROLL_OFF_LINEAR_PHASE'],
    ['--sbx-smart-volume-special-value', 'Loud', '--mixer-playback-mute', 'Disabled'],
]


_REAL_DETERMINE_TOGGLE_STATE = g6_cli.determine_toggle_state
G6_PATH = rb'\\?\HID#VID_041E&PID_3256&MI_04#7&1ce30bf2&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}'


class _FakeUsbInterface:
    bInterfaceNumber = 0
    bAlternateSetting = 0


class _FakeUsbConfiguration:
    bConfigurationValue = 1

    def __iter__(self):
        return iter([_FakeUsbInterface()])


class _FakeUsbDevice:
    bus, address, port_numbers = 1, 5, (2,)

    def __iter__(self):
        return iter([_FakeUsbConfiguration()])


def _hid_entry(interface: int, path: bytes) -> dict:
    return {'vendor_id': 0x041e, 'product_id': 0x3256, 'interface_number': interface, 'path': path}


# What the device lookup finds. ``hid`` is what hid.enumerate returns, ``usb`` what usb.core.find returns.
# The Rust port has no USB lookup (it goes by HID alone), so only these three are compared.
SCENARIOS = {
    'found': {'hid': [_hid_entry(0, b'other'), _hid_entry(4, G6_PATH)], 'usb': _FakeUsbDevice()},
    'none': {'hid': [], 'usb': None},
    'no_interface': {'hid': [_hid_entry(0, b'other')], 'usb': _FakeUsbDevice()},
}


def _run_cli(argv: list[str], scenario: str = 'none') -> dict:
    """``cli_entry`` on ``argv`` with the device lookup stubbed: stdout, stderr and the exit code.
    The toggle state file is in a temporary directory and says Speakers (so --toggle-output goes to Headphones)."""
    import tempfile
    real_toggle = _REAL_DETERMINE_TOGGLE_STATE
    out, err, code = io.StringIO(), io.StringIO(), 0
    sc = SCENARIOS[scenario]
    with tempfile.TemporaryDirectory() as tmp:
        (Path(tmp) / g6_cli.TOGGLE_STATE_TEMP_FILE_NAME).write_text('Speakers')
        with _no_hardware(), \
                mock.patch.object(g6_core.hid, 'enumerate', return_value=sc['hid']), \
                mock.patch.object(g6_core.usb.core, 'find', return_value=sc['usb']), \
                mock.patch.object(g6_cli, 'determine_toggle_state', real_toggle), \
                mock.patch.object(g6_cli.tempfile, 'gettempdir', return_value=tmp), \
                mock.patch.object(sys, 'argv', ['soundblaster-x-g6-cli'] + argv), \
                contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try:
                g6_cli.cli_entry()
            except SystemExit as e:
                code = e.code
    return {'stdout': out.getvalue(), 'stderr': err.getvalue(), 'exit': code or 0}


def build_dry_run() -> list[dict]:
    """Every dry run is recorded with the real device lookup, stubbed to find nothing (the placeholder device)."""
    return [{'argv': argv, **_run_cli(['--dry-run'] + argv)} for argv in DRY_RUN_CASES]


def build_dry_run_detect() -> list[dict]:
    """What the device lookup of a dry run prints, for a G6 that is there, isn't, or has no interface 4."""
    cases = [['--sbx', 'Enabled'], ['--debug', '--lighting-effect', 'pulsate', '--lighting-effect-duration', '0.1']]
    return [{'scenario': name, 'argv': argv, 'g6_path': G6_PATH.decode(), **_run_cli(['--dry-run'] + argv, name)}
            for name in SCENARIOS for argv in cases]


def build_writes_dry_run() -> list[dict]:
    """``--dry-run`` plus each argv of tests/vectors/writes.json (the G6 not found), as the user sees it."""
    cases = json.loads((ROOT / 'tests' / 'vectors' / 'writes.json').read_text(encoding='utf-8'))['cases']
    entries = []
    for case in cases:
        assert not {'--claim-and-release', '--reload-audio-services', '--reload-audio-services-no-sudo'} & set(case['argv'])
        entries.append({'argv': case['argv'], **_run_cli(['--dry-run'] + case['argv'])})
    return entries


EFFECT_CASES = [
    ['--lighting-effect', 'pulsate'],
    ['--lighting-effect', 'pulsate', '--lighting-rgb', '0', '0', '255', '--lighting-effect-period', '1'],
    ['--lighting-effect', 'cycle', '--lighting-effect-period', '1.5'],
    ['--lighting-effect', 'music', '--lighting-effect-duration', '1'],
    ['--lighting-effect', 'music', '--lighting-rgb', '0', '255', '0', '--lighting-effect-duration', '0.5'],
    ['--lighting-effect', 'pulsate', '--lighting-effect-period', '0.7', '--lighting-effect-duration', '5'],
]


def build_effect_writes() -> list[dict]:
    spec = importlib.util.spec_from_file_location('export_vectors', ROOT / 'tools' / 'export_vectors.py')
    exporter = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(exporter)
    with _no_hardware():
        return [exporter.record(argv) for argv in EFFECT_CASES]


def _parser():
    """The argparse parser parse_cli_args builds (caught as it parses)."""
    import argparse
    seen = []
    original = argparse.ArgumentParser.parse_args

    def capture(self, *args, **kwargs):
        seen.append(self)
        return original(self, *args, **kwargs)

    with mock.patch.object(argparse.ArgumentParser, 'parse_args', capture), contextlib.redirect_stdout(io.StringIO()):
        g6_cli.parse_cli_args(['--status'])
    return seen[0]


def _sample(action) -> list[str]:
    flag = action.option_strings[0]
    if action.nargs == 0:
        return [flag]
    if action.choices:
        return [flag, str(list(action.choices)[0])]
    if action.nargs == 3:
        return [flag, '1', '2', '3']
    if action.nargs == 2:
        return [flag, '1k', '2']
    if flag == '--eq-preset':
        return [flag, 'Rock']
    if action.type is float or getattr(action.type, '__name__', '') == '_seconds':
        return [flag, '1.5']
    return [flag, 'x']


def build_options() -> list[dict]:
    parser = _parser()
    group_of = {id(a): g.title for g in parser._action_groups for a in g._group_actions}
    entries = []
    for a in parser._actions:
        if a.dest in ('help', 'version'):
            continue
        sample = _sample(a)
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                g6_cli.parse_cli_args(sample)
            is_action = True
        except ValueError:
            is_action = False
        entries.append({
            'flags': a.option_strings, 'dest': a.dest, 'group': group_of.get(id(a)),
            'nargs': a.nargs, 'metavar': list(a.metavar) if isinstance(a.metavar, tuple) else a.metavar,
            'choices': None if a.choices is None else [str(c) for c in a.choices],
            'default': a.default, 'type': None if a.type is None else a.type.__name__,
            'kind': type(a).__name__, 'sample': sample, 'is_action': is_action,
        })
    return entries


def build_pyfmt() -> list[dict]:
    import random
    import struct
    rng = random.Random(6)
    values = [0.0, -0.0, 1.0, -1.0, 0.5, 1.5, 2.5, -2.5, 0.03125, 0.0001, 0.00001, 0.00005, 1e15, 1e16, 1e20, 1e-20,
              123456.75, 999999.5, 999999.4, 0.745, 0.125, 0.135, 74.5, 12.5, 11.99995, -3.14159, 1e300, 5e-324,
              float('inf'), float('-inf'), float('nan'), 2.675, 0.1 + 0.2, 1 / 3, 100.0, 1234567.0]
    for _ in range(400):
        values.append(struct.unpack('<f', rng.getrandbits(32).to_bytes(4, 'little'))[0])
    for _ in range(200):
        values.append(rng.uniform(-30, 30))
    for _ in range(100):
        values.append(round(rng.uniform(-13, 13) * 4) / 4)  # exact halves and quarters
    entries = []
    for v in values:
        def bits(x):
            return struct.pack('<d', x).hex()
        try:
            as_int = str(round(v))
        except (OverflowError, ValueError):
            as_int = None
        entries.append({'bits': bits(v), 'repr': repr(v), 'g': format(v, 'g'), 'round4': bits(round(v, 4)),
                        'round2': bits(round(v, 2)), 'round': as_int})
    return entries


PROFILE_TEXTS = [
    '{"g6_profile": 1, "settings": {"sbx": "Enabled", "eq-preamp": 2.50, "eq-band": [["1k", -2.5], ["31", 0]]}}',
    '{"g6_profile": true, "settings": {"decoder-mode": "Night"}}',
    '{"g6_profile": 1.0, "settings": {}}',
    '{"g6_profile": 2, "settings": {}}',
    '{"settings": {}}',
    '[1, 2]',
    '{"g6_profile": 1, "settings": []}',
    '{"g6_profile": 1, "settings": {"zzz": 1, "playback-volume": 50, "sbx": "Enabled"}}',
    '{"g6_profile": 1, "settings": {"lighting-disable": true, "lighting-volume-ring": "Disabled"}}',
    '{"g6_profile": 1, "settings": {"lighting-disable": false}}',
    '{"g6_profile": 1, "settings": {"lighting-disable": 1}}',
    '{"g6_profile": 1, "settings": {"lighting-disable": "yes"}}',
    '{"g6_profile": 1, "settings": {"eq-band": {"1k": 2}}}',
    '{"g6_profile": 1, "settings": {"eq-band": [["1k"]]}}',
    '{"g6_profile": 1, "settings": {"eq-band": [[null, true], [1.5, [1, "a"]], [{"k": false}, -0.0]]}}',
    '{"g6_profile": 1, "settings": {"eq-preamp": true}}',
    '{"g6_profile": 1, "settings": {"eq-preamp": null}}',
    r'{"g6_profile": 1, "settings": {"eq-preamp": [1, 2.0, "x\u00e9\n"]}}',
    '{"g6_profile": 1, "settings": {"eq-preamp": {"a": null}}}',
    '{"g6_profile": 1, "settings": {"eq-preamp": 1e-5, "sbx-bass-value": 1e20, "sbx-surround-value": -7}}',
    '{"g6_profile": 1, "settings": {"recording-voice-clarity-mic-eq-preset": "it\'s"}}',
    '{"g6_profile": 1, "settings": {"recording-voice-clarity-mic-eq-preset": "PRESET_1", "sbx": "Disabled", '
    '"playback-filter": "FAST_ROLL_OFF_LINEAR_PHASE", "eq": "Enabled"}}',
]


def build_profile_load() -> list[dict]:
    from g6_cli.g6_profile import profile_to_argv
    entries = []
    for text in PROFILE_TEXTS:
        try:
            entries.append({'text': text, 'argv': profile_to_argv(json.loads(text)), 'error': None})
        except ValueError as e:
            entries.append({'text': text, 'argv': None, 'error': str(e)})
    return entries


ARGV_CASES = [
    ['--eq-preamp', '-.5'], ['--eq-preamp', '-5'], ['--eq-preamp', '-1e5'], ['--eq-preamp=-1e5'],
    ['--eq-preamp', '-1e-1'], ['--eq-preamp', '-5 '], ['--eq-preamp', '-inf'], ['--eq-preamp=-inf'],
    ['--eq-preamp', ' -inf'], ['--eq-preamp', '1_0.5'], ['--eq-preamp'],
    ['--eq-band', '1k', '-.5'], ['--eq-band', '1k', '-x'], ['--eq-band=1k', '2'], ['--eq-band', '1k'],
    ['--eq-band', '1k', '1', '--eq-band', '2k', '-2'], ['--eq-band', '1k', '-.5', '--eq-preamp', '-.25'],
    ['--lighting-rgb', '1', '-2', '3'], ['--lighting-rgb', '1', '2'], ['--lighting-rgb', '1', '2', '3', '4'],
    ['--eq-preset', '--debug'], ['--eq-preset', '-'], ['--eq-preset='], ['--eq-preset', ''],
    ['--eq-preset', 'rock', 'extra'], ['--eq-preset', '-x y'], ['--eq-preset', '--x y'], ['--eq-preset', '--sbx=a b'],
    ['--eq-preset', '--zz=a b'], ['--eq-preset', '-hx y'],
    ['--sbx-s', 'Enabled'], ['--sbx-surround-v', '5'], ['--playback-vol', '5'], ['--playback-vol=5'],
    ['--playback-volume-c', 'Left'], ['--dry', '--deb', '--sta'], ['--dry-run=1'], ['--vers=1'],
    ['--sbx', 'Enabled', '--sbx', 'Disabled'], ['--sbx', 'Enabled', '--sb', 'Disabled'],
    ['--', '--status'], ['--status', '--'], ['--eq-preset', '--', 'x'], ['-x'], ['-1'], ['--status', '-1'],
    ['--status', 'x', 'y'], ['--status', '-hx'], ['--status', '-h=1'],
    ['--set-output', 'speakers'], ['--set-output=Speakers'], ['--set-output', 'Speakers', '--toggle-output'],
    ['--playback-volume', '-0'], ['--playback-volume', '1_0'], ['--playback-volume', ' 7 '],
    ['--playback-volume', '-5'], ['--playback-volume', '-.5'], ['--playback-volume', '+7'],
    ['--lighting-effect-period', 'nan'], ['--lighting-effect-duration', '-0.0'],
    ['--profile-save', '-f.json'], ['--profile-save=-f.json'], ['--profile-save', 'a b'], ['--profile-save', '-a b'],
    ['--profile-load', '--profile-save'], ['--profile-l', 'x', '--profile-s', 'y'],
    ['--lighting-rgb', '1', '2', '3', '--lighting-rgb', '4', '5', '6'], ['--eq-band', '1k', '--status'],
    ['--eq-band', '1k', '-h'], ['--eq-band', '--', '1k', '2'], ['--eq-band', '1k', '2', '3'],
    ['-hh=1'], ['-hhx'], ['-h-x'], ['-h'], ['--he'], ['--h'], ['--status='], ['--help=1'], ['--vers'],
    ['--playback-volume', 'x', '--eq-band', '1k'], ['--eq-band', '1k', '--playback-volume', 'x'],
    ['--playback-volume', 'x', '-h'], ['-h', '--playback-volume', 'x'], ['--status', 'x', '--playback-vol', '1'],
    ['--lighting-rgb', '1', 'x', 'y'], ['--set-output', 'Speakers', '--set-output', 'x'], ['--eq-preamp', '1e400'],
    ['--recording-mic-boost-db', '15'], ['--mixer-recording-line-in-volume', '55'],
    ['--recording-voice-clarity-noise-reduction', '30'], ['--eq-preamp', '-1' + chr(10)], ['--eq-preamp', chr(0x2212) + '1'],
]


def build_argv() -> list[dict]:
    parser = _parser()
    parser.prog = 'soundblaster-x-g6-cli'
    entries = []
    for argv in ARGV_CASES:
        out, err = io.StringIO(), io.StringIO()
        try:
            with contextlib.redirect_stderr(err), contextlib.redirect_stdout(out):
                ns = parser.parse_args(argv)
        except SystemExit as e:
            entries.append({'argv': argv, 'exit': e.code, 'stdout': out.getvalue(), 'stderr': err.getvalue()})
            continue
        entries.append({'argv': argv, 'values': {k: repr(v) for k, v in vars(ns).items()}})
    return entries


def main():
    data = {
        'note': 'Generated by tools/rust_fixtures.py from the Python CLI; do not edit. Only "live" is a real '
                'device transcript; the other status transcripts are made-up answers for the formatting paths.',
        'status': build_status(),
        'dry_run': build_dry_run(),
        'dry_run_detect': build_dry_run_detect(),
        'writes_dry_run': build_writes_dry_run(),
        'effect_writes': build_effect_writes(),
        'options': build_options(),
        'pyfmt': build_pyfmt(),
        'profile_load': build_profile_load(),
        'argv': build_argv(),
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(data, indent=1) + '\n', encoding='utf-8')
    print('wrote', OUT)


if __name__ == '__main__':
    main()
