//! JSON output that matches Go's `encoding/json` byte for byte.

use serde::Serialize;
use serde_json::ser::{Formatter, Serializer};
use std::io;

struct GoFormatter {
    indent: bool,
    escape_html: bool,
    depth: usize,
    has_value: bool,
}

impl GoFormatter {
    fn new(indent: bool, escape_html: bool) -> Self {
        GoFormatter {
            indent,
            escape_html,
            depth: 0,
            has_value: false,
        }
    }

    fn newline<W: ?Sized + io::Write>(&self, w: &mut W) -> io::Result<()> {
        w.write_all(b"\n")?;
        for _ in 0..self.depth {
            w.write_all(b"  ")?;
        }
        Ok(())
    }

    fn open<W: ?Sized + io::Write>(&mut self, w: &mut W, b: &[u8]) -> io::Result<()> {
        self.depth += 1;
        self.has_value = false;
        w.write_all(b)
    }

    fn close<W: ?Sized + io::Write>(&mut self, w: &mut W, b: &[u8]) -> io::Result<()> {
        self.depth -= 1;
        if self.indent && self.has_value {
            self.newline(w)?;
        }
        w.write_all(b)
    }

    fn element<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if !first {
            w.write_all(b",")?;
        }
        if self.indent {
            self.newline(w)?;
        }
        Ok(())
    }
}

/// Go's float text: integers without `.0`, shortest digits, and an exponent
/// (`1e+21`, `1e-7`) below 1e-6 or from 1e21.
fn go_float(v: f64) -> String {
    let abs = v.abs();
    if abs != 0.0 && !(1e-6..1e21).contains(&abs) {
        let s = format!("{v:e}");
        let (mantissa, exp) = s.split_once('e').expect("LowerExp has an exponent");
        let (sign, digits) = match exp.strip_prefix('-') {
            Some(d) => ('-', d),
            None => ('+', exp),
        };
        format!("{mantissa}e{sign}{digits}")
    } else {
        format!("{v}")
    }
}

impl Formatter for GoFormatter {
    fn write_f64<W: ?Sized + io::Write>(&mut self, w: &mut W, v: f64) -> io::Result<()> {
        w.write_all(go_float(v).as_bytes())
    }

    fn write_f32<W: ?Sized + io::Write>(&mut self, w: &mut W, v: f32) -> io::Result<()> {
        self.write_f64(w, f64::from(v))
    }

    fn write_string_fragment<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        let mut start = 0;
        for (i, c) in fragment.char_indices() {
            let esc: &[u8] = match c {
                '\u{2028}' => b"\\u2028",
                '\u{2029}' => b"\\u2029",
                '<' if self.escape_html => b"\\u003c",
                '>' if self.escape_html => b"\\u003e",
                '&' if self.escape_html => b"\\u0026",
                _ => continue,
            };
            w.write_all(&fragment.as_bytes()[start..i])?;
            w.write_all(esc)?;
            start = i + c.len_utf8();
        }
        w.write_all(&fragment.as_bytes()[start..])
    }

    fn begin_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.open(w, b"[")
    }
    fn end_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.close(w, b"]")
    }
    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.element(w, first)
    }
    fn end_array_value<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }
    fn begin_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.open(w, b"{")
    }
    fn end_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.close(w, b"}")
    }
    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.element(w, first)
    }
    fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        w.write_all(if self.indent { b": " } else { b":" })
    }
    fn end_object_value<W: ?Sized + io::Write>(&mut self, _w: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }
}

fn render<T: Serialize + ?Sized>(v: &T, f: GoFormatter) -> String {
    let mut buf = Vec::new();
    v.serialize(&mut Serializer::with_formatter(&mut buf, f))
        .expect("model types serialise to JSON");
    String::from_utf8(buf).expect("JSON is UTF-8")
}

/// Go's `json.Marshal` as bytes, in a buffer of exactly the right size: the
/// value is written twice, the first time only to count, so a very large
/// result is never copied by a growing buffer.
pub fn to_bytes_sized<T: Serialize + ?Sized>(v: &T) -> Vec<u8> {
    struct Count(usize);
    impl io::Write for Count {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0 += b.len();
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut n = Count(0);
    v.serialize(&mut Serializer::with_formatter(
        &mut n,
        GoFormatter::new(false, true),
    ))
    .expect("model types serialise to JSON");
    let mut buf = Vec::with_capacity(n.0);
    v.serialize(&mut Serializer::with_formatter(
        &mut buf,
        GoFormatter::new(false, true),
    ))
    .expect("model types serialise to JSON");
    buf
}

/// Go's `json.Marshal`: compact, with `<`, `>` and `&` escaped.
pub fn to_string<T: Serialize + ?Sized>(v: &T) -> String {
    render(v, GoFormatter::new(false, true))
}

/// Go's encoder with `SetIndent("", "  ")` and `SetEscapeHTML(false)`,
/// including the trailing newline.
pub fn to_string_pretty<T: Serialize + ?Sized>(v: &T) -> String {
    let mut s = render(v, GoFormatter::new(true, false));
    s.push('\n');
    s
}

/// Go's `json.MarshalIndent(v, "", "  ")`: indented, with `<`, `>` and `&`
/// escaped, and no trailing newline.
pub fn to_string_pretty_escaped<T: Serialize + ?Sized>(v: &T) -> String {
    render(v, GoFormatter::new(true, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_print_like_go() {
        assert_eq!(
            to_string(&[940.0, 1670.5, 1e21, 1e-7, 0.1]),
            "[940,1670.5,1e+21,1e-7,0.1]"
        );
        assert_eq!(
            to_string(&[1e20, 1e-6, -0.0, 1.5e300]),
            "[100000000000000000000,0.000001,-0,1.5e+300]"
        );
    }

    #[test]
    fn pretty_matches_go_encoder() {
        assert_eq!(
            to_string_pretty(&serde_json::json!({"a":[1,2],"b":"<&>"})),
            "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": \"<&>\"\n}\n"
        );
        assert_eq!(
            to_string_pretty(&serde_json::json!({"a":[],"b":{}})),
            "{\n  \"a\": [],\n  \"b\": {}\n}\n"
        );
    }

    #[test]
    fn strings_escape_like_go() {
        assert_eq!(
            to_string("a<&>\u{2028}\u{2029}\u{7f}\u{1}\u{8}é"),
            "\"a\\u003c\\u0026\\u003e\\u2028\\u2029\u{7f}\\u0001\\bé\""
        );
        assert_eq!(to_string_pretty("<\u{2028}"), "\"<\\u2028\"\n");
    }

    #[test]
    fn pretty_escaped_matches_marshal_indent() {
        assert_eq!(
            to_string_pretty_escaped(&serde_json::json!({"a":[1],"b":"<a&b>"})),
            "{\n  \"a\": [\n    1\n  ],\n  \"b\": \"\\u003ca\\u0026b\\u003e\"\n}"
        );
    }
}
