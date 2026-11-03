# Phase 1 (protocol core) implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `tmc3500-core` Rust library. It turns CUPS options into the exact TM-C3500 printer
command stream (everything except dithered raster content). Proof: byte-exact golden tests against the
Windows captures.

**Architecture:** A pure Rust library with no I/O and no runtime dependencies:
- `cupsopts`: tokenizer and length parser;
- `options`: `JobConfig` with the Windows dialog's rules;
- `geometry`: page geometry in printer units;
- `rle`: Epson run-length compression;
- `decode`: a strict ESC/P2 + REMOTE1 parser, used by the tests and later by the fake printer;
- `protocol`: command writer, `SN` media block, job encoder.

Golden tests rebuild every capture from its options. They compare everything except the raster bands:
- the command skeleton exactly;
- the RLE output exactly, for every raster block.

**Tech stack:** Rust stable (edition 2021) and Cargo. Dev-only crates: `proptest` 1, `serde` 1, `toml` 0.8.

**Spec:** `docs/superpowers/specs/2026-11-03-tmc3500-linux-driver-design.md` (sections 2, 3, 6, 7).

## Global constraints

- Workspace root: `tmc3500-linux/`. Core crate: `crates/tmc3500-core`.
- `tmc3500-core` has **no runtime dependencies** (std only) and starts with `#![forbid(unsafe_code)]`.
- No panics on any external input (options, printer streams, RLE data). Bad input → typed error.
- Units:
  - user lengths are converted to **0.1 mm** first, rounding half away from zero;
  - 0.1 mm → 1/360 in rounds half up: `(mm10*360+127)/254`;
  - `SN` sizes round up: `ceil(dots*254/360)`.
- Targets: `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`.
- Test data comes from `testdata/captures/`, copied from `../tmc3500_captures/` in Task 1. Never modify
  these files.
- Every commit message ends with:
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`
- Run `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings` before each commit.

## Review focus

1. **Real CUPS option strings.** Strings with quoted values, standard CUPS keys the core doesn't use
   (`job-uuid`, `orientation-requested`), repeated keys (the last one wins) and the `noTmX` boolean form
   must parse without errors. Tested in Task 2 and Task 3.
2. **Custom sizes in points.** CUPS sizes come in points by default and can be fractional
   (`Custom.306x495.36`). They must give the same 0.1 mm values as the inch form. Tested in Task 2.
3. **Hostile numbers.** Huge digit strings, exponents (`1e9`), empty values, `nan`, `--1`: all must give
   an `OptError`, never a panic or overflow. Tested in Task 2 (proptest).
4. **Truncated or garbage printer streams and RLE data.** The decoder and RLE decoder must return errors
   and never panic or loop forever. Tested in Task 5 and Task 6 (proptest).
5. **Copies 0 or 10000, zero pages, every page skipped as blank.** Copies outside 1..=9999 → error.
   Zero printable pages → still a valid job: header, set header, trailer, as in capture 504. Tested in
   Task 3 and Task 7.

---

### Task 1: Toolchain, workspace, test data, `units`

**Files:**
- Create: `rust-toolchain.toml`, `Cargo.toml`, `crates/tmc3500-core/Cargo.toml`,
  `crates/tmc3500-core/src/lib.rs`, `crates/tmc3500-core/src/units.rs`
- Create: `testdata/captures/` (copied files)
- Modify: `.gitignore`

**Interfaces:**
- Produces:
  - `tmc3500_core::units::mm10_to_dots(mm10: u32) -> u32`
  - `tmc3500_core::units::dots_to_mm10_ceil(dots: u32) -> u32`

- [ ] **Step 1: Install the Rust toolchain**

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --component clippy,rustfmt
. "$HOME/.cargo/env"
rustc --version
```
Expected: `rustc 1.xx.y (...)`. Note the version for Step 2.

- [ ] **Step 2: Create the workspace files**

`rust-toolchain.toml` (replace `1.xx.y` with the version printed in Step 1):
```toml
[toolchain]
channel = "1.xx.y"
components = ["clippy", "rustfmt"]
```

`Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/tmc3500-core"]

[workspace.package]
edition = "2021"
license = "MIT OR Apache-2.0"
version = "0.1.0"

[profile.release]
panic = "abort"
lto = true
```

`crates/tmc3500-core/Cargo.toml`:
```toml
[package]
name = "tmc3500-core"
edition.workspace = true
license.workspace = true
version.workspace = true

[dependencies]

[dev-dependencies]
proptest = "1"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
```

`crates/tmc3500-core/src/lib.rs`:
```rust
//! Pure (no I/O) core of the Epson TM-C3500 Linux driver.
#![forbid(unsafe_code)]

pub mod units;
```

`.gitignore`:
```
target/
```

- [ ] **Step 3: Copy the test data**

```bash
mkdir -p testdata/captures
cp ../tmc3500_captures/*.prn ../tmc3500_captures/*.drvlog.txt ../tmc3500_captures/manifest.csv ../tmc3500_captures/NOTES.md testdata/captures/
ls testdata/captures/*.prn | wc -l
```
Expected: `86`

- [ ] **Step 4: Write the failing test** in `crates/tmc3500-core/src/units.rs`

```rust
//! Length units. Lengths are kept in 0.1 mm (`mm10`, the Windows driver's internal unit) and
//! converted to dots (1/360 inch, the printer's command unit).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mm10_to_dots_matches_windows_driver() {
        // (mm10, dots) observed in the captures' driver log and ESC(S/ESC(C values
        for (mm10, dots) in [
            (1080, 1531), (1748, 2477), (508, 720), (500, 709), (300, 425),
            (1021, 1447), (1519, 2153), (254, 360), (80, 113), (3048, 4320), (762, 1080),
        ] {
            assert_eq!(mm10_to_dots(mm10), dots, "mm10={mm10}");
        }
    }

    #[test]
    fn dots_to_mm10_ceil_matches_sn_block() {
        for (dots, mm10) in [
            (1587, 1120), (1531, 1081), (776, 548), (765, 540), (1503, 1061), (416, 294),
            (2477, 1748), (1080, 762), (425, 300), (2153, 1520), (113, 80), (4320, 3048),
        ] {
            assert_eq!(dots_to_mm10_ceil(dots), mm10, "dots={dots}");
        }
    }

    #[test]
    fn conversions_saturate_instead_of_overflowing() {
        assert_eq!(mm10_to_dots(u32::MAX), u32::MAX);
        assert_eq!(dots_to_mm10_ceil(u32::MAX), 3_030_338_036);
    }
}
```

- [ ] **Step 5: Run the test to see it fail**

Run: `cargo test -p tmc3500-core units`
Expected: compile error `cannot find function mm10_to_dots`.

- [ ] **Step 6: Implement** (add above the test module in `units.rs`)

```rust
/// 0.1 mm → 1/360 inch, rounding half up (as the Windows driver does).
pub fn mm10_to_dots(mm10: u32) -> u32 {
    let v = (u64::from(mm10) * 360 + 127) / 254;
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// 1/360 inch → 0.1 mm, rounding up (used for the REMOTE1 `SN` media block).
pub fn dots_to_mm10_ceil(dots: u32) -> u32 {
    let v = (u64::from(dots) * 254 + 359) / 360;
    u32::try_from(v).unwrap_or(u32::MAX)
}
```

- [ ] **Step 7: Run the tests to see them pass**

Run: `cargo test -p tmc3500-core units`
Expected: `3 passed`.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
git add -A
git commit -m "core: workspace, test captures, unit conversions

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: CUPS option tokenizer and length parser (`cupsopts`)

**Files:**
- Create: `crates/tmc3500-core/src/error.rs`, `crates/tmc3500-core/src/cupsopts.rs`
- Modify: `crates/tmc3500-core/src/lib.rs`

**Interfaces:**
- Produces:
  - `tmc3500_core::error::OptError` (enum, `Display` + `std::error::Error`)
  - `tmc3500_core::cupsopts::tokenize(s: &str) -> Result<Vec<(String, String)>, OptError>`
  - `tmc3500_core::cupsopts::Unit` { `In`, `Mm`, `Cm`, `Pt` }
  - `tmc3500_core::cupsopts::parse_length_mm10(key: &str, s: &str, default_unit: Unit) -> Result<i64, OptError>`

- [ ] **Step 1: Create `error.rs`**

```rust
//! Errors for option parsing.
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptError {
    /// The option string itself is malformed (quotes, length).
    Syntax(String),
    /// A `Tm*` option this driver does not know.
    UnknownOption(String),
    /// A value that cannot be parsed for this option.
    BadValue { key: String, value: String },
    /// A parsable value outside the allowed range.
    OutOfRange { key: String, value: String, allowed: String },
    /// Two options that cannot be combined (e.g. Plain coating with Quality mode).
    Conflict(String),
    /// A valid printer setting whose encoding has not been captured yet.
    NotYetSupported(String),
}

impl fmt::Display for OptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(s) => write!(f, "malformed option string: {s}"),
            Self::UnknownOption(k) => write!(f, "unknown option {k}"),
            Self::BadValue { key, value } => write!(f, "invalid value {value:?} for {key}"),
            Self::OutOfRange { key, value, allowed } => {
                write!(f, "value {value:?} for {key} out of range (allowed: {allowed})")
            }
            Self::Conflict(s) => write!(f, "conflicting options: {s}"),
            Self::NotYetSupported(s) => write!(f, "not supported yet: {s}"),
        }
    }
}

impl std::error::Error for OptError {}
```

- [ ] **Step 2: Write the failing tests** in `crates/tmc3500-core/src/cupsopts.rs`

```rust
//! Parsing of the CUPS option string (filter argv[5]) and of length values.
use crate::error::OptError;

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn kv(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn tokenizes_cups_option_strings() {
        let t = tokenize("PageSize=Custom.4.25x6.88in  job-uuid=urn:uuid:1 Collate TmNotify='Job End' x=\"a b\" y=c\\ d").unwrap();
        assert_eq!(t, kv(&[
            ("PageSize", "Custom.4.25x6.88in"), ("job-uuid", "urn:uuid:1"), ("Collate", "true"),
            ("TmNotify", "Job End"), ("x", "a b"), ("y", "c d"),
        ]));
        assert_eq!(tokenize("").unwrap(), vec![]);
        assert_eq!(tokenize("   ").unwrap(), vec![]);
    }

    #[test]
    fn tokenizer_rejects_malformed_strings() {
        assert!(matches!(tokenize("a='unterminated"), Err(OptError::Syntax(_))));
        assert!(matches!(tokenize("=value"), Err(OptError::Syntax(_))));
        assert!(matches!(tokenize("a=b\\"), Err(OptError::Syntax(_))));
        assert!(matches!(tokenize(&"a=b ".repeat(5000)), Err(OptError::Syntax(_))));
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
        for s in ["", "in", "1e9in", "nan", "--1in", "1.2.3in", "１in", "99999999999in", "1.1234567in", " - 1in", "0x10mm"] {
            assert!(parse_length_mm10("k", s, Unit::In).is_err(), "{s:?} should be rejected");
        }
    }

    proptest! {
        #[test]
        fn tokenize_never_panics(s in ".{0,300}") { let _ = tokenize(&s); }
        #[test]
        fn length_never_panics(s in ".{0,40}") { let _ = parse_length_mm10("k", &s, Unit::Pt); }
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p tmc3500-core cupsopts`
Expected: compile error `cannot find function tokenize`.

- [ ] **Step 4: Implement** (above the test module in `cupsopts.rs`)

```rust
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
    let bad = || OptError::BadValue { key: key.into(), value: s.into() };
    let (num, unit) = [("in", Unit::In), ("mm", Unit::Mm), ("cm", Unit::Cm), ("pt", Unit::Pt)]
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
```

