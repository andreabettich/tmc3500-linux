//! Inverse of the weave: rebuild the dithered pages of a printer job (ours or a Windows capture).
//! Used by the colour fitter and for checking jobs in tests and the fake printer.
use crate::decode::{decode, DecodeError, Kind};
use crate::layout::MAX_PRINT_WIDTH;
use crate::raster::{Ink, InkPage};
use crate::rle;

/// Rebuild every page of a job. Columns are device columns (1/720 in in Quality modes), rows are
/// 1/360 in from the top of the page; dots outside the printable band are dropped.
pub fn reassemble(job: &[u8]) -> Result<Vec<InkPage>, DecodeError> {
    let mut pages = Vec::new();
    let mut page: Option<InkPage> = None;
    let (mut hscale, mut y, mut x) = (1u32, 0i64, 0u32);
    let le32 = |p: &[u8]| {
        p.get(..4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    for t in decode(job)? {
        match t.kind {
            Kind::Paren { cmd: b'U', payload } if payload.len() == 5 => {
                hscale = if payload[2] == 2 { 2 } else { 1 };
            }
            Kind::Paren { cmd: b'C', .. } => {
                pages.extend(page.take());
                y = 0;
                x = 0;
            }
            Kind::Paren { cmd: b'S', payload } if payload.len() == 8 => {
                let width = le32(payload).min(MAX_PRINT_WIDTH) * hscale;
                let height = le32(&payload[4..]);
                page = InkPage::new(width.max(1), height.max(1));
            }
            Kind::Paren { cmd: b'v', payload } => y += i64::from(le32(payload)),
            Kind::Paren { cmd: b'$', payload } => x = le32(payload),
            Kind::Raster {
                color,
                bytes_per_line,
                lines,
                data,
                compressed,
                ..
            } => {
                let bpl = usize::from(bytes_per_line);
                let raw = if compressed {
                    rle::decode(data, bpl * usize::from(lines)).map_err(|_| DecodeError {
                        offset: t.offset,
                        reason: "bad RLE data",
                    })?
                } else {
                    data.to_vec()
                };
                let ink = Ink::ALL.into_iter().find(|i| i.code() == color & 0x3f);
                if let (Some(p), Some(ink)) = (page.as_mut(), ink) {
                    let nr = i64::from(color >> 6 & 1);
                    for l in 0..usize::from(lines) {
                        let r = y + 2 * l as i64 - 20 + nr;
                        if r < 0 || r >= i64::from(p.height()) {
                            continue;
                        }
                        for k in 0..bpl * 4 {
                            let v = (raw[l * bpl + k / 4] >> (6 - 2 * (k % 4))) & 3;
                            if v != 0 {
                                // positions come from the stream: skip dots that would overflow
                                if let Some(col) = (k as u32)
                                    .checked_mul(hscale)
                                    .and_then(|o| x.checked_add(o))
                                {
                                    p.set(ink, col, r as u32, v);
                                }
                            }
                        }
                    }
                }
                x = 0;
            }
            _ => {}
        }
    }
    pages.extend(page);
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::JobConfig;
    use crate::protocol::job::{encode_job, JobMeta, Page};
    use crate::weave::{weave, WeaveMode};
    use std::path::PathBuf;

    fn meta() -> JobMeta {
        JobMeta {
            year: 2026,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            job_id: 1,
            host_id: [0; 6],
        }
    }

    fn pattern(w: u32, h: u32) -> InkPage {
        let mut p = InkPage::new(w, h).unwrap();
        for y in (0..h).step_by(3) {
            for x in (y % 7..w).step_by(5) {
                p.set(
                    Ink::ALL[((x + y) % 4) as usize],
                    x,
                    y,
                    ((x + 2 * y) % 3 + 1) as u8,
                );
            }
        }
        p
    }

    #[test]
    fn reassembles_our_own_jobs_exactly() {
        for (opts, mode, w) in [
            ("", WeaveMode::Speed { banding: true }, 1475),
            ("TmQuality=Quality1", WeaveMode::Quality1, 2950),
            ("TmQuality=Quality2", WeaveMode::Quality2, 2950),
        ] {
            let cfg = JobConfig::from_cups(opts, 1).unwrap();
            let page = pattern(w, 900);
            let job = encode_job(
                &cfg,
                &meta(),
                &[Page {
                    raster: weave(&page, mode, 0),
                    blank: false,
                    length: None,
                }],
            );
            let back = reassemble(&job).unwrap();
            assert_eq!(back.len(), 1);
            for y in 0..900 {
                for x in 0..w {
                    for ink in Ink::ALL {
                        assert_eq!(
                            back[0].get(ink, x, y),
                            page.get(ink, x, y),
                            "{opts} {x},{y}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn windows_capture_000_is_the_black_rectangle() {
        let b = std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../testdata/captures/000_default_t1.prn"),
        )
        .unwrap();
        let pages = reassemble(&b).unwrap();
        let p = &pages[0];
        assert_eq!(p.inked_rows(), Some((90, 179)));
        let cols: Vec<u32> = (0..p.width())
            .filter(|&x| p.get(Ink::K, x, 120) != 0)
            .collect();
        assert_eq!((cols[0], *cols.last().unwrap()), (90, 449));
    }

    #[test]
    fn huge_horizontal_position_does_not_overflow() {
        // ESC(S, ESC(v, ESC($ = u32::MAX, then one raster line with ink
        let mut b =
            b"\x1b(S\x08\0\x10\0\0\0\x10\0\0\0\x1b(v\x04\0\x14\0\0\0\x1b($\x04\0\xff\xff\xff\xff"
                .to_vec();
        b.extend_from_slice(b"\x1bi\x00\x00\x02\x01\x00\x01\x00\xff");
        let pages = reassemble(&b).unwrap();
        assert!(pages[0].is_blank());
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(reassemble(b"\x1b(G\x05\0").is_err());
    }
}
