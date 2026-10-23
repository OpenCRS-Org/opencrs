//! A small parser that behaves like Go's `flag` package (as `opencrs`'s Go
//! CLI uses it with `ContinueOnError`), including the messages it prints and
//! the `interleaved` wrapper that allows flags after positional arguments.

use std::collections::BTreeMap;
use std::io::Write;

/// What kind of value a flag takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Str,
    Bool,
    Float,
    /// A string flag that may repeat (Go's `multiFlag`).
    Multi,
}

#[derive(Debug, Clone)]
enum Value {
    Str(String),
    Bool(bool),
    Float(f64),
    Multi(Vec<String>),
}

struct Def {
    kind: Kind,
    usage: &'static str,
    /// Go's `DefValue`: the default, as text.
    default: String,
    value: Value,
}

/// Why parsing stopped. The message and usage are already printed.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    Help,
    Failed,
}

/// Go's `flag.FlagSet` with `ContinueOnError`, writing to `out`.
pub struct FlagSet<'a> {
    name: &'static str,
    defs: BTreeMap<&'static str, Def>,
    args: Vec<String>,
    out: &'a mut dyn Write,
}

impl<'a> FlagSet<'a> {
    pub fn new(name: &'static str, out: &'a mut dyn Write) -> Self {
        FlagSet {
            name,
            defs: BTreeMap::new(),
            args: Vec::new(),
            out,
        }
    }

    pub fn string(&mut self, name: &'static str, default: &str, usage: &'static str) {
        self.def(
            name,
            Kind::Str,
            usage,
            default.to_string(),
            Value::Str(default.into()),
        );
    }

    pub fn boolean(&mut self, name: &'static str, usage: &'static str) {
        self.def(name, Kind::Bool, usage, "false".into(), Value::Bool(false));
    }

    /// `default_text` is `strconv.FormatFloat(default, 'g', -1, 64)`.
    pub fn float(
        &mut self,
        name: &'static str,
        default: f64,
        default_text: &str,
        usage: &'static str,
    ) {
        self.def(
            name,
            Kind::Float,
            usage,
            default_text.to_string(),
            Value::Float(default),
        );
    }

    pub fn multi(&mut self, name: &'static str, usage: &'static str) {
        self.def(
            name,
            Kind::Multi,
            usage,
            String::new(),
            Value::Multi(Vec::new()),
        );
    }

    fn def(
        &mut self,
        name: &'static str,
        kind: Kind,
        usage: &'static str,
        default: String,
        value: Value,
    ) {
        self.defs.insert(
            name,
            Def {
                kind,
                usage,
                default,
                value,
            },
        );
    }

    pub fn get_string(&self, name: &str) -> &str {
        match self.defs.get(name).map(|d| &d.value) {
            Some(Value::Str(s)) => s,
            _ => "",
        }
    }

    pub fn get_bool(&self, name: &str) -> bool {
        matches!(
            self.defs.get(name).map(|d| &d.value),
            Some(Value::Bool(true))
        )
    }

    pub fn get_float(&self, name: &str) -> f64 {
        match self.defs.get(name).map(|d| &d.value) {
            Some(Value::Float(f)) => *f,
            _ => 0.0,
        }
    }

    pub fn get_multi(&self, name: &str) -> &[String] {
        match self.defs.get(name).map(|d| &d.value) {
            Some(Value::Multi(v)) => v,
            _ => &[],
        }
    }

    /// The arguments left after the flags.
    pub fn rest(&self) -> &[String] {
        &self.args
    }

    /// Go's `Parse`: flags up to the first non-flag argument.
    pub fn parse(&mut self, arguments: &[String]) -> Result<(), ParseError> {
        self.args = arguments.to_vec();
        loop {
            match self.parse_one()? {
                true => continue,
                false => return Ok(()),
            }
        }
    }

    /// Go's `interleaved`: flags may come before or after positionals.
    pub fn parse_interleaved(&mut self, arguments: &[String]) -> Result<(), ParseError> {
        let mut args = arguments.to_vec();
        let mut pos: Vec<String> = Vec::new();
        loop {
            self.parse(&args)?;
            if self.args.is_empty() {
                break;
            }
            pos.push(self.args[0].clone());
            args = self.args[1..].to_vec();
        }
        self.parse(&pos)
    }

    fn fail(&mut self, msg: String) -> ParseError {
        let _ = writeln!(self.out, "{msg}");
        self.usage();
        ParseError::Failed
    }

