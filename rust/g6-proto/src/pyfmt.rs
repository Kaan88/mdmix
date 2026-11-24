//! Python's number formatting and parsing, where the CLI output or an error message depends on it.
//!
//! Rust's float formatting is exact and rounds ties to even, as CPython's does, so these only rearrange the digits
//! Rust produces into the layout Python uses.

/// Splits the shortest round-trip digits of a finite, non-zero `x` into (negative, digits, exponent), with
/// `x = 0.d1d2d3... * 10^(exponent + 1)`.
fn sci_parts(s: &str) -> (bool, String, i32) {
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (mant, exp) = s.split_once('e').expect("{:e} output has an exponent");
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    (neg, digits, exp.parse().expect("integer exponent"))
}

fn exp_suffix(exp: i32) -> String {
    let sign = if exp < 0 { '-' } else { '+' };
    format!("e{sign}{:02}", exp.abs())
}

/// Lays out significant `digits` (no dot) with decimal exponent `exp` in fixed notation.
fn fixed(digits: &str, exp: i32) -> String {
    let n = digits.len() as i32;
    if exp < 0 {
        format!("0.{}{digits}", "0".repeat((-exp - 1) as usize))
    } else if exp + 1 >= n {
        format!("{digits}{}", "0".repeat((exp + 1 - n) as usize))
    } else {
        let (a, b) = digits.split_at((exp + 1) as usize);
        format!("{a}.{b}")
    }
}

fn sci(digits: &str, exp: i32) -> String {
    let (a, b) = digits.split_at(1);
    if b.is_empty() {
        format!("{a}{}", exp_suffix(exp))
    } else {
        format!("{a}.{b}{}", exp_suffix(exp))
    }
}

fn non_finite(x: f64) -> Option<&'static str> {
    if x.is_nan() {
        Some("nan")
    } else if x == f64::INFINITY {
        Some("inf")
    } else if x == f64::NEG_INFINITY {
        Some("-inf")
    } else {
        None
    }
}

/// `repr(x)` for a float: the shortest string that reads back as `x`, e.g. `1.0`, `0.0312`, `1e+20`, `1e-05`.
pub fn py_repr(x: f64) -> String {
    if let Some(s) = non_finite(x) {
        return s.to_string();
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    let (neg, digits, exp) = sci_parts(&format!("{x:e}"));
    let body = if (-4..16).contains(&exp) {
        let f = fixed(&digits, exp);
        if f.contains('.') { f } else { f + ".0" }
    } else {
        sci(&digits, exp)
    };
    if neg { format!("-{body}") } else { body }
}

/// `format(x, 'g')` / `f'{x:g}'`: six significant digits, trailing zeros dropped.
pub fn py_g(x: f64) -> String {
    if let Some(s) = non_finite(x) {
        return s.to_string();
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0" } else { "0" }.to_string();
    }
    // rounding to 6 digits first, since that can carry into the exponent (999999.5 -> 1e+06)
    let (neg, digits, exp) = sci_parts(&format!("{x:.5e}"));
    let digits = digits.trim_end_matches('0');
    let body = if (-4..6).contains(&exp) {
        fixed(digits, exp)
    } else {
        sci(digits, exp)
    };
    if neg { format!("-{body}") } else { body }
}

/// `round(x, ndigits)` for a float: the nearest float to the correctly rounded decimal (ties to even, on the exact
/// binary value). NaN and infinities come back unchanged, as in Python.
pub fn py_round(x: f64, ndigits: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let r: f64 = format!("{x:.ndigits$}")
        .parse()
        .expect("Rust reads its own output");
    // Python keeps the sign of a value that rounds to zero
    if r == 0.0 { 0.0f64.copysign(x) } else { r }
}

/// `round(x)` to an integer, ties to even. `None` where Python raises (NaN, infinities, out of range).
pub fn py_round_int(x: f64) -> Option<i64> {
    let r = x.round_ties_even();
    // 2^63 itself is out of range, so the upper bound is exclusive
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    if (-LIMIT..LIMIT).contains(&r) {
        Some(r as i64)
    } else {
        None
    }
}

/// Python allows single underscores between digits in numeric strings ("1_000"). Returns the string without them,
/// or `None` if one is misplaced.
fn strip_underscores(s: &str) -> Option<String> {
    let b = s.as_bytes();
    for (i, c) in b.iter().enumerate() {
        if *c == b'_' {
            let before = i > 0 && b[i - 1].is_ascii_digit();
            let after = i + 1 < b.len() && b[i + 1].is_ascii_digit();
            if !(before && after) {
                return None;
            }
        }
    }
    Some(s.replace('_', ""))
}

/// `float(s)`: surrounding whitespace allowed, as are `inf`, `infinity` and `nan` in any case, with a sign.
pub fn py_float(s: &str) -> Option<f64> {
    let t = strip_underscores(s.trim())?;
    let unsigned = t.strip_prefix(['+', '-']).unwrap_or(&t);
    // Rust also reads these words, but checks them here so nothing else gets through by accident
    let word = unsigned.to_ascii_lowercase();
    if word == "inf" || word == "infinity" || word == "nan" {
        let v = if word == "nan" {
            f64::NAN
        } else {
            f64::INFINITY
        };
        return Some(if t.starts_with('-') { -v } else { v });
    }
    let ok = !unsigned.is_empty()
        && unsigned
            .bytes()
            .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-'))
        && unsigned.bytes().any(|c| c.is_ascii_digit());
    if !ok {
        return None;
    }
    t.parse().ok()
}

/// `int(s)` in base 10: surrounding whitespace, a sign, digits with single underscores between them.
pub fn py_int(s: &str) -> Option<i64> {
    let t = s.trim();
    let (neg, digits) = match t.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit() || c == b'_') {
        return None;
    }
    let digits = strip_underscores(digits)?;
    let v: i64 = digits.parse().ok()?;
    Some(if neg { -v } else { v })
}

