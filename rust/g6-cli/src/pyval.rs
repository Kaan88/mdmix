//! The Python values a profile file holds, read and written the way Python's `json` module does it, and shown the
//! way `str()` / `repr()` show them (profile error messages quote values with `repr`).
//!
//! A hand-written reader rather than serde_json: Python keeps object keys in file order (with the first position
//! for a repeated key), reads `NaN` / `Infinity`, keeps integers of any size, and its error messages name the
//! character position. Profiles are tiny, so speed doesn't matter.

use g6_proto::pyfmt::{py_repr, py_str_repr};

/// A value as Python's `json.loads` returns it.
#[derive(Clone, PartialEq, Debug)]
pub enum PyVal {
    None,
    Bool(bool),
    /// Decimal digits with an optional leading '-', as `str(int)` prints them (no "-0").
    Int(String),
    Float(f64),
    Str(String),
    List(Vec<PyVal>),
    /// Keys in insertion order.
    Dict(Vec<(String, PyVal)>),
}

impl PyVal {
    pub fn int(v: i64) -> PyVal {
        PyVal::Int(v.to_string())
    }

    pub fn str(s: impl Into<String>) -> PyVal {
        PyVal::Str(s.into())
    }

    /// `dict.get(key)`.
    pub fn get(&self, key: &str) -> Option<&PyVal> {
        match self {
            PyVal::Dict(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// `self == 1` in Python: `True`, `1` and `1.0` all are.
    pub fn equals_one(&self) -> bool {
        match self {
            PyVal::Bool(b) => *b,
            PyVal::Int(s) => s == "1",
            PyVal::Float(f) => *f == 1.0,
            _ => false,
        }
    }

    /// `repr(self)`.
    pub fn repr(&self) -> String {
        match self {
            PyVal::None => "None".into(),
            PyVal::Bool(true) => "True".into(),
            PyVal::Bool(false) => "False".into(),
            PyVal::Int(s) => s.clone(),
            PyVal::Float(f) => py_repr(*f),
            PyVal::Str(s) => py_str_repr(s),
            PyVal::List(items) => {
                let inner: Vec<String> = items.iter().map(PyVal::repr).collect();
                format!("[{}]", inner.join(", "))
            }
            PyVal::Dict(items) => {
                let inner: Vec<String> = items
                    .iter()
                    .map(|(k, v)| format!("{}: {}", py_str_repr(k), v.repr()))
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
        }
    }

    /// `str(self)`: like `repr` except that a string is itself.
    pub fn py_str(&self) -> String {
        match self {
            PyVal::Str(s) => s.clone(),
            other => other.repr(),
        }
    }
}

// --- writing: json.dumps(value, indent=2) ---------------------------------------------------------------------

/// `json.dumps(value, indent=2)`: ASCII only, `", "` / `": "` separators turned into `","` / `": "` plus
/// newlines, floats as `repr`, NaN and the infinities as `NaN` / `Infinity` / `-Infinity`.
pub fn dumps_indent2(value: &PyVal) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0);
    out
}

fn newline(out: &mut String, level: usize) {
    out.push('\n');
    out.extend(std::iter::repeat_n(' ', 2 * level));
}

fn write_value(out: &mut String, value: &PyVal, level: usize) {
    match value {
        PyVal::None => out.push_str("null"),
        PyVal::Bool(true) => out.push_str("true"),
        PyVal::Bool(false) => out.push_str("false"),
        PyVal::Int(s) => out.push_str(s),
        PyVal::Float(f) => out.push_str(&json_float(*f)),
        PyVal::Str(s) => write_str(out, s),
        PyVal::List(items) if items.is_empty() => out.push_str("[]"),
        PyVal::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, level + 1);
                write_value(out, item, level + 1);
            }
            newline(out, level);
            out.push(']');
        }
        PyVal::Dict(items) if items.is_empty() => out.push_str("{}"),
        PyVal::Dict(items) => {
            out.push('{');
            for (i, (k, v)) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, level + 1);
                write_str(out, k);
                out.push_str(": ");
                write_value(out, v, level + 1);
            }
            newline(out, level);
            out.push('}');
        }
    }
}

fn json_float(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f == f64::INFINITY {
        "Infinity".into()
    } else if f == f64::NEG_INFINITY {
        "-Infinity".into()
    } else {
        py_repr(f)
    }
}

fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            // control characters, DEL is kept as is (it is ASCII), everything else as \u escapes
            '\u{7f}' => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
        }
    }
    out.push('"');
}

// --- reading: json.loads(text) --------------------------------------------------------------------------------

