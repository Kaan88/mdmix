import argparse
import contextlib
import io
import json
import math
import os.path
import sys
import tempfile
import time

from g6_cli.g6_api import G6Api
from g6_cli.g6_spec import (
    AudioFeature,
    Channel,
    PlaybackFilter,
    SmartVolumeSpecialHex,
    BOTH_CHANNELS,
)
from g6_cli.g6_profile import load_profile_argv, save_profile
from g6_cli.g6_spec.decoder import DecoderMode
from g6_cli.g6_spec.equalizer import EQ_BAND_FREQS
from g6_cli.g6_spec.eq_presets import EQ_PRESETS, eq_preset_values, find_eq_preset
from g6_cli.g6_spec.lighting_effects import (
    DEFAULT_PERIOD_S as LIGHTING_EFFECT_PERIOD,
    EFFECTS as LIGHTING_EFFECTS,
    FRAME_INTERVAL_S as LIGHTING_FRAME_INTERVAL,
    LevelFollower,
    effect_frame,
    level_to_brightness,
)
from g6_cli.g6_spec.modes import ProcessingMode
from g6_cli.g6_spec.recording import MicrophoneEqualizerPreset
from g6_cli.g6_status import DAC_FILTER_LABELS, DeviceStatus, read_status
from g6_cli.g6_util import to_bool

VERSION = '1.1.0'

# The name of the temporary file to remember the last toggle state in. If the file could not be found. The program
# lets the G6 to toggle to Speakers by default.
TOGGLE_STATE_TEMP_FILE_NAME = 'g6-cli-toggle-state'
TOGGLE_STATE_SPEAKERS = 'Speakers'
TOGGLE_STATE_HEADPHONES = 'Headphones'


def _seconds(value: str) -> float:
    """A float for a time option; NaN and the infinities are refused (a NaN duration never ended)."""
    try:
        seconds = float(value)
    except ValueError:
        raise argparse.ArgumentTypeError(f'invalid float value: {value!r}') from None
    if not math.isfinite(seconds):
        raise argparse.ArgumentTypeError(f'must be a finite number of seconds, got {value!r}')
    return seconds


def _channels_from_cli(value: str) -> set[Channel]:
    """
    Convert a CLI channels string into a set[Channel].
    :param value: One of 'Both', 'Left', 'Right'
    :return: A set[Channel] compatible with g6_api methods.
    """
    if value == 'Both':
        return BOTH_CHANNELS
    elif value == 'Left':
        return {Channel.CHANNEL_1}
    elif value == 'Right':
        return {Channel.CHANNEL_2}
    else:
        raise ValueError(f'Unsupported channels value: {value}')


def _decoder_mode_from_cli(value: str) -> DecoderMode:
    """
    Convert a CLI decoder mode string into a DecoderMode enum.
    :param value: One of 'Normal', 'Full', 'Night'
    :return: The DecoderMode enum value.
    """
    return DecoderMode[value.upper()]


# Arguments that only modify how an action is executed; they are not an action on their own.
_NON_ACTION_ARGS = {'dry_run', 'debug', 'json', 'reload_audio_services_no_sudo', 'lighting_effect_period',
                    'lighting_effect_duration', 'eq_preset_output'}


def has_action(args: argparse.Namespace) -> bool:
    """
    :return: True if at least one argument requests something to be done (a device change, status, reload, ...).
    """
    for name, value in vars(args).items():
        if name in _NON_ACTION_ARGS or name.endswith('_channels'):
            continue
        if value is not None and value is not False:
            return True
    return False