Add to `lib.rs`:
```rust
pub mod cupsopts;
pub mod error;
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test -p tmc3500-core cupsopts`
Expected: `6 passed`.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
git add -A
git commit -m "core: CUPS option tokenizer and length parser

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `JobConfig` with the Windows dialog's rules (`options`)

**Files:**
- Create: `crates/tmc3500-core/src/options.rs`
- Modify: `crates/tmc3500-core/src/lib.rs`

**Interfaces:**
- Consumes: `cupsopts::{tokenize, parse_length_mm10, Unit}`, `error::OptError` (Task 2).
- Produces (all `pub`, in `tmc3500_core::options`):
  - `enum MediaSize { Label108x1748, Custom { width: u32, length: u32 } }` (0.1 mm),
    `fn mm10(self) -> (u32, u32)`
  - `enum MediaForm { DieCutGap, DieCutBlackMark, ContinuousLabel, ContinuousPaper, ContinuousPaperBlackMark }`
    with `fn is_label(self) -> bool`, `fn is_die_cut(self) -> bool`, `fn is_continuous(self) -> bool`,
    `fn ex24_code(self) -> u8`
  - `enum MediaSaving { None, Lower, LowerUpper }`
  - `enum Coating { Plain, Synthetic, Matte, Wristband, Glossy }` with `fn sn_code(self) -> u8`
  - `enum Quality { Speed, Quality1, Quality2 }`
  - `enum AfterPrint { CutEveryPage, CutLastPage, CutEveryN(u8), PeelOff, CutPosition, PrintEnd }` with `fn is_cut(self) -> bool`
  - `enum Notify { None, PageEnd, JobEnd }`
  - `enum Vacuum { Auto, Manual(u8) }` with `fn sn_byte(self) -> u8`
  - `enum BlackMode { Rich, KOnly }`
  - `enum ColorMode { Preferred, Standard, Off }`, `enum Gamma { G18, G22 }`
  - `struct ColorSettings { mode: ColorMode, gamma: Gamma, ink_adjust: bool, ink_bkcmy: i8, ink_bk: i8, brightness: i8 }`
  - `struct JobConfig { media, form, saving, coating, quality, banding_reduction: bool, after_print, notify, borderless: bool, bidirectional: bool, vacuum, rotate180: bool, pos_adj_h: i32, pos_adj_v: i32, skip_blank: bool, black, color, copies: u16, collate: bool }`
    with `impl Default` (Windows defaults) and
    `fn from_cups(options: &str, copies: u32) -> Result<JobConfig, OptError>`

CUPS option names and values:

| key | values | default |
|---|---|---|
| `PageSize` | `Label108x174.8mm` / `Custom.<w>x<h>[in\|mm\|cm\|pt]` (no unit = pt) | `Label108x174.8mm` |
| `TmMediaForm` | `DieCutGap` `DieCutBlackMark` `ContinuousLabel` `ContinuousPaper` `ContinuousPaperBlackMark` | `DieCutGap` |
| `TmMediaSaving` | `None` `Lower` `LowerUpper` | `None` |
| `TmCoating` | `Plain` `Synthetic` `Matte` `Wristband` `Glossy` | `Matte` |
| `TmQuality` | `Speed` `Quality1` `Quality2` | `Speed` |
| `TmBandingReduction` | bool | `True` |
| `TmAfterPrint` | `CutEveryPage` `CutLastPage` `CutEveryN` `PeelOff` `CutPosition` `PrintEnd` | `CutEveryPage` |
| `TmCutInterval` | 1–255 (used with `CutEveryN`) | 1 |
| `TmNotify` | `None` `PageEnd` `JobEnd` | `None` |
| `TmBorderless` / `TmBidirectional` / `TmRotate180` / `TmSkipBlank` / `TmInkAdjust` | bool | True / True / False / False / False |
| `TmVacuum` | `Auto`, 0–9 | `Auto` |
| `TmPosAdjH` / `TmPosAdjV` | length (no unit = in), ±4.09 in / ±44 in | 0 |
| `TmBlack` | `Rich` `KOnly` | `Rich` |
| `TmColorMode` | `Preferred` `Standard` `Off` | `Preferred` |
| `TmGamma` | `1.8` `2.2` | `2.2` |
| `TmInkBkCMY` / `TmInkBk` | −6–0 | 0 |
| `TmBrightness` | −2–2 | 0 |
| `Collate` | bool | False |

Booleans accept `true/yes/on/1` and `false/no/off/0` (any case). `noTmX` means `TmX=false`. Unknown keys
starting with `Tm` → `UnknownOption`. Other unknown keys are ignored, because CUPS passes many standard
options. A later duplicate key overrides an earlier one.

Rules (from `SETTINGS_INVENTORY.md` and capture 334):
- **Custom width:** label forms 254–1080 (1.00–4.25 in); paper forms 300–1080 (1.18–4.25 in).
- **Custom length:** 79–11176. 79 (0.31 in) is clamped to 80, the driver's 8.0 mm minimum.
- **Coating → quality:** Plain allows Speed only; Wristband allows Speed or Quality1; all others allow all
  three. Anything else → `Conflict`.
- **Banding reduction:** forced off for Plain and for Quality1, forced on for Quality2.
- **Short labels:** a cut mode with a length below 150 (15 mm) is forced to `PeelOff`, as Windows does.
- **Media saving:** forced to `None` unless the form is `ContinuousLabel` or `ContinuousPaper`.
- **Vacuum:** only 0, 4 and 9 have captured encodings (30/65/90). Other values →
  `NotYetSupported("TmVacuum=<n> (capture pending)")`.
- **Copies:** 1–9999, otherwise `OutOfRange`.

- [ ] **Step 1: Write the failing tests** in `options.rs`

```rust
//! Job settings and the rules of the Windows driver dialog.
use crate::cupsopts::{parse_length_mm10, tokenize, Unit};
use crate::error::OptError;

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(o: &str) -> JobConfig {
        JobConfig::from_cups(o, 1).unwrap()
    }

    #[test]
    fn defaults_match_windows_driver() {
        let c = cfg("");
        assert_eq!(c, JobConfig::default());
        assert_eq!(c.media, MediaSize::Label108x1748);
        assert_eq!(c.form, MediaForm::DieCutGap);
        assert_eq!(c.coating, Coating::Matte);
        assert_eq!(c.quality, Quality::Speed);
        assert!(c.banding_reduction && c.borderless && c.bidirectional);
        assert_eq!(c.after_print, AfterPrint::CutEveryPage);
        assert_eq!(c.vacuum, Vacuum::Auto);
        assert_eq!((c.copies, c.collate), (1, false));
    }

    #[test]
    fn parses_custom_page_sizes_in_any_unit() {
        let want = MediaSize::Custom { width: 1080, length: 1748 };
        assert_eq!(cfg("PageSize=Custom.4.25x6.88in").media, want);
        assert_eq!(cfg("PageSize=Custom.306x495.36").media, want);
        assert_eq!(cfg("PageSize=Custom.108x174.8mm").media, want);
        assert_eq!(cfg("PageSize=Custom.1.00x0.31in").media, MediaSize::Custom { width: 254, length: 80 });
    }

    #[test]
    fn ignores_standard_cups_options_and_last_key_wins() {
        let c = cfg("job-uuid=urn:x orientation-requested=4 fit-to-page=true TmCoating=Plain TmCoating=Glossy noTmBidirectional");
        assert_eq!(c.coating, Coating::Glossy);
        assert!(!c.bidirectional);
    }

    #[test]
    fn rejects_unknown_tm_options_and_bad_values() {
        assert!(matches!(JobConfig::from_cups("TmFoo=1", 1), Err(OptError::UnknownOption(_))));
        assert!(matches!(JobConfig::from_cups("TmCoating=Shiny", 1), Err(OptError::BadValue { .. })));
        assert!(matches!(JobConfig::from_cups("TmCutInterval=256", 1), Err(OptError::OutOfRange { .. })));
        assert!(matches!(JobConfig::from_cups("PageSize=Custom.5x6in", 1), Err(OptError::OutOfRange { .. })));
        assert!(matches!(JobConfig::from_cups("PageSize=Custom.4x0.2in", 1), Err(OptError::OutOfRange { .. })));
        assert!(matches!(
            JobConfig::from_cups("PageSize=Custom.1.1x2in TmMediaForm=ContinuousPaper", 1),
            Err(OptError::OutOfRange { .. })
        ));
        assert!(matches!(JobConfig::from_cups("PageSize=Letter", 1), Err(OptError::BadValue { .. })));
    }

    #[test]
    fn applies_dialog_rules() {
        assert!(matches!(JobConfig::from_cups("TmCoating=Plain TmQuality=Quality1", 1), Err(OptError::Conflict(_))));
        assert!(matches!(JobConfig::from_cups("TmCoating=Wristband TmQuality=Quality2", 1), Err(OptError::Conflict(_))));
        assert!(!cfg("TmCoating=Plain").banding_reduction);
        assert!(!cfg("TmQuality=Quality1").banding_reduction);
        assert!(cfg("TmQuality=Quality2 TmBandingReduction=False").banding_reduction);
        assert!(!cfg("TmBandingReduction=False").banding_reduction);
        assert_eq!(cfg("PageSize=Custom.1x0.5in").after_print, AfterPrint::PeelOff);
        assert_eq!(cfg("PageSize=Custom.1x0.5in TmAfterPrint=PrintEnd").after_print, AfterPrint::PrintEnd);
        assert_eq!(cfg("TmMediaSaving=Lower").saving, MediaSaving::None);
        assert_eq!(cfg("TmMediaForm=ContinuousLabel TmMediaSaving=Lower").saving, MediaSaving::Lower);
        assert_eq!(cfg("TmAfterPrint=CutEveryN TmCutInterval=5").after_print, AfterPrint::CutEveryN(5));
    }

    #[test]
    fn vacuum_only_accepts_captured_values() {
        assert_eq!(cfg("TmVacuum=9").vacuum.sn_byte(), 0x5a);
        assert_eq!(cfg("TmVacuum=4").vacuum.sn_byte(), 0x41);
        assert_eq!(cfg("TmVacuum=0").vacuum.sn_byte(), 0x1e);
        assert_eq!(cfg("TmVacuum=Auto").vacuum.sn_byte(), 0xff);
        assert!(matches!(JobConfig::from_cups("TmVacuum=5", 1), Err(OptError::NotYetSupported(_))));
        assert!(matches!(JobConfig::from_cups("TmVacuum=10", 1), Err(OptError::OutOfRange { .. })));
    }

    #[test]
    fn copies_range_is_enforced() {
        assert!(matches!(JobConfig::from_cups("", 0), Err(OptError::OutOfRange { .. })));
        assert!(matches!(JobConfig::from_cups("", 10_000), Err(OptError::OutOfRange { .. })));
        assert_eq!(JobConfig::from_cups("Collate=True", 3).unwrap().copies, 3);
    }

    #[test]
    fn parses_position_and_color_options() {
        let c = cfg("TmPosAdjV=0.50in TmPosAdjH=-0.25 TmColorMode=Off TmGamma=1.8 TmInkAdjust=True TmInkBkCMY=-3 TmInkBk=-6 TmBrightness=2 TmBlack=KOnly");
        assert_eq!((c.pos_adj_v, c.pos_adj_h), (127, -64));
        assert_eq!(c.color, ColorSettings { mode: ColorMode::Off, gamma: Gamma::G18, ink_adjust: true, ink_bkcmy: -3, ink_bk: -6, brightness: 2 });
        assert_eq!(c.black, BlackMode::KOnly);
        assert!(matches!(JobConfig::from_cups("TmPosAdjH=4.10in", 1), Err(OptError::OutOfRange { .. })));
        assert!(matches!(JobConfig::from_cups("TmInkBk=1", 1), Err(OptError::OutOfRange { .. })));
    }

    #[test]
    fn media_form_codes() {
        use MediaForm::*;
        assert_eq!([DieCutGap, DieCutBlackMark, ContinuousLabel, ContinuousPaper, ContinuousPaperBlackMark].map(MediaForm::ex24_code), [2, 3, 0, 0, 1]);
        assert_eq!([Coating::Plain, Coating::Matte, Coating::Synthetic, Coating::Wristband, Coating::Glossy].map(Coating::sn_code), [1, 3, 4, 5, 6]);
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p tmc3500-core options`
Expected: compile errors (types not defined).

