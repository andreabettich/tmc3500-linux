//! The weave must produce the same pass and block structure as the Windows driver: pass positions,
//! horizontal offsets, colours, block line counts and widths. Dot content is not compared (our dither
//! differs); inked areas are synthesised to match the test images.
use std::path::PathBuf;
use tmc3500_core::decode::{decode, Kind};
use tmc3500_core::raster::{Ink, InkPage};
use tmc3500_core::weave::{weave, WeaveMode};

/// (y, x, colour, lines, bytes per line) for every raster block.
type Block = (u32, u32, u8, u16, u16);

fn structure(b: &[u8]) -> Vec<Block> {
    let (mut y, mut x) = (0u32, 0u32);
    let mut out = Vec::new();
    for t in decode(b).unwrap() {
        match t.kind {
            Kind::Paren { cmd: b'v', payload } => {
                y += u32::from_le_bytes(payload.try_into().unwrap())
            }
            Kind::Paren { cmd: b'$', payload } => {
                x = u32::from_le_bytes(payload.try_into().unwrap())
            }
            Kind::Paren { cmd: b'C', .. } => y = 0,
            Kind::Raster {
                color,
                bytes_per_line,
                lines,
                ..
            } => {
                out.push((y, x, color, lines, bytes_per_line));
                x = 0;
            }
            _ => {}
        }
    }
    out
}

fn capture(name: &str) -> Vec<Block> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../testdata/captures/{name}.prn"));
    structure(&std::fs::read(p).unwrap())
}

/// Page with all four inks at level 3 inside the given rectangles (x0, y0, x1, y1 exclusive).
fn page(w: u32, h: u32, rects: &[(Ink, u32, u32, u32, u32)]) -> InkPage {
    let mut p = InkPage::new(w, h).unwrap();
    for &(ink, x0, y0, x1, y1) in rects {
        for y in y0..y1 {
            for x in x0..x1 {
                p.set(ink, x, y, 3);
            }
        }
    }
    p
}

fn all_inks(x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<(Ink, u32, u32, u32, u32)> {
    Ink::ALL.iter().map(|&i| (i, x0, y0, x1, y1)).collect()
}

const W: u32 = 1475;
const H: u32 = 2477;

#[test]
fn speed_rect_matches_000() {
    let p = page(W, H, &all_inks(90, 90, 450, 180));
    assert_eq!(
        structure(&weave(&p, WeaveMode::Speed { banding: true }, 0)),
        capture("000_default_t1")
    );
}

#[test]
fn speed_rect_across_pass_boundary_matches_106() {
    let p = page(W, H, &all_inks(90, 270, 450, 360));
    assert_eq!(
        structure(&weave(&p, WeaveMode::Speed { banding: true }, 0)),
        capture("106_posadj_vert_+0.50")
    );
}

#[test]
fn speed_bars_match_002() {
    // t2: cyan rows 90..180, magenta 210..300, yellow 330..420, black 450..540 (black = all four inks)
    let mut rects = vec![
        (Ink::C, 90, 90, 630, 180),
        (Ink::M, 90, 210, 630, 300),
        (Ink::Y, 90, 330, 630, 420),
    ];
    rects.extend(all_inks(90, 450, 630, 540));
    let p = page(W, H, &rects);
    assert_eq!(
        structure(&weave(&p, WeaveMode::Speed { banding: true }, 0)),
        capture("002_default_t2")
    );
}

#[test]
fn speed_full_square_matches_003() {
    let p = page(W, H, &all_inks(0, 0, 720, 720));
    assert_eq!(
        structure(&weave(&p, WeaveMode::Speed { banding: true }, 0)),
        capture("003_default_t3")
    );
}

#[test]
fn borderless_off_offsets_grid_by_21() {
    let p = page(W, H - 42, &all_inks(0, 0, 720, 720));
    assert_eq!(
        structure(&weave(&p, WeaveMode::Speed { banding: true }, 21)),
        capture("111_borderless_off_t3")
    );
    let p = page(W, H - 42, &all_inks(90, 90, 450, 180));
    assert_eq!(
        structure(&weave(&p, WeaveMode::Speed { banding: true }, 21)),
        capture("110_borderless_off")
    );
}

#[test]
fn banding_off_matches_411() {
    let p = page(W, H, &all_inks(0, 0, 720, 720));
    assert_eq!(
        structure(&weave(&p, WeaveMode::Speed { banding: false }, 0)),
        capture("411_banding_reduction_off")
    );
}

#[test]
fn quality_mode1_matches_314() {
    let p = page(2 * W, H, &all_inks(0, 0, 1440, 720));
    assert_eq!(
        structure(&weave(&p, WeaveMode::Quality1, 0)),
        capture("314_quality_matte_mode1")
    );
}

#[test]
fn quality_mode2_matches_315() {
    let p = page(2 * W, H, &all_inks(0, 0, 1440, 720));
    assert_eq!(
        structure(&weave(&p, WeaveMode::Quality2, 0)),
        capture("315_quality_matte_mode2")
    );
}