def parse_cli_args(argv: list[str] | None = None):
    """
    Parse the CLI arguments using argparse.
    Prints the CLI help to console and raises an error if the arguments are invalid.
    :return: The parsed cli args object
    """
    numbers_0_100 = [i for i in range(0, 101)]
    numbers_0_30_step_10 = [i * 10 for i in range(0, 4)]
    numbers_0_100_step_10 = [i * 10 for i in range(0, 11)]
    numbers_0_100_step_20 = [i * 20 for i in range(0, 6)]
    enabled_disabled = ['Enabled', 'Disabled']
    channels = ['Both', 'Left', 'Right']
    decoder_modes = ['Normal', 'Full', 'Night']

    parser = argparse.ArgumentParser(usage="soundblaster-x-g6-cli", description='SoundBlaster X G6 CLI')
    #
    # Device / services
    #
    general_options_group = parser.add_argument_group('General options')
    general_options_group.add_argument('--dry-run', required=False, action='store_true',
                                       help='Used to verify the available hex_line files, without making any calls against the G6 device.')
    general_options_group.add_argument('--debug', required=False, action='store_true',
                                       help='Print communication data with the G6 device to the console.')
    general_options_group.add_argument('--version', action='version', version=f'soundblaster-x-g6-cli {VERSION}')

    general_options_group.add_argument('--claim-and-release', required=False, action='store_true',
                                       help='Let the application exclusively claim the G6\'s USB AudioControl interface from the kernel and release it afterwards.'
                                            ' This will disconnect the G6 device from the kernel sound driver "snd-usb-audio" leading the system not having any audio output.'
                                            ' Use `--reload-audio-services` to reload the kernel sound driver and make the audio output available again.')
    general_options_group.add_argument('--reload-audio-services', required=False, action='store_true',
                                       help='Reload ALSA and restart user PipeWire services.')
    general_options_group.add_argument('--reload-audio-services-no-sudo', required=False, action='store_true',
                                       help='Reload audio services, but do not use sudo for ALSA reload'
                                            ' (implies --reload-audio-services).')
    general_options_group.add_argument('--status', required=False, action='store_true',
                                       help='Read the current device state over HID (read-only) and print it.'
                                            ' Runs before any other requested change.')
    general_options_group.add_argument('--json', required=False, action='store_true',
                                       help='With --status: print the state as JSON.')

    #
    # Playback [HID / Audio]
    #
    playback_hid_group = parser.add_argument_group('Playback [HID]',
                                                   description="Control basic features using the G6's USB HID interface.")
    playback_audio_group = parser.add_argument_group('Playback [Audio]',
                                                     description="Control advanced features using the G6's USB AudioControl interface. See `--claim-and-release` how to use these features.")

    playback_hid_group.add_argument('--toggle-output', required=False, action='store_true',
                                    help='Toggles the sound output between Speakers and Headphones.')
    playback_hid_group.add_argument('--set-output', required=False, type=str,
                                    choices=[TOGGLE_STATE_SPEAKERS, TOGGLE_STATE_HEADPHONES],
                                    metavar="{Speakers|Headphones}",
                                    help='Sets the sound output to the specified option.')
    playback_audio_group.add_argument('--playback-mute', required=False, type=str, choices=enabled_disabled,
                                      metavar="{Enabled|Disabled}",
                                      help='Mute/unmute playback.')
    playback_audio_group.add_argument('--playback-volume', required=False, type=int, choices=numbers_0_100,
                                      metavar="{0..100}",
                                      help='Set playback volume as integer.')
    playback_audio_group.add_argument('--playback-volume-channels', required=False, type=str, choices=channels,
                                      default='Both',
                                      metavar="{Both|Left|Right}",
                                      help='Set playback volume channels for --playback-volume.')
    playback_audio_group.add_argument('--playback-speakers-to-stereo', required=False, action='store_true',
                                      help='Switch speakers output to stereo.')
    playback_audio_group.add_argument('--playback-speakers-to-5-1', required=False, action='store_true',
                                      help='Switch speakers output to 5.1.')
    playback_audio_group.add_argument('--playback-speakers-to-7-1', required=False, action='store_true',
                                      help='Switch speakers output to 7.1.')
    playback_audio_group.add_argument('--playback-headphones-to-stereo', required=False, action='store_true',
                                      help='Switch headphones output to stereo.')
    playback_audio_group.add_argument('--playback-headphones-to-5-1', required=False, action='store_true',
                                      help='Switch headphones output to 5.1.')
    playback_audio_group.add_argument('--playback-headphones-to-7-1', required=False, action='store_true',
                                      help='Switch headphones output to 7.1.')
    playback_hid_group.add_argument('--playback-direct-mode', required=False, type=str, choices=enabled_disabled,
                                    metavar="{Enabled|Disabled}",
                                    help='Enable/disable Direct Mode.')
    playback_hid_group.add_argument('--playback-spdif-out-direct-mode', required=False, type=str,
                                    choices=enabled_disabled,
                                    metavar="{Enabled|Disabled}",
                                    help='Enable/disable SPDIF-Out Direct Mode.')
    playback_hid_group.add_argument('--playback-filter', required=False, type=str,
                                    choices=[e.name for e in PlaybackFilter],
                                    metavar=(
                                        "{FAST_ROLL_OFF_MINIMUM_PHASE|SLOW_ROLL_OFF_MINIMUM_PHASE|"
                                        "FAST_ROLL_OFF_LINEAR_PHASE|SLOW_ROLL_OFF_LINEAR_PHASE}"
                                    ),
                                    help='Set playback filter by enum name.')
    playback_hid_group.add_argument('--playback-line-out-headphone-virtualization', required=False, type=str,
                                    choices=enabled_disabled, metavar="{Enabled|Disabled}",
                                    help='Apply headphone virtualization to Line Out too (SBC: Speakers settings).')

    #
    # Decoder [HID]
    #
    decoder_hid_group = parser.add_argument_group('Decoder [HID]',
                                                  description="Control decoder features using the G6's USB HID interface.")
    decoder_hid_group.add_argument('--decoder-mode', required=False, type=str,
                                   choices=decoder_modes,
                                   metavar="{Normal|Full|Night}",
                                   help='Set decoder mode.')

    #
    # Lighting [HID]
    #
    lighting_hid_group = parser.add_argument_group('Lighting [HID]',
                                                   description="Control lighting features using the G6's USB HID interface.")
    lighting_hid_group.add_argument('--lighting-disable', required=False, action='store_true',
                                    help='Disable device lighting.')
    lighting_hid_group.add_argument('--lighting-rgb', required=False, type=int, nargs=3,
                                    metavar=('{0..255}', '{0..255}', '{0..255}'),
                                    help='Enable lighting and set RGB.')
    lighting_hid_group.add_argument('--lighting-volume-ring', required=False,
                                    choices=['Enabled', 'Disabled'],
                                    help='Enable or disable the volume ring light (bright LED under the volume knob).')
    lighting_hid_group.add_argument('--lighting-effect', required=False, choices=list(LIGHTING_EFFECTS),
                                    help='Animate the RGB light from this computer, like SoundBlaster Command does '
                                         '(the G6 stores no effect). Runs in the foreground until Ctrl+C or '
                                         '--lighting-effect-duration, then leaves the static colour. Pulsate uses '
                                         'the --lighting-rgb colour (default red). Music follows the level of what '
                                         'the G6 plays (Windows), in the --lighting-rgb colour or a slow colour '
                                         'cycle without one.')
    lighting_hid_group.add_argument('--lighting-effect-period', required=False, type=_seconds, metavar='SECONDS',
                                    help='Seconds per round: pulsate dark-bright-dark (default 2), cycle once '
                                         'around the colour wheel (default 6), music colour cycle (default 12).')
    lighting_hid_group.add_argument('--lighting-effect-duration', required=False, type=_seconds, metavar='SECONDS',
                                    help='Stop the effect after this many seconds (default: run until Ctrl+C).')

    #
    # Mixer [Audio]
    #
    mixer_audio_group = parser.add_argument_group('Mixer [Audio]',
                                                  description="Control mixer features using the G6's USB AudioControl interface. See `--claim-and-release` how to use these features.")
    mixer_audio_group.add_argument('--mixer-playback-mute', required=False, type=str, choices=enabled_disabled,
                                   metavar="{Enabled|Disabled}",
                                   help='Mute/unmute mixer playback.')

    mixer_audio_group.add_argument('--mixer-monitoring-line-in-mute', required=False, type=str,
                                   choices=enabled_disabled,
                                   metavar="{Enabled|Disabled}",
                                   help='Mute/unmute line-in monitoring.')
    mixer_audio_group.add_argument('--mixer-monitoring-line-in-volume', required=False, type=int,
                                   choices=numbers_0_100_step_10,
                                   metavar="{0|10|20|..|100}",
                                   help='Set line-in monitoring volume as integer.')
    mixer_audio_group.add_argument('--mixer-monitoring-line-in-volume-channels', required=False, type=str,
                                   choices=channels, default='Both',
                                   metavar="{Both|Left|Right}",
                                   help='Define channels for --mixer-monitoring-line-in-volume.')

    mixer_audio_group.add_argument('--mixer-monitoring-external-mic-mute', required=False, type=str,
                                   choices=enabled_disabled,
                                   metavar="{Enabled|Disabled}",
                                   help='Mute/unmute external mic monitoring.')
    mixer_audio_group.add_argument('--mixer-monitoring-external-mic-volume', required=False, type=int,
                                   choices=numbers_0_100_step_10,
                                   metavar="{0|10|20|..|100}",
                                   help='Set external mic monitoring volume as integer.')
    mixer_audio_group.add_argument('--mixer-monitoring-external-mic-volume-channels', required=False, type=str,
                                   choices=channels, default='Both',
                                   metavar="{Both|Left|Right}",
                                   help='Define channels for --mixer-monitoring-external-mic-volume.')

    mixer_audio_group.add_argument('--mixer-monitoring-spdif-in-mute', required=False, type=str,
                                   choices=enabled_disabled,
                                   metavar="{Enabled|Disabled}",
                                   help='Mute/unmute spdif-in monitoring.')
    mixer_audio_group.add_argument('--mixer-monitoring-spdif-in-volume', required=False, type=int,
                                   choices=numbers_0_100_step_10,
                                   metavar="{0|10|20|..|100}",
                                   help='Set spdif-in monitoring volume as integer.')
    mixer_audio_group.add_argument('--mixer-monitoring-spdif-in-volume-channels', required=False, type=str,
                                   choices=channels, default='Both',
                                   metavar="{Both|Left|Right}",
                                   help='Define channels for --mixer-monitoring-spdif-in-volume.')

    mixer_audio_group.add_argument('--mixer-recording-line-in-mute', required=False, type=str, choices=enabled_disabled,
                                   metavar="{Enabled|Disabled}",
                                   help='Mute/unmute line-in recording.')
    mixer_audio_group.add_argument('--mixer-recording-line-in-volume', required=False, type=int,
                                   choices=numbers_0_100_step_10,
                                   metavar="{0|10|20|..|100}",
                                   help='Set line-in recording volume as integer.')
    mixer_audio_group.add_argument('--mixer-recording-line-in-volume-channels', required=False, type=str,
                                   choices=channels, default='Both',
                                   metavar="{Both|Left|Right}",
                                   help='Define channels for --mixer-recording-line-in-volume.')

    mixer_audio_group.add_argument('--mixer-recording-external-mic-mute', required=False, type=str,
                                   choices=enabled_disabled,
                                   metavar="{Enabled|Disabled}",
                                   help='Mute/unmute external mic recording.')
    mixer_audio_group.add_argument('--mixer-recording-external-mic-volume', required=False, type=int,
                                   choices=numbers_0_100_step_10,
                                   metavar="{0|10|20|..|100}",
                                   help='Set external mic recording volume as integer.')
    mixer_audio_group.add_argument('--mixer-recording-external-mic-volume-channels', required=False, type=str,
                                   choices=channels, default='Both',
                                   metavar="{Both|Left|Right}",
                                   help='Define channels for --mixer-recording-external-mic-volume.')

    mixer_audio_group.add_argument('--mixer-recording-spdif-in-mute', required=False, type=str,
                                   choices=enabled_disabled,
                                   metavar="{Enabled|Disabled}",
                                   help='Mute/unmute spdif-in recording.')
    mixer_audio_group.add_argument('--mixer-recording-spdif-in-volume', required=False, type=int,
                                   choices=numbers_0_100_step_10,
                                   metavar="{0|10|20|..|100}",
                                   help='Set spdif-in recording volume as integer.')
    mixer_audio_group.add_argument('--mixer-recording-spdif-in-volume-channels', required=False, type=str,
                                   choices=channels, default='Both',
                                   metavar="{Both|Left|Right}",
                                   help='Define channels for --mixer-recording-spdif-in-volume.')

    mixer_audio_group.add_argument('--mixer-recording-what-u-hear-mute', required=False, type=str,
                                   choices=enabled_disabled,
                                   metavar="{Enabled|Disabled}",
                                   help='Mute/unmute what-u-hear recording.')
    mixer_audio_group.add_argument('--mixer-recording-what-u-hear-volume', required=False, type=int,
                                   choices=numbers_0_100_step_10,
                                   metavar="{0|10|20|..|100}",
                                   help='Set what-u-hear recording volume as integer.')
    mixer_audio_group.add_argument('--mixer-recording-what-u-hear-volume-channels', required=False, type=str,
                                   choices=channels, default='Both',
                                   metavar="{Both|Left|Right}",
                                   help='Define channels for --mixer-recording-what-u-hear-volume.')

    #
    # Recording [HID / Audio]
    #
    recording_hid_group = parser.add_argument_group('Recording [HID]',
                                                    description="Control recording features using the G6's USB HID interface.")
    recording_audio_group = parser.add_argument_group('Recording [Audio]',
                                                      description="Control recording features using the G6's USB AudioControl interface. See `--claim-and-release` how to use these features.")

    recording_audio_group.add_argument('--recording-mute', required=False, type=str, choices=enabled_disabled,
                                       metavar="{Enabled|Disabled}",
                                       help='Mute/unmute recording.')
    recording_audio_group.add_argument('--recording-mic-recording-volume', required=False, type=int,
                                       choices=numbers_0_100_step_10,
                                       metavar="{0|10|20|..|100}",
                                       help='Set mic recording volume as integer.')
    recording_audio_group.add_argument('--recording-mic-recording-volume-channels', required=False, type=str,
                                       choices=channels,
                                       default='Both',
                                       metavar="{Both|Left|Right}",
                                       help='Define channels --recording-mic-recording-volume.')
    recording_hid_group.add_argument('--recording-mic-boost-db', required=False, type=int,
                                     choices=numbers_0_30_step_10,
                                     metavar="{0|10|20|30}",
                                     help='Set mic boost in dB as integer.')

    recording_audio_group.add_argument('--recording-mic-monitoring-mute', required=False, type=str,
                                       choices=enabled_disabled,
                                       metavar="{Enabled|Disabled}",
                                       help='Enable/disable mic monitoring.')
    recording_audio_group.add_argument('--recording-mic-monitoring-volume', required=False, type=int,
                                       choices=numbers_0_100_step_10,
                                       metavar="{0|10|20|..|100}",
                                       help='Set mic monitoring volume as integer.')
    recording_audio_group.add_argument('--recording-mic-monitoring-volume-channels', required=False, type=str,
                                       choices=channels,
                                       default='Both',
                                       metavar="{Both|Left|Right}",
                                       help='Define channels for --recording-mic-monitoring-volume.')

    recording_hid_group.add_argument('--recording-voice-clarity', required=False, type=str, choices=enabled_disabled,
                                     metavar="{Enabled|Disabled}",
                                     help='Enable/disable voice clarity.')
    recording_hid_group.add_argument('--recording-voice-clarity-noise-reduction', required=False, type=int,
                                     choices=numbers_0_100_step_20,
                                     metavar="{0|20|40|..|100}",
                                     help='Set noise reduction level as integer.')
    recording_hid_group.add_argument('--recording-voice-clarity-aec', required=False, type=str,
                                     choices=enabled_disabled,
                                     metavar="{Enabled|Disabled}",
                                     help='Enable/disable acoustic echo cancellation (AEC).')
    recording_hid_group.add_argument('--recording-voice-clarity-smart-volume', required=False, type=str,
                                     choices=enabled_disabled,
                                     metavar="{Enabled|Disabled}",
                                     help='Enable/disable smart volume.')
    recording_hid_group.add_argument('--recording-voice-clarity-mic-eq', required=False, type=str,
                                     choices=enabled_disabled,
                                     metavar="{Enabled|Disabled}",
                                     help='Enable/disable mic equalizer.')
    recording_hid_group.add_argument('--recording-voice-clarity-mic-eq-preset', required=False, type=str,
                                     choices=[e.name for e in MicrophoneEqualizerPreset],
                                     metavar=(
                                         "{PRESET_1|PRESET_2|PRESET_3|PRESET_4|PRESET_5|PRESET_6|PRESET_7|PRESET_8|PRESET_9|"
                                         "PRESET_10|PRESET_DM_1}"
                                     ),
                                     help='Set mic equalizer preset by enum name.')

    #
    # Sound Effects (SBX) [HID]
    #
    sbx_hid_group = parser.add_argument_group('SBX [HID]',
                                              description="Control SBX effects using the G6's USB HID interface.")
    sbx_hid_group.add_argument('--sbx-surround', required=False, type=str, choices=enabled_disabled,
                               metavar="{Enabled|Disabled}",
                               help='Enables or disables the Surround sound effect.')
    sbx_hid_group.add_argument('--sbx-surround-value', required=False, type=int, choices=numbers_0_100,
                               metavar="{0..100}",
                               help='Set the value for the Surround sound effect as integer.')

    sbx_hid_group.add_argument('--sbx-crystalizer', required=False, type=str, choices=enabled_disabled,
                               metavar="{Enabled|Disabled}",
                               help='Enables or disables the Crystalizer sound effect.')
    sbx_hid_group.add_argument('--sbx-crystalizer-value', required=False, type=int, choices=numbers_0_100,
                               metavar="{0..100}",
                               help='Set the value for the Crystalizer sound effect as integer.')

    sbx_hid_group.add_argument('--sbx-bass', required=False, type=str, choices=enabled_disabled,
                               metavar="{Enabled|Disabled}",
                               help='Enables or disables the Bass sound effect.')
    sbx_hid_group.add_argument('--sbx-bass-value', required=False, type=int, choices=numbers_0_100,
                               metavar="{0..100}",
                               help='Set the value for the Bass sound effect as integer.')

    sbx_hid_group.add_argument('--sbx-smart-volume', required=False, type=str, choices=enabled_disabled,
                               metavar="{Enabled|Disabled}",
                               help='Enables or disables the Smart-Volume sound effect.')
    sbx_hid_group.add_argument('--sbx-smart-volume-value', required=False, type=int, choices=numbers_0_100,
                               metavar="{0..100}",
                               help='Set the value for the Smart-Volume sound effect as value.')
    sbx_hid_group.add_argument('--sbx-smart-volume-special-value', required=False, type=str, choices=['Night', 'Loud'],
                               metavar="{Night|Loud}",
                               help='Set the value for the Smart-Volume sound effect as string (supersedes --set-smart-volume-value).')

    sbx_hid_group.add_argument('--sbx-dialog-plus', required=False, type=str, choices=enabled_disabled,
                               metavar="{Enabled|Disabled}",
                               help='Enables or disables the Dialog-Plus sound effect.')
    sbx_hid_group.add_argument('--sbx-dialog-plus-value', required=False, type=int, choices=numbers_0_100,
                               metavar="{0..100}",
                               help='Set the value for the Dialog-Plus sound effect as integer.')
    sbx_hid_group.add_argument('--sbx', required=False, type=str, choices=enabled_disabled,
                               metavar="{Enabled|Disabled}",
                               help='SBX master switch (SBC: SBX Profile toggle). Excludes Scout Mode.')
    sbx_hid_group.add_argument('--scout-mode', required=False, type=str, choices=enabled_disabled,
                               metavar="{Enabled|Disabled}",
                               help='Scout Mode. Excludes the SBX effects.')

    #
    # Equalizer [HID]
    #
    eq_hid_group = parser.add_argument_group('Equalizer [HID]',
                                             description="Graphic equalizer, gains in dB from -12 to 12.")
    eq_hid_group.add_argument('--eq', required=False, type=str, choices=enabled_disabled,
                              metavar="{Enabled|Disabled}", help='Enable/disable the equalizer.')
    eq_hid_group.add_argument('--eq-preamp', required=False, type=float, metavar='DB',
                              help='Equalizer preamp in dB.')
    eq_hid_group.add_argument('--eq-band', required=False, nargs=2, action='append', metavar=('FREQ', 'DB'),
                              help=f'Set one band, e.g. --eq-band 1k -2.5 (repeatable). '
                                   f'FREQ: {", ".join(EQ_BAND_FREQS)}.')
    eq_hid_group.add_argument('--eq-preset', required=False, type=str, metavar='NAME',
                              help='Apply one of SoundBlaster Command\'s presets (preamp and all ten bands), e.g. '
                                   '"Rock" or "hip hop"; --eq-preamp and --eq-band given too are applied after it. '
                                   'Uses the curve for the current output (read from the G6). Names: '
                                   + ', '.join(EQ_PRESETS) + '.')
    profile_group = parser.add_argument_group(
        'Profiles', description="Save the G6's sound settings to a JSON file and apply them again (HID settings only: "
                               "no output switch, RGB colour or volumes).")
    profile_group.add_argument('--profile-save', required=False, type=str, metavar='FILE',
                               help='Read the current settings from the G6 and write them to FILE.')
    profile_group.add_argument('--profile-load', required=False, type=str, metavar='FILE',
                               help='Apply the settings in FILE. Other settings given in the same command are applied '
                                    'after it.')
    eq_hid_group.add_argument('--eq-preset-output', required=False, type=str, choices=['Speakers', 'Headphones'],
                              help='Use the preset\'s curve for this output instead of the current one.')

    # parse args and verify
    args = parser.parse_args(argv)
    if args.reload_audio_services_no_sudo:
        args.reload_audio_services = True

    if not has_action(args):
        message = 'No meaningful argument has been specified!'
        print(message)
        parser.print_help()
        raise ValueError(message)
    elif args.toggle_output is True and args.set_output is not None:
        message = "Only one of the following CLI arguments may be specified: '--toggle-output', '--set-output'!"
        print(message)
        parser.print_help()
        raise ValueError(message)
    if args.eq_preset is not None:
        try:
            args.eq_preset = find_eq_preset(args.eq_preset)
        except ValueError as e:
            parser.error(str(e))

    return args


