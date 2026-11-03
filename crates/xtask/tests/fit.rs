//! Round trip: a chart printed by our own pipeline must fit back to the converter that printed it.
use std::path::PathBuf;
use tmc3500_core::color::Converter;
use tmc3500_core::cupsraster::Reader;
use tmc3500_core::options::JobConfig;
use tmc3500_core::pipeline::render_page;
use tmc3500_core::protocol::job::{encode_job, JobMeta};
use xtask::{fit, read_chart_png, Image, Layout};

fn layout() -> Layout {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/colour/c1_colour_chart.layout.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/colour")
        .join(name)
}

/// The chart image (1440 px wide, 360 dpi). `shear` moves each row `shear` pixels further right
/// (wrapping), like the session-2 chart as Windows decoded it.
fn chart_image(l: &Layout, shear: u32) -> Image {
    let (w, h) = (1476u32, 828u32);
    let mut rgb = vec![255u8; (w * h * 3) as usize];
    for p in &l.patches {
        for y in p.y..p.y + l.patch {
            for x in p.x..p.x + l.patch {
                let sx = (x + y * shear) % w;
                let d = ((y * w + sx) * 3) as usize;
                rgb[d..d + 3].copy_from_slice(&p.rgb);
            }
        }
    }
    Image::new(w, h, rgb).unwrap()
}

/// The image as a CUPS raster page (v3 LE) at the job's device resolution.
fn chart_raster(img: &Image, w: u32, h: u32, hscale: u32) -> Vec<u8> {
    let mut rgb = vec![255u8; (w * h * 3) as usize];
    for y in 0..img.height() {
        for x in 0..img.width().min(w / hscale) {
            for s in 0..hscale {
                let d = ((y * w + x * hscale + s) * 3) as usize;
                rgb[d..d + 3].copy_from_slice(&img.pixel(x, y));
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

fn round_trip(opts: &str, w: u32, hscale: u32, shear: u32) {
    round_trip_without(opts, w, hscale, shear, |_| false);
}

/// `drop(i)`: cube patch i is not printed; its area gets an off-grid colour 10 levels away on each
/// channel instead, so its node has to be inferred.
fn round_trip_without(opts: &str, w: u32, hscale: u32, shear: u32, drop: impl Fn(usize) -> bool) {
    let mut l = layout();
    let full = l.clone();
    for (i, p) in l.patches.iter_mut().enumerate() {
        if p.kind == "cube" && drop(i) {
            p.kind = "dropped".into();
        }
    }
    let mut img = chart_image(&l, shear);
    let dropped: Vec<[u8; 3]> = l
        .patches
        .iter()
        .filter(|p| p.kind == "dropped")
        .map(|p| p.rgb)
        .collect();
    for p in &l.patches {
        if p.kind == "dropped" {
            let off = p.rgb.map(|v| if v > 128 { v - 10 } else { v + 10 });
            img.fill(p.x, p.y, l.patch, l.patch, off);
        }
    }
    let l = full;
    let cfg = JobConfig::from_cups(opts, 1).unwrap();
    let conv = Converter::new(&cfg.color, cfg.black);
    let ras = chart_raster(&img, w, 2477, hscale);
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
    let (lut, report) = fit(&job, &img, &l).unwrap();
    let (mut worst, mut worst_inferred) = (0i64, 0i64);
    for p in l.patches.iter().filter(|p| p.kind == "cube") {
        let (want, got) = (conv.drops(p.rgb), lut.lookup(p.rgb));
        let w = if dropped.contains(&p.rgb) {
            &mut worst_inferred
        } else {
            &mut worst
        };
        for (a, b) in want.iter().flatten().zip(got.iter().flatten()) {
            *w = (*w).max((i64::from(*a) - i64::from(*b)).abs());
        }
    }
    // measured nodes: only the split curve's smoothing of the provisional kinks
    assert!(worst < 3300, "{opts}: worst node error {worst} / 65535");
    assert!(
        worst_inferred < 5300,
        "{opts}: worst inferred node error {worst_inferred} / 65535"
    );
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
    round_trip("", 1475, 1, 0);
}

#[test]
fn quality_chart_fits_back_to_the_converter() {
    round_trip("TmQuality=Quality2", 2950, 2, 0);
}

#[test]
fn colours_missing_from_the_page_are_inferred() {
    // every 5th node (never the first or last patch) replaced by a mix of its neighbours
    round_trip_without("", 1475, 1, 0, |i| i % 5 == 2 && i > 0 && i < 728);
}

#[test]
fn sheared_chart_fits_by_colour_not_position() {
    round_trip("", 1475, 1, 12);
}

#[test]
fn chart_png_is_read_as_windows_decoded_it() {
    let png = std::fs::read(testdata("c1_colour_chart_session2.png")).unwrap();
    let img = read_chart_png(&png).unwrap();
    assert_eq!((img.width(), img.height()), (1440, 828));
    // Each row that crosses patch column 40 is 36 bytes too long in the file; the decoder takes fixed
    // 4321-byte rows, so row 18 (first patch row) starts with patch 0 and row 19 is shifted by 12 px.
    assert_eq!(img.pixel(18, 18), [0, 0, 0]);
    assert_eq!(img.pixel(18 + 12, 19), [0, 0, 0]);
    assert_eq!(img.pixel(18 + 11, 19), [255, 255, 255]);
}

#[test]
fn windows_capture_fits_with_white_paper_and_ink_on_black() {
    let png = std::fs::read(testdata("c1_colour_chart_session2.png")).unwrap();
    let img = read_chart_png(&png).unwrap();
    let job = std::fs::read(testdata("701_chart_matte_speed.prn")).unwrap();
    let (lut, report) = fit(&job, &img, &layout()).unwrap();
    assert_eq!(lut.lookup([255, 255, 255]), [[0; 3]; 4]);
    // rich black: about 0.37 large K drops (measured 0.371..0.382 with different insets)
    let k_large = f64::from(lut.lookup([0, 0, 0])[0][2]) / 65535.0;
    assert!((0.33..0.42).contains(&k_large), "K large {k_large}");
    assert!(
        report.max_ramp_error < 0.15,
        "ramp error {}",
        report.max_ramp_error
    );
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
    assert!(fit(&job, &chart_image(&layout(), 0), &layout()).is_err());
}
