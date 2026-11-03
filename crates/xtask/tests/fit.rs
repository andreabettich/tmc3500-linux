//! Round trip: a chart printed by our own pipeline must fit back to the converter that printed it.
use std::path::PathBuf;
use tmc3500_core::color::Converter;
use tmc3500_core::cupsraster::Reader;
use tmc3500_core::options::JobConfig;
use tmc3500_core::pipeline::render_page;
use tmc3500_core::protocol::job::{encode_job, JobMeta};
use xtask::{fit, Layout};

fn layout() -> Layout {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/colour/c1_colour_chart.layout.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// The chart as a CUPS raster page (v3 LE) at the job's device resolution.
fn chart_raster(l: &Layout, w: u32, h: u32, hscale: u32) -> Vec<u8> {
    let mut rgb = vec![255u8; (w * h * 3) as usize];
    for p in &l.patches {
        for y in p.y..p.y + l.patch {
            for x in p.x * hscale..(p.x + l.patch) * hscale {
                let d = ((y * w + x) * 3) as usize;
                rgb[d..d + 3].copy_from_slice(&p.rgb);
            }
        }
    }
    let mut out = b"3SaR".to_vec();
    let mut hd = vec![0u8; 1796];
    for (off, v) in [
        (276, 360 * hscale),
        (280, 360),
        (372, w),
        (376, h),
        (384, 8),
        (388, 24),
        (392, w * 3),
        (400, 1),
    ] {
        hd[off..off + 4].copy_from_slice(&u32::to_le_bytes(v));
    }
    out.extend_from_slice(&hd);
    out.extend_from_slice(&rgb);
    out
}

fn round_trip(opts: &str, w: u32, hscale: u32) {
    let l = layout();
    let cfg = JobConfig::from_cups(opts, 1).unwrap();
    let conv = Converter::new(&cfg.color, cfg.black);
    let ras = chart_raster(&l, w, 2477, hscale);
    let mut rd = Reader::new(&ras[..]).unwrap();
    let h = rd.next_page().unwrap().unwrap();
    let page = render_page(&mut rd, &h, &cfg, &conv).unwrap();
    let meta = JobMeta {
        year: 2026,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
        job_id: 1,
        host_id: [0; 6],
    };
    let job = encode_job(&cfg, &meta, &[page]);
    let (lut, report) = fit(&job, &l).unwrap();
    let mut worst = 0i64;
    for p in l.patches.iter().filter(|p| p.kind == "cube") {
        let (want, got) = (conv.drops(p.rgb), lut.lookup(p.rgb));
        for (a, b) in want.iter().flatten().zip(got.iter().flatten()) {
            worst = worst.max((i64::from(*a) - i64::from(*b)).abs());
        }
    }
    assert!(worst < 3300, "{opts}: worst node error {worst} / 65535");
    // Between nodes the provisional model has sharp kinks (small→medium drops at 1/3 and 2/3 ink) that a
    // 9-level grid cannot follow; this only checks the ramp patches are measured at the right place.
    assert!(
        report.max_ramp_error < 0.2,
        "{opts}: ramp error {}",
        report.max_ramp_error
    );
}

#[test]
fn speed_chart_fits_back_to_the_converter() {
    round_trip("", 1475, 1);
}

#[test]
fn quality_chart_fits_back_to_the_converter() {
    round_trip("TmQuality=Quality2", 2950, 2);
}

#[test]
fn a_job_without_the_chart_is_rejected() {
    let cfg = JobConfig::default();
    let meta = JobMeta {
        year: 2026,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
        job_id: 1,
        host_id: [0; 6],
    };
    let job = encode_job(&cfg, &meta, &[]);
    assert!(fit(&job, &layout()).is_err());
}