    fn usage(&mut self) {
        let _ = writeln!(self.out, "Usage of {}:", self.name);
        let mut text = String::new();
        for (name, d) in &self.defs {
            text.push_str(&describe(name, d));
        }
        let _ = self.out.write_all(text.as_bytes());
    }

    fn parse_one(&mut self) -> Result<bool, ParseError> {
        let Some(s) = self.args.first().cloned() else {
            return Ok(false);
        };
        if s.len() < 2 || !s.starts_with('-') {
            return Ok(false);
        }
        let mut minuses = 1;
        if s.as_bytes()[1] == b'-' {
            minuses += 1;
            if s.len() == 2 {
                self.args.remove(0);
                return Ok(false);
            }
        }
        let name = &s[minuses..];
        if name.is_empty() || name.starts_with('-') || name.starts_with('=') {
            return Err(self.fail(format!("bad flag syntax: {s}")));
        }
        self.args.remove(0);
        let (name, value) = match name.char_indices().skip(1).find(|&(_, c)| c == '=') {
            Some((i, _)) => (&name[..i], Some(name[i + 1..].to_string())),
            None => (name, None),
        };
        let Some(kind) = self.defs.get(name).map(|d| d.kind) else {
            if name == "help" || name == "h" {
                self.usage();
                return Err(ParseError::Help);
            }
            return Err(self.fail(format!("flag provided but not defined: -{name}")));
        };
        if kind == Kind::Bool {
            match value {
                Some(v) => match parse_bool(&v) {
                    Some(b) => self.set(name, Value::Bool(b)),
                    None => {
                        return Err(self.fail(format!(
                            "invalid boolean value {} for -{name}: parse error",
                            go_quote(&v)
                        )));
                    }
                },
                None => self.set(name, Value::Bool(true)),
            }
            return Ok(true);
        }
        let value = match value {
            Some(v) => v,
            None if !self.args.is_empty() => self.args.remove(0),
            None => return Err(self.fail(format!("flag needs an argument: -{name}"))),
        };
        match kind {
            Kind::Str => self.set(name, Value::Str(value)),
            Kind::Multi => {
                if let Some(Def {
                    value: Value::Multi(v),
                    ..
                }) = self.defs.get_mut(name)
                {
                    v.push(value);
                }
            }
            Kind::Float => match parse_float(&value) {
                Ok(f) => self.set(name, Value::Float(f)),
                Err(why) => {
                    return Err(self.fail(format!(
                        "invalid value {} for flag -{name}: {why}",
                        go_quote(&value)
                    )));
                }
            },
            Kind::Bool => unreachable!(),
        }
        Ok(true)
    }

    fn set(&mut self, name: &str, v: Value) {
        if let Some(d) = self.defs.get_mut(name) {
            d.value = v;
        }
    }
}

/// Go's `PrintDefaults` line for one flag.
fn describe(name: &str, d: &Def) -> String {
    let mut b = format!("  -{name}");
    let (arg, usage) = unquote_usage(d);
    if !arg.is_empty() {
        b.push(' ');
        b.push_str(&arg);
    }
    if b.len() <= 4 {
        b.push('\t');
    } else {
        b.push_str("\n    \t");
    }
    b.push_str(&usage.replace('\n', "\n    \t"));
    let zero = match d.kind {
        Kind::Str | Kind::Multi => "",
        Kind::Bool => "false",
        Kind::Float => "0",
    };
    if d.default != zero {
        if d.kind == Kind::Str {
            b.push_str(&format!(" (default {})", go_quote(&d.default)));
        } else {
            b.push_str(&format!(" (default {})", d.default));
        }
    }
    b.push('\n');
    b
}

/// Go's `UnquoteUsage`: a back-quoted name in the usage text, or the type.
fn unquote_usage(d: &Def) -> (String, String) {
    let usage = d.usage;
    if let Some(i) = usage.find('`')
        && let Some(j) = usage[i + 1..].find('`')
    {
        let name = &usage[i + 1..i + 1 + j];
        let text = format!("{}{}{}", &usage[..i], name, &usage[i + 2 + j..]);
        return (name.to_string(), text);
    }
    let name = match d.kind {
        Kind::Bool => "",
        Kind::Float => "float",
        Kind::Str => "string",
        Kind::Multi => "value",
    };
    (name.to_string(), usage.to_string())
}

/// Go's `strconv.ParseBool`.
fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "1" | "t" | "T" | "true" | "TRUE" | "True" => Some(true),
        "0" | "f" | "F" | "false" | "FALSE" | "False" => Some(false),
        _ => None,
    }
}

