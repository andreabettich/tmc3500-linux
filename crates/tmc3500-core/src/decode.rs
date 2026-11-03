//! Strict parser for the TM-C3500 stream (ESC/P2 + REMOTE1). Used by tests and the fake printer.
use crate::rle;

pub const ESC: u8 = 0x1b;
const REMOTE1: &[u8] = b"\0REMOTE1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind<'a> {
    Data,
    Ejl,
    Init,
    Exit,
    Paren {
        cmd: u8,
        payload: &'a [u8],
    },
    RemoteEnter,
    Remote {
        cmd: [u8; 2],
        payload: &'a [u8],
    },
    Raster {
        color: u8,
        compressed: bool,
        bits: u8,
        bytes_per_line: u16,
        lines: u16,
        data: &'a [u8],
    },
    Short {
        cmd: u8,
        arg: u8,
    },
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
            let end = b[o..]
                .iter()
                .position(|&c| c == ESC)
                .map_or(b.len(), |p| o + p);
            out.push(Token {
                offset: o,
                raw: &b[o..end],
                kind: Kind::Data,
            });
            o = end;
            continue;
        }
        let c = get(b, o + 1, 1)?[0];
        let (len, kind) = match c {
            0x01 => {
                let end = b[o + 2..]
                    .iter()
                    .position(|&x| x == ESC)
                    .ok_or(err(o, "unterminated EJL"))?;
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
                    out.push(Token {
                        offset: o,
                        raw: &b[o..o + 5 + n],
                        kind: Kind::RemoteEnter,
                    });
                    o = remote(b, o + 5 + n, &mut out)?;
                    continue;
                }
                (5 + n, Kind::Paren { cmd, payload })
            }
            b'i' => {
                let h = get(b, o + 2, 7)?;
                let (color, comp, bits) = (h[0], h[1], h[2]);
                let (bpl, lines) = (u16le(&h[3..5]), u16le(&h[5..7]));
                let total = usize::from(bpl)
                    .checked_mul(usize::from(lines))
                    .ok_or(err(o, "raster size overflow"))?;
                let used = match comp {
                    0 => total,
                    1 => rle::encoded_len(b.get(o + 9..).unwrap_or(&[]), total)
                        .map_err(|_| err(o, "bad RLE data"))?,
                    _ => return Err(err(o, "unknown compression")),
                };
                let data = get(b, o + 9, used)?;
                (
                    9 + used,
                    Kind::Raster {
                        color,
                        compressed: comp == 1,
                        bits,
                        bytes_per_line: bpl,
                        lines,
                        data,
                    },
                )
            }
            b'U' | b'r' | b'+' => (
                3,
                Kind::Short {
                    cmd: c,
                    arg: get(b, o + 2, 1)?[0],
                },
            ),
            _ => return Err(err(o, "unknown ESC command")),
        };
        get(b, o, len)?;
        out.push(Token {
            offset: o,
            raw: &b[o..o + len],
            kind,
        });
        o += len;
    }
    Ok(out)
}

fn remote<'a>(b: &'a [u8], mut o: usize, out: &mut Vec<Token<'a>>) -> Result<usize, DecodeError> {
    loop {
        if b.get(o..o + 4) == Some(&[ESC, 0, 0, 0][..]) {
            out.push(Token {
                offset: o,
                raw: &b[o..o + 4],
                kind: Kind::Exit,
            });
            return Ok(o + 4);
        }
        let h = get(b, o, 4)?;
        let cmd = [h[0], h[1]];
        if !cmd.iter().all(u8::is_ascii_uppercase) {
            return Err(err(o, "bad REMOTE1 command"));
        }
        let n = u16le(&h[2..4]) as usize;
        let payload = get(b, o + 4, n)?;
        out.push(Token {
            offset: o,
            raw: &b[o..o + 4 + n],
            kind: Kind::Remote { cmd, payload },
        });
        o += 4 + n;
    }
}

/// Stream without raster content: drops ESC i, ESC($, ESC(v and CR bytes.
pub fn skeleton(b: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let mut out = Vec::with_capacity(b.len().min(1 << 20));
    for t in decode(b)? {
        match t.kind {
            Kind::Raster { .. }
            | Kind::Paren {
                cmd: b'$' | b'v', ..
            } => {}
            Kind::Data => out.extend(t.raw.iter().filter(|&&c| c != b'\r')),
            _ => out.extend_from_slice(t.raw),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const HDR: &[u8] = b"\0\0\0\x1b\x01@EJL 1284.4\n@EJL     \n\x1b@\x1b@\x1b(R\x08\0\0REMOTE1EX\x06\0\0\0\0\0\x24\x02SV\0\0\x1b\0\0\0";

    #[test]
    fn decodes_job_header() {
        let t = decode(HDR).unwrap();
        let kinds: Vec<_> = t.iter().map(|t| t.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                Kind::Data,
                Kind::Ejl,
                Kind::Init,
                Kind::Init,
                Kind::RemoteEnter,
                Kind::Remote {
                    cmd: *b"EX",
                    payload: &[0, 0, 0, 0, 0x24, 2]
                },
                Kind::Remote {
                    cmd: *b"SV",
                    payload: &[]
                },
                Kind::Exit,
            ]
        );
        assert_eq!(t.iter().map(|t| t.raw.len()).sum::<usize>(), HDR.len());
    }

    #[test]
    fn decodes_paren_short_and_raster() {
        // ESC(G 01, ESC U 00, ESC i c=2 comp=1 bits=2 bpl=2 lines=1, RLE [ff 00] = two zero bytes
        let b = b"\x1b(G\x01\0\x01\x1bU\0\x1bi\x02\x01\x02\x02\0\x01\0\xff\x00\r\x0c";
        let t = decode(b).unwrap();
        assert_eq!(
            t[0].kind,
            Kind::Paren {
                cmd: b'G',
                payload: &[1]
            }
        );
        assert_eq!(t[1].kind, Kind::Short { cmd: b'U', arg: 0 });
        assert_eq!(
            t[2].kind,
            Kind::Raster {
                color: 2,
                compressed: true,
                bits: 2,
                bytes_per_line: 2,
                lines: 1,
                data: &[0xff, 0]
            }
        );
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
