//! The G6 protocol without any I/O: the packets every setting sends, the status queries and their parsers, the
//! EQ presets and the lighting effects. Everything here mirrors the Python package `g6_cli` byte for byte; the
//! vectors in `tests/vectors/` check that.

#![forbid(unsafe_code)]

pub mod apply;
pub mod decoder;
pub mod effects;
pub mod eq_presets;
mod eq_presets_table;
pub mod equalizer;
pub mod lighting;
pub mod mixer;
pub mod modes;
pub mod packet;
pub mod playback;
pub mod pyfmt;
pub mod recording;
pub mod sbx;
pub mod status;
mod tables;
pub mod types;
