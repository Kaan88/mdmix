//! SoundBlaster Command's G6 EQ presets and the name lookup the CLI uses (Python: `g6_spec/eq_presets.py`).

use crate::eq_presets_table::TABLE;
use crate::equalizer::EQ_BAND_FREQS;
use crate::packet::{ProtoError, err};
use crate::pyfmt::py_str_repr;
use crate::types::Output;

/// Preset ids, in SBC's order.
pub fn preset_ids() -> impl Iterator<Item = &'static str> {
    TABLE.iter().map(|(id, _, _)| *id)
}

/// Lower-case letters and digits only, so "hip hop", "Hip-Hop" and "HIPHOP" all name the same preset.
fn key(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// The preset id `name` refers to.
pub fn find_eq_preset(name: &str) -> Result<&'static str, ProtoError> {
    let k = key(name);
    // Python builds a dict keyed by the normalised id, so a later id wins if two ever normalised alike
    match preset_ids().filter(|id| key(id) == k).last() {
        Some(id) => Ok(id),
        None => err(format!(
            "unknown EQ preset {}, expected one of {}",
            py_str_repr(name),
            preset_ids().collect::<Vec<_>>().join(", ")
        )),
    }
}

/// The ten bands of a preset as (frequency label, dB).
pub type PresetBands = [(&'static str, f64); 10];

/// A preset's preamp and its ten bands as (frequency label, dB), for one output. Some presets have a separate
/// headphone curve.
pub fn eq_preset_values(name: &str, output: Output) -> Result<(f64, PresetBands), ProtoError> {
    let id = find_eq_preset(name)?;
    let (_, speakers, headphones) = TABLE
        .iter()
        .find(|(i, _, _)| *i == id)
        .expect("found above");
    let values = match output {
        Output::Speakers => speakers,
        Output::Headphones => headphones,
    };
    let bands = std::array::from_fn(|i| (EQ_BAND_FREQS[i], values[i + 1]));
    Ok((values[0], bands))
}

/// All presets as (id, speakers, headphones), each preamp then the ten bands.
pub fn all() -> &'static [(&'static str, [f64; 11], [f64; 11])] {
    &TABLE
}
