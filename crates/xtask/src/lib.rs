//! Colour-table fitting: measure the drop-size fractions of every chart patch in a printer job
//! (a Windows capture of `c1_colour_chart.png`) and build the 3-D table from the RGB cube patches.
#![forbid(unsafe_code)]
use serde::Deserialize;
use tmc3500_core::color::{Drops, Lut};
use tmc3500_core::layout::MAX_PRINT_WIDTH;
use tmc3500_core::raster::{Ink, InkPage};
use tmc3500_core::reassemble::reassemble;

#[derive(Debug, Clone, Deserialize)]
pub struct Patch {
    pub kind: String,
    pub rgb: [u8; 3],
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Layout {
    pub patch: u32,
    pub patches: Vec<Patch>,
}

/// Pixels ignored at each patch edge (dither and pass seams).
const INSET: u32 = 6;

#[derive(Debug, Default)]
pub struct Report {
    /// Largest |table − measured| over grey and ink-ramp patches, as a fraction (0..1).
    pub max_ramp_error: f64,
    pub lines: Vec<String>,
}

/// Measured drop fractions of one patch, per ink [K, C, M, Y].
fn measure(page: &InkPage, p: &Patch, size: u32, hscale: u32) -> [Drops; 4] {
    let mut counts = [[0u32; 4]; 4];
    let mut total = 0u32;
    for y in p.y + INSET..p.y + size - INSET {
        for x in (p.x + INSET) * hscale..(p.x + size - INSET) * hscale {
            total += 1;
            for (i, ink) in Ink::ALL.iter().enumerate() {
                counts[i][usize::from(page.get(*ink, x, y))] += 1;
            }
        }
    }
    counts.map(|c| {
        [1, 2, 3].map(|lvl| ((u64::from(c[lvl]) * 65535) / u64::from(total.max(1))) as u16)
    })
}

fn frac_err(a: &[Drops; 4], b: &[Drops; 4]) -> f64 {
    a.iter()
        .flatten()
        .zip(b.iter().flatten())
        .map(|(x, y)| (f64::from(*x) - f64::from(*y)).abs() / 65535.0)
        .fold(0.0, f64::max)
}

/// Fit the table from a job containing the chart on its first page.
pub fn fit(job: &[u8], layout: &Layout) -> Result<(Lut, Report), String> {
    let pages = reassemble(job).map_err(|e| format!("job does not decode: {e:?}"))?;
    let page = pages.first().ok_or("job has no page")?;
    let hscale = if page.width() > MAX_PRINT_WIDTH { 2 } else { 1 };
    let cube: Vec<&Patch> = layout.patches.iter().filter(|p| p.kind == "cube").collect();
    let n = (1..=33)
        .find(|n| n * n * n == cube.len())
        .ok_or("cube patch count is not n³")?;
    let nodes: Vec<[Drops; 4]> = cube
        .iter()
        .map(|p| measure(page, p, layout.patch, hscale))
        .collect();
    if nodes.iter().all(|d| d.iter().flatten().all(|&f| f == 0)) {
        return Err("no ink found where the chart should be (wrong job or layout?)".into());
    }
    let lut = Lut::from_nodes(n, nodes).ok_or("measured fractions out of range")?;
    let mut report = Report::default();
    for p in &layout.patches {
        let measured = measure(page, p, layout.patch, hscale);
        if p.kind == "grey" || p.kind.starts_with("ramp_") {
            let e = frac_err(&lut.lookup(p.rgb), &measured);
            report.max_ramp_error = report.max_ramp_error.max(e);
            report
                .lines
                .push(format!("{:8} {:3?}  table error {:.3}", p.kind, p.rgb, e));
        } else if let Some(name) = p.kind.strip_prefix("brand:") {
            report.lines.push(format!(
                "brand {name} {:3?}  measured K,C,M,Y [small,medium,large] = {measured:?}",
                p.rgb
            ));
        }
    }
    Ok((lut, report))
}
