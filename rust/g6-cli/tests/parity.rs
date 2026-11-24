//! The CLI against what the Python CLI does: the write vectors (`tests/vectors/writes.json`) replayed through clap
//! and a recording transport, and the outputs in `tests/fixtures/python_output.json`, which
//! `tools/rust_fixtures.py` captures from the Python code (see rust/README.md to regenerate it).
//! Nothing here touches a device.

use std::io::Write;
use std::path::PathBuf;

use g6_cli::args;
use g6_cli::env::Env;
use g6_cli::profile::{load_profile_argv, profile_file_text, profile_from_status, profile_to_argv};
use g6_cli::pyval::{dumps_indent2, loads};
use g6_cli::run::{main_with, parse_cli_args, without_option};
use g6_cli::transport::{DryRunTransport, RecordingTransport, Transport};
use g6_cli::{CliError, Io};
use g6_proto::packet::{HidReport, hex, pad_report, unhex};
use g6_proto::pyfmt::{py_g, py_repr, py_round, py_round_int, py_str_repr};
use g6_proto::status::{DeviceStatus, collect_status};
use serde_json::Value;

fn read_json(path: PathBuf) -> Value {
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("valid JSON")
}

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn python_output() -> Value {
    read_json(manifest().join("tests/fixtures/python_output.json"))
}

fn writes_vectors() -> Value {
    read_json(manifest().join("../../tests/vectors/writes.json"))
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

/// A fresh empty directory under the system temp dir, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("g6-rust-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

type Exchanges = Vec<(Vec<u8>, Option<HidReport>)>;

fn exchanges(v: &Value) -> Exchanges {
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let q = unhex(e["query"].as_str().unwrap()).unwrap();
            let a = e["response"]
                .as_str()
                .map(|r| pad_report(&unhex(r).unwrap()).unwrap());
            (q, a)
        })
        .collect()
}

fn status_from(ex: &Exchanges) -> DeviceStatus {
    let mut device = g6_cli::transport::FakeTransport::new(ex.clone());
    collect_status(|q| device.query(q))
}

enum Device {
    Recording(RecordingTransport),
    /// The dry-run transport, without the placeholder line (the fixtures patch Python's detect_device the same way).
    DryRun,
}

/// The outside world for a test run: a fixed device, a status transcript, a toggle file in a temp dir and a clock
/// that never moves.
struct TestEnv {
    device: Device,
    status: Option<Exchanges>,
    toggle: PathBuf,
}

impl Env for TestEnv {
    fn open_device(
        &mut self,
        dry_run: bool,
        debug: bool,
        _out: &mut dyn Write,
    ) -> Result<Box<dyn Transport>, CliError> {
        assert!(dry_run, "tests only run dry");
        Ok(match &self.device {
            Device::Recording(r) => Box::new(r.clone()),
            Device::DryRun => Box::new(DryRunTransport::new(debug)),
        })
    }

    fn read_status(&mut self) -> Result<DeviceStatus, CliError> {
        match &self.status {
            Some(ex) => Ok(status_from(ex)),
            None => panic!("this run shouldn't read the status"),
        }
    }

    fn toggle_state_path(&self) -> PathBuf {
        self.toggle.clone()
    }

    fn reload_audio_services(
        &mut self,
        _sudo: bool,
        _dry_run: bool,
        _out: &mut dyn Write,
    ) -> Result<(), CliError> {
        panic!("never reload audio services here");
    }

    fn monotonic(&mut self) -> f64 {
        0.0
    }

    fn sleep(&mut self, _seconds: f64) {}

    fn output_meter(&mut self) -> Option<Box<dyn FnMut() -> f64>> {
        None
    }
}

struct Output {
    result: Result<(), CliError>,
    out: String,
    err: String,
}