- [ ] **Step 3: Implement** (above the test module in `options.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSize {
    /// The driver's predefined "108 x 174.8 mm - Die-cut Label".
    Label108x1748,
    /// Width and length in 0.1 mm.
    Custom { width: u32, length: u32 },
}

impl MediaSize {
    pub fn mm10(self) -> (u32, u32) {
        match self {
            Self::Label108x1748 => (1080, 1748),
            Self::Custom { width, length } => (width, length),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaForm { DieCutGap, DieCutBlackMark, ContinuousLabel, ContinuousPaper, ContinuousPaperBlackMark }

impl MediaForm {
    /// Labels on a liner (edges added to the width).
    pub fn is_label(self) -> bool { matches!(self, Self::DieCutGap | Self::DieCutBlackMark | Self::ContinuousLabel) }
    pub fn is_die_cut(self) -> bool { matches!(self, Self::DieCutGap | Self::DieCutBlackMark) }
    pub fn is_continuous(self) -> bool { !self.is_die_cut() }
    /// Value of the REMOTE1 `EX .. 24 nn` media-form command.
    pub fn ex24_code(self) -> u8 {
        match self {
            Self::DieCutGap => 2,
            Self::DieCutBlackMark => 3,
            Self::ContinuousLabel | Self::ContinuousPaper => 0,
            Self::ContinuousPaperBlackMark => 1,
        }
    }
    fn supports_saving(self) -> bool { matches!(self, Self::ContinuousLabel | Self::ContinuousPaper) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSaving { None, Lower, LowerUpper }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coating { Plain, Synthetic, Matte, Wristband, Glossy }

impl Coating {
    /// Byte 8 of the `SN` media block.
    pub fn sn_code(self) -> u8 {
        match self { Self::Plain => 1, Self::Matte => 3, Self::Synthetic => 4, Self::Wristband => 5, Self::Glossy => 6 }
    }
    fn allows(self, q: Quality) -> bool {
        match self {
            Self::Plain => q == Quality::Speed,
            Self::Wristband => q != Quality::Quality2,
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality { Speed, Quality1, Quality2 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterPrint { CutEveryPage, CutLastPage, CutEveryN(u8), PeelOff, CutPosition, PrintEnd }

impl AfterPrint {
    pub fn is_cut(self) -> bool { matches!(self, Self::CutEveryPage | Self::CutLastPage | Self::CutEveryN(_)) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notify { None, PageEnd, JobEnd }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vacuum { Auto, Manual(u8) }

impl Vacuum {
    /// Byte 18 of the `SN` media block. Only captured slider positions are constructed.
    pub fn sn_byte(self) -> u8 {
        match self {
            Self::Auto => 0xff,
            Self::Manual(0) => 30,
            Self::Manual(4) => 65,
            Self::Manual(_) => 90,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlackMode { Rich, KOnly }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode { Preferred, Standard, Off }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gamma { G18, G22 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorSettings {
    pub mode: ColorMode,
    pub gamma: Gamma,
    pub ink_adjust: bool,
    pub ink_bkcmy: i8,
    pub ink_bk: i8,
    pub brightness: i8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobConfig {
    pub media: MediaSize,
    pub form: MediaForm,
    pub saving: MediaSaving,
    pub coating: Coating,
    pub quality: Quality,
    pub banding_reduction: bool,
    pub after_print: AfterPrint,
    pub notify: Notify,
    pub borderless: bool,
    pub bidirectional: bool,
    pub vacuum: Vacuum,
    pub rotate180: bool,
    /// Print position adjustment in 0.1 mm (signed).
    pub pos_adj_h: i32,
    pub pos_adj_v: i32,
    pub skip_blank: bool,
    pub black: BlackMode,
    pub color: ColorSettings,
    pub copies: u16,
    pub collate: bool,
}

impl Default for JobConfig {
    fn default() -> Self {
        Self {
            media: MediaSize::Label108x1748,
            form: MediaForm::DieCutGap,
            saving: MediaSaving::None,
            coating: Coating::Matte,
            quality: Quality::Speed,
            banding_reduction: true,
            after_print: AfterPrint::CutEveryPage,
            notify: Notify::None,
            borderless: true,
            bidirectional: true,
            vacuum: Vacuum::Auto,
            rotate180: false,
            pos_adj_h: 0,
            pos_adj_v: 0,
            skip_blank: false,
            black: BlackMode::Rich,
            color: ColorSettings { mode: ColorMode::Preferred, gamma: Gamma::G22, ink_adjust: false, ink_bkcmy: 0, ink_bk: 0, brightness: 0 },
            copies: 1,
            collate: false,
        }
    }
}

const BOOL_KEYS: &[&str] = &["TmBandingReduction", "TmBorderless", "TmBidirectional", "TmRotate180", "TmSkipBlank", "TmInkAdjust", "Collate"];

fn bad(key: &str, value: &str) -> OptError { OptError::BadValue { key: key.into(), value: value.into() } }

fn range(key: &str, value: &str, allowed: &str) -> OptError {
    OptError::OutOfRange { key: key.into(), value: value.into(), allowed: allowed.into() }
}

fn parse_bool(key: &str, v: &str) -> Result<bool, OptError> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err(bad(key, v)),
    }
}

fn parse_int(key: &str, v: &str, lo: i64, hi: i64) -> Result<i64, OptError> {
    if v.is_empty() || v.len() > 12 {
        return Err(bad(key, v));
    }
    let n: i64 = v.parse().map_err(|_| bad(key, v))?;
    if n < lo || n > hi {
        return Err(range(key, v, &format!("{lo}..={hi}")));
    }
    Ok(n)
}

fn pick<T: Copy>(key: &str, v: &str, table: &[(&str, T)]) -> Result<T, OptError> {
    table.iter().find(|(name, _)| *name == v).map(|(_, t)| *t).ok_or_else(|| bad(key, v))
}

fn parse_page_size(v: &str) -> Result<MediaSize, OptError> {
    const KEY: &str = "PageSize";
    if v == "Label108x174.8mm" {
        return Ok(MediaSize::Label108x1748);
    }
    let body = v.strip_prefix("Custom.").ok_or_else(|| bad(KEY, v))?;
    let (body, unit) = [("in", Unit::In), ("mm", Unit::Mm), ("cm", Unit::Cm), ("pt", Unit::Pt)]
        .iter()
        .find_map(|(sfx, u)| body.strip_suffix(sfx).map(|b| (b, *u)))
        .unwrap_or((body, Unit::Pt));
    let (w, l) = body.split_once('x').ok_or_else(|| bad(KEY, v))?;
    let w = parse_length_mm10(KEY, w, unit)?;
    let l = parse_length_mm10(KEY, l, unit)?;
    let to_u32 = |x: i64| u32::try_from(x).map_err(|_| range(KEY, v, "positive size"));
    Ok(MediaSize::Custom { width: to_u32(w)?, length: to_u32(l)? })
}

fn parse_pos(key: &str, v: &str, max_mm10: i64, allowed: &str) -> Result<i32, OptError> {
    let n = parse_length_mm10(key, v, Unit::In)?;
    if n.abs() > max_mm10 {
        return Err(range(key, v, allowed));
    }
    Ok(n as i32)
}

impl JobConfig {
    /// Build a validated configuration from the CUPS option string and copy count.
    pub fn from_cups(options: &str, copies: u32) -> Result<JobConfig, OptError> {
        let mut c = JobConfig::default();
        if !(1..=9999).contains(&copies) {
            return Err(range("copies", &copies.to_string(), "1..=9999"));
        }
        c.copies = copies as u16;
        let mut quality_explicit_banding: Option<bool> = None;
        let mut after: Option<String> = None;
        let mut interval: u8 = 1;
        for (k, v) in tokenize(options)? {
            let (k, v) = match k.strip_prefix("no") {
                Some(base) if BOOL_KEYS.contains(&base) && v == "true" => (base.to_string(), "false".to_string()),
                _ => (k, v),
            };
            let (k, v) = (k.as_str(), v.as_str());
            match k {
                "PageSize" => c.media = parse_page_size(v)?,
                "TmMediaForm" => {
                    use MediaForm::*;
                    c.form = pick(k, v, &[("DieCutGap", DieCutGap), ("DieCutBlackMark", DieCutBlackMark), ("ContinuousLabel", ContinuousLabel), ("ContinuousPaper", ContinuousPaper), ("ContinuousPaperBlackMark", ContinuousPaperBlackMark)])?
                }
                "TmMediaSaving" => c.saving = pick(k, v, &[("None", MediaSaving::None), ("Lower", MediaSaving::Lower), ("LowerUpper", MediaSaving::LowerUpper)])?,
                "TmCoating" => {
                    use Coating::*;
                    c.coating = pick(k, v, &[("Plain", Plain), ("Synthetic", Synthetic), ("Matte", Matte), ("Wristband", Wristband), ("Glossy", Glossy)])?
                }
                "TmQuality" => c.quality = pick(k, v, &[("Speed", Quality::Speed), ("Quality1", Quality::Quality1), ("Quality2", Quality::Quality2)])?,
                "TmBandingReduction" => quality_explicit_banding = Some(parse_bool(k, v)?),
                "TmAfterPrint" => {
                    pick(k, v, &[("CutEveryPage", ()), ("CutLastPage", ()), ("CutEveryN", ()), ("PeelOff", ()), ("CutPosition", ()), ("PrintEnd", ())])?;
                    after = Some(v.to_string());
                }
                "TmCutInterval" => interval = parse_int(k, v, 1, 255)? as u8,
                "TmNotify" => c.notify = pick(k, v, &[("None", Notify::None), ("PageEnd", Notify::PageEnd), ("JobEnd", Notify::JobEnd)])?,
                "TmBorderless" => c.borderless = parse_bool(k, v)?,
                "TmBidirectional" => c.bidirectional = parse_bool(k, v)?,
                "TmRotate180" => c.rotate180 = parse_bool(k, v)?,
                "TmSkipBlank" => c.skip_blank = parse_bool(k, v)?,
                "TmVacuum" => {
                    c.vacuum = if v == "Auto" {
                        Vacuum::Auto
                    } else {
                        match parse_int(k, v, 0, 9)? {
                            n @ (0 | 4 | 9) => Vacuum::Manual(n as u8),
                            n => return Err(OptError::NotYetSupported(format!("TmVacuum={n} (capture pending)"))),
                        }
                    }
                }
                "TmPosAdjH" => c.pos_adj_h = parse_pos(k, v, 1039, "-4.09in..=4.09in")?,
                "TmPosAdjV" => c.pos_adj_v = parse_pos(k, v, 11176, "-44in..=44in")?,
                "TmBlack" => c.black = pick(k, v, &[("Rich", BlackMode::Rich), ("KOnly", BlackMode::KOnly)])?,
                "TmColorMode" => c.color.mode = pick(k, v, &[("Preferred", ColorMode::Preferred), ("Standard", ColorMode::Standard), ("Off", ColorMode::Off)])?,
                "TmGamma" => c.color.gamma = pick(k, v, &[("1.8", Gamma::G18), ("2.2", Gamma::G22)])?,
                "TmInkAdjust" => c.color.ink_adjust = parse_bool(k, v)?,
                "TmInkBkCMY" => c.color.ink_bkcmy = parse_int(k, v, -6, 0)? as i8,
                "TmInkBk" => c.color.ink_bk = parse_int(k, v, -6, 0)? as i8,
                "TmBrightness" => c.color.brightness = parse_int(k, v, -2, 2)? as i8,
                "Collate" => c.collate = parse_bool(k, v)?,
                other if other.starts_with("Tm") => return Err(OptError::UnknownOption(other.into())),
                _ => {}
            }
        }
        c.after_print = match after.as_deref() {
            None | Some("CutEveryPage") => AfterPrint::CutEveryPage,
            Some("CutLastPage") => AfterPrint::CutLastPage,
            Some("CutEveryN") => AfterPrint::CutEveryN(interval),
            Some("PeelOff") => AfterPrint::PeelOff,
            Some("CutPosition") => AfterPrint::CutPosition,
            _ => AfterPrint::PrintEnd,
        };
        c.apply_rules(quality_explicit_banding)?;
        Ok(c)
    }

    fn apply_rules(&mut self, banding: Option<bool>) -> Result<(), OptError> {
        if let MediaSize::Custom { width, length } = &mut self.media {
            let (wmin, wtxt) = if self.form.is_label() { (254, "1.00in..=4.25in") } else { (300, "1.18in..=4.25in") };
            if !(wmin..=1080).contains(width) {
                return Err(range("PageSize", &format!("width {width} (0.1mm)"), wtxt));
            }
            if !(79..=11176).contains(length) {
                return Err(range("PageSize", &format!("length {length} (0.1mm)"), "0.31in..=44in"));
            }
            *length = (*length).max(80);
        }
        if !self.coating.allows(self.quality) {
            return Err(OptError::Conflict(format!("{:?} coating does not allow {:?}", self.coating, self.quality)));
        }
        self.banding_reduction = match (self.coating, self.quality) {
            (Coating::Plain, _) | (_, Quality::Quality1) => false,
            (_, Quality::Quality2) => true,
            _ => banding.unwrap_or(true),
        };
        if self.after_print.is_cut() && self.media.mm10().1 < 150 {
            self.after_print = AfterPrint::PeelOff;
        }
        if !self.form.supports_saving() {
            self.saving = MediaSaving::None;
        }
        Ok(())
    }
}
```

Add `pub mod options;` to `lib.rs`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p tmc3500-core options`
Expected: `9 passed`.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
git add -A
git commit -m "core: JobConfig from CUPS options with Windows dialog rules

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Page geometry (`geometry`)

**Files:**
- Create: `crates/tmc3500-core/src/geometry.rs`
- Modify: `crates/tmc3500-core/src/lib.rs`

**Interfaces:**
- Consumes: `options::{JobConfig, MediaSize, MediaForm}` (Task 3), `units` (Task 1).
- Produces:
  - `tmc3500_core::geometry::Geometry { page_length: u32, width: u32, height: u32, left: u32, top: i32, bottom: u32, sn_width: u16, sn_length: u16 }`
  - `tmc3500_core::geometry::geometry(cfg: &JobConfig) -> Geometry`

Media saving (305–308) depends on the printed area and is done in phase 2. In this phase, `geometry`
ignores `cfg.saving`.

- [ ] **Step 1: Write the failing test** (`geometry.rs`)

```rust
//! Page geometry in printer units (1/360 in), as sent in ESC(C / ESC(S / ESC(l / ESC(c and SN.
use crate::options::{JobConfig, MediaForm, MediaSize};
use crate::units::{dots_to_mm10_ceil, mm10_to_dots};

#[cfg(test)]
mod tests {
    use super::*;

    fn g(opts: &str) -> Geometry {
        geometry(&JobConfig::from_cups(opts, 1).unwrap())
    }

    /// (options, page_length, width, height, left, bottom, sn_width, sn_length), all from the captures.
    #[test]
    fn matches_every_captured_geometry() {
        let c = "PageSize=Custom.4.25x6.88in";
        let cases: &[(&str, u32, u32, u32, u32, u32, u16, u16)] = &[
            ("", 2520, 1587, 2477, 56, 2477, 1120, 1748),                                    // 000
            ("TmBorderless=False", 2520, 1587, 2477, 56, 2456, 1120, 1748),                   // 110
            (c, 2519, 1587, 2477, 56, 2477, 1120, 1748),                                      // 210
            (&format!("{c} TmMediaForm=DieCutBlackMark"), 2519, 1587, 2519, 56, 2477, 1120, 1748), // 301
            (&format!("{c} TmMediaForm=ContinuousLabel"), 2477, 1587, 2477, 56, 2477, 1120, 0xffff), // 302
            (&format!("{c} TmMediaForm=ContinuousPaper"), 2477, 1531, 2477, 28, 2477, 1081, 0xffff), // 303
            (&format!("{c} TmMediaForm=ContinuousPaperBlackMark"), 2477, 1531, 2477, 28, 2477, 1081, 0xffff), // 304
            ("PageSize=Custom.2.00x6.88in", 2519, 776, 2477, 28, 2477, 548, 1748),            // 330
            ("PageSize=Custom.4.25x3.00in", 1122, 1587, 1080, 56, 1080, 1120, 762),           // 331
            ("PageSize=Custom.1.97x1.18in", 467, 765, 425, 28, 425, 540, 300),                // 332
            ("PageSize=Custom.4.02x5.98in", 2195, 1503, 2153, 28, 2153, 1061, 1520),          // 333
            ("PageSize=Custom.1.00x0.31in", 155, 416, 113, 28, 113, 294, 80),                 // 334
            ("PageSize=Custom.4.25x12.00in", 4362, 1587, 4320, 56, 4320, 1120, 3048),         // 335
        ];
        for &(o, pl, w, h, l, b, snw, snl) in cases {
            let got = g(o);
            assert_eq!(
                (got.page_length, got.width, got.height, got.left, got.top, got.bottom, got.sn_width, got.sn_length),
                (pl, w, h, l, -20, b, snw, snl),
                "options {o:?}"
            );
        }
    }
}
```

- [ ] **Step 2: Run the test to see it fail**

Run: `cargo test -p tmc3500-core geometry`
Expected: compile error `cannot find function geometry`.

- [ ] **Step 3: Implement** (above the test module)

```rust
/// Left/right liner edge added to the width of label forms.
pub const EDGE_DOTS: u32 = 28;
/// Widest paper width (108 mm). Wider label+edge widths shift the left margin
/// (an assumption that fits both captured cases; see spec section 3).
pub const MAX_PAPER_DOTS: u32 = 1531;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    /// ESC(C page length (label + gap).
    pub page_length: u32,
    /// ESC(S width and height.
    pub width: u32,
    pub height: u32,
    /// ESC(l left margin.
    pub left: u32,
    /// ESC(c top and bottom margins.
    pub top: i32,
    pub bottom: u32,
    /// SN media block width/length in 0.1 mm (length 0xffff = continuous).
    pub sn_width: u16,
    pub sn_length: u16,
}

