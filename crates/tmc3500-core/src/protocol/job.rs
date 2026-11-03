//! Whole-job encoder.
use super::sn::sn_payload;
use super::writer::Writer;
use crate::geometry::geometry;
use crate::options::{AfterPrint, JobConfig, Notify, Quality};

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
    w.remote(
        b"EX",
        &[
            0,
            0,
            0,
            0,
            0x27,
            0x02,
            if cfg.after_print.is_cut() { 2 } else { 1 },
        ],
    );
    w.remote_exit();

    block_start(&mut w);
    let [yh, yl] = meta.year.to_be_bytes();
    w.remote(
        b"TI",
        &[
            0,
            yh,
            yl,
            meta.month,
            meta.day,
            meta.hour,
            meta.minute,
            meta.second,
        ],
    );
    w.remote(b"JS", &[0, 0, 0, 0]);
    let mut jh = vec![0x00, 0x01];
    jh.extend_from_slice(&meta.job_id.to_le_bytes());
    jh.extend_from_slice(&meta.host_id);
    w.remote(b"JH", &jh);
    w.remote_exit();

    for _ in 0..2 {
        w.paren(b'd', &[0u8; 32767]);
    }

    let printable: Vec<&Page> = pages
        .iter()
        .filter(|p| !(cfg.skip_blank && p.blank))
        .collect();
    let copies = usize::from(cfg.copies.max(1));
    let sets: Vec<Vec<&Page>> = if cfg.collate && copies > 1 {
        vec![printable.clone(); copies]
    } else {
        vec![printable
            .iter()
            .flat_map(|p| std::iter::repeat_n(*p, copies))
            .collect()]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{decode, Kind};

    fn meta() -> JobMeta {
        JobMeta {
            year: 2026,
            month: 11,
            day: 2,
            hour: 23,
            minute: 24,
            second: 57,
            job_id: 2,
            host_id: [0x7c, 0xed, 0x8d, 0x69, 0xeb, 0xc7],
        }
    }

    fn page() -> Page {
        Page {
            raster: vec![],
            blank: false,
        }
    }

    fn remotes(b: &[u8]) -> Vec<String> {
        decode(b)
            .unwrap()
            .iter()
            .filter_map(|t| match t.kind {
                Kind::Remote { cmd, payload } => {
                    Some(format!("{}{:02x?}", String::from_utf8_lossy(&cmd), payload))
                }
                _ => None,
            })
            .collect()
    }

    /// Form feeds in data tokens (a raw byte count would also hit the JH length field, 0x0c).
    fn form_feeds(b: &[u8]) -> usize {
        decode(b)
            .unwrap()
            .iter()
            .filter(|t| t.kind == Kind::Data)
            .map(|t| t.raw.iter().filter(|&&x| x == 0x0c).count())
            .sum()
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
        let parens: Vec<(u8, Vec<u8>)> = decode(&b)
            .unwrap()
            .iter()
            .filter_map(|t| match t.kind {
                Kind::Paren { cmd, payload } if cmd != b'd' => Some((cmd, payload.to_vec())),
                _ => None,
            })
            .collect();
        assert_eq!(
            parens,
            vec![
                (b'G', vec![1]),
                (b'U', vec![4, 4, 4, 0xa0, 5]),
                (b'i', vec![0]),
                (b'C', vec![0xd8, 9, 0, 0]),
                (b'S', vec![0x33, 6, 0, 0, 0xad, 9, 0, 0]),
                (b'l', vec![0x38, 0, 0, 0]),
                (b'c', vec![0xec, 0xff, 0xff, 0xff, 0xad, 9, 0, 0]),
                (b'D', vec![0x40, 0x38, 0x50, 0x28]),
                (b'm', vec![0x23]),
                (b'e', vec![0, 0x11]),
            ]
        );
    }

    #[test]
    fn copies_and_collate_structure() {
        let count =
            |b: &[u8], want: &str| remotes(b).iter().filter(|s| s.starts_with(want)).count();
        let c3 = JobConfig::from_cups("", 3).unwrap();
        let b = encode_job(&c3, &meta(), &[page(), page()]);
        assert_eq!((count(&b, "PM"), count(&b, "LD")), (1, 2));
        assert_eq!(form_feeds(&b), 6);
        let c3c = JobConfig::from_cups("Collate=True", 3).unwrap();
        let b = encode_job(&c3c, &meta(), &[page(), page()]);
        assert_eq!((count(&b, "PM"), count(&b, "LD")), (3, 4));
    }

    #[test]
    fn zero_or_all_skipped_pages_still_make_a_valid_job() {
        let c = JobConfig::from_cups("TmSkipBlank=True", 1).unwrap();
        for pages in [
            vec![],
            vec![Page {
                raster: vec![],
                blank: true,
            }],
        ] {
            let b = encode_job(&c, &meta(), &pages);
            assert!(decode(&b).is_ok());
            assert_eq!(form_feeds(&b), 0);
            assert_eq!(
                remotes(&b).iter().filter(|s| s.starts_with("PM")).count(),
                1
            );
        }
    }

    #[test]
    fn cut_notify_and_quality_commands() {
        let r = |o: &str| {
            remotes(&encode_job(
                &JobConfig::from_cups(o, 1).unwrap(),
                &meta(),
                &[page()],
            ))
        };
        assert!(r("TmAfterPrint=CutLastPage").contains(&"BC[00, 01, 00, 00]".to_string()));
        assert!(!r("TmAfterPrint=CutLastPage").contains(&"BC[00, 00, 01, 00]".to_string()));
        assert!(
            r("TmAfterPrint=CutEveryN TmCutInterval=5").contains(&"BC[00, 00, 05, 00]".to_string())
        );
        let p = r("TmAfterPrint=PeelOff");
        assert!(
            p.contains(&"EX[00, 00, 00, 00, 27, 02, 01]".to_string())
                && p.contains(&"EX[00, 00, 00, 00, 25, 02]".to_string())
        );
        assert!(r("TmNotify=PageEnd").contains(&"BB[00, 00, 00, 01, 00]".to_string()));
        let b = encode_job(
            &JobConfig::from_cups("TmQuality=Quality2 TmBidirectional=False", 1).unwrap(),
            &meta(),
            &[page()],
        );
        let toks = decode(&b).unwrap();
        assert!(toks.iter().any(|t| t.kind
            == Kind::Paren {
                cmd: b'm',
                payload: &[0x47]
            }));
        assert!(toks.iter().any(|t| t.kind
            == Kind::Paren {
                cmd: b'U',
                payload: &[4, 4, 2, 0xa0, 5]
            }));
        assert!(toks
            .iter()
            .any(|t| t.kind == Kind::Short { cmd: b'U', arg: 1 }));
    }
}
