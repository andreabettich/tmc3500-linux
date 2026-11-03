//! Epson ESC i compression mode 1 (TIFF-style run-length encoding), as the TM-C3500 driver emits it.

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
fn walk(
    enc: &[u8],
    expected: usize,
    mut emit: impl FnMut(usize, bool, usize),
) -> Result<usize, RleError> {
    let (mut i, mut got) = (0usize, 0usize);
    while got < expected {
        let c = *enc.get(i).ok_or(RleError::Truncated)? as usize;
        let (literal, count, adv) = if c < 128 {
            (true, c + 1, c + 2)
        } else {
            (false, 257 - c, 2)
        };
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
            out.extend(std::iter::repeat_n(enc[i + 1], count));
        }
    })?;
    Ok(out)
}

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
