//! Output that can't move a terminal. `fetch` and `publisher` print text a
//! publisher wrote (names, URLs, reasons, error messages), and a terminal
//! obeys escape sequences in what it is shown: ESC, CSI and OSC can retitle
//! a window, rewrite what is on screen or put text in the clipboard. Control
//! characters in that text are shown as `\u00XX` instead. That is also a
//! valid escape inside a JSON string, so JSON output passes through it
//! unharmed (serde has already escaped the C0 controls there; this catches
//! DEL and the C1 controls, which JSON allows raw).

use std::io::{self, Write};

/// Whether `c` could be obeyed by a terminal; newline and tab are only
/// layout.
fn is_control(c: char) -> bool {
    c.is_control() && c != '\n' && c != '\t'
}

/// A writer that escapes the control characters of what is written to it.
/// Each `write` is whole `str` fragments, as `write!` makes them; anything
/// that isn't UTF-8 is shown as replacement characters.
pub struct Escaped<'a>(pub &'a mut dyn Write);

impl Write for Escaped<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        if !text.chars().any(is_control) {
            self.0.write_all(buf)?;
            return Ok(buf.len());
        }
        let mut out = String::with_capacity(text.len() + 8);
        for c in text.chars() {
            if is_control(c) {
                out.push_str(&format!("\\u{:04x}", c as u32));
            } else {
                out.push(c);
            }
        }
        self.0.write_all(out.as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn through(s: &str) -> String {
        let mut out = Vec::new();
        write!(Escaped(&mut out), "{s}").unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn terminal_controls_are_shown_not_sent() {
        // A title-setting OSC, a clear-screen CSI, a bell, DEL and the C1 CSI.
        assert_eq!(
            through("a\x1b]0;pwned\x07b\x1b[2Jc\x7fd\u{9b}31m"),
            "a\\u001b]0;pwned\\u0007b\\u001b[2Jc\\u007fd\\u009b31m"
        );
        assert_eq!(
            through("plain\ttext\nand ünïcode ✓"),
            "plain\ttext\nand ünïcode ✓"
        );
    }

    #[test]
    fn escaped_json_stays_json() {
        let raw = format!("{{\"reason\": \"x{}y{}\"}}", '\u{7f}', '\u{9b}');
        let v: serde_json::Value = serde_json::from_str(&through(&raw)).unwrap();
        assert_eq!(v["reason"], "x\u{7f}y\u{9b}");
    }
}
