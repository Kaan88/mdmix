"""
RGB lighting effects, computed on the host (pure, no I/O).

The G6 stores no effect. SoundBlaster Command animates Pulsate and Cycle itself by streaming colour frames
(``lighting_frame``) to the device (g6-sbc-session-c, markers "effect pulsate" / "effect cycle"):
- Pulsate: the colour stays, the brightness byte ramps 0x00 -> 0xff -> 0x00 (about 2 s per round trip in that capture).
- Cycle: brightness 0xff, the colour crossfades linearly red -> blue -> green -> red.
- Music (SBC: Music Reactive): brightness follows the level of what the G6 plays. SBC sent no frames in the capture
  (nothing was playing), so this one is our own design on the same frame: the peak level in dB mapped to brightness,
  with a short decay so beats don't flicker. The colour is the given one, or a slow cycle when none is given.
Solo is just the static colour (``--lighting-rgb``).
"""
from __future__ import annotations

import math
from typing import Optional

EFFECTS = ('pulsate', 'cycle', 'music')
DEFAULT_PERIOD_S = {'pulsate': 2.0, 'cycle': 6.0, 'music': 12.0}  # music: the colour cycle, when no colour is given
FRAME_INTERVAL_S = 1 / 30  # SBC sends 20-60 frames per second
MUSIC_FLOOR_DB = -45.0  # peak levels at or below this are dark, 0 dBFS is full brightness
MUSIC_DECAY_S = 0.25

Rgb = tuple[int, int, int]


def _mix(a: Rgb, b: Rgb, f: float) -> Rgb:
    return tuple(round(x + (y - x) * f) for x, y in zip(a, b))  # type: ignore[return-value]


_WHEEL: tuple[Rgb, ...] = ((255, 0, 0), (0, 0, 255), (0, 255, 0))


def _wheel(phase: float) -> Rgb:
    pos = phase * len(_WHEEL)
    i = int(pos)
    return _mix(_WHEEL[i], _WHEEL[(i + 1) % len(_WHEEL)], pos - i)


def level_to_brightness(peak: float, floor_db: float = MUSIC_FLOOR_DB) -> float:
    """Linear peak (0..1) -> brightness 0..1 on a dB scale, so quiet passages still show."""
    if peak <= 0:
        return 0.0
    db = 20 * math.log10(min(peak, 1.0))
    return max(0.0, min(1.0, (db - floor_db) / -floor_db))


class LevelFollower:
    """Instant attack, exponential decay: rises with each beat, then fades instead of flickering."""

    def __init__(self, decay_s: float = MUSIC_DECAY_S):
        self.decay_s = decay_s
        self.value = 0.0

    def update(self, level: float, dt: float) -> float:
        self.value = max(level, self.value * math.exp(-max(dt, 0.0) / self.decay_s))
        return self.value


def effect_frame(effect: str, t: float, period: float, rgb: Optional[Rgb],
                 level: Optional[float] = None) -> tuple[int, int, int, int]:
    """
    The frame of ``effect`` at ``t`` seconds after the start: (red, green, blue, brightness), all 0..255.
    :param period: seconds for one full round (pulsate: dark -> bright -> dark; cycle and music: once around the
                   colour wheel).
    :param rgb: the base colour (pulsate, and music when given; None lets music cycle the colour).
    :param level: music only: the smoothed brightness 0..1.
    """
    if period <= 0:
        raise ValueError(f'period must be > 0, got {period}')
    phase = (t / period) % 1.0
    if effect == 'pulsate':
        brightness = round(255 * (1 - abs(2 * phase - 1)))
        return (*(rgb or _WHEEL[0]), brightness)
    if effect == 'cycle':
        return (*_wheel(phase), 255)
    if effect == 'music':
        colour = rgb if rgb is not None else _wheel(phase)
        return (*colour, round(255 * max(0.0, min(1.0, level or 0.0))))
    raise ValueError(f'unknown effect {effect!r}, expected one of {EFFECTS}')