pub fn geometry(cfg: &JobConfig) -> Geometry {
    let (w_mm10, l_mm10) = cfg.media.mm10();
    let media_w = mm10_to_dots(w_mm10);
    let media_l = mm10_to_dots(l_mm10);
    let width = if cfg.form.is_label() { media_w + 2 * EDGE_DOTS } else { media_w };
    let left = if width > MAX_PAPER_DOTS { width - MAX_PAPER_DOTS } else { EDGE_DOTS };
    let gap = match (cfg.form.is_die_cut(), cfg.media) {
        (false, _) => 0,
        (true, MediaSize::Label108x1748) => 43,
        (true, MediaSize::Custom { .. }) => 42,
    };
    let height = if cfg.form == MediaForm::DieCutBlackMark { media_l + gap } else { media_l };
    let to_u16 = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
    Geometry {
        page_length: media_l + gap,
        width,
        height,
        left,
        top: -20,
        bottom: media_l.saturating_sub(if cfg.borderless { 0 } else { 21 }),
        sn_width: to_u16(dots_to_mm10_ceil(width)),
        sn_length: if cfg.form.is_continuous() { 0xffff } else { to_u16(dots_to_mm10_ceil(media_l)) },
    }
}
```

Add `pub mod geometry;` to `lib.rs`.

- [ ] **Step 4: Run the test to see it pass**

Run: `cargo test -p tmc3500-core geometry`
Expected: `1 passed`.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
git add -A
git commit -m "core: page geometry matching all captured sizes and forms

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Epson RLE (`rle`)

**Files:**
- Create: `crates/tmc3500-core/src/rle.rs`
- Modify: `crates/tmc3500-core/src/lib.rs`

**Interfaces:**
- Produces:
  - `rle::encode_line(line: &[u8], out: &mut Vec<u8>)`
  - `rle::encode(data: &[u8], bytes_per_line: usize) -> Vec<u8>`
  - `rle::encoded_len(enc: &[u8], expected: usize) -> Result<usize, RleError>`
  - `rle::decode(enc: &[u8], expected: usize) -> Result<Vec<u8>, RleError>`
  - `enum RleError { Truncated, Overrun }`

Algorithm, verified on all 283,202 captured lines:
- applied per line;
- a run of 3–128 equal bytes → `[257 − n, byte]`;
- otherwise a literal of up to 127 bytes, ended early where a run of 3 or more starts → `[n − 1, bytes…]`.

The exact check against the captures happens in Task 8.

- [ ] **Step 1: Write the failing tests** (`rle.rs`)

```rust
//! Epson ESC i compression mode 1 (TIFF-style run-length encoding), as the TM-C3500 driver emits it.

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn encodes_like_the_windows_driver() {
        let mut out = Vec::new();
        encode_line(&[0, 0, 0, 0, 0, 0x0b, 0xdf, 0xdf, 0xf3], &mut out);
        assert_eq!(out, vec![252, 0, 3, 0x0b, 0xdf, 0xdf, 0xf3]);
        let mut out = Vec::new();
        encode_line(&[7; 300], &mut out);
        assert_eq!(out, vec![129, 7, 129, 7, 213, 7]); // 128 + 128 + 44
        let lit: Vec<u8> = (0..130u8).collect();
        let mut out = Vec::new();
        encode_line(&lit, &mut out);
        assert_eq!(out[0], 126); // 127-byte literal first
        assert_eq!(out[128], 2); // then 3
    }

    #[test]
    fn runs_never_cross_lines() {
        assert_eq!(encode(&[5, 5, 5, 5, 5, 5], 3), vec![254, 5, 254, 5]);
    }

    #[test]
    fn decode_errors_instead_of_panicking() {
        assert_eq!(decode(&[5, 1, 2], 6), Err(RleError::Truncated));
        assert_eq!(decode(&[129, 7], 10), Err(RleError::Overrun));
        assert_eq!(encoded_len(&[], 1), Err(RleError::Truncated));
        assert_eq!(encoded_len(&[], 0), Ok(0));
    }

    proptest! {
        #[test]
        fn roundtrip(data in proptest::collection::vec(prop_oneof![Just(0u8), Just(255u8), any::<u8>()], 1..2000), bpl in 1usize..200) {
            let lines = data.len() / bpl;
            prop_assume!(lines > 0);
            let data = &data[..lines * bpl];
            let enc = encode(data, bpl);
            prop_assert_eq!(encoded_len(&enc, data.len()), Ok(enc.len()));
            prop_assert_eq!(decode(&enc, data.len()).unwrap(), data.to_vec());
        }
        #[test]
        fn decode_never_panics(enc in proptest::collection::vec(any::<u8>(), 0..300), n in 0usize..5000) {
            let _ = decode(&enc, n);
        }
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p tmc3500-core rle`
Expected: compile errors.

- [ ] **Step 3: Implement** (above the test module)

```rust
pub const MIN_RUN: usize = 3;
pub const MAX_RUN: usize = 128;
pub const MAX_LITERAL: usize = 127;

fn run_len(d: &[u8], i: usize) -> usize {
    let v = d[i];
    let mut j = i + 1;
    while j < d.len() && d[j] == v && j - i < MAX_RUN {
        j += 1;
    }
    j - i
}

/// Compress one raster line.
pub fn encode_line(line: &[u8], out: &mut Vec<u8>) {
    let n = line.len();
    let mut i = 0;
    while i < n {
        let r = run_len(line, i);
        if r >= MIN_RUN {
            out.push((257 - r) as u8);
            out.push(line[i]);
            i += r;
            continue;
        }
        let s = i;
        while i < n && i - s < MAX_LITERAL {
            if run_len(line, i) >= MIN_RUN {
                break;
            }
            i += 1;
        }
        out.push((i - s - 1) as u8);
        out.extend_from_slice(&line[s..i]);
    }
}

/// Compress a block of lines (`data.len()` must be a multiple of `bytes_per_line`, which must be > 0).
pub fn encode(data: &[u8], bytes_per_line: usize) -> Vec<u8> {
    assert!(bytes_per_line > 0, "bytes_per_line must be > 0");
    let mut out = Vec::with_capacity(data.len() / 2);
    for line in data.chunks(bytes_per_line) {
        encode_line(line, &mut out);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RleError {
    /// Input ended before `expected` bytes were produced.
    Truncated,
    /// A control would produce more than `expected` bytes.
    Overrun,
}

/// Step through controls; call `emit(control_offset, literal?, count)` for each.
fn walk(enc: &[u8], expected: usize, mut emit: impl FnMut(usize, bool, usize)) -> Result<usize, RleError> {
    let (mut i, mut got) = (0usize, 0usize);
    while got < expected {
        let c = *enc.get(i).ok_or(RleError::Truncated)? as usize;
        let (literal, count, adv) = if c < 128 { (true, c + 1, c + 2) } else { (false, 257 - c, 2) };
        if i + adv > enc.len() {
            return Err(RleError::Truncated);
        }
        got += count;
        if got > expected {
            return Err(RleError::Overrun);
        }
        emit(i, literal, count);
        i += adv;
    }
    Ok(i)
}

/// Number of encoded bytes that decode to exactly `expected` bytes.
pub fn encoded_len(enc: &[u8], expected: usize) -> Result<usize, RleError> {
    walk(enc, expected, |_, _, _| {})
}

pub fn decode(enc: &[u8], expected: usize) -> Result<Vec<u8>, RleError> {
    let mut out = Vec::new();
    walk(enc, expected, |i, literal, count| {
        if literal {
            out.extend_from_slice(&enc[i + 1..i + 1 + count]);
        } else {
            out.extend(std::iter::repeat(enc[i + 1]).take(count));
        }
    })?;
    Ok(out)
}
```

Add `pub mod rle;` to `lib.rs`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p tmc3500-core rle`
Expected: `5 passed`.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
git add -A
git commit -m "core: Epson RLE encoder/decoder

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Strict stream decoder (`decode`)

**Files:**
- Create: `crates/tmc3500-core/src/decode.rs`
- Modify: `crates/tmc3500-core/src/lib.rs`

**Interfaces:**
- Consumes: `rle::encoded_len` (Task 5).
- Produces:
  - `decode::Kind<'a>` enum: `Data`, `Ejl`, `Init`, `Exit`, `Paren { cmd: u8, payload: &'a [u8] }`,
    `RemoteEnter`, `Remote { cmd: [u8; 2], payload: &'a [u8] }`,
    `Raster { color: u8, compressed: bool, bits: u8, bytes_per_line: u16, lines: u16, data: &'a [u8] }`,
    `Short { cmd: u8, arg: u8 }`
  - `decode::Token<'a> { offset: usize, raw: &'a [u8], kind: Kind<'a> }`
  - `decode::DecodeError { offset: usize, reason: &'static str }`
  - `decode::decode(b: &[u8]) -> Result<Vec<Token<'_>>, DecodeError>`
  - `decode::skeleton(b: &[u8]) -> Result<Vec<u8>, DecodeError>`: the raw bytes of all tokens except
    `Raster`, `ESC($`, `ESC(v`, with `0x0d` removed from `Data` tokens (empty ones dropped).

- [ ] **Step 1: Write the failing tests** (`decode.rs`)

```rust
//! Strict parser for the TM-C3500 stream (ESC/P2 + REMOTE1). Used by tests and the fake printer.
use crate::rle;

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const HDR: &[u8] = b"\0\0\0\x1b\x01@EJL 1284.4\n@EJL     \n\x1b@\x1b@\x1b(R\x08\0\0REMOTE1EX\x06\0\0\0\0\0\x24\x02SV\0\0\x1b\0\0\0";

    #[test]
    fn decodes_job_header() {
        let t = decode(HDR).unwrap();
        let kinds: Vec<_> = t.iter().map(|t| t.kind.clone()).collect();
        assert_eq!(kinds, vec![
            Kind::Data, Kind::Ejl, Kind::Init, Kind::Init, Kind::RemoteEnter,
            Kind::Remote { cmd: *b"EX", payload: &[0, 0, 0, 0, 0x24, 2] },
            Kind::Remote { cmd: *b"SV", payload: &[] },
            Kind::Exit,
        ]);
        assert_eq!(t.iter().map(|t| t.raw.len()).sum::<usize>(), HDR.len());
    }

    #[test]
    fn decodes_paren_short_and_raster() {
        // ESC(G 01, ESC U 00, ESC i c=2 comp=1 bits=2 bpl=2 lines=1, RLE [ff 00] = two zero bytes
        let b = b"\x1b(G\x01\0\x01\x1bU\0\x1bi\x02\x01\x02\x02\0\x01\0\xff\x00\r\x0c";
        let t = decode(b).unwrap();
        assert_eq!(t[0].kind, Kind::Paren { cmd: b'G', payload: &[1] });
        assert_eq!(t[1].kind, Kind::Short { cmd: b'U', arg: 0 });
        assert_eq!(t[2].kind, Kind::Raster { color: 2, compressed: true, bits: 2, bytes_per_line: 2, lines: 1, data: &[0xff, 0] });
        assert_eq!(t[3].kind, Kind::Data);
    }

    #[test]
    fn skeleton_drops_raster_positioning_and_cr() {
        let b = b"\x1b(G\x01\0\x01\x1b($\x04\0\x50\0\0\0\x1bi\x02\x01\x02\x02\0\x01\0\xff\x00\r\x1b(v\x04\0\x4a\x01\0\0\r\x0c";
        assert_eq!(skeleton(b).unwrap(), b"\x1b(G\x01\0\x01\x0c".to_vec());
    }

    #[test]
    fn rejects_truncated_and_unknown() {
        assert!(decode(b"\x1b(G\x05\0\x01").is_err());
        assert!(decode(b"\x1b(R\x08\0\0REMOTE1E").is_err());
        assert!(decode(b"\x1b(R\x08\0\0REMOTE1ex\0\0\x1b\0\0\0").is_err());
        assert!(decode(b"\x1bZ").is_err());
        assert!(decode(b"\x1b").is_err());
        assert!(decode(b"\x1bi\x02\x01\x02\x10\0\x10\0\x05").is_err());
    }

    proptest! {
        #[test]
        fn never_panics(b in proptest::collection::vec(prop_oneof![Just(0x1bu8), Just(b'('), Just(b'i'), any::<u8>()], 0..400)) {
            let _ = decode(&b);
            let _ = skeleton(&b);
        }
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p tmc3500-core decode`
Expected: compile errors.

- [ ] **Step 3: Implement** (above the test module)

```rust
pub const ESC: u8 = 0x1b;
const REMOTE1: &[u8] = b"\0REMOTE1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind<'a> {
    Data,
    Ejl,
    Init,
    Exit,
    Paren { cmd: u8, payload: &'a [u8] },
    RemoteEnter,
    Remote { cmd: [u8; 2], payload: &'a [u8] },
    Raster { color: u8, compressed: bool, bits: u8, bytes_per_line: u16, lines: u16, data: &'a [u8] },
    Short { cmd: u8, arg: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token<'a> {
    pub offset: usize,
    pub raw: &'a [u8],
    pub kind: Kind<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError {
    pub offset: usize,
    pub reason: &'static str,
}

fn err(offset: usize, reason: &'static str) -> DecodeError {
    DecodeError { offset, reason }
}

fn get(b: &[u8], start: usize, n: usize) -> Result<&[u8], DecodeError> {
    let end = start.checked_add(n).ok_or(err(start, "length overflow"))?;
    b.get(start..end).ok_or(err(start, "truncated"))
}

fn u16le(s: &[u8]) -> u16 {
    u16::from_le_bytes([s[0], s[1]])
}

pub fn decode(b: &[u8]) -> Result<Vec<Token<'_>>, DecodeError> {
    let mut out = Vec::new();
    let mut o = 0usize;
    while o < b.len() {
        if b[o] != ESC {
            let end = b[o..].iter().position(|&c| c == ESC).map_or(b.len(), |p| o + p);
            out.push(Token { offset: o, raw: &b[o..end], kind: Kind::Data });
            o = end;
            continue;
        }
        let c = get(b, o + 1, 1)?[0];
        let (len, kind) = match c {
            0x01 => {
                let end = b[o + 2..].iter().position(|&x| x == ESC).ok_or(err(o, "unterminated EJL"))?;
                (2 + end, Kind::Ejl)
            }
            b'@' => (2, Kind::Init),
            0x00 => {
                if get(b, o + 2, 2)? != [0, 0] {
                    return Err(err(o, "bad ESC 00 sequence"));
                }
                (4, Kind::Exit)
            }
            b'(' => {
                let h = get(b, o + 2, 3)?;
                let (cmd, n) = (h[0], u16le(&h[1..3]) as usize);
                let payload = get(b, o + 5, n)?;
                if cmd == b'R' && payload == REMOTE1 {
                    out.push(Token { offset: o, raw: &b[o..o + 5 + n], kind: Kind::RemoteEnter });
                    o = remote(b, o + 5 + n, &mut out)?;
                    continue;
                }
                (5 + n, Kind::Paren { cmd, payload })
            }
            b'i' => {
                let h = get(b, o + 2, 7)?;
                let (color, comp, bits) = (h[0], h[1], h[2]);
                let (bpl, lines) = (u16le(&h[3..5]), u16le(&h[5..7]));
                let total = usize::from(bpl).checked_mul(usize::from(lines)).ok_or(err(o, "raster size overflow"))?;
                let used = match comp {
                    0 => total,
                    1 => rle::encoded_len(b.get(o + 9..).unwrap_or(&[]), total).map_err(|_| err(o, "bad RLE data"))?,
                    _ => return Err(err(o, "unknown compression")),
                };
                let data = get(b, o + 9, used)?;
                (9 + used, Kind::Raster { color, compressed: comp == 1, bits, bytes_per_line: bpl, lines, data })
            }
            b'U' | b'r' | b'+' => (3, Kind::Short { cmd: c, arg: get(b, o + 2, 1)?[0] }),
            _ => return Err(err(o, "unknown ESC command")),
        };
        get(b, o, len)?;
        out.push(Token { offset: o, raw: &b[o..o + len], kind });
        o += len;
    }
    Ok(out)
}

fn remote<'a>(b: &'a [u8], mut o: usize, out: &mut Vec<Token<'a>>) -> Result<usize, DecodeError> {
    loop {
        if b.get(o..o + 4) == Some(&[ESC, 0, 0, 0][..]) {
            out.push(Token { offset: o, raw: &b[o..o + 4], kind: Kind::Exit });
            return Ok(o + 4);
        }
        let h = get(b, o, 4)?;
        let cmd = [h[0], h[1]];
        if !cmd.iter().all(u8::is_ascii_uppercase) {
            return Err(err(o, "bad REMOTE1 command"));
        }
        let n = u16le(&h[2..4]) as usize;
        let payload = get(b, o + 4, n)?;
        out.push(Token { offset: o, raw: &b[o..o + 4 + n], kind: Kind::Remote { cmd, payload } });
        o += 4 + n;
    }
}

/// Stream without raster content: drops ESC i, ESC($, ESC(v and CR bytes.
pub fn skeleton(b: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let mut out = Vec::with_capacity(b.len().min(1 << 20));
    for t in decode(b)? {
        match t.kind {
            Kind::Raster { .. } | Kind::Paren { cmd: b'$' | b'v', .. } => {}
            Kind::Data => out.extend(t.raw.iter().filter(|&&c| c != b'\r')),
            _ => out.extend_from_slice(t.raw),
        }
    }
    Ok(out)
}
```

Add `pub mod decode;` to `lib.rs`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p tmc3500-core decode`
Expected: `5 passed`.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
git add -A
git commit -m "core: strict ESC/P2 + REMOTE1 stream decoder

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Command writer, `SN` block, job encoder (`protocol`)

**Files:**
- Create: `crates/tmc3500-core/src/protocol/mod.rs`, `protocol/writer.rs`, `protocol/sn.rs`, `protocol/job.rs`
- Modify: `crates/tmc3500-core/src/lib.rs`

**Interfaces:**
- Consumes: `options::*` (Task 3), `geometry::{geometry, Geometry}` (Task 4), `decode::{decode, Kind}` (Task 6, tests only).
- Produces:
  - `protocol::writer::Writer` with `new()`, `bytes(&[u8])`, `nuls3()`, `ejl_1284()`, `init()`,
    `paren(u8, &[u8])`, `remote_enter()`, `remote(&[u8; 2], &[u8])`, `remote_exit()`,
    `short(u8, u8)`, `into_bytes() -> Vec<u8>`
  - `protocol::sn::sn_payload(g: &Geometry, cfg: &JobConfig) -> [u8; 26]`
  - `protocol::job::JobMeta { year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8, job_id: u32, host_id: [u8; 6] }`
  - `protocol::job::Page { raster: Vec<u8>, blank: bool }`. `raster` holds the complete band commands
    for that page (ESC($ / ESC i / CR / ESC(v), produced in phase 2. Empty in phase 1.
  - `protocol::job::encode_job(cfg: &JobConfig, meta: &JobMeta, pages: &[Page]) -> Vec<u8>`

Job layout (from the captures, see spec section 3):
```
[set header] = 00 00 00, EJL, ESC@ ESC@, REMOTE1{ PM 00 00 | EX 00 00 00 00 2d 01 | SN | [BB] | cut cmds }, exit
job   = 00 00 00, EJL, ESC@ ESC@, REMOTE1{ EX ..24 form | SV | EX ..27 02 (02 cut / 01 no cut) }, exit
        00 00 00, EJL, ESC@ ESC@, REMOTE1{ TI | JS 00 00 00 00 | JH 00 01 <id u32le> <host 6> }, exit
        ESC(d ff 7f <32767 zeros> ×2
        for each set: [set header], pages…, then (not last set) ESC@ REMOTE1{LD} exit
        ESC@ REMOTE1{LD} exit, ESC@ REMOTE1{LD | JE 00} exit
page  = (first page of set: ESC(G 01, ESC(U 04 04 v a0 05, ESC U bidi, ESC(i q)
        ESC(C, ESC(S, ESC(l, ESC(c, ESC(D 40 38 50 28, ESC(m mode, ESC(e 00 e, raster, FF
cut   = CutEveryPage: BC 00 00 01 00, BC 00 01 00 00 | CutEveryN(n): BC 00 00 n 00, BC 00 01 00 00
        CutLastPage: BC 00 01 00 00 | PeelOff/CutPosition/PrintEnd: EX 00 00 00 00 25 02/01/00
BB    = JobEnd: 00 01 00 00 00 | PageEnd: 00 00 00 01 00
sets  = collate && copies>1: `copies` sets of all pages; otherwise one set, each page repeated `copies` times
quality: Speed v=4 q=0 e=0x11, Quality1/2 v=2 q=1 e=0x14
mode  = (Speed: 0x20|banding, Quality1: 0x41, Quality2: 0x45) | (borderless ? 0x02 : 0)
skip_blank: pages with blank=true are dropped
```

- [ ] **Step 1: Write the failing tests** (`protocol/job.rs`)

```rust
//! Whole-job encoder.
use super::sn::sn_payload;
use super::writer::Writer;
use crate::geometry::geometry;
use crate::options::{AfterPrint, JobConfig, Notify, Quality};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{decode, Kind};

    fn meta() -> JobMeta {
        JobMeta { year: 2026, month: 11, day: 2, hour: 23, minute: 24, second: 57, job_id: 2, host_id: [0x7c, 0xed, 0x8d, 0x69, 0xeb, 0xc7] }
    }

    fn page() -> Page {
        Page { raster: vec![], blank: false }
    }

    fn remotes(b: &[u8]) -> Vec<String> {
        decode(b).unwrap().iter().filter_map(|t| match t.kind {
            Kind::Remote { cmd, payload } => Some(format!("{}{:02x?}", String::from_utf8_lossy(&cmd), payload)),
            _ => None,
        }).collect()
    }

    #[test]
    fn default_job_header_is_byte_exact() {
        let b = encode_job(&JobConfig::default(), &meta(), &[page()]);
        let want_start: &[u8] = b"\0\0\0\x1b\x01@EJL 1284.4\n@EJL     \n\x1b@\x1b@\x1b(R\x08\0\0REMOTE1EX\x06\0\0\0\0\0\x24\x02SV\0\0EX\x07\0\0\0\0\0\x27\x02\x02\x1b\0\0\0\0\0\0\x1b\x01@EJL 1284.4\n@EJL     \n\x1b@\x1b@\x1b(R\x08\0\0REMOTE1TI\x08\0\0\x07\xea\x0b\x02\x17\x18\x39JS\x04\0\0\0\0\0JH\x0c\0\0\x01\x02\0\0\0\x7c\xed\x8d\x69\xeb\xc7\x1b\0\0\0\x1b(d\xff\x7f";
        assert_eq!(&b[..want_start.len()], want_start);
        let want_end: &[u8] = b"\x0c\x1b@\x1b(R\x08\0\0REMOTE1LD\0\0\x1b\0\0\0\x1b@\x1b(R\x08\0\0REMOTE1LD\0\0JE\x01\0\0\x1b\0\0\0";
        assert!(b.ends_with(want_end));
    }

    #[test]
    fn default_set_header_and_page_setup() {
        let b = encode_job(&JobConfig::default(), &meta(), &[page()]);
        let r = remotes(&b);
        assert_eq!(&r[6..], &[
            "PM[00, 00]", "EX[00, 00, 00, 00, 2d, 01]",
            "SN[00, 40, ff, ff, 60, 04, d4, 06, 03, 00, ff, ff, ff, ff, ff, ff, ff, ff, ff, ff, ff, ff, ff, ff, 31, 11]",
            "BC[00, 00, 01, 00]", "BC[00, 01, 00, 00]", "LD[]", "LD[]", "JE[00]",
        ]);
        let parens: Vec<(u8, Vec<u8>)> = decode(&b).unwrap().iter().filter_map(|t| match t.kind {
            Kind::Paren { cmd, payload } if cmd != b'd' => Some((cmd, payload.to_vec())),
            _ => None,
        }).collect();
        assert_eq!(parens, vec![
            (b'G', vec![1]), (b'U', vec![4, 4, 4, 0xa0, 5]), (b'i', vec![0]),
            (b'C', vec![0xd8, 9, 0, 0]), (b'S', vec![0x33, 6, 0, 0, 0xad, 9, 0, 0]), (b'l', vec![0x38, 0, 0, 0]),
            (b'c', vec![0xec, 0xff, 0xff, 0xff, 0xad, 9, 0, 0]), (b'D', vec![0x40, 0x38, 0x50, 0x28]),
            (b'm', vec![0x23]), (b'e', vec![0, 0x11]),
        ]);
    }

    #[test]
    fn copies_and_collate_structure() {
        let count = |b: &[u8], want: &str| remotes(b).iter().filter(|s| s.starts_with(want)).count();
        let c3 = JobConfig::from_cups("", 3).unwrap();
        let b = encode_job(&c3, &meta(), &[page(), page()]);
        assert_eq!((count(&b, "PM"), count(&b, "LD")), (1, 2));
        assert_eq!(b.iter().filter(|&&x| x == 0x0c).count(), 6);
        let c3c = JobConfig::from_cups("Collate=True", 3).unwrap();
        let b = encode_job(&c3c, &meta(), &[page(), page()]);
        assert_eq!((count(&b, "PM"), count(&b, "LD")), (3, 4));
    }

    #[test]
    fn zero_or_all_skipped_pages_still_make_a_valid_job() {
        let c = JobConfig::from_cups("TmSkipBlank=True", 1).unwrap();
        for pages in [vec![], vec![Page { raster: vec![], blank: true }]] {
            let b = encode_job(&c, &meta(), &pages);
            assert!(decode(&b).is_ok());
            assert!(!b.contains(&0x0c));
            assert_eq!(remotes(&b).iter().filter(|s| s.starts_with("PM")).count(), 1);
        }
    }

    #[test]
    fn cut_notify_and_quality_commands() {
        let r = |o: &str| remotes(&encode_job(&JobConfig::from_cups(o, 1).unwrap(), &meta(), &[page()]));
        assert!(r("TmAfterPrint=CutLastPage").contains(&"BC[00, 01, 00, 00]".to_string()));
        assert!(!r("TmAfterPrint=CutLastPage").contains(&"BC[00, 00, 01, 00]".to_string()));
        assert!(r("TmAfterPrint=CutEveryN TmCutInterval=5").contains(&"BC[00, 00, 05, 00]".to_string()));
        let p = r("TmAfterPrint=PeelOff");
        assert!(p.contains(&"EX[00, 00, 00, 00, 27, 02, 01]".to_string()) && p.contains(&"EX[00, 00, 00, 00, 25, 02]".to_string()));
        assert!(r("TmNotify=PageEnd").contains(&"BB[00, 00, 00, 01, 00]".to_string()));
        let b = encode_job(&JobConfig::from_cups("TmQuality=Quality2 TmBidirectional=False", 1).unwrap(), &meta(), &[page()]);
        let toks = decode(&b).unwrap();
        assert!(toks.iter().any(|t| t.kind == Kind::Paren { cmd: b'm', payload: &[0x47] }));
        assert!(toks.iter().any(|t| t.kind == Kind::Paren { cmd: b'U', payload: &[4, 4, 2, 0xa0, 5] }));
        assert!(toks.iter().any(|t| t.kind == Kind::Short { cmd: b'U', arg: 1 }));
    }
}
```

`protocol/sn.rs` test:
```rust
//! REMOTE1 `SN` media block (26 bytes, checksum = 16-bit sum of bytes 2..24).
use crate::geometry::Geometry;
use crate::options::JobConfig;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::geometry;

    fn sn(o: &str) -> [u8; 26] {
        let c = JobConfig::from_cups(o, 1).unwrap();
        sn_payload(&geometry(&c), &c)
    }

    #[test]
    fn matches_captured_blocks() {
        let hex = |s: &str| -> Vec<u8> { s.split(' ').map(|x| u8::from_str_radix(x, 16).unwrap()).collect() };
        assert_eq!(sn("").to_vec(), hex("00 40 ff ff 60 04 d4 06 03 00 ff ff ff ff ff ff ff ff ff ff ff ff ff ff 31 11"));
        assert_eq!(sn("TmVacuum=0").to_vec(), hex("00 40 ff ff 60 04 d4 06 03 00 ff ff ff ff ff ff ff ff 1e ff ff ff ff ff 50 10"));
        assert_eq!(sn("PageSize=Custom.4.25x6.88in TmMediaForm=ContinuousPaper").to_vec(), hex("00 40 ff ff 39 04 ff ff 02 00 ff ff ff ff ff ff ff ff ff ff ff ff ff ff 2d 12"));
        assert_eq!(sn("PageSize=Custom.1.97x1.18in").to_vec(), hex("00 40 ff ff 1c 02 2c 01 03 00 ff ff ff ff ff ff ff ff ff ff ff ff ff ff 3e 10"));
        assert_eq!(sn("TmCoating=Glossy").to_vec(), hex("00 40 ff ff 60 04 d4 06 06 00 ff ff ff ff ff ff ff ff ff ff ff ff ff ff 34 11"));
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p tmc3500-core protocol`
Expected: compile errors.

- [ ] **Step 3: Implement `protocol/writer.rs`**

```rust
//! Low-level command writer.
use crate::decode::ESC;

#[derive(Debug, Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { buf: Vec::with_capacity(70_000) }
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    pub fn nuls3(&mut self) {
        self.bytes(&[0, 0, 0]);
    }
    /// ESC 01 "@EJL 1284.4\n@EJL     \n" (enter IEEE 1284.4 / packet mode).
    pub fn ejl_1284(&mut self) {
        self.bytes(b"\x1b\x01@EJL 1284.4\n@EJL     \n");
    }
    pub fn init(&mut self) {
        self.bytes(&[ESC, b'@']);
    }
    /// ESC ( cmd <len u16le> payload. Payloads are produced internally and are < 64 KiB.
    pub fn paren(&mut self, cmd: u8, payload: &[u8]) {
        let n = u16::try_from(payload.len()).expect("ESC( payload < 64 KiB (internal invariant)");
        self.bytes(&[ESC, b'(', cmd]);
        self.bytes(&n.to_le_bytes());
        self.bytes(payload);
    }
    pub fn remote_enter(&mut self) {
        self.paren(b'R', b"\0REMOTE1");
    }
    pub fn remote(&mut self, cmd: &[u8; 2], payload: &[u8]) {
        let n = u16::try_from(payload.len()).expect("REMOTE1 payload < 64 KiB (internal invariant)");
        self.bytes(cmd);
        self.bytes(&n.to_le_bytes());
        self.bytes(payload);
    }
    pub fn remote_exit(&mut self) {
        self.bytes(&[ESC, 0, 0, 0]);
    }
    pub fn short(&mut self, cmd: u8, arg: u8) {
        self.bytes(&[ESC, cmd, arg]);
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}
```

- [ ] **Step 4: Implement `protocol/sn.rs`** (above its test module)

```rust
pub fn sn_payload(g: &Geometry, cfg: &JobConfig) -> [u8; 26] {
    let mut p = [0xffu8; 26];
    p[0] = 0x00;
    p[1] = 0x40;
    p[4..6].copy_from_slice(&g.sn_width.to_le_bytes());
    p[6..8].copy_from_slice(&g.sn_length.to_le_bytes());
    p[8] = if cfg.form.is_label() { cfg.coating.sn_code() } else { 2 };
    p[9] = 0x00;
    p[18] = cfg.vacuum.sn_byte();
    let sum = p[2..24].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
    p[24..26].copy_from_slice(&sum.to_le_bytes());
    p
}
```

- [ ] **Step 5: Implement `protocol/job.rs`** (above its test module) and `protocol/mod.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobMeta {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Spooler job id (REMOTE1 JH).
    pub job_id: u32,
    /// Per-installation id (REMOTE1 JH; Windows host used 7c ed 8d 69 eb c7).
    pub host_id: [u8; 6],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Complete band commands for the page (ESC($ / ESC i / CR / ESC(v). Empty = nothing printed.
    pub raster: Vec<u8>,
    /// True if the page has no ink at all (for Skip Blank Page).
    pub blank: bool,
}

fn block_start(w: &mut Writer) {
    w.nuls3();
    w.ejl_1284();
    w.init();
    w.init();
    w.remote_enter();
}

pub fn encode_job(cfg: &JobConfig, meta: &JobMeta, pages: &[Page]) -> Vec<u8> {
    let g = geometry(cfg);
    let mut w = Writer::new();

    block_start(&mut w);
    w.remote(b"EX", &[0, 0, 0, 0, 0x24, cfg.form.ex24_code()]);
    w.remote(b"SV", &[]);
    w.remote(b"EX", &[0, 0, 0, 0, 0x27, 0x02, if cfg.after_print.is_cut() { 2 } else { 1 }]);
    w.remote_exit();

    block_start(&mut w);
    let [yh, yl] = meta.year.to_be_bytes();
    w.remote(b"TI", &[0, yh, yl, meta.month, meta.day, meta.hour, meta.minute, meta.second]);
    w.remote(b"JS", &[0, 0, 0, 0]);
    let mut jh = vec![0x00, 0x01];
    jh.extend_from_slice(&meta.job_id.to_le_bytes());
    jh.extend_from_slice(&meta.host_id);
    w.remote(b"JH", &jh);
    w.remote_exit();

    for _ in 0..2 {
        w.paren(b'd', &[0u8; 32767]);
    }

    let printable: Vec<&Page> = pages.iter().filter(|p| !(cfg.skip_blank && p.blank)).collect();
    let copies = usize::from(cfg.copies.max(1));
    let sets: Vec<Vec<&Page>> = if cfg.collate && copies > 1 {
        vec![printable.clone(); copies]
    } else {
        vec![printable.iter().flat_map(|p| std::iter::repeat(*p).take(copies)).collect()]
    };

    let (v, q, e) = match cfg.quality {
        Quality::Speed => (4u8, 0u8, 0x11u8),
        Quality::Quality1 | Quality::Quality2 => (2, 1, 0x14),
    };
    let mode = match cfg.quality {
        Quality::Speed => 0x20 | u8::from(cfg.banding_reduction),
        Quality::Quality1 => 0x41,
        Quality::Quality2 => 0x45,
    } | if cfg.borderless { 0x02 } else { 0 };
    let sn = sn_payload(&g, cfg);

    for (si, set) in sets.iter().enumerate() {
        block_start(&mut w);
        w.remote(b"PM", &[0, 0]);
        w.remote(b"EX", &[0, 0, 0, 0, 0x2d, 0x01]);
        w.remote(b"SN", &sn);
        match cfg.notify {
            Notify::None => {}
            Notify::JobEnd => w.remote(b"BB", &[0, 1, 0, 0, 0]),
            Notify::PageEnd => w.remote(b"BB", &[0, 0, 0, 1, 0]),
        }
        match cfg.after_print {
            AfterPrint::CutEveryPage => {
                w.remote(b"BC", &[0, 0, 1, 0]);
                w.remote(b"BC", &[0, 1, 0, 0]);
            }
            AfterPrint::CutEveryN(n) => {
                w.remote(b"BC", &[0, 0, n, 0]);
                w.remote(b"BC", &[0, 1, 0, 0]);
            }
            AfterPrint::CutLastPage => w.remote(b"BC", &[0, 1, 0, 0]),
            AfterPrint::PeelOff => w.remote(b"EX", &[0, 0, 0, 0, 0x25, 2]),
            AfterPrint::CutPosition => w.remote(b"EX", &[0, 0, 0, 0, 0x25, 1]),
            AfterPrint::PrintEnd => w.remote(b"EX", &[0, 0, 0, 0, 0x25, 0]),
        }
        w.remote_exit();

        for (pi, page) in set.iter().enumerate() {
            if pi == 0 {
                w.paren(b'G', &[1]);
                w.paren(b'U', &[4, 4, v, 0xa0, 0x05]);
                w.short(b'U', u8::from(!cfg.bidirectional));
                w.paren(b'i', &[q]);
            }
            w.paren(b'C', &g.page_length.to_le_bytes());
            let mut s = g.width.to_le_bytes().to_vec();
            s.extend_from_slice(&g.height.to_le_bytes());
            w.paren(b'S', &s);
            w.paren(b'l', &g.left.to_le_bytes());
            let mut c = g.top.to_le_bytes().to_vec();
            c.extend_from_slice(&g.bottom.to_le_bytes());
            w.paren(b'c', &c);
            w.paren(b'D', &[0x40, 0x38, 0x50, 0x28]);
            w.paren(b'm', &[mode]);
            w.paren(b'e', &[0, e]);
            w.bytes(&page.raster);
            w.bytes(&[0x0c]);
        }

        w.init();
        w.remote_enter();
        w.remote(b"LD", &[]);
        if si + 1 == sets.len() {
            w.remote_exit();
            w.init();
            w.remote_enter();
            w.remote(b"LD", &[]);
            w.remote(b"JE", &[0]);
        }
        w.remote_exit();
    }
    w.into_bytes()
}
```

`protocol/mod.rs`:
```rust
//! Printer command generation.
pub mod job;
pub mod sn;
pub mod writer;
```

Add `pub mod protocol;` to `lib.rs`.

- [ ] **Step 6: Run the tests to see them pass**

Run: `cargo test -p tmc3500-core protocol`
Expected: `6 passed`.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
git add -A
git commit -m "core: job encoder, SN media block, command writer

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Golden tests against all Windows captures

**Files:**
- Create: `testdata/golden.toml`, `crates/tmc3500-core/tests/golden.rs`

**Interfaces:**
- Consumes: `options::JobConfig::from_cups`, `protocol::job::{encode_job, JobMeta, Page}`,
  `decode::{decode, skeleton, Kind}`, `rle::{decode, encode}` (Tasks 3, 5, 6, 7).

The test reads `testdata/captures/manifest.csv` (columns: timestamp, name, image, copies, pages, …):
- `pages` comes from the manifest;
- `blank` is true when image = `t4_blank.png`;
- `copies` comes from the manifest unless `golden.toml` overrides it.

For each capture it:
- takes `TI`/`JH` from the capture to build `JobMeta`;
- encodes the job with empty rasters;
- requires `skeleton(ours) == skeleton(capture)`.

A second test re-encodes every compressed raster block in every capture and requires identical bytes.

- [ ] **Step 1: Create `testdata/golden.toml`**

```toml
# CUPS options that reproduce each Windows capture (see testdata/captures/NOTES.md).
# skip = reason → excluded with that reason. copies overrides manifest.csv (driver-dialog copies).

[captures]
"000_default_t1" = {}
"001_default_t1_again" = {}
"002_default_t2" = {}
"003_default_t3" = {}
"004_default_t4_blank" = {}
"005_default_t1_copies3" = {}
"006_default_t1_pages2" = {}
"100_orient_landscape" = { options = "orientation-requested=4" }
"101_rotate180_on" = { options = "TmRotate180=True" }
"102_copies3_dialog" = { copies = 3 }
"103_collate_on" = { options = "Collate=True" }
"104_collate_on_copies3" = { options = "Collate=True", copies = 3 }
"105_reverse_order_pages2" = { options = "outputorder=reverse" }
"106_posadj_vert_+0.50" = { options = "TmPosAdjV=0.50in" }
"107_posadj_vert_-0.25" = { options = "TmPosAdjV=-0.25in" }
"108_posadj_horiz_+0.50" = { options = "TmPosAdjH=0.50in" }
"109_posadj_horiz_-0.25" = { options = "TmPosAdjH=-0.25in" }
"110_borderless_off" = { options = "TmBorderless=False" }
"111_borderless_off_t3" = { options = "TmBorderless=False" }
"200_notify_beep_page" = { options = "TmNotify=PageEnd" }
"201_notify_beep_job" = { options = "TmNotify=JobEnd" }
"202_print_preview_on" = {}
"210_media_custom_t1" = { options = "PageSize=Custom.4.25x6.88in" }
"211_media_custom_t3" = { options = "PageSize=Custom.4.25x6.88in" }
"301_form_blackmark_label" = { options = "PageSize=Custom.4.25x6.88in TmMediaForm=DieCutBlackMark" }
"302_form_continuous_label" = { options = "PageSize=Custom.4.25x6.88in TmMediaForm=ContinuousLabel" }
"303_form_continuous_paper" = { options = "PageSize=Custom.4.25x6.88in TmMediaForm=ContinuousPaper" }
"304_form_continuous_paper_blackmark" = { options = "PageSize=Custom.4.25x6.88in TmMediaForm=ContinuousPaperBlackMark" }
"305_save_lower_contlabel" = { skip = "phase 2: media-saving length depends on printed area" }
"306_save_lowerupper_contlabel" = { skip = "phase 2: media-saving length depends on printed area" }
"307_save_lower_contpaper" = { skip = "phase 2: media-saving length depends on printed area" }
"308_save_lowerupper_contpaper" = { skip = "phase 2: media-saving length depends on printed area" }
"310_coating_plain" = { options = "PageSize=Custom.4.25x6.88in TmCoating=Plain" }
"311_coating_synthetic" = { options = "PageSize=Custom.4.25x6.88in TmCoating=Synthetic" }
"312_coating_wristband" = { options = "PageSize=Custom.4.25x6.88in TmCoating=Wristband" }
"313_coating_glossy" = { options = "PageSize=Custom.4.25x6.88in TmCoating=Glossy" }
"314_quality_matte_mode1" = { options = "PageSize=Custom.4.25x6.88in TmQuality=Quality1" }
"315_quality_matte_mode2" = { options = "PageSize=Custom.4.25x6.88in TmQuality=Quality2" }
"316_quality_synthetic_mode1" = { options = "PageSize=Custom.4.25x6.88in TmCoating=Synthetic TmQuality=Quality1" }
"317_quality_synthetic_mode2" = { options = "PageSize=Custom.4.25x6.88in TmCoating=Synthetic TmQuality=Quality2" }
"318_quality_wristband_mode1" = { options = "PageSize=Custom.4.25x6.88in TmCoating=Wristband TmQuality=Quality1" }
"319_quality_glossy_mode1" = { options = "PageSize=Custom.4.25x6.88in TmCoating=Glossy TmQuality=Quality1" }
"320_quality_glossy_mode2" = { options = "PageSize=Custom.4.25x6.88in TmCoating=Glossy TmQuality=Quality2" }
"321_autocut_last_page" = { options = "PageSize=Custom.4.25x6.88in TmAfterPrint=CutLastPage" }
"322_autocut_every_n1" = { options = "PageSize=Custom.4.25x6.88in TmAfterPrint=CutEveryN TmCutInterval=1" }
"323_autocut_every_n2_pages6" = { options = "PageSize=Custom.4.25x6.88in TmAfterPrint=CutEveryN TmCutInterval=2" }
"324_autocut_every_n5_pages6" = { options = "PageSize=Custom.4.25x6.88in TmAfterPrint=CutEveryN TmCutInterval=5" }
"325_nocut_peel_off" = { options = "PageSize=Custom.4.25x6.88in TmAfterPrint=PeelOff" }
"326_nocut_cut_position" = { options = "PageSize=Custom.4.25x6.88in TmAfterPrint=CutPosition" }
"327_nocut_print_end" = { options = "PageSize=Custom.4.25x6.88in TmAfterPrint=PrintEnd" }
"328_autocut_default_pages2" = { options = "PageSize=Custom.4.25x6.88in" }
"330_size_width_2.00" = { options = "PageSize=Custom.2.00x6.88in" }
"331_size_length_3.00" = { options = "PageSize=Custom.4.25x3.00in" }
"332_size_50x30mm" = { options = "PageSize=Custom.1.97x1.18in" }
"333_size_102x152mm" = { options = "PageSize=Custom.4.02x5.98in" }
"334_size_min_1.00x0.31" = { options = "PageSize=Custom.1.00x0.31in" }
"335_size_length_12.00" = { options = "PageSize=Custom.4.25x12.00in" }
"340_named_media_50x30mm" = { options = "PageSize=Custom.1.97x1.18in" }
"400_color_disable_correction" = { options = "PageSize=Custom.4.25x6.88in TmColorMode=Off" }
"401_color_mode_epson_standard" = { options = "PageSize=Custom.4.25x6.88in TmColorMode=Standard" }
"402_gamma_1.8" = { options = "PageSize=Custom.4.25x6.88in TmGamma=1.8" }
"403_inkprofile_on" = { options = "PageSize=Custom.4.25x6.88in TmInkAdjust=True" }
"404_inkprofile_bkcmy_-3" = { options = "PageSize=Custom.4.25x6.88in TmInkAdjust=True TmInkBkCMY=-3" }
"405_inkprofile_bkcmy_-6" = { options = "PageSize=Custom.4.25x6.88in TmInkAdjust=True TmInkBkCMY=-6" }
"406_inkprofile_bk_-3" = { options = "PageSize=Custom.4.25x6.88in TmInkAdjust=True TmInkBk=-3" }
"407_inkprofile_bk_-6" = { options = "PageSize=Custom.4.25x6.88in TmInkAdjust=True TmInkBk=-6" }
"408_brightness_-2" = { options = "PageSize=Custom.4.25x6.88in TmInkAdjust=True TmBrightness=-2" }
"409_brightness_+2" = { options = "PageSize=Custom.4.25x6.88in TmInkAdjust=True TmBrightness=2" }
"410_bidirectional_off" = { options = "PageSize=Custom.4.25x6.88in TmBidirectional=False" }
"411_banding_reduction_off" = { options = "PageSize=Custom.4.25x6.88in TmBandingReduction=False" }
"500_hscopy_off_copies3" = {}
"501_hscopy_off" = {}
"502_always_spool_raw" = {}
"503_print_as_bitmap_t3" = {}
"504_skip_blank_page_t4" = { options = "TmSkipBlank=True" }
"505_skip_blank_page_t1" = { options = "TmSkipBlank=True" }
"506_join_barcodes_off" = {}
"507_units_mm" = {}
"508_status_monitor_on" = {}
"600_reduction_fit" = { options = "fit-to-page=true" }
"601_reduction_custom_50pct" = { options = "scaling=50" }
"602_reduction_fit_before_8.27x11.69" = { options = "fit-to-page=true" }
"610_vacuum_manual_default9" = { options = "TmVacuum=9" }
"611_vacuum_manual_4" = { options = "TmVacuum=4" }
"612_vacuum_manual_0" = { options = "TmVacuum=0" }
"900_default_t1_logged" = {}
```

- [ ] **Step 2: Write the golden test** (`crates/tmc3500-core/tests/golden.rs`)

```rust
//! Byte-exact comparison with the Windows driver captures.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tmc3500_core::decode::{decode, skeleton, Kind};
use tmc3500_core::options::JobConfig;
use tmc3500_core::protocol::job::{encode_job, JobMeta, Page};
use tmc3500_core::rle;

#[derive(Deserialize)]
struct Golden {
    captures: BTreeMap<String, Entry>,
}

#[derive(Deserialize, Default)]
struct Entry {
    #[serde(default)]
    options: String,
    copies: Option<u32>,
    skip: Option<String>,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
}

struct ManifestRow {
    image: String,
    copies: u32,
    pages: usize,
}

fn manifest() -> BTreeMap<String, ManifestRow> {
    let text = std::fs::read_to_string(root().join("captures/manifest.csv")).unwrap();
    text.lines()
        .skip(1)
        .map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            (f[1].to_string(), ManifestRow { image: f[2].into(), copies: f[3].parse().unwrap(), pages: f[4].parse().unwrap() })
        })
        .collect()
}

fn meta_from_capture(b: &[u8]) -> JobMeta {
    let toks = decode(b).unwrap();
    let ti = toks.iter().find_map(|t| match t.kind { Kind::Remote { cmd: [b'T', b'I'], payload } => Some(payload), _ => None }).unwrap();
    let jh = toks.iter().find_map(|t| match t.kind { Kind::Remote { cmd: [b'J', b'H'], payload } => Some(payload), _ => None }).unwrap();
    JobMeta {
        year: u16::from_be_bytes([ti[1], ti[2]]),
        month: ti[3],
        day: ti[4],
        hour: ti[5],
        minute: ti[6],
        second: ti[7],
        job_id: u32::from_le_bytes([jh[2], jh[3], jh[4], jh[5]]),
        host_id: jh[6..12].try_into().unwrap(),
    }
}

/// Human-readable first difference, for failure messages.
fn first_diff(a: &[u8], b: &[u8]) -> String {
    let i = a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
    let s = i.saturating_sub(16);
    format!("at skeleton offset {i:#x}: ours {:02x?} vs windows {:02x?}", &a[s..(i + 16).min(a.len())], &b[s..(i + 16).min(b.len())])
}

#[test]
fn every_capture_has_a_golden_entry() {
    let g: Golden = toml::from_str(&std::fs::read_to_string(root().join("golden.toml")).unwrap()).unwrap();
    for name in manifest().keys() {
        assert!(g.captures.contains_key(name), "capture {name} missing from golden.toml");
    }
}

#[test]
fn command_skeleton_matches_windows_byte_for_byte() {
    let g: Golden = toml::from_str(&std::fs::read_to_string(root().join("golden.toml")).unwrap()).unwrap();
    let m = manifest();
    let mut failures = Vec::new();
    let mut checked = 0;
    for (name, e) in &g.captures {
        if let Some(reason) = &e.skip {
            eprintln!("skip {name}: {reason}");
            continue;
        }
        let row = &m[name];
        let capture = std::fs::read(root().join(format!("captures/{name}.prn"))).unwrap();
        let cfg = match JobConfig::from_cups(&e.options, e.copies.unwrap_or(row.copies)) {
            Ok(c) => c,
            Err(err) => {
                failures.push(format!("{name}: options error {err}"));
                continue;
            }
        };
        let blank = row.image == "t4_blank.png";
        let pages = vec![Page { raster: vec![], blank }; row.pages];
        let ours = encode_job(&cfg, &meta_from_capture(&capture), &pages);
        let (a, b) = (skeleton(&ours).unwrap(), skeleton(&capture).unwrap());
        if a != b {
            failures.push(format!("{name}: {}", first_diff(&a, &b)));
        }
        checked += 1;
    }
    assert!(failures.is_empty(), "{} of {checked} captures differ:\n{}", failures.len(), failures.join("\n"));
    assert!(checked >= 82, "only {checked} captures checked");
}

#[test]
fn rle_reproduces_every_captured_raster_block() {
    let mut blocks = 0;
    for entry in std::fs::read_dir(root().join("captures")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("prn") {
            continue;
        }
        let b = std::fs::read(&path).unwrap();
        for t in decode(&b).unwrap() {
            if let Kind::Raster { compressed: true, bytes_per_line, lines, data, .. } = t.kind {
                let n = usize::from(bytes_per_line) * usize::from(lines);
                let raw = rle::decode(data, n).unwrap();
                assert_eq!(rle::encode(&raw, usize::from(bytes_per_line)), data, "{} @ {:#x}", path.display(), t.offset);
                blocks += 1;
            }
        }
    }
    assert!(blocks > 1500, "only {blocks} raster blocks found");
}
```

- [ ] **Step 3: Run the golden tests**

Run: `cargo test -p tmc3500-core --test golden -- --nocapture`

Expected:
- `3 passed`;
- 4 `skip` lines (305–308);
- **82** captures checked;
- more than 1500 raster blocks re-encoded identically.

If a capture differs, the failure message shows the first differing bytes. Fix the encoder rule in
Task 7's files, never the expected data. If a difference shows a rule the captures don't explain, stop
and report it rather than guessing.

- [ ] **Step 4: Run the whole suite and lints**

Run: `cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test --workspace`
Expected: all tests pass, no warnings.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "core: golden tests - command skeleton and RLE match all Windows captures

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## After this phase

Phase 2 gets its own plan:
- weave and pass layout;
- the dither interface;
- media-saving geometry (305–308);
- position adjustment and rotate 180;
- the CUPS filter binary and PPD.

That plan starts from the decoded raster blocks of these captures.

Items to add to the colour-chart VM session (phase 3):
- vacuum slider positions 1, 2, 3, 5, 6, 7, 8;
- a label width just above 4.07 in, to confirm the `ESC(l` rule;
- borderless off combined with Quality modes.
