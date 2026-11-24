//! `g6`: the binary. Without `--dry-run` it talks to the G6 over HID.

#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::process::ExitCode;

use g6_cli::env::SystemEnv;
use g6_cli::{CliError, Io, run};

/// Python's stdout and stderr are text streams: on Windows every "\n" goes out as "\r\n".
struct TextStream<W: Write>(W);

impl<W: Write> Write for TextStream<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if cfg!(windows) {
            for part in buf.split_inclusive(|b| *b == b'\n') {
                match part.strip_suffix(b"\n") {
                    Some(line) => {
                        self.0.write_all(line)?;
                        self.0.write_all(b"\r\n")?;
                    }
                    None => self.0.write_all(part)?,
                }
            }
        } else {
            self.0.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let mut out = TextStream(io::stdout());
    let mut err = TextStream(io::stderr());
    let mut env = SystemEnv::default();
    let result = run::main_with(
        &argv,
        &mut env,
        &mut Io {
            out: &mut out,
            err: &mut err,
        },
    );
    let _ = out.flush();
    let code = match result {
        Ok(()) => 0,
        Err(e) => {
            if let CliError::Msg(m) = &e {
                let _ = writeln!(err, "error: {m}");
            }
            e.code()
        }
    };
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
