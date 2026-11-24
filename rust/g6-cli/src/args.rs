//! The command line, as Python's `parse_cli_args` defines it: same flags, value names, choices and defaults,
//! grouped under the same headings. [`crate::argparse`] reads the arguments the way argparse does (abbreviations,
//! values that start with "-", its error messages) and clap then fills in [`Cli`]. A repeated flag keeps the last
//! value; `--eq-band` collects every pair.

use clap::{ArgAction, CommandFactory, FromArgMatches, Parser};

use crate::argparse::Early;

use g6_proto::apply::{MixerSetting, Request, SbxSetting, Volume};
use g6_proto::eq_presets::preset_ids;
use g6_proto::equalizer::EQ_BAND_FREQS;
use g6_proto::pyfmt::{py_float, py_int, py_repr, py_str_repr};
use g6_proto::types::{
    Channels, DecoderMode, MicEqPreset, Output, PlaybackFilter, SmartVolumeSpecial,
};

/// Python's `VERSION`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// What argparse calls the program in usage lines and errors.
pub const PROG: &str = "soundblaster-x-g6-cli";

const GENERAL: &str = "General options";
const PLAYBACK_HID: &str = "Playback [HID]";
const PLAYBACK_AUDIO: &str = "Playback [Audio]";
const DECODER: &str = "Decoder [HID]";
const LIGHTING: &str = "Lighting [HID]";
const MIXER: &str = "Mixer [Audio]";
const RECORDING_HID: &str = "Recording [HID]";
const RECORDING_AUDIO: &str = "Recording [Audio]";
const SBX: &str = "SBX [HID]";
const EQUALIZER: &str = "Equalizer [HID]";
const PROFILES: &str = "Profiles";

const ED: [&str; 2] = ["Enabled", "Disabled"];
const CHANNELS: [&str; 3] = ["Both", "Left", "Right"];

// argparse converts with int() first and then checks the choices, naming them all
fn int_choice(s: &str, choices: impl Iterator<Item = i64> + Clone) -> Result<i64, String> {
    let v = any_int(s)?;
    if choices.clone().any(|c| c == v) {
        return Ok(v);
    }
    let names: Vec<String> = choices.map(|c| format!("'{c}'")).collect();
    Err(format!(
        "invalid choice: '{v}' (choose from {})",
        names.join(", ")
    ))
}

fn int_0_100(s: &str) -> Result<i64, String> {
    int_choice(s, 0..=100)
}

fn int_0_100_step_10(s: &str) -> Result<i64, String> {
    int_choice(s, (0..=100).step_by(10))
}

fn int_0_30_step_10(s: &str) -> Result<i64, String> {
    int_choice(s, (0..=30).step_by(10))
}

fn int_0_100_step_20(s: &str) -> Result<i64, String> {
    int_choice(s, (0..=100).step_by(20))
}

fn any_int(s: &str) -> Result<i64, String> {
    py_int(s).ok_or_else(|| format!("invalid int value: {}", py_str_repr(s)))
}

fn any_float(s: &str) -> Result<f64, String> {
    py_float(s).ok_or_else(|| format!("invalid float value: {}", py_str_repr(s)))
}

/// A time option: NaN and the infinities are refused, as Python's `_seconds` does.
fn seconds(s: &str) -> Result<f64, String> {
    let v = any_float(s)?;
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!(
            "must be a finite number of seconds, got {}",
            py_str_repr(s)
        ))
    }
}

fn filter_names() -> [&'static str; 4] {
    PlaybackFilter::ALL.map(PlaybackFilter::name)
}

fn mic_eq_preset_names() -> Vec<&'static str> {
    MicEqPreset::all().map(MicEqPreset::name).collect()
}

fn eq_band_help() -> String {
    format!(
        "Set one band, e.g. --eq-band 1k -2.5 (repeatable). FREQ: {}.",
        EQ_BAND_FREQS.join(", ")
    )
}

fn eq_preset_help() -> String {
    format!(
        "Apply one of SoundBlaster Command's presets (preamp and all ten bands), e.g. \"Rock\" or \"hip hop\"; \
         --eq-preamp and --eq-band given too are applied after it. Uses the curve for the current output (read from \
         the G6). Names: {}.",
        preset_ids().collect::<Vec<_>>().join(", ")
    )
}

/// One field per argparse option, named after its `dest`, in argparse's group order.
#[derive(Parser, Debug, Clone, Default)]
#[command(
    name = PROG,
    bin_name = PROG,
    override_usage = PROG,
    about = "SoundBlaster X G6 CLI",
    version = VERSION,
    args_override_self = true,
    disable_version_flag = true,
    disable_help_flag = true
)]
pub struct Cli {
    #[arg(short = 'h', long, action = ArgAction::Help, help = "show this help message and exit")]
    pub help: Option<bool>,