/// Why `json.loads` failed.
#[derive(Clone, PartialEq, Debug)]
pub enum LoadError {
    /// A `json.JSONDecodeError`, already formatted as Python prints it.
    Decode(String),
    /// Some other `ValueError` (an integer too long to convert).
    Value(String),
}

/// Python refuses to convert integers with more digits than this (`sys.get_int_max_str_digits()`).
const INT_MAX_STR_DIGITS: usize = 4300;

/// `json.loads(text)` for a `str` (Python's C scanner, which reports the positions given here).
pub fn loads(text: &str) -> Result<PyVal, LoadError> {
    let s: Vec<char> = text.chars().collect();
    let p = Parser { s: &s };
    if s.first() == Some(&'\u{feff}') {
        return Err(p.error("Unexpected UTF-8 BOM (decode using utf-8-sig)", 0));
    }
    let start = p.skip_ws(0);
    let (value, end) = match p.scan_once(start) {
        Ok(v) => v,
        Err(Fail::Stop(at)) => return Err(p.error("Expecting value", at)),
        Err(Fail::Error(e)) => return Err(e),
    };
    let end = p.skip_ws(end);
    if end != s.len() {
        return Err(p.error("Extra data", end));
    }
    Ok(value)
}

/// The C scanner's two ways to fail: "no value starts here" (Python's StopIteration, which the caller reports as
/// "Expecting value") and a finished error.
enum Fail {
    Stop(usize),
    Error(LoadError),
}

impl From<LoadError> for Fail {
    fn from(e: LoadError) -> Self {
        Fail::Error(e)
    }
}

struct Parser<'a> {
    s: &'a [char],
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

