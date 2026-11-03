//! Whole-job encoder.
use super::sn::sn_payload;
use super::writer::Writer;
use crate::geometry::{geometry, Geometry};
use crate::options::{AfterPrint, JobConfig, Notify, Quality};
use std::io::{self, Write};

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
    /// Page length in dots when Media Saving shortened it (continuous forms only).
    pub length: Option<u32>,
}

fn block_start(w: &mut Writer) {
    w.nuls3();
    w.ejl_1284();
    w.init();
    w.init();
    w.remote_enter();
}

/// Streaming job encoder. Call `page` for every page of a set, `end_set` after each set (one set
/// unless collated copies), then `finish`. Output goes to `out` as it is produced.
pub struct JobWriter<W: Write> {
    out: W,
    cfg: JobConfig,
    g: Geometry,
    sn: [u8; 26],
    set_open: bool,
    pages_in_set: usize,
    sets: usize,
}

impl<W: Write> JobWriter<W> {
    /// Write the job header (pre-job block, job-start block, padding).
    pub fn begin(mut out: W, cfg: &JobConfig, meta: &JobMeta) -> io::Result<Self> {
        let g = geometry(cfg);
        let mut w = Writer::new();
        block_start(&mut w);
        w.remote(b"EX", &[0, 0, 0, 0, 0x24, cfg.form.ex24_code()]);
        w.remote(b"SV", &[]);
        let cut = if cfg.after_print.is_cut() { 2 } else { 1 };
        w.remote(b"EX", &[0, 0, 0, 0, 0x27, 0x02, cut]);
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
        out.write_all(&w.into_bytes())?;
        Ok(Self {
            out,
            cfg: cfg.clone(),
            g,
            sn: sn_payload(&g, cfg),
            set_open: false,
            pages_in_set: 0,
            sets: 0,
        })
    }

    fn open_set(&mut self) -> io::Result<()> {
        let cfg = &self.cfg;
        let mut w = Writer::new();
        block_start(&mut w);
        w.remote(b"PM", &[0, 0]);
        w.remote(b"EX", &[0, 0, 0, 0, 0x2d, 0x01]);
        w.remote(b"SN", &self.sn);
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
        self.out.write_all(&w.into_bytes())?;
        self.set_open = true;
        self.pages_in_set = 0;
        Ok(())
    }

    /// Write one page (opening a set first if needed).
    pub fn page(&mut self, page: &Page) -> io::Result<()> {
        if !self.set_open {
            self.open_set()?;
        }
        let cfg = &self.cfg;
        let g = &self.g;
        let (v, q, e) = match cfg.quality {
            Quality::Speed => (4u8, 0u8, 0x11u8),
            Quality::Quality1 | Quality::Quality2 => (2, 1, 0x14),
        };
        let mode = match cfg.quality {
            Quality::Speed => 0x20 | u8::from(cfg.banding_reduction),
            Quality::Quality1 => 0x41,
            Quality::Quality2 => 0x45,
        } | if cfg.borderless { 0x02 } else { 0 };
        let (page_length, height, bottom) = match page.length {
            None => (g.page_length, g.height, g.bottom),
            Some(l) => (l, l, l.saturating_sub(if cfg.borderless { 0 } else { 21 })),
        };
        let mut w = Writer::new();
        if self.pages_in_set == 0 {
            w.paren(b'G', &[1]);
            w.paren(b'U', &[4, 4, v, 0xa0, 0x05]);
            w.short(b'U', u8::from(!cfg.bidirectional));
            w.paren(b'i', &[q]);
        }
        w.paren(b'C', &page_length.to_le_bytes());
        let mut s = g.width.to_le_bytes().to_vec();
        s.extend_from_slice(&height.to_le_bytes());
        w.paren(b'S', &s);
        w.paren(b'l', &g.left.to_le_bytes());
        let mut c = g.top.to_le_bytes().to_vec();
        c.extend_from_slice(&bottom.to_le_bytes());
        w.paren(b'c', &c);
        w.paren(b'D', &[0x40, 0x38, 0x50, 0x28]);
        w.paren(b'm', &[mode]);
        w.paren(b'e', &[0, e]);
        self.out.write_all(&w.into_bytes())?;
        // Skip Blank Page: Windows still sends a skipped page's setup commands but omits its raster
        // and form feed (capture 504 vs 004).
        if !(cfg.skip_blank && page.blank) {
            self.out.write_all(&page.raster)?;
            self.out.write_all(&[0x0c])?;
        }
        self.pages_in_set += 1;
        Ok(())
    }

    /// Close the current set (no-op if none is open).
    pub fn end_set(&mut self) -> io::Result<()> {
        if self.set_open {
            let mut w = Writer::new();
            w.init();
            w.remote_enter();
            w.remote(b"LD", &[]);
            w.remote_exit();
            self.out.write_all(&w.into_bytes())?;
            self.set_open = false;
            self.sets += 1;
        }
        Ok(())
    }

