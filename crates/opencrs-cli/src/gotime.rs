//! Go's `time.Parse(time.RFC3339, s)`, with its error messages, for the
//! `-now` flag.

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone};

const LAYOUT: &str = "2006-01-02T15:04:05Z07:00";

/// Go's `time` package quote: bytes outside printable ASCII become `\xNN`.
fn quote(s: &[u8]) -> String {
    let mut out = String::from("\"");
    for &b in s {
        if !(b' '..0x80).contains(&b) {
            out.push_str(&format!("\\x{b:02x}"));
        } else {
            if b == b'"' || b == b'\\' {
                out.push('\\');
            }
            out.push(b as char);
        }
    }
    out.push('"');
    out
}

struct Fail<'a> {
    value: &'a [u8],
    msg: Option<String>,
    layout_elem: String,
    value_elem: Vec<u8>,
}

impl Fail<'_> {
    fn text(&self) -> String {
        match &self.msg {
            None => format!(
                "parsing time {} as {}: cannot parse {} as {}",
                quote(self.value),
                quote(LAYOUT.as_bytes()),
                quote(&self.value_elem),
                quote(self.layout_elem.as_bytes())
            ),
            Some(m) => format!("parsing time {}{m}", quote(self.value)),
        }
    }
}

fn is_digit(s: &[u8], i: usize) -> bool {
    s.get(i).is_some_and(u8::is_ascii_digit)
}

/// Go's `getnum`.
fn getnum(s: &[u8], fixed: bool) -> Result<(i64, &[u8]), ()> {
    if !is_digit(s, 0) {
        return Err(());
    }
    if !is_digit(s, 1) {
        if fixed {
            return Err(());
        }
        return Ok(((s[0] - b'0') as i64, &s[1..]));
    }
    Ok((((s[0] - b'0') as i64) * 10 + (s[1] - b'0') as i64, &s[2..]))
}

