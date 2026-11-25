//! g6-gui: the Sound BlasterX G6 control panel, drawn with the toolkit's `mdshared-ui`.
//!
//! Steps G2 to G4 of `doc/gui-plan.md`: the window, the pages and the settings dialog, fed by a status read from the
//! device, and the HID settings the `g6` CLI writes, written the same way (`intent`, `desk`, `worker`). `--shot`
//! renders a page to a PNG with no window and no device.

#![forbid(unsafe_code)]

mod app;
mod args;
mod desk;
mod dialog;
mod facts;
mod frame;
mod fx;
mod icons;
mod intent;
mod kit;
mod model;
mod pages;
mod pick;
mod prefs;
mod sample;
mod shot;
mod worker;

use std::path::Path;
use std::process::ExitCode;

use args::Mode;

/// Where shots go unless `--out` says: under `target/`, which git ignores.
const SHOT_DIR: &str = "target/g6-gui-shots";

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match args::parse(&argv) {
        Err(e) => {
            eprintln!("g6-gui: {e}\n\n{}", args::USAGE);
            ExitCode::from(2)
        }
        Ok(Mode::Help) => {
            print!("{}", args::USAGE);
            ExitCode::SUCCESS
        }
        Ok(Mode::Run { page }) => ExitCode::from(u8::try_from(app::run(page)).unwrap_or(1)),
        Ok(Mode::Shot(a)) => match shot::write(&a, Path::new(SHOT_DIR)) {
            Ok(path) => {
                println!("{}", path.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("g6-gui: {e}");
                ExitCode::FAILURE
            }
        },
        Ok(Mode::Shots(dir)) => match shot::all(&dir) {
            Ok(made) => {
                println!("{} shots in {}", made.len(), dir.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("g6-gui: {e}");
                ExitCode::FAILURE
            }
        },
    }
}