    // General options
    #[arg(long, help_heading = GENERAL, action = ArgAction::SetTrue,
          help = "Used to verify the available hex_line files, without making any calls against the G6 device.")]
    pub dry_run: bool,
    #[arg(long, help_heading = GENERAL, action = ArgAction::SetTrue,
          help = "Print communication data with the G6 device to the console.")]
    pub debug: bool,
    #[arg(long, help_heading = GENERAL, action = ArgAction::Version,
          help = "show program's version number and exit")]
    pub version: Option<bool>,
    #[arg(long, help_heading = GENERAL, action = ArgAction::SetTrue,
          help = "Let the application exclusively claim the G6's USB AudioControl interface from the kernel and \
                  release it afterwards. This will disconnect the G6 device from the kernel sound driver \
                  \"snd-usb-audio\" leading the system not having any audio output. Use `--reload-audio-services` \
                  to reload the kernel sound driver and make the audio output available again.")]
    pub claim_and_release: bool,
    #[arg(long, help_heading = GENERAL, action = ArgAction::SetTrue,
          help = "Reload ALSA and restart user PipeWire services.")]
    pub reload_audio_services: bool,
    #[arg(long, help_heading = GENERAL, action = ArgAction::SetTrue,
          help = "Reload audio services, but do not use sudo for ALSA reload (implies --reload-audio-services).")]
    pub reload_audio_services_no_sudo: bool,
    #[arg(long, help_heading = GENERAL, action = ArgAction::SetTrue,
          help = "Read the current device state over HID (read-only) and print it. Runs before any other \
                  requested change.")]
    pub status: bool,
    #[arg(long, help_heading = GENERAL, action = ArgAction::SetTrue,
          help = "With --status: print the state as JSON.")]
    pub json: bool,

    // Playback [HID]
    #[arg(long, help_heading = PLAYBACK_HID, action = ArgAction::SetTrue,
          help = "Toggles the sound output between Speakers and Headphones.")]
    pub toggle_output: bool,
    #[arg(long, help_heading = PLAYBACK_HID, value_name = "{Speakers|Headphones}",
          value_parser = ["Speakers", "Headphones"], hide_possible_values = true,
          help = "Sets the sound output to the specified option.")]
    pub set_output: Option<String>,
    #[arg(long, help_heading = PLAYBACK_HID, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Enable/disable Direct Mode.")]
    pub playback_direct_mode: Option<String>,
    #[arg(long, help_heading = PLAYBACK_HID, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Enable/disable SPDIF-Out Direct Mode.")]
    pub playback_spdif_out_direct_mode: Option<String>,
    #[arg(long, help_heading = PLAYBACK_HID,
          value_name = "{FAST_ROLL_OFF_MINIMUM_PHASE|SLOW_ROLL_OFF_MINIMUM_PHASE|FAST_ROLL_OFF_LINEAR_PHASE|SLOW_ROLL_OFF_LINEAR_PHASE}",
          value_parser = filter_names(), hide_possible_values = true,
          help = "Set playback filter by enum name.")]
    pub playback_filter: Option<String>,
    #[arg(long, help_heading = PLAYBACK_HID, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true,
          help = "Apply headphone virtualization to Line Out too (SBC: Speakers settings).")]
    pub playback_line_out_headphone_virtualization: Option<String>,

    // Playback [Audio]
    #[arg(long, help_heading = PLAYBACK_AUDIO, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Mute/unmute playback.")]
    pub playback_mute: Option<String>,
    #[arg(long, help_heading = PLAYBACK_AUDIO, value_name = "{0..100}", value_parser = int_0_100,
          help = "Set playback volume as integer.")]
    pub playback_volume: Option<i64>,
    #[arg(long, help_heading = PLAYBACK_AUDIO, value_name = "{Both|Left|Right}", value_parser = CHANNELS,
          default_value = "Both", hide_possible_values = true, hide_default_value = true,
          help = "Set playback volume channels for --playback-volume.")]
    pub playback_volume_channels: String,
    #[arg(long, help_heading = PLAYBACK_AUDIO, action = ArgAction::SetTrue, help = "Switch speakers output to stereo.")]
    pub playback_speakers_to_stereo: bool,
    #[arg(long = "playback-speakers-to-5-1", help_heading = PLAYBACK_AUDIO, action = ArgAction::SetTrue,
          help = "Switch speakers output to 5.1.")]
    pub playback_speakers_to_5_1: bool,
    #[arg(long = "playback-speakers-to-7-1", help_heading = PLAYBACK_AUDIO, action = ArgAction::SetTrue,
          help = "Switch speakers output to 7.1.")]
    pub playback_speakers_to_7_1: bool,
    #[arg(long, help_heading = PLAYBACK_AUDIO, action = ArgAction::SetTrue,
          help = "Switch headphones output to stereo.")]
    pub playback_headphones_to_stereo: bool,
    #[arg(long = "playback-headphones-to-5-1", help_heading = PLAYBACK_AUDIO, action = ArgAction::SetTrue,
          help = "Switch headphones output to 5.1.")]
    pub playback_headphones_to_5_1: bool,
    #[arg(long = "playback-headphones-to-7-1", help_heading = PLAYBACK_AUDIO, action = ArgAction::SetTrue,
          help = "Switch headphones output to 7.1.")]
    pub playback_headphones_to_7_1: bool,

    // Decoder [HID]
    #[arg(long, help_heading = DECODER, value_name = "{Normal|Full|Night}", value_parser = ["Normal", "Full", "Night"],
          hide_possible_values = true, help = "Set decoder mode.")]
    pub decoder_mode: Option<String>,

    // Lighting [HID]
    #[arg(long, help_heading = LIGHTING, action = ArgAction::SetTrue, help = "Disable device lighting.")]
    pub lighting_disable: bool,
    #[arg(long, help_heading = LIGHTING, num_args = 3, value_names = ["{0..255}", "{0..255}", "{0..255}"],
          value_parser = any_int, allow_hyphen_values = true,
          // a store, as in argparse: given twice, the second colour replaces the first
          action = ArgAction::Set, help = "Enable lighting and set RGB.")]
    pub lighting_rgb: Option<Vec<i64>>,
    #[arg(long, help_heading = LIGHTING, value_name = "{Enabled,Disabled}", value_parser = ED,
          hide_possible_values = true,
          help = "Enable or disable the volume ring light (bright LED under the volume knob).")]
    pub lighting_volume_ring: Option<String>,
    #[arg(long, help_heading = LIGHTING, value_name = "{pulsate,cycle,music}",
          value_parser = ["pulsate", "cycle", "music"], hide_possible_values = true,
          help = "Animate the RGB light from this computer, like SoundBlaster Command does (the G6 stores no \
                  effect). Runs in the foreground until Ctrl+C or --lighting-effect-duration, then leaves the static \
                  colour. Pulsate uses the --lighting-rgb colour (default red). Music follows the level of what the \
                  G6 plays (Windows), in the --lighting-rgb colour or a slow colour cycle without one.")]
    pub lighting_effect: Option<String>,
    #[arg(long, help_heading = LIGHTING, value_name = "SECONDS", value_parser = seconds,
          help = "Seconds per round: pulsate dark-bright-dark (default 2), cycle once around the colour wheel \
                  (default 6), music colour cycle (default 12).")]
    pub lighting_effect_period: Option<f64>,
    #[arg(long, help_heading = LIGHTING, value_name = "SECONDS", value_parser = seconds,
          help = "Stop the effect after this many seconds (default: run until Ctrl+C).")]
    pub lighting_effect_duration: Option<f64>,

    // Mixer [Audio]
    #[arg(long, help_heading = MIXER, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Mute/unmute mixer playback.")]
    pub mixer_playback_mute: Option<String>,
    #[arg(long, help_heading = MIXER, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Mute/unmute line-in monitoring.")]
    pub mixer_monitoring_line_in_mute: Option<String>,
    #[arg(long, help_heading = MIXER, value_name = "{0|10|20|..|100}", value_parser = int_0_100_step_10,
          help = "Set line-in monitoring volume as integer.")]
    pub mixer_monitoring_line_in_volume: Option<i64>,
    #[arg(long, help_heading = MIXER, value_name = "{Both|Left|Right}", value_parser = CHANNELS,
          default_value = "Both", hide_possible_values = true, hide_default_value = true,
          help = "Define channels for --mixer-monitoring-line-in-volume.")]
    pub mixer_monitoring_line_in_volume_channels: String,
    #[arg(long, help_heading = MIXER, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Mute/unmute external mic monitoring.")]
    pub mixer_monitoring_external_mic_mute: Option<String>,
    #[arg(long, help_heading = MIXER, value_name = "{0|10|20|..|100}", value_parser = int_0_100_step_10,
          help = "Set external mic monitoring volume as integer.")]
    pub mixer_monitoring_external_mic_volume: Option<i64>,
    #[arg(long, help_heading = MIXER, value_name = "{Both|Left|Right}", value_parser = CHANNELS,
          default_value = "Both", hide_possible_values = true, hide_default_value = true,
          help = "Define channels for --mixer-monitoring-external-mic-volume.")]
    pub mixer_monitoring_external_mic_volume_channels: String,
    #[arg(long, help_heading = MIXER, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Mute/unmute spdif-in monitoring.")]
    pub mixer_monitoring_spdif_in_mute: Option<String>,
    #[arg(long, help_heading = MIXER, value_name = "{0|10|20|..|100}", value_parser = int_0_100_step_10,
          help = "Set spdif-in monitoring volume as integer.")]
    pub mixer_monitoring_spdif_in_volume: Option<i64>,
    #[arg(long, help_heading = MIXER, value_name = "{Both|Left|Right}", value_parser = CHANNELS,
          default_value = "Both", hide_possible_values = true, hide_default_value = true,
          help = "Define channels for --mixer-monitoring-spdif-in-volume.")]
    pub mixer_monitoring_spdif_in_volume_channels: String,
    #[arg(long, help_heading = MIXER, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Mute/unmute line-in recording.")]
    pub mixer_recording_line_in_mute: Option<String>,
    #[arg(long, help_heading = MIXER, value_name = "{0|10|20|..|100}", value_parser = int_0_100_step_10,
          help = "Set line-in recording volume as integer.")]
    pub mixer_recording_line_in_volume: Option<i64>,
    #[arg(long, help_heading = MIXER, value_name = "{Both|Left|Right}", value_parser = CHANNELS,
          default_value = "Both", hide_possible_values = true, hide_default_value = true,
          help = "Define channels for --mixer-recording-line-in-volume.")]
    pub mixer_recording_line_in_volume_channels: String,
    #[arg(long, help_heading = MIXER, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Mute/unmute external mic recording.")]
    pub mixer_recording_external_mic_mute: Option<String>,
    #[arg(long, help_heading = MIXER, value_name = "{0|10|20|..|100}", value_parser = int_0_100_step_10,
          help = "Set external mic recording volume as integer.")]
    pub mixer_recording_external_mic_volume: Option<i64>,
    #[arg(long, help_heading = MIXER, value_name = "{Both|Left|Right}", value_parser = CHANNELS,
          default_value = "Both", hide_possible_values = true, hide_default_value = true,
          help = "Define channels for --mixer-recording-external-mic-volume.")]
    pub mixer_recording_external_mic_volume_channels: String,
    #[arg(long, help_heading = MIXER, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Mute/unmute spdif-in recording.")]
    pub mixer_recording_spdif_in_mute: Option<String>,
    #[arg(long, help_heading = MIXER, value_name = "{0|10|20|..|100}", value_parser = int_0_100_step_10,
          help = "Set spdif-in recording volume as integer.")]
    pub mixer_recording_spdif_in_volume: Option<i64>,
    #[arg(long, help_heading = MIXER, value_name = "{Both|Left|Right}", value_parser = CHANNELS,
          default_value = "Both", hide_possible_values = true, hide_default_value = true,
          help = "Define channels for --mixer-recording-spdif-in-volume.")]
    pub mixer_recording_spdif_in_volume_channels: String,
    #[arg(long, help_heading = MIXER, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Mute/unmute what-u-hear recording.")]
    pub mixer_recording_what_u_hear_mute: Option<String>,
    #[arg(long, help_heading = MIXER, value_name = "{0|10|20|..|100}", value_parser = int_0_100_step_10,
          help = "Set what-u-hear recording volume as integer.")]
    pub mixer_recording_what_u_hear_volume: Option<i64>,
    #[arg(long, help_heading = MIXER, value_name = "{Both|Left|Right}", value_parser = CHANNELS,
          default_value = "Both", hide_possible_values = true, hide_default_value = true,
          help = "Define channels for --mixer-recording-what-u-hear-volume.")]
    pub mixer_recording_what_u_hear_volume_channels: String,

    // Recording [HID]
    #[arg(long, help_heading = RECORDING_HID, value_name = "{0|10|20|30}", value_parser = int_0_30_step_10,
          help = "Set mic boost in dB as integer.")]
    pub recording_mic_boost_db: Option<i64>,
    #[arg(long, help_heading = RECORDING_HID, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Enable/disable voice clarity.")]
    pub recording_voice_clarity: Option<String>,
    #[arg(long, help_heading = RECORDING_HID, value_name = "{0|20|40|..|100}", value_parser = int_0_100_step_20,
          help = "Set noise reduction level as integer.")]
    pub recording_voice_clarity_noise_reduction: Option<i64>,
    #[arg(long, help_heading = RECORDING_HID, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true,
          help = "Enable/disable acoustic echo cancellation (AEC).")]
    pub recording_voice_clarity_aec: Option<String>,
    #[arg(long, help_heading = RECORDING_HID, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Enable/disable smart volume.")]
    pub recording_voice_clarity_smart_volume: Option<String>,
    #[arg(long, help_heading = RECORDING_HID, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Enable/disable mic equalizer.")]
    pub recording_voice_clarity_mic_eq: Option<String>,
    #[arg(long, help_heading = RECORDING_HID,
          value_name = "{PRESET_1|PRESET_2|PRESET_3|PRESET_4|PRESET_5|PRESET_6|PRESET_7|PRESET_8|PRESET_9|PRESET_10|PRESET_DM_1}",
          value_parser = mic_eq_preset_names(), hide_possible_values = true,
          help = "Set mic equalizer preset by enum name.")]
    pub recording_voice_clarity_mic_eq_preset: Option<String>,

    // Recording [Audio]
    #[arg(long, help_heading = RECORDING_AUDIO, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Mute/unmute recording.")]
    pub recording_mute: Option<String>,
    #[arg(long, help_heading = RECORDING_AUDIO, value_name = "{0|10|20|..|100}", value_parser = int_0_100_step_10,
          help = "Set mic recording volume as integer.")]
    pub recording_mic_recording_volume: Option<i64>,
    #[arg(long, help_heading = RECORDING_AUDIO, value_name = "{Both|Left|Right}", value_parser = CHANNELS,
          default_value = "Both", hide_possible_values = true, hide_default_value = true,
          help = "Define channels --recording-mic-recording-volume.")]
    pub recording_mic_recording_volume_channels: String,
    #[arg(long, help_heading = RECORDING_AUDIO, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Enable/disable mic monitoring.")]
    pub recording_mic_monitoring_mute: Option<String>,
    #[arg(long, help_heading = RECORDING_AUDIO, value_name = "{0|10|20|..|100}", value_parser = int_0_100_step_10,
          help = "Set mic monitoring volume as integer.")]
    pub recording_mic_monitoring_volume: Option<i64>,
    #[arg(long, help_heading = RECORDING_AUDIO, value_name = "{Both|Left|Right}", value_parser = CHANNELS,
          default_value = "Both", hide_possible_values = true, hide_default_value = true,
          help = "Define channels for --recording-mic-monitoring-volume.")]
    pub recording_mic_monitoring_volume_channels: String,

    // SBX [HID]
    #[arg(long, help_heading = SBX, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true,
          help = "Enables or disables the Surround sound effect.")]
    pub sbx_surround: Option<String>,
    #[arg(long, help_heading = SBX, value_name = "{0..100}", value_parser = int_0_100,
          help = "Set the value for the Surround sound effect as integer.")]
    pub sbx_surround_value: Option<i64>,
    #[arg(long, help_heading = SBX, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true,
          help = "Enables or disables the Crystalizer sound effect.")]
    pub sbx_crystalizer: Option<String>,
    #[arg(long, help_heading = SBX, value_name = "{0..100}", value_parser = int_0_100,
          help = "Set the value for the Crystalizer sound effect as integer.")]
    pub sbx_crystalizer_value: Option<i64>,
    #[arg(long, help_heading = SBX, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true,
          help = "Enables or disables the Bass sound effect.")]
    pub sbx_bass: Option<String>,
    #[arg(long, help_heading = SBX, value_name = "{0..100}", value_parser = int_0_100,
          help = "Set the value for the Bass sound effect as integer.")]
    pub sbx_bass_value: Option<i64>,
    #[arg(long, help_heading = SBX, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true,
          help = "Enables or disables the Smart-Volume sound effect.")]
    pub sbx_smart_volume: Option<String>,
    #[arg(long, help_heading = SBX, value_name = "{0..100}", value_parser = int_0_100,
          help = "Set the value for the Smart-Volume sound effect as value.")]
    pub sbx_smart_volume_value: Option<i64>,
    #[arg(long, help_heading = SBX, value_name = "{Night|Loud}", value_parser = ["Night", "Loud"],
          hide_possible_values = true,
          help = "Set the value for the Smart-Volume sound effect as string (supersedes --set-smart-volume-value).")]
    pub sbx_smart_volume_special_value: Option<String>,
    #[arg(long, help_heading = SBX, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true,
          help = "Enables or disables the Dialog-Plus sound effect.")]
    pub sbx_dialog_plus: Option<String>,
    #[arg(long, help_heading = SBX, value_name = "{0..100}", value_parser = int_0_100,
          help = "Set the value for the Dialog-Plus sound effect as integer.")]
    pub sbx_dialog_plus_value: Option<i64>,
    #[arg(long, help_heading = SBX, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true,
          help = "SBX master switch (SBC: SBX Profile toggle). Excludes Scout Mode.")]
    pub sbx: Option<String>,
    #[arg(long, help_heading = SBX, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Scout Mode. Excludes the SBX effects.")]
    pub scout_mode: Option<String>,

    // Equalizer [HID]
    #[arg(long, help_heading = EQUALIZER, value_name = "{Enabled|Disabled}", value_parser = ED,
          hide_possible_values = true, help = "Enable/disable the equalizer.")]
    pub eq: Option<String>,
    #[arg(long, help_heading = EQUALIZER, value_name = "DB", value_parser = any_float,
          help = "Equalizer preamp in dB.")]
    pub eq_preamp: Option<f64>,
    #[arg(long, help_heading = EQUALIZER, num_args = 2, value_names = ["FREQ", "DB"],
          action = ArgAction::Append, allow_hyphen_values = true, help = eq_band_help())]
    /// Every FREQ DB pair given, flattened (clap's derive can't keep them grouped; each `--eq-band` takes exactly
    /// two values, so pairs are recovered two by two).
    pub eq_band: Option<Vec<String>>,
    #[arg(long, help_heading = EQUALIZER, value_name = "NAME",
          help = eq_preset_help())]
    pub eq_preset: Option<String>,
    #[arg(long, help_heading = EQUALIZER, value_name = "{Speakers,Headphones}",
          value_parser = ["Speakers", "Headphones"], hide_possible_values = true,
          help = "Use the preset's curve for this output instead of the current one.")]
    pub eq_preset_output: Option<String>,

    // Profiles
    #[arg(long, help_heading = PROFILES, value_name = "FILE",
          help = "Read the current settings from the G6 and write them to FILE.")]
    pub profile_save: Option<String>,
    #[arg(long, help_heading = PROFILES, value_name = "FILE",
          help = "Apply the settings in FILE. Other settings given in the same command are applied after it.")]
    pub profile_load: Option<String>,
}