/// Parses an RFC 3339 time as Go does, keeping its offset.
pub fn parse_rfc3339(input: &str) -> Result<DateTime<FixedOffset>, String> {
    let all = input.as_bytes();
    let mut value = all;
    let mut year = 0i64;
    let (mut month, mut day, mut hour, mut min, mut sec, mut nsec) =
        (-1i64, -1i64, 0i64, 0i64, 0i64, 0i64);
    let mut offset_s: Option<i64> = None;

    // (literal prefix, layout element)
    let chunks = [
        ("", "2006"),
        ("-", "01"),
        ("-", "02"),
        ("T", "15"),
        (":", "04"),
        (":", "05"),
        ("", "Z07:00"),
    ];
    for (prefix, elem) in chunks {
        let fail = |hold: &[u8], msg: Option<String>| Fail {
            value: all,
            msg,
            layout_elem: elem.to_string(),
            value_elem: hold.to_vec(),
        };
        // The literal before the element.
        let mut p = prefix.as_bytes();
        let mut v = value;
        while let Some((&pc, prest)) = p.split_first() {
            if v.first() != Some(&pc) {
                // newParseError(alayout, avalue, prefix, value, "")
                return Err(Fail {
                    value: all,
                    msg: None,
                    layout_elem: prefix.to_string(),
                    value_elem: v.to_vec(),
                }
                .text());
            }
            p = prest;
            v = &v[1..];
        }
        value = v;
        let hold = value;
        let mut range = "";
        let mut bad = false;
        match elem {
            "2006" => {
                if value.len() < 4
                    || !is_digit(value, 0)
                    || !value[..4].iter().all(u8::is_ascii_digit)
                {
                    bad = true;
                } else {
                    year = std::str::from_utf8(&value[..4]).unwrap().parse().unwrap();
                    value = &value[4..];
                }
            }
            "01" => match getnum(value, true) {
                Ok((n, r)) => {
                    month = n;
                    value = r;
                    if n <= 0 || 12 < n {
                        range = "month";
                    }
                }
                Err(()) => bad = true,
            },
            "02" => match getnum(value, true) {
                Ok((n, r)) => {
                    day = n;
                    value = r;
                }
                Err(()) => bad = true,
            },
            "15" => match getnum(value, false) {
                Ok((n, r)) => {
                    hour = n;
                    value = r;
                    if 24 <= n {
                        range = "hour";
                    }
                }
                Err(()) => bad = true,
            },
            "04" => match getnum(value, true) {
                Ok((n, r)) => {
                    min = n;
                    value = r;
                    if 60 <= n {
                        range = "minute";
                    }
                }
                Err(()) => bad = true,
            },
            "05" => match getnum(value, true) {
                Ok((n, r)) => {
                    sec = n;
                    value = r;
                    if 60 <= n {
                        range = "second";
                    } else if value.len() >= 2
                        && matches!(value[0], b'.' | b',')
                        && is_digit(value, 1)
                    {
                        let mut n = 2;
                        while n < value.len() && is_digit(value, n) {
                            n += 1;
                        }
                        // Go keeps at most nine fractional digits.
                        let digits = &value[1..n.min(10)];
                        let mut ns: i64 = std::str::from_utf8(digits).unwrap().parse().unwrap();
                        for _ in digits.len()..9 {
                            ns *= 10;
                        }
                        nsec = ns;
                        value = &value[n..];
                    }
                }
                Err(()) => bad = true,
            },
            _ => {
                // Z07:00
                if value.first() == Some(&b'Z') {
                    value = &value[1..];
                    offset_s = Some(0);
                } else if value.len() < 6 || value[3] != b':' {
                    bad = true;
                } else {
                    let (sign, h, m) = (value[0], &value[1..3], &value[4..6]);
                    value = &value[6..];
                    let hr = getnum(h, true).map(|x| x.0);
                    let mm = hr.and_then(|_| getnum(m, true).map(|x| x.0));
                    match (hr, mm) {
                        (Ok(hr), Ok(mm)) => {
                            if hr > 24 {
                                range = "time zone offset hour";
                            }
                            if mm > 60 {
                                range = "time zone offset minute";
                            }
                            let off = (hr * 60 + mm) * 60;
                            match sign {
                                b'+' => offset_s = Some(off),
                                b'-' => offset_s = Some(-off),
                                _ => bad = true,
                            }
                        }
                        _ => bad = true,
                    }
                }
            }
        }
        if !range.is_empty() {
            return Err(fail(hold, Some(format!(": {range} out of range"))).text());
        }
        if bad {
            return Err(fail(hold, None).text());
        }
    }
    if !value.is_empty() {
        return Err(Fail {
            value: all,
            msg: Some(format!(": extra text: {}", quote(value))),
            layout_elem: String::new(),
            value_elem: Vec::new(),
        }
        .text());
    }
    let day_ok = NaiveDate::from_ymd_opt(year as i32, month as u32, 1).is_some_and(|first| {
        let next = if month == 12 {
            NaiveDate::from_ymd_opt(year as i32 + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(year as i32, month as u32 + 1, 1)
        };
        next.is_some_and(|n| day >= 1 && day <= (n - first).num_days())
    });
    if !day_ok {
        return Err(format!("parsing time {}: day out of range", quote(all)));
    }
    let out_of_range = || {
        format!(
            "time {} is outside the range this build supports",
            quote(all)
        )
    };
    let off = FixedOffset::east_opt(offset_s.unwrap_or(0) as i32).ok_or_else(out_of_range)?;
    off.with_ymd_and_hms(
        year as i32,
        month as u32,
        day as u32,
        hour as u32,
        min as u32,
        sec as u32,
    )
    .single()
    .and_then(|t| t.with_nanosecond_checked(nsec as u32))
    .ok_or_else(out_of_range)
}

trait WithNanos: Sized {
    fn with_nanosecond_checked(self, ns: u32) -> Option<Self>;
}

impl WithNanos for DateTime<FixedOffset> {
    fn with_nanosecond_checked(self, ns: u32) -> Option<Self> {
        chrono::Timelike::with_nanosecond(&self, ns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_offset() {
        let t = parse_rfc3339("2026-10-22T12:00:00+11:00").unwrap();
        assert_eq!(t.offset().local_minus_utc(), 11 * 3600);
        assert_eq!(t.to_rfc3339(), "2026-10-22T12:00:00+11:00");
        let z = parse_rfc3339("2026-10-22T12:00:00.5Z").unwrap();
        assert_eq!(chrono::Timelike::nanosecond(&z), 500_000_000);
    }

    #[test]
    fn errors_read_like_gos() {
        let l = "2006-01-02T15:04:05Z07:00";
        assert_eq!(
            parse_rfc3339("nope").unwrap_err(),
            format!("parsing time \"nope\" as \"{l}\": cannot parse \"nope\" as \"2006\"")
        );
        assert_eq!(
            parse_rfc3339("2026-13-01T00:00:00Z").unwrap_err(),
            "parsing time \"2026-13-01T00:00:00Z\": month out of range"
        );
        assert_eq!(
            parse_rfc3339("2026-02-30T00:00:00Z").unwrap_err(),
            "parsing time \"2026-02-30T00:00:00Z\": day out of range"
        );
        assert_eq!(
            parse_rfc3339("2026-10-22T12:00:00Zx").unwrap_err(),
            "parsing time \"2026-10-22T12:00:00Zx\": extra text: \"x\""
        );
        assert_eq!(
            parse_rfc3339("2026-10-22 12:00:00Z").unwrap_err(),
            format!(
                "parsing time \"2026-10-22 12:00:00Z\" as \"{l}\": cannot parse \" 12:00:00Z\" as \"T\""
            )
        );
    }
}
