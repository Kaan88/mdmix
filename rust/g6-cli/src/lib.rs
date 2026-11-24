//! The `g6` command line, a port of Python's `soundblaster-x-g6-cli`. The device sits behind [`transport::Transport`]
//! and the rest of the system behind [`env::Env`], so the whole flow runs in tests without a G6.

#![forbid(unsafe_code)]

pub mod argparse;
pub mod args;
pub mod env;
pub mod profile;
pub mod pyval;
pub mod run;
pub mod status_fmt;
pub mod transport;

use std::fmt;
use std::io::{self, Write};

use g6_proto::packet::ProtoError;
use g6_proto::pyfmt::py_str_repr;

pub use args::VERSION;

/// Why a run stopped early.
#[derive(Debug)]
pub enum CliError {
    /// Exit with this code; what to say has been said (argparse's `--help`, usage errors).
    Exit(i32),
    /// Print `error: {message}` and exit with 1, as Python's `cli_entry` does for the errors it expects.
    Msg(String),
}

impl CliError {
    pub fn msg(text: impl Into<String>) -> Self {
        CliError::Msg(text.into())
    }

    /// The process exit code.
    pub fn code(&self) -> i32 {
        match self {
            CliError::Exit(code) => *code,
            CliError::Msg(_) => 1,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Exit(code) => write!(f, "exit {code}"),
            CliError::Msg(m) => f.write_str(m),
        }
    }
}

impl From<io::Error> for CliError {
    fn from(e: io::Error) -> Self {
        CliError::Msg(e.to_string())
    }
}

impl From<ProtoError> for CliError {
    fn from(e: ProtoError) -> Self {
        CliError::Msg(e.0)
    }
}

/// How Python prints an `OSError` about `path` for the common cases; other errors keep Rust's wording.
pub fn os_error_text(e: &io::Error, path: &str) -> String {
    let (errno, what) = match e.kind() {
        io::ErrorKind::NotFound => (2, "No such file or directory"),
        io::ErrorKind::PermissionDenied => (13, "Permission denied"),
        io::ErrorKind::IsADirectory => (21, "Is a directory"),
        _ => return format!("{e}: {}", py_str_repr(path)),
    };
    format!("[Errno {errno}] {what}: {}", py_str_repr(path))
}

pub fn os_error(e: &io::Error, path: &str) -> CliError {
    CliError::Msg(os_error_text(e, path))
}

/// Where a run prints: Python's `sys.stdout` and `sys.stderr`.
pub struct Io<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}