/// Go's `strconv.ParseFloat` as `flag` reports it ("parse error" or "value
/// out of range"). Hexadecimal floats and underscores are not supported.
fn parse_float(s: &str) -> Result<f64, &'static str> {
    let body = s.strip_prefix(['+', '-']).unwrap_or(s);
    let lower = body.to_ascii_lowercase();
    let named = matches!(lower.as_str(), "inf" | "infinity" | "nan");
    let plain = !body.is_empty()
        && body
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
    if !named && !plain {
        return Err("parse error");
    }
    match s.parse::<f64>() {
        Ok(f) if f.is_infinite() && !named => Err("value out of range"),
        Ok(f) => Ok(f),
        Err(_) => Err("parse error"),
    }
}

fn go_quote(s: &str) -> String {
    opencrs::gofmt::quote(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn check_set<'a>(out: &'a mut Vec<u8>) -> FlagSet<'a> {
        let mut fs = FlagSet::new("check", out);
        fs.multi("spec", "screens.json to check against (repeatable)");
        fs.string("files", "", "directory holding the creative files");
        fs.boolean("json", "print reports as JSON");
        fs.float("margin", 1.25, "1.25", "warn margin");
        fs
    }

    #[test]
    fn forms_and_interleaving() {
        let mut out = Vec::new();
        let mut fs = check_set(&mut out);
        fs.parse_interleaved(&s(&[
            "c.json",
            "-spec",
            "a",
            "--spec=b",
            "-json=false",
            "-files=x=y",
            "--margin",
            "2",
        ]))
        .unwrap();
        assert_eq!(fs.get_multi("spec"), ["a", "b"]);
        assert!(!fs.get_bool("json"));
        assert_eq!(fs.get_string("files"), "x=y");
        assert_eq!(fs.get_float("margin"), 2.0);
        assert_eq!(fs.rest(), ["c.json"]);
    }

    #[test]
    fn double_dash_ends_flags_then_interleaved_reparses() {
        let mut out = Vec::new();
        let mut fs = check_set(&mut out);
        fs.parse_interleaved(&s(&["--", "-json", "x"])).unwrap();
        // Go's interleaved feeds the positionals back through Parse.
        assert!(fs.get_bool("json"));
        assert_eq!(fs.rest(), ["x"]);
    }

    #[test]
    fn errors_print_go_messages_and_usage() {
        let mut out = Vec::new();
        let mut fs = check_set(&mut out);
        assert_eq!(fs.parse(&s(&["-nope"])), Err(ParseError::Failed));
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("flag provided but not defined: -nope\nUsage of check:\n"));
        assert!(text.contains("  -files string\n    \tdirectory holding the creative files\n"));
        assert!(text.contains("  -json\n    \tprint reports as JSON\n"));
        assert!(text.contains("  -margin float\n    \twarn margin (default 1.25)\n"));
        assert!(text.contains("  -spec value\n    \tscreens.json to check against (repeatable)\n"));

        let mut out = Vec::new();
        let mut fs = check_set(&mut out);
        assert!(fs.parse(&s(&["-margin", "x"])).is_err());
        assert!(
            String::from_utf8(out)
                .unwrap()
                .starts_with("invalid value \"x\" for flag -margin: parse error\n")
        );
        let mut out = Vec::new();
        let mut fs = check_set(&mut out);
        assert!(fs.parse(&s(&["-margin", "1e999"])).is_err());
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("value out of range")
        );
        let mut out = Vec::new();
        let mut fs = check_set(&mut out);
        assert!(fs.parse(&s(&["-json=maybe"])).is_err());
        assert!(
            String::from_utf8(out)
                .unwrap()
                .starts_with("invalid boolean value \"maybe\" for -json: parse error\n")
        );
        let mut out = Vec::new();
        let mut fs = check_set(&mut out);
        assert!(fs.parse(&s(&["-files"])).is_err());
        assert!(
            String::from_utf8(out)
                .unwrap()
                .starts_with("flag needs an argument: -files\n")
        );
        let mut out = Vec::new();
        let mut fs = check_set(&mut out);
        assert!(fs.parse(&s(&["---x"])).is_err());
        assert!(
            String::from_utf8(out)
                .unwrap()
                .starts_with("bad flag syntax: ---x\n")
        );
        let mut out = Vec::new();
        let mut fs = check_set(&mut out);
        assert_eq!(fs.parse(&s(&["-h"])), Err(ParseError::Help));
        assert!(
            String::from_utf8(out)
                .unwrap()
                .starts_with("Usage of check:\n")
        );
    }
}