def read_toggle_state_file(toggle_state_file_path):
    """
    Read the toggle state from the temporary file to determine the state from previous runs.
    :param toggle_state_file_path: The path to the file, where the last toggle state has been remembered in.
    :return: The content of the file. Should be either 'Speakers' or 'Headphones'.
    """
    with open(toggle_state_file_path, 'r') as file:
        return file.read()


def write_toggle_state_file(toggle_state_file_path, toggle_state_value):
    """
    Write the currently used toggle_state to the temporary file for next runs.
    :param toggle_state_file_path: The path to the file for remembering the last set toggle state.
    :param toggle_state_value: The toggle state to write to the file. Should be either 'Speakers' or 'Headphones'.
    """
    with open(toggle_state_file_path, 'w') as file:
        file.write(str(toggle_state_value))


def determine_toggle_state():
    """
    Reads the last used toggle_state value from the temporary file to determine the next value.
    If the temporary file does not exist, 'Speakers' is used by default.
    :return: The just set and now active toggle state value.
    """
    toggle_state_file_path = os.path.join(tempfile.gettempdir(), TOGGLE_STATE_TEMP_FILE_NAME)
    # determine toggle state from a temporary file or use SPEAKERS by default
    if os.path.exists(toggle_state_file_path):
        current_toggle_state = read_toggle_state_file(toggle_state_file_path)
        next_toggle_state = TOGGLE_STATE_SPEAKERS \
            if current_toggle_state == TOGGLE_STATE_HEADPHONES \
            else TOGGLE_STATE_HEADPHONES
        print(
            f'Toggle from '
            f'{current_toggle_state} -> {next_toggle_state}')
    else:
        next_toggle_state = TOGGLE_STATE_SPEAKERS
        print(f'Toggle to {next_toggle_state}')
    # write the next toggle state to a temporary file
    write_toggle_state_file(toggle_state_file_path, next_toggle_state)
    # return the next toggle state to send it to the G6
    return next_toggle_state


