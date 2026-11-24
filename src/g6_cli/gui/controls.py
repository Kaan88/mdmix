"""
Declarative description of the GUI controls and the CLI flags they drive.

The window (app.py) is built from these tables, and tests/gui/controls_test.py feeds every flag/value they can
produce through the real CLI parser, so a renamed or mistyped flag fails in CI instead of on a button click.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Optional

from g6_cli.g6_spec import PlaybackFilter
from g6_cli.g6_spec.equalizer import EQ_BAND_FREQS
from g6_cli.g6_spec.eq_presets import EQ_PRESETS
from g6_cli.g6_spec.recording import MicrophoneEqualizerPreset
from g6_cli.g6_status import DAC_FILTER_LABELS

ENABLED_DISABLED = ('Enabled', 'Disabled')
CHANNELS = ('Both', 'Left', 'Right')
OUTPUTS = ('Speakers', 'Headphones')
DECODER_MODES = ('Normal', 'Full', 'Night')

# label shown in the GUI -> --playback-filter value
DAC_FILTERS: dict[str, str] = {DAC_FILTER_LABELS[pf.name]: pf.name for pf in PlaybackFilter}

SPEAKER_LAYOUT_FLAGS: dict[tuple[str, str], str] = {
    ('Speakers', 'Stereo'): '--playback-speakers-to-stereo',
    ('Speakers', '5.1'): '--playback-speakers-to-5-1',
    ('Speakers', '7.1'): '--playback-speakers-to-7-1',
    ('Headphones', 'Stereo'): '--playback-headphones-to-stereo',
    ('Headphones', '5.1'): '--playback-headphones-to-5-1',
    ('Headphones', '7.1'): '--playback-headphones-to-7-1',
}


@dataclass(frozen=True)
class Toggle:
    label: str
    flag: str  # without leading '--'


@dataclass(frozen=True)
class Slider:
    label: str
    flag: str
    lo: int = 0
    hi: int = 100
    step: int = 1
    default: int = 0
    channels_flag: Optional[str] = None  # if set, a Both/Left/Right selector is sent along with the value
    args: tuple[str, ...] = ()  # sent between the flag and the value, e.g. the band of --eq-band FREQ DB

    def values(self) -> range:
        return range(self.lo, self.hi + 1, self.step)


@dataclass(frozen=True)
class Option:
    label: str
    flag: str
    values: tuple[str, ...]


@dataclass(frozen=True)
class Section:
    title: str
    controls: tuple
    audio: bool = False  # True: needs the USB AudioControl interface (claim/release on Linux)
    subtitle: Optional[str] = None


def _volume_group(base: str, step: int = 10, label_prefix: str = '') -> tuple:
    return (
        Toggle(f'{label_prefix}Mute', f'{base}-mute'),
        Slider(f'{label_prefix}Volume', f'{base}-volume', step=step, channels_flag=f'{base}-volume-channels'),
    )


# --- SBX -----------------------------------------------------------------------------------------------------------

SBX_MODE = Section('Processing Mode', (
    Toggle('SBX', 'sbx'),
    Toggle('Scout Mode', 'scout-mode'),
), subtitle='SBX and Scout Mode exclude each other; Direct mode turns both off.')

EQUALIZER = Section('Equalizer', (
    Toggle('Enable', 'eq'),
    Option('SBC Preset', 'eq-preset', tuple(EQ_PRESETS)),
    Slider('Preamp (dB)', 'eq-preamp', lo=-12, hi=12),
    *(Slider(f'{freq} Hz' if not freq.endswith('k') else f'{freq}Hz', 'eq-band', lo=-12, hi=12, args=(freq,))
      for freq in EQ_BAND_FREQS),
))

SBX_SECTIONS: tuple[Section, ...] = tuple(
    Section(name, (Toggle('Enable', f'sbx-{key}'), Slider('Intensity (0–100)', f'sbx-{key}-value')) + extra)
    for name, key, extra in [
        ('Surround', 'surround', ()),
        ('Crystalizer', 'crystalizer', ()),
        ('Bass Boost', 'bass', ()),
        ('Smart Volume', 'smart-volume', (Option('Special Mode', 'sbx-smart-volume-special-value', ('Night', 'Loud')),)),
        ('Dialog Plus', 'dialog-plus', ()),
    ]
)

# --- Playback ------------------------------------------------------------------------------------------------------

PLAYBACK_VOLUME = Section('Volume & Mute', (
    Toggle('Mute Playback', 'playback-mute'),
    Slider('Volume (0–100)', 'playback-volume', channels_flag='playback-volume-channels'),
), audio=True)

LINE_OUT = Section('Line Out', (
    Toggle('Headphone Virtualization', 'playback-line-out-headphone-virtualization'),
))

# --- Recording -----------------------------------------------------------------------------------------------------

VOICE_CLARITY = Section('Voice Clarity', (
    Option('Mic Boost (dB)', 'recording-mic-boost-db', ('0', '10', '20', '30')),
    Toggle('Enable Voice Clarity', 'recording-voice-clarity'),
    Slider('Noise Reduction (0–100)', 'recording-voice-clarity-noise-reduction', step=20),
    Toggle('AEC', 'recording-voice-clarity-aec'),
    Toggle('Smart Volume', 'recording-voice-clarity-smart-volume'),
    Toggle('Mic EQ', 'recording-voice-clarity-mic-eq'),
    Option('Mic EQ Preset', 'recording-voice-clarity-mic-eq-preset',
           tuple(p.name for p in MicrophoneEqualizerPreset)),
))

MIC_RECORDING = Section('Microphone Recording', (
    Toggle('Mute', 'recording-mute'),
    Slider('Volume', 'recording-mic-recording-volume', step=10,
           channels_flag='recording-mic-recording-volume-channels'),
), audio=True)

MIC_MONITORING = Section('Microphone Monitoring', _volume_group('recording-mic-monitoring'), audio=True)

# --- Mixer ---------------------------------------------------------------------------------------------------------

MIXER_PLAYBACK = Section('Playback Mix', (Toggle('Mute Playback Mix', 'mixer-playback-mute'),), audio=True)


@dataclass(frozen=True)
class MixerGroup:
    title: str
    sources: tuple[tuple[str, tuple], ...] = field(default_factory=tuple)  # (source label, controls)


MIXER_MONITORING = MixerGroup('Monitoring', tuple(
    (label, _volume_group(f'mixer-monitoring-{key}', label_prefix='  '))
    for label, key in [('Line In', 'line-in'), ('External Mic', 'external-mic'), ('SPDIF In', 'spdif-in')]
))

MIXER_RECORDING = MixerGroup('Recording Sources', tuple(
    (label, _volume_group(f'mixer-recording-{key}', label_prefix='  '))
    for label, key in [('Line In', 'line-in'), ('External Mic', 'external-mic'), ('SPDIF In', 'spdif-in'),
                       ('What U Hear', 'what-u-hear')]
))


def all_sections() -> list[Section]:
    sections = [SBX_MODE, *SBX_SECTIONS, EQUALIZER, PLAYBACK_VOLUME, LINE_OUT, VOICE_CLARITY, MIC_RECORDING, MIC_MONITORING, MIXER_PLAYBACK]
    for group in (MIXER_MONITORING, MIXER_RECORDING):
        sections.extend(Section(f'{group.title}: {label}', controls, audio=True) for label, controls in group.sources)
    return sections


def command_for(control, value, channels: str = 'Both') -> list[str]:
    """The CLI arguments a control sends for ``value`` (``channels`` only applies to sliders with channels)."""
    cmd = [f'--{control.flag}', *getattr(control, 'args', ()), str(value)]
    if isinstance(control, Slider) and control.channels_flag:
        cmd += [f'--{control.channels_flag}', channels]
    return cmd


def every_command() -> list[list[str]]:
    """Every CLI argument list the table-driven controls can produce (used by tests)."""
    cmds: list[list[str]] = []
    for section in all_sections():
        for c in section.controls:
            if isinstance(c, Toggle):
                values = ENABLED_DISABLED
            elif isinstance(c, Slider):
                values = (c.lo, c.hi, c.values()[len(c.values()) // 2])
                if c.channels_flag:
                    cmds.extend(command_for(c, c.hi, ch) for ch in CHANNELS)
            elif isinstance(c, Option):
                values = c.values
            else:  # pragma: no cover
                raise TypeError(c)
            cmds.extend(command_for(c, v) for v in values)
    return cmds