impl Parser<'_> {
    fn at(&self, i: usize) -> Option<char> {
        self.s.get(i).copied()
    }

    fn skip_ws(&self, mut i: usize) -> usize {
        while self.at(i).is_some_and(is_ws) {
            i += 1;
        }
        i
    }

    /// `JSONDecodeError(msg, doc, pos)` as `str()` shows it.
    fn error(&self, msg: &str, pos: usize) -> LoadError {
        let before = &self.s[..pos.min(self.s.len())];
        let line = before.iter().filter(|c| **c == '\n').count() + 1;
        let col = match before.iter().rposition(|c| *c == '\n') {
            Some(nl) => pos - nl,
            None => pos + 1,
        };
        LoadError::Decode(format!("{msg}: line {line} column {col} (char {pos})"))
    }

    fn starts_with(&self, i: usize, word: &str) -> bool {
        let w: Vec<char> = word.chars().collect();
        self.s.get(i..i + w.len()) == Some(&w[..])
    }

    fn scan_once(&self, i: usize) -> Result<(PyVal, usize), Fail> {
        let Some(c) = self.at(i) else {
            return Err(Fail::Stop(i));
        };
        let word = |w: &str, v: PyVal| {
            if self.starts_with(i, w) {
                Some((v, i + w.chars().count()))
            } else {
                None
            }
        };
        let found = match c {
            '"' => return self.scan_string(i + 1).map(|(s, e)| (PyVal::Str(s), e)),
            '{' => return self.parse_object(i + 1),
            '[' => return self.parse_array(i + 1),
            'n' => word("null", PyVal::None),
            't' => word("true", PyVal::Bool(true)),
            'f' => word("false", PyVal::Bool(false)),
            'N' => word("NaN", PyVal::Float(f64::NAN)),
            'I' => word("Infinity", PyVal::Float(f64::INFINITY)),
            '-' => word("-Infinity", PyVal::Float(f64::NEG_INFINITY)),
            _ => None,
        };
        match found {
            Some(v) => Ok(v),
            None => self.match_number(i),
        }
    }

    fn match_number(&self, start: usize) -> Result<(PyVal, usize), Fail> {
        let digit = |i: usize| self.at(i).is_some_and(|c| c.is_ascii_digit());
        let mut i = start;
        if self.at(i) == Some('-') {
            i += 1;
        }
        match self.at(i) {
            Some('1'..='9') => {
                i += 1;
                while digit(i) {
                    i += 1;
                }
            }
            Some('0') => i += 1,
            _ => return Err(Fail::Stop(start)),
        }
        let int_end = i;
        let mut is_float = false;
        if self.at(i) == Some('.') && digit(i + 1) {
            is_float = true;
            i += 2;
            while digit(i) {
                i += 1;
            }
        }
        if matches!(self.at(i), Some('e' | 'E')) {
            let e_start = i;
            i += 1;
            if matches!(self.at(i), Some('-' | '+')) {
                i += 1;
            }
            while digit(i) {
                i += 1;
            }
            // an exponent without digits isn't part of the number
            if digit(i - 1) {
                is_float = true;
            } else {
                i = e_start;
            }
        }
        let text: String = self.s[start..i].iter().collect();
        if is_float {
            let f: f64 = text
                .parse()
                .expect("the scanner only takes valid float syntax");
            return Ok((PyVal::Float(f), i));
        }
        debug_assert_eq!(int_end, i);
        let (neg, digits) = match text.strip_prefix('-') {
            Some(d) => (true, d),
            None => (false, text.as_str()),
        };
        if digits.len() > INT_MAX_STR_DIGITS {
            return Err(Fail::Error(LoadError::Value(format!(
                "Exceeds the limit ({INT_MAX_STR_DIGITS} digits) for integer string conversion: value has {} \
                 digits; use sys.set_int_max_str_digits() to increase the limit",
                digits.len()
            ))));
        }
        let int = if neg && digits != "0" {
            format!("-{digits}")
        } else {
            digits.to_string()
        };
        Ok((PyVal::Int(int), i))
    }

    /// The string whose opening quote is just before `end`; returns it and the index after the closing quote.
    fn scan_string(&self, mut end: usize) -> Result<(String, usize), Fail> {
        let begin = end - 1;
        let len = self.s.len();
        let mut out = String::new();
        loop {
            let mut next = end;
            let mut c = '\0';
            while next < len {
                c = self.s[next];
                if c == '"' || c == '\\' {
                    break;
                }
                if (c as u32) <= 0x1f {
                    return Err(self.error("Invalid control character at", next).into());
                }
                next += 1;
            }
            if next >= len {
                return Err(self.error("Unterminated string starting at", begin).into());
            }
            out.extend(&self.s[end..next]);
            if c == '"' {
                return Ok((out, next + 1));
            }
            // a backslash
            next += 1;
            if next == len {
                return Err(self.error("Unterminated string starting at", begin).into());
            }
            let esc = self.s[next];
            if esc != 'u' {
                end = next + 1;
                let ch = match esc {
                    '"' => '"',
                    '\\' => '\\',
                    '/' => '/',
                    'b' => '\u{8}',
                    'f' => '\u{c}',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    _ => return Err(self.error("Invalid \\escape", end - 2).into()),
                };
                out.push(ch);
                continue;
            }
            next += 1;
            end = next + 4;
            if end >= len {
                return Err(self.error("Invalid \\uXXXX escape", next - 1).into());
            }
            let mut code = self
                .hex4(next)
                .ok_or_else(|| self.error("Invalid \\uXXXX escape", end - 5))?;
            // a high surrogate followed by an escaped low one is a single character
            if (0xd800..=0xdbff).contains(&code)
                && end + 6 < len
                && self.s[end] == '\\'
                && self.s[end + 1] == 'u'
            {
                let low = self
                    .hex4(end + 2)
                    .ok_or_else(|| self.error("Invalid \\uXXXX escape", end + 1))?;
                if (0xdc00..=0xdfff).contains(&low) {
                    code = 0x10000 + (((code - 0xd800) << 10) | (low - 0xdc00));
                    end += 6;
                }
            }
            // Python keeps a lone surrogate in the string; a Rust string can't hold one
            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
        }
    }

    fn hex4(&self, at: usize) -> Option<u32> {
        let mut v = 0;
        for i in at..at + 4 {
            v = v * 16 + self.at(i)?.to_digit(16)?;
        }
        Some(v)
    }

    fn parse_object(&self, mut i: usize) -> Result<(PyVal, usize), Fail> {
        let mut items: Vec<(String, PyVal)> = Vec::new();
        i = self.skip_ws(i);
        if self.at(i) != Some('}') {
            loop {
                if self.at(i) != Some('"') {
                    return Err(self
                        .error("Expecting property name enclosed in double quotes", i)
                        .into());
                }
                let (key, next) = self.scan_string(i + 1)?;
                i = self.skip_ws(next);
                if self.at(i) != Some(':') {
                    return Err(self.error("Expecting ':' delimiter", i).into());
                }
                i = self.skip_ws(i + 1);
                let (value, next) = self.scan_once(i)?;
                // dict(pairs): a repeated key keeps its first place and takes the last value
                match items.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 = value,
                    None => items.push((key, value)),
                }
                i = self.skip_ws(next);
                if self.at(i) == Some('}') {
                    break;
                }
                if self.at(i) != Some(',') {
                    return Err(self.error("Expecting ',' delimiter", i).into());
                }
                let comma = i;
                i = self.skip_ws(i + 1);
                if self.at(i) == Some('}') {
                    return Err(self
                        .error("Illegal trailing comma before end of object", comma)
                        .into());
                }
            }
        }
        Ok((PyVal::Dict(items), i + 1))
    }

    fn parse_array(&self, mut i: usize) -> Result<(PyVal, usize), Fail> {
        let mut items = Vec::new();
        i = self.skip_ws(i);
        if self.at(i) != Some(']') {
            loop {
                let (value, next) = self.scan_once(i)?;
                items.push(value);
                i = self.skip_ws(next);
                if self.at(i) == Some(']') {
                    break;
                }
                if self.at(i) != Some(',') {
                    return Err(self.error("Expecting ',' delimiter", i).into());
                }
                let comma = i;
                i = self.skip_ws(i + 1);
                if self.at(i) == Some(']') {
                    return Err(self
                        .error("Illegal trailing comma before end of array", comma)
                        .into());
                }
            }
        }
        Ok((PyVal::List(items), i + 1))
    }
}