def device_toggle_output(api: G6Api):
    """
    Toggles the device's output. Either Speakers -> Headphones or Headphones -> Speakers.
    :param api: The G6Api instance to use for communication with the device.
    """
    # determine the next toggle state
    toggle_state = determine_toggle_state()
    # switch to the state just printed and remembered (this used to send the opposite)
    device_set_output(api=api, toggle_state=toggle_state)


def _current_output(dry_run: bool) -> str:
    """'Speakers' or 'Headphones' as the G6 reports it (HID read only); Headphones when it can't be read."""
    if not dry_run:
        try:
            output = read_status().output
        except (IOError, OSError, ValueError):
            output = None
        if output in ('Speakers', 'Headphones'):
            return output
    return 'Headphones'


def device_set_output(api: G6Api, toggle_state: str):
    """
    Set a specific device output. Either 'Speakers' or 'Headphones'
    :param api: The G6Api instance to use for communication with the device.
    :param toggle_state: The toggle_state value to set the G6's output to. Should be either 'Speakers' or 'Headphones'.
    """
    # toggle playback output to the specified toggle_state
    if toggle_state == TOGGLE_STATE_SPEAKERS:
        api.playback_toggle_to_speakers()
    elif toggle_state == TOGGLE_STATE_HEADPHONES:
        api.playback_toggle_to_headphones()
    else:
        raise ValueError(
            f'The given toggle_state must either be {TOGGLE_STATE_SPEAKERS} or {TOGGLE_STATE_HEADPHONES}, '
            f'but was {toggle_state}!')


