//! Go formatting helpers whose output must match byte for byte.

use unicode_general_category::{GeneralCategory as G, get_general_category};

/// Go's `unicode.IsPrint`: letters, marks, numbers, punctuation, symbols
/// and the ASCII space.
fn is_print(c: char) -> bool {
    if c == ' ' {
        return true;
    }
    !matches!(
        get_general_category(c),
        G::Control
            | G::Format
            | G::Surrogate
            | G::PrivateUse
            | G::Unassigned
            | G::SpaceSeparator
            | G::LineSeparator
            | G::ParagraphSeparator
    )
}

/// Go's `strconv.Quote`.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ if is_print(c) => out.push(c),
            '\x07' => out.push_str("\\a"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x0b' => out.push_str("\\v"),
            _ if c < ' ' || c == '\x7f' => out.push_str(&format!("\\x{:02x}", c as u32)),
            _ if (c as u32) < 0x10000 => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push_str(&format!("\\U{:08x}", c as u32)),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_matches_go() {
        assert_eq!(quote("HC-L1-002"), "\"HC-L1-002\"");
        assert_eq!(quote("a\"b\\c\n\u{7f}é"), "\"a\\\"b\\\\c\\n\\x7fé\"");
        assert_eq!(
            quote("\u{2028}\u{0}\t😀\u{ad}\u{a0}"),
            "\"\\u2028\\x00\\t😀\\u00ad\\u00a0\""
        );
    }
}