/// `repr(s)` for a str: single quotes unless the text holds a single quote and no double quote.
pub fn py_str_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            // Python escapes more non-printable code points than this; the CLI only ever shows user input here
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_matches_python() {
        let cases = [
            (1.0, "1.0"),
            (-0.0, "-0.0"),
            (0.0312, "0.0312"),
            (2.5, "2.5"),
            (1e20, "1e+20"),
            (1.0000000200408773e20, "1.0000000200408773e+20"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (1e15, "1000000000000000.0"),
            (1e16, "1e+16"),
            (123456.789, "123456.789"),
            (-3.5, "-3.5"),
            (f64::NAN, "nan"),
            (f64::NEG_INFINITY, "-inf"),
        ];
        for (x, want) in cases {
            assert_eq!(py_repr(x), want, "{x:e}");
        }
    }

    #[test]
    fn g_matches_python() {
        let cases = [
            (0.0, "0"),
            (-0.0, "-0"),
            (12.0, "12"),
            (-12.0, "-12"),
            (2.5, "2.5"),
            (123456.7, "123457"),
            (999999.5, "1e+06"),
            (1e20, "1e+20"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.0000000200408773e20, "1e+20"),
            (0.03125, "0.03125"),
            (12.5, "12.5"),
            (3.33, "3.33"),
            (f64::INFINITY, "inf"),
        ];
        for (x, want) in cases {
            assert_eq!(py_g(x), want, "{x:e}");
        }
    }

    #[test]
    fn round_matches_python() {
        assert_eq!(py_round(0.03125, 4), 0.0312); // exact tie, to even
        assert_eq!(py_round(0.03135, 4), 0.0314); // not a tie in binary: 0.0313500000000000029
        assert_eq!(py_round(2.675, 2), 2.67);
        assert_eq!(py_round(-0.00001, 4).to_bits(), (-0.0f64).to_bits());
        assert_eq!(py_round(1.0000000200408773e20, 4), 1.0000000200408773e20);
        assert!(py_round(f64::NAN, 4).is_nan());
        assert_eq!(py_round_int(0.5), Some(0));
        assert_eq!(py_round_int(1.5), Some(2));
        assert_eq!(py_round_int(-2.5), Some(-2));
        assert_eq!(py_round_int(f64::INFINITY), None);
    }

    #[test]
    fn parsing_matches_python() {
        assert_eq!(py_float(" 2.5 "), Some(2.5));
        assert_eq!(py_float("1_0.5"), Some(10.5));
        assert_eq!(py_float("-Infinity"), Some(f64::NEG_INFINITY));
        assert!(py_float("nan").unwrap().is_nan());
        assert_eq!(py_float(".5"), Some(0.5));
        assert_eq!(py_float("5."), Some(5.0));
        assert_eq!(py_float("1e3"), Some(1000.0));
        for bad in ["", "1__0", "_1", "1_", "abc", "1,5", "0x10", "--1", "in f"] {
            assert_eq!(py_float(bad), None, "{bad:?}");
        }
        assert_eq!(py_int(" 010 "), Some(10));
        assert_eq!(py_int("+7"), Some(7));
        assert_eq!(py_int("-1_000"), Some(-1000));
        for bad in ["", "1.0", "1__0", "_1", "- 1", "0x1"] {
            assert_eq!(py_int(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn str_repr_matches_python() {
        assert_eq!(py_str_repr("abc"), "'abc'");
        assert_eq!(py_str_repr("it's"), "\"it's\"");
        assert_eq!(py_str_repr("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(py_str_repr("x\u{1}"), "'x\\x01'");
    }
}