def device_set_audio_effects(api: G6Api, args: argparse.Namespace):
    """
    Processes all CLI arguments and calls the required API functions.
    :param api: The G6Api instance to use for communication with the device.
    :param args: The CLI arguments, recently parsed by argparse in parse_cli_args()
    """

    # Double-check if the G6's USB HID interface is available
    if not api.is_hid_interface_available():
        raise RuntimeError('The G6 HID interface should be available at this point!')

    # Try to claim the USB AudioControl interface first, if requested.
    if args.claim_and_release:
        # claim the USB AudioControl interface
        api.claim_audio_interface()

        # double-check if the USB AudioControl interface is available
        if not api.is_audio_interface_available():
            raise RuntimeError(
                'The G6 AudioControl interface should be available at this point! Claiming the interface failed.')

    # Process audio features
    try:

        #
        # Playback
        #
        # toggle or set playback output
        if args.toggle_output:
            device_toggle_output(api=api)
        elif args.set_output is not None:
            device_set_output(api=api, toggle_state=args.set_output)
        # playback mute
        if args.playback_mute is not None:
            # 'Enabled' means mute=True (same semantics as other toggles)
            api.playback_mute(mute=to_bool(args.playback_mute))
        # playback volume
        if args.playback_volume is not None:
            api.playback_volume(
                volume_percent=args.playback_volume,
                channels=_channels_from_cli(args.playback_volume_channels))
        # playback output mode
        if args.playback_speakers_to_stereo:
            api.playback_speakers_to_stereo()
        if args.playback_speakers_to_5_1:
            api.playback_speakers_to_5_1()
        if args.playback_speakers_to_7_1:
            api.playback_speakers_to_7_1()
        if args.playback_headphones_to_stereo:
            api.playback_headphones_to_stereo()
        if args.playback_headphones_to_5_1:
            api.playback_headphones_to_5_1()
        if args.playback_headphones_to_7_1:
            api.playback_headphones_to_7_1()
        # playback direct mode
        if args.playback_direct_mode is not None:
            api.playback_enable_direct_mode(enable=to_bool(args.playback_direct_mode))
        # playback spdif-out direct mode
        if args.playback_spdif_out_direct_mode is not None:
            api.playback_enable_spdif_out_direct_mode(enable=to_bool(args.playback_spdif_out_direct_mode))
        # playback filter
        if args.playback_filter is not None:
            api.playback_filter(playback_filter_enum=PlaybackFilter[args.playback_filter])
        # playback decoder mode
        if args.decoder_mode is not None:
            api.decoder_mode(decoder_mode_enum=_decoder_mode_from_cli(args.decoder_mode))

        #
        # Lighting
        #
        if args.lighting_disable:
            api.lighting_disable()
        if args.lighting_rgb is not None:
            red, green, blue = args.lighting_rgb
            api.lighting_enable_set_rgb(red=red, green=green, blue=blue)
        if args.lighting_volume_ring is not None:
            api.lighting_volume_ring(enable=(args.lighting_volume_ring == 'Enabled'))

        #
        # Mixer
        #
        # playback mute
        if args.mixer_playback_mute is not None:
            api.mixer_playback_mute(mute=to_bool(args.mixer_playback_mute))
        # monitoring > line-in
        if args.mixer_monitoring_line_in_mute is not None:
            api.mixer_monitoring_line_in_mute(mute=to_bool(args.mixer_monitoring_line_in_mute))
        if args.mixer_monitoring_line_in_volume is not None:
            api.mixer_monitoring_line_in_volume(
                volume_percent=args.mixer_monitoring_line_in_volume,
                channels=_channels_from_cli(args.mixer_monitoring_line_in_volume_channels))
        # monitoring > external-mic
        if args.mixer_monitoring_external_mic_mute is not None:
            api.mixer_monitoring_external_mic_mute(mute=to_bool(args.mixer_monitoring_external_mic_mute))
        if args.mixer_monitoring_external_mic_volume is not None:
            api.mixer_monitoring_external_mic_volume(
                volume_percent=args.mixer_monitoring_external_mic_volume,
                channels=_channels_from_cli(args.mixer_monitoring_external_mic_volume_channels))
        # monitoring > spdif-in
        if args.mixer_monitoring_spdif_in_mute is not None:
            api.mixer_monitoring_spdif_in_mute(mute=to_bool(args.mixer_monitoring_spdif_in_mute))
        if args.mixer_monitoring_spdif_in_volume is not None:
            api.mixer_monitoring_spdif_in_volume(
                volume_percent=args.mixer_monitoring_spdif_in_volume,
                channels=_channels_from_cli(args.mixer_monitoring_spdif_in_volume_channels))
        # recording > line-in
        if args.mixer_recording_line_in_mute is not None:
            api.mixer_recording_line_in_mute(mute=to_bool(args.mixer_recording_line_in_mute))
        if args.mixer_recording_line_in_volume is not None:
            api.mixer_recording_line_in_volume(
                volume_percent=args.mixer_recording_line_in_volume,
                channels=_channels_from_cli(args.mixer_recording_line_in_volume_channels))
        # recording > external-mic
        if args.mixer_recording_external_mic_mute is not None:
            api.mixer_recording_external_mic_mute(mute=to_bool(args.mixer_recording_external_mic_mute))
        if args.mixer_recording_external_mic_volume is not None:
            api.mixer_recording_external_mic_volume(
                volume_percent=args.mixer_recording_external_mic_volume,
                channels=_channels_from_cli(args.mixer_recording_external_mic_volume_channels))
        # recording > spdif-in
        if args.mixer_recording_spdif_in_mute is not None:
            api.mixer_recording_spdif_in_mute(mute=to_bool(args.mixer_recording_spdif_in_mute))
        if args.mixer_recording_spdif_in_volume is not None:
            api.mixer_recording_spdif_in_volume(
                volume_percent=args.mixer_recording_spdif_in_volume,
                channels=_channels_from_cli(args.mixer_recording_spdif_in_volume_channels))
        # recording > what-u-hear
        if args.mixer_recording_what_u_hear_mute is not None:
            api.mixer_recording_what_u_hear_mute(mute=to_bool(args.mixer_recording_what_u_hear_mute))
        if args.mixer_recording_what_u_hear_volume is not None:
            api.mixer_recording_what_u_hear_volume(
                volume_percent=args.mixer_recording_what_u_hear_volume,
                channels=_channels_from_cli(args.mixer_recording_what_u_hear_volume_channels))

        #
        # Recording
        #
        # mic recording
        if args.recording_mute is not None:
            api.recording_mute(mute=to_bool(args.recording_mute))

        if args.recording_mic_recording_volume is not None:
            api.recording_mic_recording_volume(
                volume_percent=args.recording_mic_recording_volume,
                channels=_channels_from_cli(args.recording_mic_recording_volume_channels))
        # mic boost
        if args.recording_mic_boost_db is not None:
            api.recording_mic_boost(decibel=args.recording_mic_boost_db)
        # mic monitoring
        if args.recording_mic_monitoring_mute is not None:
            api.recording_mic_monitoring_mute(mute=to_bool(args.recording_mic_monitoring_mute))
        if args.recording_mic_monitoring_volume is not None:
            api.recording_mic_monitoring_volume(
                volume_percent=args.recording_mic_monitoring_volume,
                channels=_channels_from_cli(args.recording_mic_monitoring_volume_channels))
        # voice clarity
        if args.recording_voice_clarity is not None:
            api.recording_voice_clarity_enabled(enable=to_bool(args.recording_voice_clarity))
        if args.recording_voice_clarity_noise_reduction is not None:
            api.recording_voice_clarity_noise_reduction_level(
                level_percent=args.recording_voice_clarity_noise_reduction)
        if args.recording_voice_clarity_aec is not None:
            api.recording_voice_clarity_acoustic_echo_cancellation_enabled(
                enable=to_bool(args.recording_voice_clarity_aec))
        if args.recording_voice_clarity_smart_volume is not None:
            api.recording_voice_clarity_smart_volume_enabled(
                enable=to_bool(args.recording_voice_clarity_smart_volume))
        if args.recording_voice_clarity_mic_eq is not None:
            api.recording_voice_clarity_mic_equalizer_enabled(enable=to_bool(args.recording_voice_clarity_mic_eq))
        if args.recording_voice_clarity_mic_eq_preset is not None:
            api.recording_voice_clarity_mic_equalizer_preset(
                preset=MicrophoneEqualizerPreset[args.recording_voice_clarity_mic_eq_preset])

        #
        # SBX Effects
        #
        # surround
        if args.sbx_surround is not None:
            api.sbx_toggle(AudioFeature.SURROUND_TOGGLE, to_bool(args.sbx_surround))
        if args.sbx_surround_value is not None:
            api.sbx_slider(AudioFeature.SURROUND_SLIDER, args.sbx_surround_value)
        # crystalizer
        if args.sbx_crystalizer is not None:
            api.sbx_toggle(AudioFeature.CRYSTALIZER_TOGGLE, to_bool(args.sbx_crystalizer))
        if args.sbx_crystalizer_value is not None:
            api.sbx_slider(AudioFeature.CRYSTALIZER_SLIDER, args.sbx_crystalizer_value)
        # bass
        if args.sbx_bass is not None:
            api.sbx_toggle(AudioFeature.BASS_TOGGLE, to_bool(args.sbx_bass))
        if args.sbx_bass_value is not None:
            api.sbx_slider(AudioFeature.BASS_SLIDER, args.sbx_bass_value)
        # smart-volume
        if args.sbx_smart_volume is not None:
            api.sbx_toggle(AudioFeature.SMART_VOLUME_TOGGLE, to_bool(args.sbx_smart_volume))
        if args.sbx_smart_volume_value is not None:
            api.sbx_slider(AudioFeature.SMART_VOLUME_SLIDER, args.sbx_smart_volume_value)
        if args.sbx_smart_volume_special_value is not None:
            if args.sbx_smart_volume_special_value == 'Night':
                api.sbx_smart_volume_special(SmartVolumeSpecialHex.SMART_VOLUME_NIGHT)
            elif args.sbx_smart_volume_special_value == 'Loud':
                api.sbx_smart_volume_special(SmartVolumeSpecialHex.SMART_VOLUME_LOUD)
            else:
                raise ValueError(
                    f"Expected one of the following values for --sbx-smart-volume-special-value: "
                    f"['Night', 'Loud'], but was '{args.sbx_smart_volume_special_value}'!"
                )
        # dialog-plus
        if args.sbx_dialog_plus is not None:
            api.sbx_toggle(AudioFeature.DIALOG_PLUS_TOGGLE, to_bool(args.sbx_dialog_plus))
        if args.sbx_dialog_plus_value is not None:
            api.sbx_slider(AudioFeature.DIALOG_PLUS_SLIDER, args.sbx_dialog_plus_value)

        # processing modes: switch the old one off before the new one on (they exclude each other)
        for mode, value in sorted([(ProcessingMode.SBX, args.sbx), (ProcessingMode.SCOUT_MODE, args.scout_mode)],
                                  key=lambda mv: mv[1] == 'Enabled'):
            if value is not None:
                api.processing_mode(mode, enable=to_bool(value))

        # equalizer
        if args.eq is not None:
            api.equalizer_enabled(enable=to_bool(args.eq))
        if args.eq_preset is not None:
            preamp, bands = eq_preset_values(args.eq_preset, args.eq_preset_output or _current_output(args.dry_run))
            api.equalizer_preamp(db=preamp)
            for freq, db in bands.items():
                api.equalizer_band(freq=freq, db=db)
        if args.eq_preamp is not None:
            api.equalizer_preamp(db=args.eq_preamp)
        for freq, db in args.eq_band or []:
            api.equalizer_band(freq=freq, db=float(db))

        if args.playback_line_out_headphone_virtualization is not None:
            api.playback_line_out_headphone_virtualization(
                enable=to_bool(args.playback_line_out_headphone_virtualization))
    finally:
        # Release the USB AudioControl interface if it has been claimed.
        if args.claim_and_release and api.is_audio_interface_available():
            api.release_audio_interface()

        # Reload ALSA and PipeWire, if requested.
        if args.reload_audio_services:
            use_sudo = not args.reload_audio_services_no_sudo
            api.reload_alsa_and_pipewire(sudo=use_sudo)