/// The clap command.
pub fn command() -> clap::Command {
    Cli::command()
}

/// Why [`parse`] gave no [`Cli`].
#[derive(Debug)]
pub enum ParseError {
    /// clap's help or version display.
    Clap(clap::Error),
    /// argparse's usage error: print "usage: ..." and this message on stderr, exit 2.
    Usage(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Clap(e) => write!(f, "{e}"),
            ParseError::Usage(m) => write!(f, "error: {m}"),
        }
    }
}

/// The clap ids in the order Python's `parse_cli_args` registers them: help first, `--version` after `--debug`.
fn registration_order() -> Vec<&'static str> {
    let mut order = vec!["help"];
    for (dest, _) in Cli::default().py_values() {
        order.push(dest);
        if dest == "debug" {
            order.push("version");
        }
    }
    order
}

/// Parse `argv` (without the program name).
pub fn parse(argv: &[String]) -> Result<Cli, ParseError> {
    let mut cmd = command();
    cmd.build();
    let spelled = match crate::argparse::read(&cmd, &registration_order(), argv) {
        Ok(spelled) => spelled,
        Err(Early::Help) => vec!["--help".into()],
        Err(Early::Version) => vec!["--version".into()],
        Err(Early::Usage(m)) => return Err(ParseError::Usage(m)),
    };
    let matches = cmd
        .try_get_matches_from_mut(std::iter::once(PROG.to_string()).chain(spelled))
        .map_err(ParseError::Clap)?;
    Cli::from_arg_matches(&matches).map_err(|e| ParseError::Clap(e.format(&mut cmd)))
}