/// How Python's text mode reads a file opened with `encoding='utf-8'`: invalid UTF-8 is an error (worded as
/// Python's codec words it), and `\r\n` / `\r` become `\n`.
pub fn decode_text_file(bytes: &[u8]) -> Result<String, String> {
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(e) => {
            let pos = e.valid_up_to();
            let reason = match e.error_len() {
                None => "unexpected end of data",
                Some(_) if matches!(bytes[pos], 0x80..=0xc1 | 0xf5..=0xff) => "invalid start byte",
                Some(_) => "invalid continuation byte",
            };
            return Err(format!(
                "'utf-8' codec can't decode byte 0x{:02x} in position {pos}: {reason}",
                bytes[pos]
            ));
        }
    };
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(text: &str) -> String {
        match loads(text) {
            Err(LoadError::Decode(m)) | Err(LoadError::Value(m)) => m,
            Ok(v) => panic!("{text:?} parsed as {v:?}"),
        }
    }

    #[test]
    fn errors_match_python() {
        // from Python 3.13's json.loads
        let cases = [
            ("", "Expecting value: line 1 column 1 (char 0)"),
            (" ", "Expecting value: line 1 column 2 (char 1)"),
            (
                "{",
                "Expecting property name enclosed in double quotes: line 1 column 2 (char 1)",
            ),
            (
                "{\"a\"",
                "Expecting ':' delimiter: line 1 column 5 (char 4)",
            ),
            ("{\"a\":", "Expecting value: line 1 column 6 (char 5)"),
            (
                "{\"a\":1",
                "Expecting ',' delimiter: line 1 column 7 (char 6)",
            ),
            (
                "{\"a\":1,}",
                "Illegal trailing comma before end of object: line 1 column 7 (char 6)",
            ),
            (
                "[1,]",
                "Illegal trailing comma before end of array: line 1 column 3 (char 2)",
            ),
            (
                "\"abc",
                "Unterminated string starting at: line 1 column 1 (char 0)",
            ),
            ("\"a\\x\"", "Invalid \\escape: line 1 column 3 (char 2)"),
            (
                "\"a\\u12\"",
                "Invalid \\uXXXX escape: line 1 column 4 (char 3)",
            ),
            (
                "\"\\uZZZZ\"",
                "Invalid \\uXXXX escape: line 1 column 3 (char 2)",
            ),
            (
                "\"a\u{1}\"",
                "Invalid control character at: line 1 column 3 (char 2)",
            ),
            ("[1]x", "Extra data: line 1 column 4 (char 3)"),
            ("01", "Extra data: line 1 column 2 (char 1)"),
            ("{\"a\":1}\n\n x", "Extra data: line 3 column 2 (char 10)"),
            (
                "\u{feff}{}",
                "Unexpected UTF-8 BOM (decode using utf-8-sig): line 1 column 1 (char 0)",
            ),
            ("[-]", "Expecting value: line 1 column 2 (char 1)"),
            (
                "{\"a\":1 , }",
                "Illegal trailing comma before end of object: line 1 column 8 (char 7)",
            ),
        ];
        for (text, want) in cases {
            assert_eq!(err(text), want, "{text:?}");
        }
    }

    #[test]
    fn values_match_python() {
        let v = loads("{\"a\":1,\"a\":2,\"b\":[-0, 1e400, NaN, \"\\ud83d\\ude00\"]}").unwrap();
        let PyVal::Dict(items) = &v else { panic!() };
        assert_eq!(items[0], ("a".into(), PyVal::int(2)));
        assert_eq!(v.get("b").unwrap().repr(), "[0, inf, nan, '\u{1f600}']");
        assert_eq!(
            dumps_indent2(&loads("{\"x\": [], \"y\": {}}").unwrap()),
            "{\n  \"x\": [],\n  \"y\": {}\n}"
        );
        assert_eq!(
            dumps_indent2(&PyVal::str("\u{e9}\u{1}\u{1f600}")),
            "\"\\u00e9\\u0001\\ud83d\\ude00\""
        );
    }
}
