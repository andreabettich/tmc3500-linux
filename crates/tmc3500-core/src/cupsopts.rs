//! Parsing of the CUPS option string (filter argv[5]) and of length values.
use crate::error::OptError;

const MAX_OPTION_STRING: usize = 64 * 1024;
const MAX_OPTIONS: usize = 4096;

/// Split a CUPS option string into (name, value) pairs, in order.
/// Accepts `name=value`, `name='quoted'`, `name="quoted"`, backslash escapes,
/// and a bare `name` (value "true").
pub fn tokenize(s: &str) -> Result<Vec<(String, String)>, OptError> {
    if s.len() > MAX_OPTION_STRING {
        return Err(OptError::Syntax("option string too long".into()));
    }
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c.is_whitespace() {
                break;
            }
            name.push(c);
            chars.next();
        }
        if name.is_empty() {
            return Err(OptError::Syntax("option without a name".into()));
        }
        let mut value = String::from("true");
        if chars.peek() == Some(&'=') {
            chars.next();
            value.clear();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                chars.next();
                match c {
                    '\'' | '"' => loop {
                        match chars.next() {
                            Some(x) if x == c => break,
                            Some('\\') => value.push(escaped(chars.next())?),
                            Some(x) => value.push(x),
                            None => return Err(OptError::Syntax("unterminated quote".into())),
                        }
                    },
                    '\\' => value.push(escaped(chars.next())?),
                    _ => value.push(c),
                }
            }
        }
        out.push((name, value));
        if out.len() > MAX_OPTIONS {
            return Err(OptError::Syntax("too many options".into()));
        }
    }
    Ok(out)
}

fn escaped(c: Option<char>) -> Result<char, OptError> {
    c.ok_or_else(|| OptError::Syntax("dangling backslash".into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    In,
    Mm,
    Cm,
    Pt,
}

/// "-4.25" → (-425, 100). At most 9 integer and 6 fraction digits; ASCII only.
fn parse_decimal(s: &str) -> Option<(i128, i128)> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    if (int.is_empty() && frac.is_empty()) || int.len() > 9 || frac.len() > 6 {
        return None;
    }
    if !int.bytes().chain(frac.bytes()).all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut m: i128 = 0;
    for c in int.bytes().chain(frac.bytes()) {
        m = m * 10 + i128::from(c - b'0');
    }
    let scale = 10i128.pow(frac.len() as u32);
    Some((if neg { -m } else { m }, scale))
}

/// n / d rounded half away from zero (d > 0).
fn div_round(n: i128, d: i128) -> i128 {
    if n >= 0 {
        (2 * n + d) / (2 * d)
    } else {
        -((2 * -n + d) / (2 * d))
    }
}

/// Parse "4.25in", "108mm", "10.8cm", "306pt" (a bare number uses `default_unit`) into 0.1 mm.
pub fn parse_length_mm10(key: &str, s: &str, default_unit: Unit) -> Result<i64, OptError> {
    let bad = || OptError::BadValue {
        key: key.into(),
        value: s.into(),
    };
    let (num, unit) = [
        ("in", Unit::In),
        ("mm", Unit::Mm),
        ("cm", Unit::Cm),
        ("pt", Unit::Pt),
    ]
    .iter()
    .find_map(|(sfx, u)| s.strip_suffix(sfx).map(|n| (n, *u)))
    .unwrap_or((s, default_unit));
    let (m, scale) = parse_decimal(num).ok_or_else(bad)?;
    let (fnum, fden): (i128, i128) = match unit {
        Unit::In => (254, 1),
        Unit::Mm => (10, 1),
        Unit::Cm => (100, 1),
        Unit::Pt => (254, 72),
    };
    i64::try_from(div_round(m * fnum, scale * fden)).map_err(|_| bad())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn kv(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn tokenizes_cups_option_strings() {
        let t = tokenize("PageSize=Custom.4.25x6.88in  job-uuid=urn:uuid:1 Collate TmNotify='Job End' x=\"a b\" y=c\\ d").unwrap();
        assert_eq!(
            t,
            kv(&[
                ("PageSize", "Custom.4.25x6.88in"),
                ("job-uuid", "urn:uuid:1"),
                ("Collate", "true"),
                ("TmNotify", "Job End"),
                ("x", "a b"),
                ("y", "c d"),
            ])
        );
        assert_eq!(tokenize("").unwrap(), vec![]);
        assert_eq!(tokenize("   ").unwrap(), vec![]);
    }

    #[test]
    fn tokenizer_rejects_malformed_strings() {
        assert!(matches!(
            tokenize("a='unterminated"),
            Err(OptError::Syntax(_))
        ));
        assert!(matches!(tokenize("=value"), Err(OptError::Syntax(_))));
        assert!(matches!(tokenize("a=b\\"), Err(OptError::Syntax(_))));
        assert!(matches!(
            tokenize(&"a=b ".repeat(5000)),
            Err(OptError::Syntax(_))
        ));
    }

    #[test]
    fn parses_lengths_to_mm10() {
        let p = |s| parse_length_mm10("k", s, Unit::Pt).unwrap();
        assert_eq!(p("4.25in"), 1080);
        assert_eq!(p("6.88in"), 1748);
        assert_eq!(p("1.97in"), 500);
        assert_eq!(p("0.31in"), 79);
        assert_eq!(p("108mm"), 1080);
        assert_eq!(p("174.8mm"), 1748);
        assert_eq!(p("10.8cm"), 1080);
        assert_eq!(p("306pt"), 1080);
        assert_eq!(p("306"), 1080); // bare number = default unit (CUPS: points)
        assert_eq!(p("495.36"), 1748);
        assert_eq!(p("0.50in"), 127);
        assert_eq!(p("-0.25in"), -64);
        assert_eq!(p("+2in"), 508);
    }

    #[test]
    fn rejects_hostile_lengths() {
        for s in [
            "",
            "in",
            "1e9in",
            "nan",
            "--1in",
            "1.2.3in",
            "１in",
            "99999999999in",
            "1.1234567in",
            " - 1in",
            "0x10mm",
        ] {
            assert!(
                parse_length_mm10("k", s, Unit::In).is_err(),
                "{s:?} should be rejected"
            );
        }
    }

    proptest! {
        #[test]
        fn tokenize_never_panics(s in ".{0,300}") { let _ = tokenize(&s); }
        #[test]
        fn length_never_panics(s in ".{0,40}") { let _ = parse_length_mm10("k", &s, Unit::Pt); }
    }
}