def format_mixer_rows(mixer) -> list:
    """(label, text) rows for the mixer part of the status; empty when the mixer could not be read."""
    if not mixer:
        return []

    def level(entry):
        parts = []
        if entry.get('level_percent') is not None:
            parts.append(f"{entry['level_percent']}%")
        if entry.get('level_db') is not None:
            parts.append(f"{entry['level_db']:+.1f} dB")
        if not parts:  # mute-only control
            return {True: 'muted', False: 'not muted'}.get(entry.get('muted'), '?')
        return ' '.join(parts) + (' (muted)' if entry.get('muted') else '')

    rows = []
    for section, label in (('playback', 'Playback vol'), ('recording', 'Recording'), ('monitoring', 'Monitoring')):
        for i, (name, entry) in enumerate(mixer.get(section, {}).items()):
            rows.append((label if i == 0 else '', f'{name}: {level(entry)}'))
    if mixer.get('speaker_layout'):
        rows.append(('Speaker setup', f"{mixer['speaker_layout']} (Windows)"))
    return rows


def format_status(status: DeviceStatus, as_json: bool) -> str:
    """
    Render a DeviceStatus for the console, either as JSON or as aligned 'key: value' lines.
    """
    if as_json:
        return json.dumps(status.to_dict(), indent=2)

    def fmt(value):
        if value is None:
            return '?'
        if isinstance(value, bool):
            return 'On' if value else 'Off'
        return str(value)

    rows = [
        ('Firmware', status.firmware),
        ('Output', status.output),
        ('Decoder mode', status.decoder_mode),
        ('DAC filter', DAC_FILTER_LABELS.get(status.dac_filter, status.dac_filter)),
        ('Direct mode', status.direct_mode),
        ('SPDIF direct', status.spdif_direct),
        ('Line Out virt', status.line_out_headphone_virtualization),
        ('RGB light', status.rgb_enabled),
        ('Volume ring', status.ring_enabled),
    ]
    rows.append(('SBX', status.sbx_enabled))
    rows.append(('Scout mode', status.scout_mode))
    sbx_labels = {'surround': 'Surround', 'crystalizer': 'Crystalizer', 'bass': 'Bass',
                  'smart_volume': 'Smart volume', 'dialog_plus': 'Dialog+'}
    for name, effect in (status.sbx or {}).items():
        level = '?' if effect['level'] is None else effect['level']
        value = f"{fmt(effect['enabled'])}, {level}"
        if name == 'smart_volume' and status.smart_volume_mode:
            value += f', {status.smart_volume_mode}'
        rows.append((sbx_labels.get(name, name), value))
    rows.append(('Equalizer', status.eq_enabled))
    if status.eq_bands_db is not None:
        def db(v):
            return '?' if v is None else f'{v:g}'
        bands = ' '.join(f'{freq}:{db(v)}' for freq, v in status.eq_bands_db.items())
        rows.append(('EQ (dB)', f'preamp {db(status.eq_preamp_db)} | {bands}'))
    rows.append(('Mic boost', None if status.mic_boost_db is None else f'{status.mic_boost_db} dB'))
    rows.append(('Voice clarity', status.voice_clarity))
    rec = status.recording
    if rec is not None:
        level = '?' if rec['noise_reduction_level'] is None else rec['noise_reduction_level']
        rows.append(('Noise reduct.', f"{fmt(rec['noise_reduction'])}, {level}"))
        rows.append(('Echo cancel.', rec['acoustic_echo_cancellation']))
        rows.append(('Mic smart vol', rec['smart_volume']))
        mic_eq = fmt(rec['mic_eq'])
        if status.mic_eq_preset:
            mic_eq += f', {status.mic_eq_preset}'
        rows.append(('Mic EQ', mic_eq))
    rows.extend(format_mixer_rows(status.mixer))
    return '\n'.join(f'{label:<13} {fmt(value)}' for label, value in rows)