    /// Close the job. A job without any page still gets one (empty) set, as Windows does.
    pub fn finish(mut self) -> io::Result<W> {
        if self.sets == 0 && !self.set_open {
            self.open_set()?;
        }
        self.end_set()?;
        let mut w = Writer::new();
        w.init();
        w.remote_enter();
        w.remote(b"LD", &[]);
        w.remote(b"JE", &[0]);
        w.remote_exit();
        self.out.write_all(&w.into_bytes())?;
        self.out.flush()?;
        Ok(self.out)
    }
}

/// Encode a whole job in memory: copies and collation are expanded here.
pub fn encode_job(cfg: &JobConfig, meta: &JobMeta, pages: &[Page]) -> Vec<u8> {
    let copies = usize::from(cfg.copies.max(1));
    let run = || -> io::Result<Vec<u8>> {
        let mut jw = JobWriter::begin(Vec::new(), cfg, meta)?;
        if cfg.collate && copies > 1 {
            for _ in 0..copies {
                for p in pages {
                    jw.page(p)?;
                }
                jw.end_set()?;
            }
        } else {
            for p in pages {
                for _ in 0..copies {
                    jw.page(p)?;
                }
            }
        }
        jw.finish()
    };
    run().expect("writing to a Vec cannot fail")
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
            length: None,
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
    fn job_writer_streams_the_same_bytes_as_encode_job() {
        let c = JobConfig::from_cups("Collate=True", 2).unwrap();
        let pages = [
            Page {
                raster: vec![],
                blank: false,
                length: None,
            },
            page(),
        ];
        let mut jw = JobWriter::begin(Vec::new(), &c, &meta()).unwrap();
        for _ in 0..2 {
            for p in &pages {
                jw.page(p).unwrap();
            }
            jw.end_set().unwrap();
        }
        assert_eq!(jw.finish().unwrap(), encode_job(&c, &meta(), &pages));
    }

    #[test]
    fn job_writer_with_no_pages_still_writes_a_set() {
        let c = JobConfig::default();
        let jw = JobWriter::begin(Vec::new(), &c, &meta()).unwrap();
        assert_eq!(jw.finish().unwrap(), encode_job(&c, &meta(), &[]));
    }

    #[test]
    fn page_length_override_changes_only_page_length_commands() {
        let c = JobConfig::from_cups("PageSize=Custom.4.25x6.88in TmMediaForm=ContinuousLabel", 1)
            .unwrap();
        let b = encode_job(
            &c,
            &meta(),
            &[Page {
                raster: vec![],
                blank: false,
                length: Some(212),
            }],
        );
        let toks = decode(&b).unwrap();
        let p = |cmd: u8| {
            toks.iter()
                .find_map(|t| match t.kind {
                    Kind::Paren { cmd: c, payload } if c == cmd => Some(payload.to_vec()),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(p(b'C'), vec![212, 0, 0, 0]);
        assert_eq!(p(b'S'), vec![0x33, 6, 0, 0, 212, 0, 0, 0]);
        assert_eq!(p(b'c'), vec![0xec, 0xff, 0xff, 0xff, 212, 0, 0, 0]);
    }

    #[test]
    fn skipped_blank_page_keeps_setup_but_drops_raster_and_form_feed() {
        // Capture 504 vs 004: Windows still sends the page setup (ESC(G .. ESC(e) for a skipped
        // blank page and omits only its raster and FF.
        let c = JobConfig::from_cups("TmSkipBlank=True", 1).unwrap();
        let b = encode_job(
            &c,
            &meta(),
            &[Page {
                raster: vec![0xaa],
                blank: true,
                length: None,
            }],
        );
        let toks = decode(&b).unwrap();
        assert!(toks
            .iter()
            .any(|t| matches!(t.kind, Kind::Paren { cmd: b'C', .. })));
        assert!(toks
            .iter()
            .any(|t| matches!(t.kind, Kind::Paren { cmd: b'e', .. })));
        assert_eq!(form_feeds(&b), 0);
        assert!(!b.contains(&0xaa));
        let without = JobConfig::from_cups("", 1).unwrap();
        let full = encode_job(
            &without,
            &meta(),
            &[Page {
                raster: vec![],
                blank: true,
                length: None,
            }],
        );
        assert_eq!(
            full.len(),
            encode_job(
                &c,
                &meta(),
                &[Page {
                    raster: vec![],
                    blank: true,
                    length: None,
                }]
            )
            .len()
                + 1
        );
    }

    #[test]
    fn zero_or_all_skipped_pages_still_make_a_valid_job() {
        let c = JobConfig::from_cups("TmSkipBlank=True", 1).unwrap();
        for pages in [
            vec![],
            vec![Page {
                raster: vec![],
                blank: true,
                length: None,
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
