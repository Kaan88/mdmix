"""
RGB lighting effects, computed on the host (pure, no I/O).

The G6 stores no effect. SoundBlaster Command animates Pulsate and Cycle itself by streaming colour frames
(``lighting_frame``) to the device (g6-sbc-session-c, markers "effect pulsate" / "effect cycle"):
- Pulsate: the colour stays, the brightness byte ramps 0x00 -> 0xff -> 0x00 (about 2 s per round trip in that capture).
- Cycle: brightness 0xff, the colour crossfades linearly red -> blue -> green -> red.
Music Reactive needs the audio signal and is not implemented; Solo is just the static colour (``--lighting-rgb``).
"""
from __future__ import annotations

EFFECTS = ('pulsate', 'cycle')
DEFAULT_PERIOD_S = {'pulsate': 2.0, 'cycle': 6.0}
FRAME_INTERVAL_S = 0.05  # 20 frames per second; SBC sends 20-60

Rgb = tuple[int, int, int]


def _mix(a: Rgb, b: Rgb, f: float) -> Rgb:
    return tuple(round(x + (y - x) * f) for x, y in zip(a, b))  # type: ignore[return-value]


_WHEEL: tuple[Rgb, ...] = ((255, 0, 0), (0, 0, 255), (0, 255, 0))


def effect_frame(effect: str, t: float, period: float, rgb: Rgb) -> tuple[int, int, int, int]:
    """
    The frame of ``effect`` at ``t`` seconds after the start: (red, green, blue, brightness), all 0..255.
    :param period: seconds for one full round (pulsate: dark -> bright -> dark; cycle: once around the wheel).
    :param rgb: the base colour (only pulsate uses it).
    """
    if period <= 0:
        raise ValueError(f'period must be > 0, got {period}')
    phase = (t / period) % 1.0
    if effect == 'pulsate':
        brightness = round(255 * (1 - abs(2 * phase - 1)))
        return (*rgb, brightness)
    if effect == 'cycle':
        pos = phase * len(_WHEEL)
        i = int(pos)
        return (*_mix(_WHEEL[i], _WHEEL[(i + 1) % len(_WHEEL)], pos - i), 255)
    raise ValueError(f'unknown effect {effect!r}, expected one of {EFFECTS}')