def _only_profile_save(args: argparse.Namespace) -> bool:
    saved = args.profile_save
    args.profile_save = None
    try:
        return not has_action(args)
    finally:
        args.profile_save = saved


def _only_status_requested(args: argparse.Namespace) -> bool:
    saved = args.status
    args.status = False
    try:
        return not has_action(args)
    finally:
        args.status = saved


def _without_option(argv: list[str], option: str) -> list[str]:
    """``argv`` without ``option VALUE`` / ``option=VALUE``."""
    out, skip = [], False
    for a in argv:
        if skip:
            skip = False
        elif a == option:
            skip = True
        elif not a.startswith(option + '='):
            out.append(a)
    return out


def main(argv: list[str] | None = None):
    # parse CLI arguments
    args = parse_cli_args(argv)

    # profiles: save first (so "--profile-save old.json --profile-load new.json" keeps a backup), then expand the
    # loaded profile into ordinary arguments, ahead of the ones given on the command line
    if args.profile_save is not None:
        profile = save_profile(args.profile_save, read_status())
        print(f"Saved {len(profile['settings'])} settings to {args.profile_save}")
    if args.profile_load is not None:
        rest = _without_option(_without_option(sys.argv[1:] if argv is None else list(argv), '--profile-load'),
                               '--profile-save')
        args = parse_cli_args(load_profile_argv(args.profile_load) + rest)
    elif args.profile_save is not None and _only_profile_save(args):
        return

    # read-only status query (HID only, no USB claim needed)
    if args.status:
        status = read_status()
        print(format_status(status, as_json=args.json))
        if _only_status_requested(args):
            return

    # look up the G6 device
    api = G6Api(dry_run=args.dry_run, debug=args.debug)

    # process arguments and call the api
    device_set_audio_effects(api=api, args=args)

    # host-side lighting effect: runs last, in the foreground
    if args.lighting_effect is not None:
        default_rgb = None if args.lighting_effect == 'music' else (255, 0, 0)
        run_lighting_effect(api, args.lighting_effect,
                            rgb=tuple(args.lighting_rgb) if args.lighting_rgb else default_rgb,
                            period=args.lighting_effect_period, duration=args.lighting_effect_duration,
                            dry_run=args.dry_run, quiet=not args.debug)


