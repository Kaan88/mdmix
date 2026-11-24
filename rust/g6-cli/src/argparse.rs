//! How Python 3.13's argparse reads a command line, ahead of clap. clap and argparse disagree on edge cases that
//! decide what gets sent: argparse takes `--eq-preamp -.5` but refuses `--eq-preamp -1e5`, lets `--eq-band 1k -x`
//! through as a pair, abbreviates flags with its own ambiguity rule, and reports problems in its own words. So
//! this walks the arguments the way `_parse_known_args` does (option matching, value counting, conversion and
//! choices, in that order) and hands clap a spelled-out command line that clap can only read one way.

use std::collections::HashMap;
use std::error::Error as _;

use clap::{Arg, Command};
use g6_proto::pyfmt::py_str_repr;

/// Why reading stopped before there was anything for clap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Early {
    /// `-h`/`--help` was taken: show the help and exit 0.
    Help,
    /// `--version` was taken.
    Version,
    /// argparse's `parser.error(message)`: usage and message on stderr, exit 2.
    Usage(String),
}

/// One registered option.
struct Opt<'c> {
    arg: &'c Arg,
    /// How argparse names it in "argument X: ..." messages, e.g. "-h/--help".
    name: String,
    /// What clap is given.
    canonical: String,
    nargs: usize,
}

/// One way argparse read an option-like token (`_parse_optional`'s tuples).
#[derive(Clone)]
struct Match {
    /// None for a token that looks like an option nobody registered.
    opt: Option<usize>,
    string: String,
    /// argparse's `sep`; only whether it is "=" (truthy) matters.
    sep: bool,
    explicit: Option<String>,
}

struct Reader<'c> {
    cmd: &'c Command,
    opts: Vec<Opt<'c>>,
    /// Every option string with its option, in registration order (argparse's `_option_string_actions`).
    strings: Vec<(String, usize)>,
}

/// Read `argv` like argparse. `order` holds the clap ids in the order Python registers them (it decides how an
/// ambiguity is worded); `cmd` must be built. On success, the arguments to give clap.
pub fn read(cmd: &Command, order: &[&str], argv: &[String]) -> Result<Vec<String>, Early> {
    let reader = Reader::new(cmd, order);
    reader.read(argv)
}

impl<'c> Reader<'c> {
    fn new(cmd: &'c Command, order: &[&str]) -> Self {
        let mut opts = Vec::new();
        let mut strings = Vec::new();
        for id in order {
            let arg = cmd
                .get_arguments()
                .find(|a| a.get_id() == id)
                .unwrap_or_else(|| panic!("no argument {id}"));
            let mut own = Vec::new();
            if let Some(c) = arg.get_short() {
                own.push(format!("-{c}"));
            }
            if let Some(l) = arg.get_long() {
                own.push(format!("--{l}"));
            }
            let nargs = if arg.get_action().takes_values() {
                arg.get_num_args().map_or(1, |r| r.max_values())
            } else {
                0
            };
            let i = opts.len();
            strings.extend(own.iter().map(|s| (s.clone(), i)));
            opts.push(Opt {
                arg,
                name: own.join("/"),
                canonical: own.last().cloned().expect("every option has a long name"),
                nargs,
            });
        }
        Reader { cmd, opts, strings }
    }

    fn lookup(&self, s: &str) -> Option<usize> {
        self.strings.iter().find(|(o, _)| o == s).map(|(_, i)| *i)
    }

    fn arg_error(&self, o: usize, msg: impl std::fmt::Display) -> Early {
        Early::Usage(format!("argument {}: {msg}", self.opts[o].name))
    }

    fn expected(&self, o: usize) -> Early {
        match self.opts[o].nargs {
            1 => self.arg_error(o, "expected one argument"),
            n => self.arg_error(o, format!("expected {n} arguments")),
        }
    }

    /// `_parse_optional`: None for a value, else every option the token could be.
    fn parse_optional(&self, s: &str) -> Option<Vec<Match>> {
        if !s.starts_with('-') {
            return None;
        }
        if let Some(i) = self.lookup(s) {
            return Some(vec![Match {
                opt: Some(i),
                string: s.to_string(),
                sep: false,
                explicit: None,
            }]);
        }
        if s.len() == 1 {
            return None;
        }
        if let Some((name, value)) = s.split_once('=')
            && let Some(i) = self.lookup(name)
        {
            return Some(vec![Match {
                opt: Some(i),
                string: name.to_string(),
                sep: true,
                explicit: Some(value.to_string()),
            }]);
        }
        let tuples = self.option_tuples(s);
        if !tuples.is_empty() {
            return Some(tuples);
        }
        // no registered option looks like a negative number, so these are values
        if looks_negative(s) || s.contains(' ') {
            return None;
        }
        Some(vec![Match {
            opt: None,
            string: s.to_string(),
            sep: false,
            explicit: None,
        }])
    }

