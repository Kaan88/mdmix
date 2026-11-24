//! Host-side lighting effects, as SoundBlaster Command animates them: the G6 stores no effect, the host streams
//! colour frames (Python: `g6_spec/lighting_effects.py`).

use crate::packet::{ProtoError, err};
use crate::pyfmt::py_repr;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    Pulsate,
    Cycle,
    Music,
}

impl Effect {
    pub const ALL: [Effect; 3] = [Effect::Pulsate, Effect::Cycle, Effect::Music];

    pub fn name(self) -> &'static str {
        match self {
            Effect::Pulsate => "pulsate",
            Effect::Cycle => "cycle",
            Effect::Music => "music",
        }
    }

    pub fn from_name(s: &str) -> Result<Effect, ProtoError> {
        Self::ALL.into_iter().find(|e| e.name() == s).map_or_else(
            || {
                err(format!(
                    "unknown effect {}, expected one of ('pulsate', 'cycle', 'music')",
                    crate::pyfmt::py_str_repr(s)
                ))
            },
            Ok,
        )
    }

    /// Seconds per round: pulsate dark-bright-dark, cycle once around the wheel, music's colour cycle.
    pub fn default_period(self) -> f64 {
        match self {
            Effect::Pulsate => 2.0,
            Effect::Cycle => 6.0,
            Effect::Music => 12.0,
        }
    }
}

pub const FRAME_INTERVAL_S: f64 = 1.0 / 30.0;
/// Output peaks below this many dBFS leave the light dark.
pub const MUSIC_FLOOR_DB: f64 = -45.0;
/// How long a peak takes to fade to 1/e.
pub const MUSIC_DECAY_S: f64 = 0.25;

pub type Rgb = (u8, u8, u8);

/// Red, blue, green: the order SBC's colour cycle runs in.
const WHEEL: [Rgb; 3] = [(255, 0, 0), (0, 0, 255), (0, 255, 0)];

// Python's min() and max() keep the first argument unless the second compares strictly smaller (larger). That
// decides what NaN turns into, so they are copied rather than using f64::min / f64::max.
fn py_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

fn round_u8(x: f64) -> u8 {
    // every caller passes a value in 0..=255; round() in Python rounds ties to even
    x.round_ties_even() as u8
}

fn mix(a: Rgb, b: Rgb, f: f64) -> Rgb {
    let m = |x: u8, y: u8| {
        let (x, y) = (f64::from(x), f64::from(y));
        round_u8(x + (y - x) * f)
    };
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// A colour on the wheel for `phase` in [0, 1).
fn wheel(phase: f64) -> Rgb {
    let pos = phase * WHEEL.len() as f64;
    let i = pos as usize;
    mix(WHEEL[i], WHEEL[(i + 1) % WHEEL.len()], pos - i as f64)
}

/// Output peak (0..1, linear) to light level 0..1, on a dB scale from `floor_db` up to 0 dBFS.
pub fn level_to_brightness(peak: f64, floor_db: f64) -> f64 {
    if peak <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * py_min(peak, 1.0).log10();
    py_max(0.0, py_min(1.0, (db - floor_db) / -floor_db))
}

/// Rises with the level at once, then falls off exponentially, so the light doesn't flicker between beats.
#[derive(Clone, Debug)]
pub struct LevelFollower {
    pub decay_s: f64,
    pub value: f64,
}

impl Default for LevelFollower {
    fn default() -> Self {
        LevelFollower {
            decay_s: MUSIC_DECAY_S,
            value: 0.0,
        }
    }
}

impl LevelFollower {
    pub fn update(&mut self, level: f64, dt: f64) -> f64 {
        self.value = py_max(level, self.value * (-py_max(dt, 0.0) / self.decay_s).exp());
        self.value
    }
}

/// (red, green, blue, brightness) of `effect` at `t` seconds. `rgb` is the user's colour (pulsate falls back to
/// red, music to the wheel); `level` is music's light level 0..1.
pub fn effect_frame(
    effect: Effect,
    t: f64,
    period: f64,
    rgb: Option<Rgb>,
    level: Option<f64>,
) -> Result<(u8, u8, u8, u8), ProtoError> {
    // NaN fails too, as in Python
    if period.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return err(format!("period must be > 0, got {}", py_repr(period)));
    }
    // Python's float % takes the sign of the divisor
    let phase = (t / period).rem_euclid(1.0);
    Ok(match effect {
        Effect::Pulsate => {
            let (r, g, b) = rgb.unwrap_or(WHEEL[0]);
            (r, g, b, round_u8(255.0 * (1.0 - (2.0 * phase - 1.0).abs())))
        }
        Effect::Cycle => {
            let (r, g, b) = wheel(phase);
            (r, g, b, 255)
        }
        Effect::Music => {
            let (r, g, b) = rgb.unwrap_or_else(|| wheel(phase));
            // `level or 0.0`: None and 0.0 both mean dark
            let l = level.filter(|l| *l != 0.0).unwrap_or(0.0);
            (r, g, b, round_u8(255.0 * py_max(0.0, py_min(1.0, l))))
        }
    })
}
