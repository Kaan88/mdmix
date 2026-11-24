"""
Host-side lighting effects. Frame bytes are pinned to the reports SoundBlaster Command streamed in
payloads/raw/g6-sbc-session-c.pcap (markers "effect pulsate" and "effect cycle"), trailing zero padding removed.
"""
from __future__ import annotations

import pytest

from g6_cli import main as cli_main, run_lighting_effect
from g6_cli.g6_spec.lighting import lighting_enable_set_rgb, lighting_frame
from g6_cli.g6_spec.lighting_effects import FRAME_INTERVAL_S, effect_frame


def _reports(fragments) -> list[str]:
    return [bytes.fromhex(str(f)).rstrip(b'\x00').hex() for f in fragments]


@pytest.mark.parametrize('rgb,brightness,colour_report', [
    ((0, 0, 255), 0x0c, '5a3a090a000301010cff'),       # pulsate, blue, dim
    ((0, 0, 255), 0xe5, '5a3a090a00030101e5ff'),       # pulsate, blue, bright
    ((255, 0, 0), 0xff, '5a3a090a00030101ff0000ff'),   # lighting on, red
    ((0x21, 0x00, 0xde), 0xff, '5a3a090a00030101ffde0021'),  # cycle, between red and blue
    ((0x09, 0xf6, 0x00), 0xff, '5a3a090a00030101ff00f609'),  # cycle, between green and red
])
def test_lighting_frame_matches_capture(rgb, brightness, colour_report):
    assert _reports(lighting_frame(*rgb, brightness)) == ['5a3a020601', '5a3a06040003010001', colour_report]


def test_set_rgb_unchanged():
    # the refactor kept the static colour write as it was: three times pre1, pre2, colour with brightness ff
    assert _reports(lighting_enable_set_rgb(17, 34, 51)) == \
        ['5a3a020601', '5a3a06040003010001', '5a3a090a00030101ff332211'] * 3


@pytest.mark.parametrize('bad', [(-1, 0, 0, 0), (0, 256, 0, 0), (0, 0, 0, 300)])
def test_lighting_frame_range(bad):
    with pytest.raises(ValueError):
        lighting_frame(*bad)


def test_pulsate_ramps_brightness():
    rgb = (0, 0, 255)
    assert effect_frame('pulsate', 0, 2, rgb) == (0, 0, 255, 0)
    assert effect_frame('pulsate', 1, 2, rgb) == (0, 0, 255, 255)
    assert effect_frame('pulsate', 0.5, 2, rgb) == (0, 0, 255, 128)
    assert effect_frame('pulsate', 2, 2, rgb)[3] == 0  # wraps around


def test_cycle_walks_the_wheel():
    assert effect_frame('cycle', 0, 3, (1, 2, 3)) == (255, 0, 0, 255)
    assert effect_frame('cycle', 1, 3, (1, 2, 3)) == (0, 0, 255, 255)
    assert effect_frame('cycle', 2, 3, (1, 2, 3)) == (0, 255, 0, 255)
    assert effect_frame('cycle', 0.5, 3, (1, 2, 3)) == (128, 0, 128, 255)
    for i in range(60):
        r, g, b, brightness = effect_frame('cycle', i / 20, 3, (0, 0, 0))
        assert brightness == 255 and r + g + b in (254, 255, 256)


@pytest.mark.parametrize('effect,period', [('strobe', 1), ('pulsate', 0)])
def test_effect_frame_rejects(effect, period):
    with pytest.raises(ValueError):
        effect_frame(effect, 0, period, (0, 0, 0))


class FakeApi:
    def __init__(self, interrupt_after=None):
        self.frames, self.static, self.interrupt_after = [], [], interrupt_after

    def lighting_frame(self, *frame):
        if self.interrupt_after is not None and len(self.frames) == self.interrupt_after:
            raise KeyboardInterrupt
        self.frames.append(frame)

    def lighting_enable_set_rgb(self, *rgb):
        self.static.append(rgb)


class FakeClock:
    def __init__(self):
        self.now = 100.0

    def __call__(self):
        return self.now

    def sleep(self, s):
        assert s >= 0
        self.now += s


def test_run_for_duration_then_static_colour():
    api, clock = FakeApi(), FakeClock()
    n = run_lighting_effect(api, 'pulsate', rgb=(0, 0, 255), duration=1.0, clock=clock, sleep=clock.sleep)
    assert n == round(1.0 / FRAME_INTERVAL_S)
    assert api.static == [(0, 0, 255)]
    assert api.frames[0] == (0, 0, 255, 0)


def test_ctrl_c_restores_static_colour():
    api, clock = FakeApi(interrupt_after=5), FakeClock()
    assert run_lighting_effect(api, 'cycle', clock=clock, sleep=clock.sleep) == 5
    assert api.static == [(255, 0, 0)]


def test_dry_run_sends_one_period_without_waiting():
    api = FakeApi()
    run_lighting_effect(api, 'cycle', period=1.0, dry_run=True, sleep=lambda s: pytest.fail('slept'))
    assert len(api.frames) == round(1.0 / FRAME_INTERVAL_S)


def test_bad_period():
    with pytest.raises(ValueError):
        run_lighting_effect(FakeApi(), 'pulsate', period=-1)


def test_cli_dry_run(capsys):
    cli_main(['--dry-run', '--debug', '--lighting-effect', 'pulsate', '--lighting-rgb', '0', '0', '255',
              '--lighting-effect-duration', '0.1'])
    out = capsys.readouterr().out
    assert '005a3a090a0003010100ff00' in out  # first pulsate frame: blue at brightness 0
    assert '005a3a090a000301010dff00' in out