    /// `_get_option_tuples`: abbreviations (allow_abbrev is on), and `-hVALUE` for the one short option.
    fn option_tuples(&self, s: &str) -> Vec<Match> {
        let (prefix, explicit) = match s.split_once('=') {
            Some((p, v)) => (p, Some(v.to_string())),
            None => (s, None),
        };
        let sep = explicit.is_some();
        let mut out = Vec::new();
        if s[1..].starts_with('-') {
            for (string, i) in &self.strings {
                if string.starts_with(prefix) {
                    out.push(Match {
                        opt: Some(*i),
                        string: string.clone(),
                        sep,
                        explicit: explicit.clone(),
                    });
                }
            }
        } else {
            let cut = s.char_indices().nth(2).map_or(s.len(), |(i, _)| i);
            let (short, rest) = s.split_at(cut);
            for (string, i) in &self.strings {
                if string == short {
                    out.push(Match {
                        opt: Some(*i),
                        string: string.clone(),
                        sep: false,
                        explicit: Some(rest.to_string()),
                    });
                } else if string.starts_with(prefix) {
                    out.push(Match {
                        opt: Some(*i),
                        string: string.clone(),
                        sep,
                        explicit: explicit.clone(),
                    });
                }
            }
        }
        out
    }

    fn read(&self, argv: &[String]) -> Result<Vec<String>, Early> {
        // argparse's pattern: 'O' option, 'A' value, '-' for "--" (everything after it is a value)
        let mut pattern = Vec::with_capacity(argv.len());
        let mut options: HashMap<usize, Vec<Match>> = HashMap::new();
        let mut after_dashes = false;
        for (i, a) in argv.iter().enumerate() {
            if after_dashes {
                pattern.push(b'A');
            } else if a == "--" {
                after_dashes = true;
                pattern.push(b'-');
            } else if let Some(m) = self.parse_optional(a) {
                options.insert(i, m);
                pattern.push(b'O');
            } else {
                pattern.push(b'A');
            }
        }

        let mut taken = Vec::new();
        let mut extras: Vec<String> = Vec::new();
        let mut i = 0;
        if let Some(&last) = options.keys().max() {
            while i <= last {
                // no positionals here, so whatever sits before the next option is left over
                let next = (i..=last)
                    .find(|j| options.contains_key(j))
                    .expect("last is an option");
                extras.extend(argv[i..next].iter().cloned());
                i = self.consume(
                    next,
                    argv,
                    &pattern,
                    &options[&next],
                    &mut extras,
                    &mut taken,
                )?;
            }
        }
        extras.extend(argv[i..].iter().cloned());
        if !extras.is_empty() {
            return Err(Early::Usage(format!(
                "unrecognized arguments: {}",
                extras.join(" ")
            )));
        }
        Ok(taken.concat())
    }

    /// `consume_optional`: take the option at `i` and its values; returns where the next token starts.
    fn consume(
        &self,
        i: usize,
        argv: &[String],
        pattern: &[u8],
        matches: &[Match],
        extras: &mut Vec<String>,
        taken: &mut Vec<Vec<String>>,
    ) -> Result<usize, Early> {
        if matches.len() > 1 {
            let names: Vec<&str> = matches.iter().map(|m| m.string.as_str()).collect();
            return Err(Early::Usage(format!(
                "ambiguous option: {} could match {}",
                argv[i],
                names.join(", ")
            )));
        }
        let Match {
            mut opt,
            mut string,
            mut sep,
            mut explicit,
        } = matches[0].clone();
        let mut here: Vec<(usize, Vec<String>)> = Vec::new();
        let stop = loop {
            let Some(o) = opt else {
                extras.push(argv[i].clone());
                return Ok(i + 1);
            };
            let nargs = self.opts[o].nargs;
            match explicit.take() {
                Some(e) => {
                    // an attached value only ever counts as one argument
                    let count = match nargs {
                        0 | 1 => nargs,
                        _ => return Err(self.expected(o)),
                    };
                    let single_dash = !string[1..].starts_with('-');
                    if count == 0 && single_dash && !e.is_empty() {
                        // "-hh": more short options glued on
                        if sep || e.starts_with('-') {
                            return Err(self.arg_error(o, ignored(&e)));
                        }
                        here.push((o, Vec::new()));
                        let first = e.chars().next().expect("not empty");
                        let next = format!("-{first}");
                        if let Some(o2) = self.lookup(&next) {
                            opt = Some(o2);
                            string = next;
                            let rest = &e[first.len_utf8()..];
                            (sep, explicit) = match rest.strip_prefix('=') {
                                _ if rest.is_empty() => (false, None),
                                Some(r) => (true, Some(r.to_string())),
                                None => (false, Some(rest.to_string())),
                            };
                        } else {
                            extras.push(format!("-{e}"));
                            break i + 1;
                        }
                    } else if count == 1 {
                        here.push((o, vec![e]));
                        break i + 1;
                    } else {
                        return Err(self.arg_error(o, ignored(&e)));
                    }
                }
                None => {
                    // a single value must be 'A'; a fixed count of two or more takes 'O' tokens too
                    // (argparse's `[A]` and `[AO]{n}`); "--" ends both
                    let start = i + 1;
                    let ahead = &pattern[start.min(pattern.len())..];
                    let fits = match nargs {
                        0 => true,
                        1 => ahead.first() == Some(&b'A'),
                        n => ahead.len() >= n && ahead[..n].iter().all(|p| *p != b'-'),
                    };
                    if !fits {
                        return Err(self.expected(o));
                    }
                    here.push((o, argv[start..start + nargs].to_vec()));
                    break start + nargs;
                }
            }
        };
        for (o, values) in here {
            taken.push(self.take(o, values)?);
        }
        Ok(stop)
    }

