"""
SoundBlaster Command's equalizer presets for the G6, as SBC installs them
(%ProgramData%/Creative/SBCommand/Product/G6/Presets/EQ/<Id>.json). Generated from those files, values unchanged.

Each preset has one curve for speakers and one for headphones (many game presets are flat on speakers).
Values: (preamp, 31, 62, 125, 250, 500, 1k, 2k, 4k, 8k, 16k) in dB. SBC writes them with the ordinary EQ
parameters (payloads/raw/g6-sbc-session-c.pcap, presets Rock and Hip Hop match these values), so a preset is just
``equalizer_preamp`` plus ten ``equalizer_band`` writes; the G6 stores no preset name.
"""
from __future__ import annotations

from g6_cli.g6_spec.equalizer import EQ_BAND_FREQS

# Id: (speakers, headphones), in SBC's list order
EQ_PRESETS: dict[str, tuple[tuple[float, ...], tuple[float, ...]]] = {
    'Acoustic': ((0, 0, 0, 2, 0, 0, 2.5, 2, 2, 2, 3),) * 2,
    'Classical': ((0, 0, 0, 1.5, 2, 1, 3, 1, 0, 3, 3),) * 2,
    'Country': ((0, 0, 0, 0, 0, 2, 2.5, 2.5, 2, 3, 3),) * 2,
    'Dance': ((0, 0, 0, 1, 1.5, 0.5, 1.5, 2, 1, 2.5, 2.5),) * 2,
    'Flat': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),) * 2,
    'HipHop': ((0, 0, 0, 2, 2.5, 1.5, 2.5, 0, -2.5, -1, -1.5),) * 2,
    'Jazz': ((0, 0, 0, 0, 2.5, 2.5, 3, 1, 1, 3, 3),) * 2,
    'Pop': ((0, 0, 0, 1, 1, 0, 2.5, 0.5, 0.5, 3, 3),) * 2,
    'RnB': ((0, 0, 0, 0, -1.5, 2.5, 1.5, 2, 0, 2, 1),) * 2,
    'Rock': ((0, 0, 0, 2, 1, 0, 3, 1, -1, 3, 3),) * 2,
    'Vocal': ((0, 0, 0, -1.5, -0.5, 3, 3, 3, 0, 0, 1),) * 2,
    'Gaming': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),) * 2,
    'AdventureAndAction': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                          (0, 4, 4, -1, -2, 0, 0, 3, 0, 0, 0)),
    'ApexLegend': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                  (0, 2, 3, -1, 0, -2, 0, 2, -3, 2, 0)),
    'ArenaOfValor': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                    (0, 2, 2, 2, -2, -2, 2, 2, -3, 2, 0)),
    'BattleRoyale': ((0, 0, 3, 0, 0, 3, 0, 0, 0, 3, 0),
                    (0, 2, 2, 0, 0, 2, 2, 2, -3, 0, 0)),
    'CallofDutySeries': ((0, 0, 0, 2, 0, 0, 2, 3, 0, 0, 0),
                        (0, 4, 3, -1, -2, 2, 2, 2, -4, 0, 2)),
    'CounterStrikeGlobalOffensive': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                                    (0, -5, 3, -3, -3, -2, -2, 2, 0, 0, 3)),
    'Cyberpunk2077': ((0, 3, 4, 0, 0, 0, 3, 3, 0, 3, 0),
                     (0, 3, 3, 0, -2, 0, 2, 2, -3, 2, 0)),
    'DeathStranding': ((0, 3, 3, 2, 0, 0, 0, 0, 0, 3, 0),
                      (0, 2, 2, 2, 0, 0, 2, 2, 0, 2, 0)),
    'DoomEternal': ((0, 3, 3, 0, 0, 0, 0, -2, 0, -2, 0),
                   (0, 2, 2, -2, 0, 0, 2, 2, 0, 2, 0)),
    'Dota2': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
             (0, -5, 0, -3, -4, -5, -1, -3, -2, -3, -1)),
    'DrivingSimulation': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                         (0, 4, 4, 2, 2, -2, -2, 2, 0, 2, 0)),
    'EFT': ((0, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0),
           (0, 2, 2, -1, 0, 0, 2, 2, -2, 2, 2)),
    'Fortnite': ((0, 0, 2, -2, -2, 0, 0, 3, -2, 2, 0),
                (0, 2, 2, -2, -2, 0, 0, 2, -2, 0, 0)),
    'FPS': ((0, 3, 3, 0, 0, 0, 3, 3, -3, 0, 0),
           (0, 2, 3, -2, -2, 0, 2, 2, -5, 2, 1)),
    'GenshinImpact': ((0, 0, 2, 2, 0, 0, 2, 2, 0, 2, 2),
                     (0, 2, 2, 0, 0, 0, 2, 5, 0, 0, 0)),
    'LeagueOfLegends': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                       (0, -3, 0, -3, -4, -5, -1, -3, -2, 2, 0)),
    'MetalGearSolidVThePhantomPain': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                                     (0, 2, 2, -2, 1, 1, 1, 2, -1, -1, 0)),
    'MOBA': ((0, 0, 0, 0, 3, 0, 0, 3, 0, 0, 0),
            (0, 0, 3, 0, 0, 2, 0, 2, -3, 0, 0)),
    'Overwatch': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                 (0, 2, 2.5, 1.5, -1.5, 1.5, 2, 2, -3.5, 1.5, 2)),
    'PlayerUnknown': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                     (0, 0, 3, -4, -4, 0, 2, 2, 0, 2, 0)),
    'ProjectCars': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                   (0, 0, 3, 0, 2, 2, 0, 2, 0, -1, -1)),
    'RealTimeStrategy': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                        (0, 0, -2, -3, 0, -2, 2, 0, -3, -1, 0)),
    'RDR2': ((0, 0, 3, 0, 0, 0, 3, 3, 0, 0, 0),
            (0, 0, 2, 2, 0, -2, 3, 2, -2, 0, 0)),
    'RocketLeague': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                    (0, 3, 3, -2, 2, 2, 0, 2, 0, -2, 0)),
    'RPG': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),) * 2,
    'Stadium': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
               (0, 0, 0, -3, 2, 3, 3, 2, -2, 3, 0)),
    'TheWitcher3WildHunt': ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                           (0, 3, 4, -2, 0, 0, 2, 2, 0, -2, 0)),
    'Rainbow6Siege': ((0, 3, 3, 2, 0, 0, 3, 3, 0, 0, 0),
                     (0, 3, 3, 0, 0, -2, 2, 3, -3, 0, 0)),
    'Music': ((0, 3, 2, 0, -2, 0, 1, 2, 3, 3, 3.5),) * 2,
    'Movie': ((0, 4, 3, -2, -2, 2, 2, 1, 0, 1, 2),) * 2,
    'FootstepEnhancer': ((0, 2, -4, -5, -3, -5, -3, 4, 5, -4, -4),) * 2,
}


def _key(name: str) -> str:
    return ''.join(c for c in name.lower() if c.isalnum())


def find_eq_preset(name: str) -> str:
    """The preset Id for ``name`` (case, spaces and punctuation ignored: 'hip hop' -> 'HipHop')."""
    by_key = {_key(pid): pid for pid in EQ_PRESETS}
    try:
        return by_key[_key(name)]
    except KeyError:
        raise ValueError(f'unknown EQ preset {name!r}, expected one of {", ".join(EQ_PRESETS)}') from None


def eq_preset_values(name: str, output: str = 'Headphones') -> tuple[float, dict[str, float]]:
    """(preamp dB, {band: dB}) of preset ``name`` for ``output`` ('Speakers' or 'Headphones')."""
    if output not in ('Speakers', 'Headphones'):
        raise ValueError(f"output must be 'Speakers' or 'Headphones', got {output!r}")
    values = EQ_PRESETS[find_eq_preset(name)][0 if output == 'Speakers' else 1]
    return values[0], dict(zip(EQ_BAND_FREQS, values[1:]))