def run_lighting_effect(api, effect: str, rgb=(255, 0, 0), period: float | None = None,
                        duration: float | None = None, dry_run: bool = False, quiet: bool = False,
                        clock=time.monotonic, sleep=time.sleep, meter=None) -> int:
    """
    Stream effect frames until ``duration`` runs out or Ctrl+C, then set the static colour again (red if ``rgb``
    is None). ``meter`` (music only) returns the output peak 0..1; by default the G6's Windows output meter.
    A dry run sends one period's frames without waiting. Returns the number of frames sent.
    The HID handle stays open for the whole run (opening it per frame caps the stream at ~10 frames/s).
    ``quiet`` drops the transport's per-frame log lines.
    """
    period = period or LIGHTING_EFFECT_PERIOD[effect]
    if period <= 0:
        raise ValueError(f'--lighting-effect-period must be > 0, got {period}')
    follower = None
    if effect == 'music':
        if meter is None:
            if dry_run:
                meter = lambda: 0.0  # noqa: E731
            else:
                from g6_cli.g6_os_audio import open_output_meter
                meter = open_output_meter()
        if meter is None:
            raise ValueError('--lighting-effect music needs the G6 output level, which is only readable on Windows')
        follower = LevelFollower()
    if dry_run:
        duration = min(duration, period) if duration is not None else period
    elif duration is None:
        print(f'Lighting effect {effect} running, Ctrl+C to stop.', file=sys.stderr)
    frames = 0
    held = getattr(api, 'hid_held_open', None)
    start = clock()
    try:
        with held() if held else contextlib.nullcontext():
            while True:
                t = frames * LIGHTING_FRAME_INTERVAL if dry_run else clock() - start
                if duration is not None and t >= duration:
                    break
                level = None
                if follower is not None:
                    level = follower.update(level_to_brightness(meter()), LIGHTING_FRAME_INTERVAL)
                with contextlib.redirect_stdout(io.StringIO()) if quiet else contextlib.nullcontext():
                    api.lighting_frame(*effect_frame(effect, t, period, rgb, level))
                frames += 1
                if not dry_run:
                    sleep(max(0.0, start + frames * LIGHTING_FRAME_INTERVAL - clock()))
    except KeyboardInterrupt:
        pass
    finally:
        api.lighting_enable_set_rgb(*(rgb or (255, 0, 0)))
    return frames


def cli_entry() -> None:
    """
    Console-script entry point: like main(), but turns expected failures into a message and exit code 1
    instead of a traceback, so the CLI behaves well in scripts and automation.
    """
    try:
        main()
    except (IOError, OSError, RuntimeError, ValueError) as e:
        print(f'error: {e}', file=sys.stderr)
        sys.exit(1)