    /// `take_action`: convert and check the values (argparse does it here, so a bad value earlier on the line is
    /// reported before a problem later on it). Returns the option spelled out for clap.
    fn take(&self, o: usize, values: Vec<String>) -> Result<Vec<String>, Early> {
        let opt = &self.opts[o];
        match opt.arg.get_id().as_str() {
            "help" => return Err(Early::Help),
            "version" => return Err(Early::Version),
            _ => {}
        }
        let spelled = match opt.nargs {
            0 => vec![opt.canonical.clone()],
            // "=" keeps clap from reading a value such as "-.5" or "--x y" as an option
            1 => vec![format!("{}={}", opt.canonical, values[0])],
            // the clap argument allows hyphen values, and counts exactly nargs
            _ => std::iter::once(opt.canonical.clone())
                .chain(values.iter().cloned())
                .collect(),
        };
        if opt.nargs > 0 {
            self.check(o, &values, &spelled)?;
        }
        Ok(spelled)
    }

    /// `_get_value` and `_check_value`: the clap argument's choices, or its value parser.
    fn check(&self, o: usize, values: &[String], spelled: &[String]) -> Result<(), Early> {
        let choices = self.opts[o].arg.get_possible_values();
        if !choices.is_empty() {
            for v in values {
                if !choices.iter().any(|c| c.get_name() == v) {
                    let names: Vec<String> =
                        choices.iter().map(|c| py_str_repr(c.get_name())).collect();
                    return Err(self.arg_error(
                        o,
                        format!(
                            "invalid choice: {} (choose from {})",
                            py_str_repr(v),
                            names.join(", ")
                        ),
                    ));
                }
            }
            return Ok(());
        }
        // clap doesn't expose a value parser on its own, so parse just this option; the args.rs parsers word
        // their errors as argparse does
        let line = std::iter::once(self.cmd.get_name().to_string()).chain(spelled.iter().cloned());
        match self.cmd.clone().try_get_matches_from(line) {
            Ok(_) => Ok(()),
            Err(e) => Err(self.arg_error(
                o,
                e.source().map_or_else(|| e.to_string(), |s| s.to_string()),
            )),
        }
    }
}

fn ignored(e: &str) -> String {
    format!("ignored explicit argument {}", py_str_repr(e))
}

/// argparse's `^-\d+$|^-\d*\.\d+$` (`$` also matches before a final newline). Only ASCII digits: Python's `\d`
/// takes any Unicode decimal digit, which this doesn't follow.
fn looks_negative(s: &str) -> bool {
    let s = s.strip_suffix('\n').unwrap_or(s);
    let Some(body) = s.strip_prefix('-') else {
        return false;
    };
    let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    match body.split_once('.') {
        None => digits(body),
        Some((int, frac)) => (int.is_empty() || digits(int)) && digits(frac),
    }
}

#[cfg(test)]
mod tests {
    use super::looks_negative;

    #[test]
    fn negative_numbers() {
        for s in ["-1", "-10", "-.5", "-1.5", "-0", "-1\n"] {
            assert!(looks_negative(s), "{s:?}");
        }
        for s in [
            "-", "-1e5", "-1.", "-x", "-1.2.3", "1", "-1_0", "- 1", "-1\n\n",
        ] {
            assert!(!looks_negative(s), "{s:?}");
        }
    }
}