fn run(argv: &[String], env: &mut TestEnv) -> Output {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let result = main_with(
        argv,
        env,
        &mut Io {
            out: &mut out,
            err: &mut err,
        },
    );
    Output {
        result,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}

fn dry(argv: &[String]) -> Vec<String> {
    std::iter::once("--dry-run".to_string())
        .chain(argv.iter().cloned())
        .collect()
}

/// Python's recorder stores a report without its trailing zeros.
fn stripped_hex(r: &HidReport) -> String {
    let end = r.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
    hex(&r[..end])
}

/// Replays one recorded case; returns what didn't match.
fn replay(case: &Value, tmp: &TempDir) -> Option<String> {
    let argv = strings(&case["argv"]);
    let toggle = tmp.0.join("toggle");
    // the vectors run with determine_toggle_state returning Headphones: the file says Speakers was set last
    std::fs::write(&toggle, "Speakers").unwrap();
    let rec = RecordingTransport::default();
    let mut env = TestEnv {
        device: Device::Recording(rec.clone()),
        status: None,
        toggle,
    };
    let o = run(&dry(&argv), &mut env);
    if let Err(e) = o.result {
        return Some(format!("{argv:?}: failed: {e}\n{}", o.err));
    }
    let log = rec.log.borrow();
    let hid: Vec<String> = log.hid.iter().map(stripped_hex).collect();
    let want_hid = strings(&case["hid"]);
    if hid != want_hid {
        return Some(format!("{argv:?}: HID\n got  {hid:?}\n want {want_hid:?}"));
    }
    let usb: Vec<Vec<String>> = log
        .usb
        .iter()
        .map(|u| {
            vec![
                format!("{:02x}", u.bm_request_type),
                format!("{:02x}", u.b_request),
                hex(&u.w_value),
                hex(&u.w_index),
                hex(&u.w_length),
                hex(&u.data),
            ]
        })
        .collect();
    let want_usb: Vec<Vec<String>> = case["usb"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|u| {
                    [
                        "bmRequestType",
                        "bRequest",
                        "wValue",
                        "wIndex",
                        "wLength",
                        "data",
                    ]
                    .iter()
                    .map(|k| u[*k].as_str().unwrap().to_string())
                    .collect()
                })
                .collect()
        })
        .unwrap_or_default();
    if usb != want_usb {
        return Some(format!("{argv:?}: USB\n got  {usb:?}\n want {want_usb:?}"));
    }
    None
}