/// Python's help text is argparse's; this is clap's rendering of the same options.
pub fn render_help() -> String {
    command().render_help().to_string()
}

/// Settings that only change how an action runs; not an action on their own.
const NON_ACTION: [&str; 7] = [
    "dry_run",
    "debug",
    "json",
    "reload_audio_services_no_sudo",
    "lighting_effect_period",
    "lighting_effect_duration",
    "eq_preset_output",
];

fn to_bool(s: &str) -> bool {
    // the parser only lets "Enabled" and "Disabled" through
    s == "Enabled"
}

fn channels(s: &str) -> Channels {
    Channels::from_name(s).expect("the parser only lets known channels through")
}

impl Cli {
    /// Every argparse `dest` and the `repr` of its value in Python's namespace, in argparse's order.
    pub fn py_values(&self) -> Vec<(&'static str, String)> {
        let none = || "None".to_string();
        let s = |v: &Option<String>| v.as_deref().map_or_else(none, py_str_repr);
        let i = |v: &Option<i64>| v.map_or_else(none, |v| v.to_string());
        let f = |v: &Option<f64>| v.map_or_else(none, py_repr);
        let b = |v: bool| if v { "True" } else { "False" }.to_string();
        let list = |items: Vec<String>| format!("[{}]", items.join(", "));
        let rgb = self
            .lighting_rgb
            .as_ref()
            .map_or_else(none, |v| list(v.iter().map(i64::to_string).collect()));
        let bands = self.eq_band.as_ref().map_or_else(none, |v| {
            list(
                v.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| list(pair.iter().map(|x| py_str_repr(x)).collect()))
                    .collect(),
            )
        });
        vec![
            ("dry_run", b(self.dry_run)),
            ("debug", b(self.debug)),
            ("claim_and_release", b(self.claim_and_release)),
            ("reload_audio_services", b(self.reload_audio_services)),
            (
                "reload_audio_services_no_sudo",
                b(self.reload_audio_services_no_sudo),
            ),
            ("status", b(self.status)),
            ("json", b(self.json)),
            ("toggle_output", b(self.toggle_output)),
            ("set_output", s(&self.set_output)),
            ("playback_mute", s(&self.playback_mute)),
            ("playback_volume", i(&self.playback_volume)),
            (
                "playback_volume_channels",
                py_str_repr(&self.playback_volume_channels),
            ),
            (
                "playback_speakers_to_stereo",
                b(self.playback_speakers_to_stereo),
            ),
            ("playback_speakers_to_5_1", b(self.playback_speakers_to_5_1)),
            ("playback_speakers_to_7_1", b(self.playback_speakers_to_7_1)),
            (
                "playback_headphones_to_stereo",
                b(self.playback_headphones_to_stereo),
            ),
            (
                "playback_headphones_to_5_1",
                b(self.playback_headphones_to_5_1),
            ),
            (
                "playback_headphones_to_7_1",
                b(self.playback_headphones_to_7_1),
            ),
            ("playback_direct_mode", s(&self.playback_direct_mode)),
            (
                "playback_spdif_out_direct_mode",
                s(&self.playback_spdif_out_direct_mode),
            ),
            ("playback_filter", s(&self.playback_filter)),
            (
                "playback_line_out_headphone_virtualization",
                s(&self.playback_line_out_headphone_virtualization),
            ),
            ("decoder_mode", s(&self.decoder_mode)),
            ("lighting_disable", b(self.lighting_disable)),
            ("lighting_rgb", rgb),
            ("lighting_volume_ring", s(&self.lighting_volume_ring)),
            ("lighting_effect", s(&self.lighting_effect)),
            ("lighting_effect_period", f(&self.lighting_effect_period)),
            (
                "lighting_effect_duration",
                f(&self.lighting_effect_duration),
            ),
            ("mixer_playback_mute", s(&self.mixer_playback_mute)),
            (
                "mixer_monitoring_line_in_mute",
                s(&self.mixer_monitoring_line_in_mute),
            ),
            (
                "mixer_monitoring_line_in_volume",
                i(&self.mixer_monitoring_line_in_volume),
            ),
            (
                "mixer_monitoring_line_in_volume_channels",
                py_str_repr(&self.mixer_monitoring_line_in_volume_channels),
            ),
            (
                "mixer_monitoring_external_mic_mute",
                s(&self.mixer_monitoring_external_mic_mute),
            ),
            (
                "mixer_monitoring_external_mic_volume",
                i(&self.mixer_monitoring_external_mic_volume),
            ),
            (
                "mixer_monitoring_external_mic_volume_channels",
                py_str_repr(&self.mixer_monitoring_external_mic_volume_channels),
            ),
            (
                "mixer_monitoring_spdif_in_mute",
                s(&self.mixer_monitoring_spdif_in_mute),
            ),
            (
                "mixer_monitoring_spdif_in_volume",
                i(&self.mixer_monitoring_spdif_in_volume),
            ),
            (
                "mixer_monitoring_spdif_in_volume_channels",
                py_str_repr(&self.mixer_monitoring_spdif_in_volume_channels),
            ),
            (
                "mixer_recording_line_in_mute",
                s(&self.mixer_recording_line_in_mute),
            ),
            (
                "mixer_recording_line_in_volume",
                i(&self.mixer_recording_line_in_volume),
            ),
            (
                "mixer_recording_line_in_volume_channels",
                py_str_repr(&self.mixer_recording_line_in_volume_channels),
            ),
            (
                "mixer_recording_external_mic_mute",
                s(&self.mixer_recording_external_mic_mute),
            ),
            (
                "mixer_recording_external_mic_volume",
                i(&self.mixer_recording_external_mic_volume),
            ),
            (
                "mixer_recording_external_mic_volume_channels",
                py_str_repr(&self.mixer_recording_external_mic_volume_channels),
            ),
            (
                "mixer_recording_spdif_in_mute",
                s(&self.mixer_recording_spdif_in_mute),
            ),
            (
                "mixer_recording_spdif_in_volume",
                i(&self.mixer_recording_spdif_in_volume),
            ),
            (
                "mixer_recording_spdif_in_volume_channels",
                py_str_repr(&self.mixer_recording_spdif_in_volume_channels),
            ),
            (
                "mixer_recording_what_u_hear_mute",
                s(&self.mixer_recording_what_u_hear_mute),
            ),
            (
                "mixer_recording_what_u_hear_volume",
                i(&self.mixer_recording_what_u_hear_volume),
            ),
            (
                "mixer_recording_what_u_hear_volume_channels",
                py_str_repr(&self.mixer_recording_what_u_hear_volume_channels),
            ),
            ("recording_mute", s(&self.recording_mute)),
            (
                "recording_mic_recording_volume",
                i(&self.recording_mic_recording_volume),
            ),
            (
                "recording_mic_recording_volume_channels",
                py_str_repr(&self.recording_mic_recording_volume_channels),
            ),
            ("recording_mic_boost_db", i(&self.recording_mic_boost_db)),
            (
                "recording_mic_monitoring_mute",
                s(&self.recording_mic_monitoring_mute),
            ),
            (
                "recording_mic_monitoring_volume",
                i(&self.recording_mic_monitoring_volume),
            ),
            (
                "recording_mic_monitoring_volume_channels",
                py_str_repr(&self.recording_mic_monitoring_volume_channels),
            ),
            ("recording_voice_clarity", s(&self.recording_voice_clarity)),
            (
                "recording_voice_clarity_noise_reduction",
                i(&self.recording_voice_clarity_noise_reduction),
            ),
            (
                "recording_voice_clarity_aec",
                s(&self.recording_voice_clarity_aec),
            ),
            (
                "recording_voice_clarity_smart_volume",
                s(&self.recording_voice_clarity_smart_volume),
            ),
            (
                "recording_voice_clarity_mic_eq",
                s(&self.recording_voice_clarity_mic_eq),
            ),
            (
                "recording_voice_clarity_mic_eq_preset",
                s(&self.recording_voice_clarity_mic_eq_preset),
            ),
            ("sbx_surround", s(&self.sbx_surround)),
            ("sbx_surround_value", i(&self.sbx_surround_value)),
            ("sbx_crystalizer", s(&self.sbx_crystalizer)),
            ("sbx_crystalizer_value", i(&self.sbx_crystalizer_value)),
            ("sbx_bass", s(&self.sbx_bass)),
            ("sbx_bass_value", i(&self.sbx_bass_value)),
            ("sbx_smart_volume", s(&self.sbx_smart_volume)),
            ("sbx_smart_volume_value", i(&self.sbx_smart_volume_value)),
            (
                "sbx_smart_volume_special_value",
                s(&self.sbx_smart_volume_special_value),
            ),
            ("sbx_dialog_plus", s(&self.sbx_dialog_plus)),
            ("sbx_dialog_plus_value", i(&self.sbx_dialog_plus_value)),
            ("sbx", s(&self.sbx)),
            ("scout_mode", s(&self.scout_mode)),
            ("eq", s(&self.eq)),
            ("eq_preamp", f(&self.eq_preamp)),
            ("eq_band", bands),
            ("eq_preset", s(&self.eq_preset)),
            ("profile_save", s(&self.profile_save)),
            ("profile_load", s(&self.profile_load)),
            ("eq_preset_output", s(&self.eq_preset_output)),
        ]
    }

    /// Python's `has_action`: something to do besides options that only change how it is done.
    pub fn has_action(&self) -> bool {
        self.py_values().into_iter().any(|(name, value)| {
            value != "None"
                && value != "False"
                && !NON_ACTION.contains(&name)
                && !name.ends_with("_channels")
        })
    }

    /// The device changes requested, for [`Request::actions`].
    pub fn to_request(&self) -> Request {
        let b = |v: &Option<String>| v.as_deref().map(to_bool);
        let vol = |v: Option<i64>, ch: &str| {
            v.map(|percent| Volume {
                percent,
                channels: channels(ch),
            })
        };
        let mixer = |mute: &Option<String>, v: Option<i64>, ch: &str| MixerSetting {
            mute: b(mute),
            volume: vol(v, ch),
        };
        let sbx = |on: &Option<String>, level: Option<i64>| SbxSetting {
            enabled: b(on),
            level,
        };
        Request {
            toggle_output: self.toggle_output,
            set_output: self.set_output.as_deref().and_then(Output::from_name),
            playback_mute: b(&self.playback_mute),
            playback_volume: vol(self.playback_volume, &self.playback_volume_channels),
            speakers_to_stereo: self.playback_speakers_to_stereo,
            speakers_to_5_1: self.playback_speakers_to_5_1,
            speakers_to_7_1: self.playback_speakers_to_7_1,
            headphones_to_stereo: self.playback_headphones_to_stereo,
            headphones_to_5_1: self.playback_headphones_to_5_1,
            headphones_to_7_1: self.playback_headphones_to_7_1,
            direct_mode: b(&self.playback_direct_mode),
            spdif_out_direct_mode: b(&self.playback_spdif_out_direct_mode),
            playback_filter: self
                .playback_filter
                .as_deref()
                .and_then(PlaybackFilter::from_name),
            decoder_mode: self
                .decoder_mode
                .as_deref()
                .and_then(DecoderMode::from_name),
            lighting_disable: self.lighting_disable,
            lighting_rgb: self.lighting_rgb.as_ref().map(|v| [v[0], v[1], v[2]]),
            lighting_volume_ring: b(&self.lighting_volume_ring),
            mixer_playback_mute: b(&self.mixer_playback_mute),
            mixer: [
                mixer(
                    &self.mixer_monitoring_line_in_mute,
                    self.mixer_monitoring_line_in_volume,
                    &self.mixer_monitoring_line_in_volume_channels,
                ),
                mixer(
                    &self.mixer_monitoring_external_mic_mute,
                    self.mixer_monitoring_external_mic_volume,
                    &self.mixer_monitoring_external_mic_volume_channels,
                ),
                mixer(
                    &self.mixer_monitoring_spdif_in_mute,
                    self.mixer_monitoring_spdif_in_volume,
                    &self.mixer_monitoring_spdif_in_volume_channels,
                ),
                mixer(
                    &self.mixer_recording_line_in_mute,
                    self.mixer_recording_line_in_volume,
                    &self.mixer_recording_line_in_volume_channels,
                ),
                mixer(
                    &self.mixer_recording_external_mic_mute,
                    self.mixer_recording_external_mic_volume,
                    &self.mixer_recording_external_mic_volume_channels,
                ),
                mixer(
                    &self.mixer_recording_spdif_in_mute,
                    self.mixer_recording_spdif_in_volume,
                    &self.mixer_recording_spdif_in_volume_channels,
                ),
                mixer(
                    &self.mixer_recording_what_u_hear_mute,
                    self.mixer_recording_what_u_hear_volume,
                    &self.mixer_recording_what_u_hear_volume_channels,
                ),
            ],
            recording_mute: b(&self.recording_mute),
            mic_recording_volume: vol(
                self.recording_mic_recording_volume,
                &self.recording_mic_recording_volume_channels,
            ),
            mic_boost_db: self.recording_mic_boost_db,
            mic_monitoring_mute: b(&self.recording_mic_monitoring_mute),
            mic_monitoring_volume: vol(
                self.recording_mic_monitoring_volume,
                &self.recording_mic_monitoring_volume_channels,
            ),
            voice_clarity: b(&self.recording_voice_clarity),
            noise_reduction_level: self.recording_voice_clarity_noise_reduction,
            aec: b(&self.recording_voice_clarity_aec),
            mic_smart_volume: b(&self.recording_voice_clarity_smart_volume),
            mic_eq: b(&self.recording_voice_clarity_mic_eq),
            mic_eq_preset: self
                .recording_voice_clarity_mic_eq_preset
                .as_deref()
                .and_then(MicEqPreset::from_name),
            sbx_effects: [
                sbx(&self.sbx_surround, self.sbx_surround_value),
                sbx(&self.sbx_crystalizer, self.sbx_crystalizer_value),
                sbx(&self.sbx_bass, self.sbx_bass_value),
                sbx(&self.sbx_smart_volume, self.sbx_smart_volume_value),
                sbx(&self.sbx_dialog_plus, self.sbx_dialog_plus_value),
            ],
            smart_volume_special: self.sbx_smart_volume_special_value.as_deref().map(|v| {
                if v == "Night" {
                    SmartVolumeSpecial::Night
                } else {
                    SmartVolumeSpecial::Loud
                }
            }),
            sbx: b(&self.sbx),
            scout_mode: b(&self.scout_mode),
            eq: b(&self.eq),
            eq_preset: self.eq_preset.clone().map(|id| {
                let output = self.eq_preset_output.as_deref().and_then(Output::from_name);
                (id, output)
            }),
            eq_preamp: self.eq_preamp,
            eq_bands: self
                .eq_band
                .iter()
                .flat_map(|v| v.as_chunks::<2>().0)
                .map(|[freq, db]| (freq.clone(), db.clone()))
                .collect(),
            line_out_headphone_virtualization: b(&self.playback_line_out_headphone_virtualization),
        }
    }
}