#[test]
fn every_write_vector() {
    let v = writes_vectors();
    let cases = v["cases"].as_array().unwrap();
    assert!(cases.len() > 200, "writes.json looks truncated");
    let tmp = TempDir::new("writes");
    let failures: Vec<String> = cases.iter().filter_map(|c| replay(c, &tmp)).collect();
    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
fn lighting_effect_writes() {
    let v = python_output();
    let cases = v["effect_writes"].as_array().unwrap();
    assert!(!cases.is_empty());
    let tmp = TempDir::new("effects");
    let failures: Vec<String> = cases.iter().filter_map(|c| replay(c, &tmp)).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn toggle_output_follows_the_state_file() {
    let tmp = TempDir::new("toggle");
    let toggle = tmp.0.join("toggle");
    let rec = RecordingTransport::default();
    let mut env = TestEnv {
        device: Device::Recording(rec.clone()),
        status: None,
        toggle: toggle.clone(),
    };
    let argv = dry(&["--toggle-output".to_string()]);
    let speakers = stripped_hex(&g6_proto::playback::toggle_to_speakers()[0]);
    let headphones = stripped_hex(&g6_proto::playback::toggle_to_headphones()[0]);

    // no file: "Toggle to Speakers", and the reports for Speakers go out
    let o = run(&argv, &mut env);
    assert!(o.result.is_ok());
    assert!(o.out.contains("Toggle to Speakers\n"), "{}", o.out);
    assert_eq!(std::fs::read_to_string(&toggle).unwrap(), "Speakers");
    assert_eq!(stripped_hex(&rec.log.borrow().hid[0]), speakers);

    rec.log.borrow_mut().hid.clear();
    let o = run(&argv, &mut env);
    assert!(
        o.out.contains("Toggle from Speakers -> Headphones\n"),
        "{}",
        o.out
    );
    assert_eq!(std::fs::read_to_string(&toggle).unwrap(), "Headphones");
    assert_eq!(stripped_hex(&rec.log.borrow().hid[0]), headphones);
}

#[test]
fn dry_run_output_matches_python() {
    let v = python_output();
    let tmp = TempDir::new("dry");
    for case in v["dry_run"].as_array().unwrap() {
        let argv = strings(&case["argv"]);
        let mut env = TestEnv {
            device: Device::DryRun,
            status: None,
            toggle: tmp.0.join("toggle"),
        };
        let o = run(&dry(&argv), &mut env);
        assert!(o.result.is_ok(), "{argv:?}: {:?}", o.result);
        assert_eq!(o.out, case["stdout"].as_str().unwrap(), "{argv:?}");
    }
}

#[test]
fn status_text_and_json_match_python() {
    let v = python_output();
    let entries = v["status"].as_array().unwrap();
    assert_eq!(entries.len(), 5);
    let tmp = TempDir::new("status");
    for e in entries {
        let name = e["name"].as_str().unwrap();
        let ex = exchanges(&e["exchanges"]);
        let mut env = TestEnv {
            device: Device::DryRun,
            status: Some(ex.clone()),
            toggle: tmp.0.join("toggle"),
        };
        for (argv, key) in [
            (vec!["--status"], "text"),
            (vec!["--status", "--json"], "json"),
        ] {
            let argv: Vec<String> = argv.into_iter().map(String::from).collect();
            let o = run(&argv, &mut env);
            assert!(o.result.is_ok(), "{name}: {:?}", o.result);
            assert_eq!(o.out, e[key].as_str().unwrap(), "{name} {key}");
        }

        let profile = profile_from_status(&status_from(&ex));
        let saved =
            (g6_cli::profile::settings_len(&profile) > 0).then(|| dumps_indent2(&profile) + "\n");
        assert_eq!(saved.as_deref(), e["profile"].as_str(), "{name} profile");
        assert_eq!(
            profile_to_argv(&profile).unwrap(),
            strings(&e["profile_argv"]),
            "{name} profile argv"
        );
    }
}

#[test]
fn profile_save_then_load() {
    let v = python_output();
    let tmp = TempDir::new("profile");
    for e in v["status"].as_array().unwrap() {
        let name = e["name"].as_str().unwrap();
        let ex = exchanges(&e["exchanges"]);
        let path = tmp.0.join(format!("{name}.json"));
        let path_s = path.to_string_lossy().into_owned();
        let mut env = TestEnv {
            device: Device::DryRun,
            status: Some(ex.clone()),
            toggle: tmp.0.join("toggle"),
        };
        let o = run(&["--profile-save".into(), path_s.clone()], &mut env);
        let Some(want) = e["profile"].as_str() else {
            match o.result {
                Err(CliError::Msg(m)) => {
                    assert_eq!(m, "the G6 reported no settings, nothing to save")
                }
                r => panic!("{name}: {r:?}"),
            }
            continue;
        };
        assert!(o.result.is_ok(), "{name}: {:?}", o.result);
        let n = profile_to_argv(&loads(want).unwrap()).unwrap();
        let settings = g6_cli::profile::settings_len(&loads(want).unwrap());
        assert_eq!(o.out, format!("Saved {settings} settings to {path_s}\n"));
        // Python writes in text mode: "\r\n" on Windows
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, profile_file_text(&loads(want).unwrap()), "{name}");
        assert_eq!(written.replace("\r\n", "\n"), want, "{name}");
        assert_eq!(load_profile_argv(&path_s).unwrap(), n);

        // loading it in a dry run writes what the same arguments on the command line write
        let rec = RecordingTransport::default();
        env.device = Device::Recording(rec.clone());
        // (or fails the same way: odd_values holds a gain the G6 can't take, as Python would save it too)
        let via_profile = run(&dry(&["--profile-load".into(), path_s.clone()]), &mut env);
        let loaded = std::mem::take(&mut *rec.log.borrow_mut());
        let direct_run = run(&dry(&n), &mut env);
        assert_eq!(
            format!("{:?}", via_profile.result),
            format!("{:?}", direct_run.result),
            "{name}"
        );
        let direct = rec.log.borrow();
        assert_eq!(loaded.hid, direct.hid, "{name}");
        assert_eq!(loaded.usb, direct.usb, "{name}");
    }
}

#[test]
fn profile_to_argv_matches_python() {
    let v = python_output();
    for case in v["profile_load"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        let got = profile_to_argv(&loads(text).unwrap());
        match case["error"].as_str() {
            Some(err) => assert_eq!(got, Err(err.to_string()), "{text}"),
            None => assert_eq!(got.unwrap(), strings(&case["argv"]), "{text}"),
        }
    }
}

#[test]
fn profile_load_file_errors() {
    let tmp = TempDir::new("load");
    let missing = tmp.0.join("missing.json").to_string_lossy().into_owned();
    match load_profile_argv(&missing) {
        Err(CliError::Msg(m)) => assert_eq!(
            m,
            format!(
                "[Errno 2] No such file or directory: {}",
                py_str_repr(&missing)
            )
        ),
        r => panic!("{r:?}"),
    }
    let bad = tmp.0.join("bad.json");
    std::fs::write(&bad, "{\"g6_profile\": 1,}").unwrap();
    let bad_s = bad.to_string_lossy().into_owned();
    match load_profile_argv(&bad_s) {
        Err(CliError::Msg(m)) => assert_eq!(
            m,
            format!(
                "{bad_s}: not valid JSON (Illegal trailing comma before end of object: line 1 column 17 (char 16))"
            )
        ),
        r => panic!("{r:?}"),
    }
}

/// Python's `has_action` and the flag definitions: names, groups, value names, value counts, defaults, choices.
#[test]
fn options_match_argparse() {
    let v = python_output();
    let options = v["options"].as_array().unwrap();
    let mut cmd = args::command();
    cmd.build();
    let defaults = args::parse(&[]).unwrap();
    let dests: Vec<&str> = defaults.py_values().iter().map(|(d, _)| *d).collect();
    let want_dests: Vec<&str> = options
        .iter()
        .map(|o| o["dest"].as_str().unwrap())
        .collect();
    assert_eq!(dests, want_dests, "argparse dests, in order");

    for o in options {
        let flag = o["flags"][0].as_str().unwrap();
        let dest = o["dest"].as_str().unwrap();
        let arg = cmd
            .get_arguments()
            .find(|a| a.get_long() == Some(&flag[2..]))
            .unwrap_or_else(|| panic!("{flag} missing"));
        assert_eq!(arg.get_id().as_str(), dest, "{flag}");
        assert_eq!(arg.get_help_heading(), o["group"].as_str(), "{flag} group");

        let nargs = o["nargs"].as_u64();
        let names: Vec<String> = arg
            .get_value_names()
            .map(|n| n.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default();
        match nargs {
            Some(0) => assert!(!arg.get_action().takes_values(), "{flag} takes no value"),
            n => {
                let n = n.unwrap_or(1) as usize;
                let range = arg.get_num_args().expect("set when built");
                assert_eq!(
                    (range.min_values(), range.max_values()),
                    (n, n),
                    "{flag} values"
                );
                // argparse shows "{A,B}" for choices without a metavar
                let want: Vec<String> = match &o["metavar"] {
                    Value::String(m) => vec![m.clone(); n],
                    Value::Array(m) => m.iter().map(|s| s.as_str().unwrap().to_string()).collect(),
                    _ => vec![format!("{{{}}}", strings(&o["choices"]).join(","))],
                };
                assert_eq!(names, want, "{flag} value names");
            }
        }
        let default: Vec<String> = arg
            .get_default_values()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        match o["default"].as_str() {
            Some(d) => assert_eq!(default, [d], "{flag} default"),
            None => assert!(
                default.is_empty() || default == ["false"],
                "{flag} default {default:?}"
            ),
        }

        // every choice parses, and something just outside them doesn't
        if let Some(choices) = o["choices"].as_array() {
            for c in choices {
                let c = c.as_str().unwrap();
                assert!(args::parse(&[flag.into(), c.into()]).is_ok(), "{flag} {c}");
            }
            let outside = if o["type"] == "int" {
                let ints: Vec<i64> = choices
                    .iter()
                    .map(|c| c.as_str().unwrap().parse().unwrap())
                    .collect();
                vec![
                    (ints.iter().max().unwrap() + 1).to_string(),
                    (ints[0] - 1).to_string(),
                    "x".into(),
                ]
            } else {
                // choices are case-sensitive in argparse
                let c0 = choices[0].as_str().unwrap();
                let flipped = if c0.to_lowercase() == c0 {
                    c0.to_uppercase()
                } else {
                    c0.to_lowercase()
                };
                vec!["x".into(), flipped]
            };
            for bad in outside {
                assert!(
                    args::parse(&[flag.into(), bad.clone()]).is_err(),
                    "{flag} {bad}"
                );
            }
        }

        // is the sample an action on its own (after --reload-audio-services-no-sudo implies the reload)
        let mut cli = args::parse(&strings(&o["sample"])).unwrap_or_else(|e| panic!("{flag}: {e}"));
        if cli.reload_audio_services_no_sudo {
            cli.reload_audio_services = true;
        }
        assert_eq!(
            cli.has_action(),
            o["is_action"].as_bool().unwrap(),
            "{flag} has_action"
        );
    }
}

#[test]
fn argparse_behaviour() {
    let p = |a: &[&str]| args::parse(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    // abbreviations, the last value wins, --eq-band pairs collect
    let c = p(&["--sbx-surround-v", "5", "--sbx-surround-value", "7"]).unwrap();
    assert_eq!(c.sbx_surround_value, Some(7));
    // a prefix of two flags is ambiguous, as in argparse
    assert!(p(&["--playback-vol", "5"]).is_err(), "ambiguous");
    // "--sbx" is a flag of its own even though longer flags start with it
    assert_eq!(
        p(&["--sbx", "Enabled"]).unwrap().sbx.as_deref(),
        Some("Enabled")
    );
    let c = p(&["--eq-band", "1k", "-2.5", "--eq-band", "31", "3"]).unwrap();
    assert_eq!(
        c.to_request().eq_bands,
        [("1k".into(), "-2.5".into()), ("31".into(), "3".into())]
    );
    assert_eq!(p(&["--eq-preamp", "-3"]).unwrap().eq_preamp, Some(-3.0));
    assert_eq!(
        p(&["--lighting-rgb", "1", "2", "3"]).unwrap().lighting_rgb,
        Some(vec![1, 2, 3])
    );
    assert_eq!(
        p(&["--playback-volume", " 0050 "]).unwrap().playback_volume,
        Some(50)
    );
    assert!(p(&["--playback-volume", "101"]).is_err());
    assert!(p(&["--recording-mic-boost-db", "15"]).is_err());

    assert_eq!(
        without_option(
            &[
                "--profile-load",
                "a",
                "--sbx",
                "Enabled",
                "--profile-load=b",
                "--profile-l",
                "c"
            ]
            .map(String::from),
            "--profile-load"
        ),
        ["--sbx", "Enabled", "--profile-l", "c"]
    );
}

/// Awkward command lines (values starting with "-", abbreviations, "=", "--", glued "-h") read as argparse reads
/// them: the same value for every dest, or the same usage error on stderr, or help (exit 0; clap's help text).
#[test]
fn argv_matches_argparse() {
    let v = python_output();
    let cases = v["argv"].as_array().unwrap();
    assert!(cases.len() > 90);
    for case in cases {
        let argv = strings(&case["argv"]);
        if let Some(want) = case["values"].as_object() {
            let cli = args::parse(&argv).unwrap_or_else(|e| panic!("{argv:?}: {e}"));
            let got = cli.py_values();
            // serde_json keeps keys sorted; the order itself is checked in options_match_argparse
            let mut names: Vec<&str> = got.iter().map(|(d, _)| *d).collect();
            names.sort_unstable();
            let want_names: Vec<&str> = want.keys().map(String::as_str).collect();
            assert_eq!(names, want_names, "{argv:?} dests");
            for (dest, value) in got {
                assert_eq!(value, want[dest].as_str().unwrap(), "{argv:?} {dest}");
            }
            continue;
        }
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = parse_cli_args(
            &argv,
            &mut Io {
                out: &mut out,
                err: &mut err,
            },
        );
        let code = case["exit"].as_i64().unwrap() as i32;
        match result {
            Err(CliError::Exit(c)) => assert_eq!(c, code, "{argv:?} exit"),
            r => panic!("{argv:?}: {r:?}"),
        }
        let err = String::from_utf8(err).unwrap();
        assert_eq!(err, case["stderr"].as_str().unwrap(), "{argv:?} stderr");
        let (out, want_out) = (
            String::from_utf8(out).unwrap(),
            case["stdout"].as_str().unwrap(),
        );
        if want_out.starts_with("usage: ") {
            // the help text itself is clap's (see doc/rust-port.md)
            assert_eq!(out, args::render_help(), "{argv:?} help");
        } else {
            assert_eq!(out, want_out, "{argv:?} stdout");
        }
    }
}

#[test]
fn usage_errors() {
    let tmp = TempDir::new("usage");
    let mut env = TestEnv {
        device: Device::DryRun,
        status: None,
        toggle: tmp.0.join("toggle"),
    };
    let o = run(&dry(&[]), &mut env);
    match &o.result {
        Err(CliError::Msg(m)) => assert_eq!(m, "No meaningful argument has been specified!"),
        r => panic!("{r:?}"),
    }
    assert!(
        o.out
            .starts_with("No meaningful argument has been specified!\n")
    );

    let o = run(
        &dry(&[
            "--toggle-output".into(),
            "--set-output".into(),
            "Speakers".into(),
        ]),
        &mut env,
    );
    assert!(o.out.starts_with(
        "Only one of the following CLI arguments may be specified: '--toggle-output', '--set-output'!\n"
    ));
    assert!(matches!(o.result, Err(CliError::Msg(_))));

    let o = run(&dry(&["--eq-preset".into(), "nope".into()]), &mut env);
    assert!(matches!(o.result, Err(CliError::Exit(2))));
    assert!(o.err.starts_with("usage: soundblaster-x-g6-cli\nsoundblaster-x-g6-cli: error: unknown EQ preset 'nope', expected one of Acoustic, "));

    let o = run(&["--version".into()], &mut env);
    assert!(matches!(o.result, Err(CliError::Exit(0))));
    assert_eq!(
        o.out,
        format!("soundblaster-x-g6-cli {}\n", g6_cli::VERSION)
    );

    let o = run(&dry(&["--bogus".into()]), &mut env);
    assert!(matches!(o.result, Err(CliError::Exit(2))));
    assert!(!o.err.is_empty());

    // a period of 0 means the default (Python's `period or default`); a negative one is refused
    let o = run(
        &dry(&[
            "--lighting-effect",
            "pulsate",
            "--lighting-effect-period",
            "-1",
        ]
        .map(String::from)),
        &mut env,
    );
    match &o.result {
        Err(CliError::Msg(m)) => assert_eq!(m, "--lighting-effect-period must be > 0, got -1.0"),
        r => panic!("{r:?}"),
    }
}

#[test]
fn version_matches_python() {
    let init = std::fs::read_to_string(manifest().join("../../src/g6_cli/__init__.py")).unwrap();
    let line = init
        .lines()
        .find(|l| l.starts_with("VERSION"))
        .expect("VERSION in g6_cli/__init__.py");
    assert!(line.contains(&format!("'{}'", g6_cli::VERSION)), "{line}");
}

/// Python's float formatting and rounding over a sweep of values (repr, "g", round to 4 and 2 places, to int).
#[test]
fn python_float_formatting() {
    let v = python_output();
    let entries = v["pyfmt"].as_array().unwrap();
    assert!(entries.len() > 700);
    let float = |s: &Value| {
        let b = unhex(s.as_str().unwrap()).unwrap();
        f64::from_le_bytes(b.try_into().unwrap())
    };
    for e in entries {
        let x = float(&e["bits"]);
        assert_eq!(py_repr(x), e["repr"].as_str().unwrap(), "repr {x:e}");
        assert_eq!(py_g(x), e["g"].as_str().unwrap(), "g {x:e}");
        for (n, key) in [(4, "round4"), (2, "round2")] {
            let (got, want) = (py_round(x, n), float(&e[key]));
            assert!(
                got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan()),
                "{key} {x:e}"
            );
        }
        match (py_round_int(x), e["round"].as_str()) {
            (Some(got), Some(want)) => assert_eq!(got.to_string(), want, "round {x:e}"),
            // Python has big ints; past i64 the port says None, which only NaN, inf and huge values reach
            (None, Some(want)) => assert!(want.len() > 18, "round {x:e}: {want}"),
            (None, None) => {}
            (Some(got), None) => panic!("round {x:e}: Python fails, port gives {got}"),
        }
    }
}
