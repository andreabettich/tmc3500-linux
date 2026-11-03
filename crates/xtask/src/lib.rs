//! Colour-table fitting: measure the drop-size fractions of every chart patch in a printer job
//! (a Windows capture of `c1_colour_chart.png`) and build the 3-D table from the RGB cube colours.
#![forbid(unsafe_code)]
use serde::Deserialize;
use std::collections::HashMap;
use tmc3500_core::color::{tetra_weights, Drops, Lut, SPLIT_POINTS};
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

/// The chart image exactly as the Windows application printed it (8-bit RGB, 360 dpi), placed at the
/// page origin.
#[derive(Debug, Clone)]
pub struct Image {
    w: u32,
    h: u32,
    rgb: Vec<u8>,
}

impl Image {
    pub fn new(w: u32, h: u32, rgb: Vec<u8>) -> Option<Self> {
        (rgb.len() == w as usize * h as usize * 3).then_some(Self { w, h, rgb })
    }
    pub fn width(&self) -> u32 {
        self.w
    }
    pub fn height(&self) -> u32 {
        self.h
    }
    /// Paints a rectangle (clipped to the image).
    pub fn fill(&mut self, x: u32, y: u32, w: u32, h: u32, rgb: [u8; 3]) {
        for yy in y..(y + h).min(self.h) {
            for xx in x..(x + w).min(self.w) {
                let o = (yy as usize * self.w as usize + xx as usize) * 3;
                self.rgb[o..o + 3].copy_from_slice(&rgb);
            }
        }
    }
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let o = (y as usize * self.w as usize + x as usize) * 3;
        [self.rgb[o], self.rgb[o + 1], self.rgb[o + 2]]
    }
}

/// Reads a chart PNG from `tools/make_colour_chart.py` (8-bit RGB, not interlaced, filter 0) the way
/// Windows decoded it: fixed rows of 1 + 3·width bytes, the filter byte ignored.
///
/// The session-2 chart (`testdata/colour/c1_colour_chart_session2.png`) was written with rows that
/// cross the last patch column 36 bytes too long. Windows printed it anyway, sheared by 12 px per such
/// row; this reproduces that, and for a well-formed chart it is an ordinary decode.
pub fn read_chart_png(b: &[u8]) -> Result<Image, String> {
    use std::io::Read;
    let mut rest = b
        .strip_prefix(b"\x89PNG\r\n\x1a\n".as_slice())
        .ok_or("not a PNG")?;
    let (mut w, mut h) = (0u32, 0u32);
    let mut idat = Vec::new();
    while rest.len() >= 12 {
        let len = u32::from_be_bytes(rest[0..4].try_into().unwrap()) as usize;
        let kind = &rest[4..8];
        let data = rest.get(8..8 + len).ok_or("truncated chunk")?;
        match kind {
            b"IHDR" => {
                if data.len() != 13 || data[8..13] != [8, 2, 0, 0, 0] {
                    return Err("only 8-bit RGB, non-interlaced PNGs are supported".into());
                }
                w = u32::from_be_bytes(data[0..4].try_into().unwrap());
                h = u32::from_be_bytes(data[4..8].try_into().unwrap());
            }
            b"IDAT" => idat.extend_from_slice(data),
            _ => {}
        }
        rest = rest.get(12 + len..).ok_or("truncated chunk")?;
    }
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return Err("bad image size".into());
    }
    let stride = 1 + 3 * w as usize;
    let need = stride * h as usize;
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&idat[..])
        .take(need as u64)
        .read_to_end(&mut raw)
        .map_err(|e| format!("image data: {e}"))?;
    if raw.len() < need {
        return Err("image data too short".into());
    }
    let rgb = raw
        .chunks_exact(stride)
        .flat_map(|r| &r[1..])
        .copied()
        .collect();
    Ok(Image::new(w, h, rgb).expect("size checked"))
}

/// Fewest pixels a colour must be measured from to be used.
const MIN_SAMPLES: u64 = 40;
/// Pixel count at which a colour gets full weight in the fit.
const FULL_WEIGHT_SAMPLES: u64 = 600;
/// Grid levels of the chart's RGB cube (9×9×9 nodes, measured directly where printed).
pub const CHART_GRID: usize = 9;
/// Grid levels of the fitted table (twice as fine: greys and ramps pin the extra nodes).
pub const TABLE_GRID: usize = 2 * CHART_GRID - 1;
/// Weight of the smoothness term (second differences along each grid axis) against one fully
/// weighted colour; it only decides chart nodes that were not printed.
const SMOOTHNESS: f64 = 0.01;
/// Weight that keeps a fine node at the coarse table's value against one fully weighted colour.
const PRIOR: f64 = 0.1;
/// Smallest amount kept in the table; below it (one dot in 1000) is fitting noise.
const MIN_AMOUNT: f64 = 0.001;
/// Width (in amount, 0..1) of the kernel that averages measured splits into a split curve.
const SPLIT_KERNEL: f64 = 0.008;

/// One measured colour: drop fractions per ink [K, C, M, Y] × [small, medium, large] (0..1), and its
/// pixel count.
#[derive(Debug, Clone)]
pub struct Sample {
    pub rgb: [u8; 3],
    pub fractions: [f64; 12],
    pub pixels: u64,
}

impl Sample {
    /// Ink amounts (small + 2·medium + 3·large) / 3, as in `Lut`.
    pub fn amounts(&self) -> [f64; 4] {
        let f = &self.fractions;
        std::array::from_fn(|i| (f[i * 3] + 2.0 * f[i * 3 + 1] + 3.0 * f[i * 3 + 2]) / 3.0)
    }
    fn weight(&self) -> f64 {
        self.pixels.min(FULL_WEIGHT_SAMPLES) as f64 / FULL_WEIGHT_SAMPLES as f64
    }
}

#[derive(Debug, Default)]
pub struct Report {
    /// Largest |table − measured| over grey and ink-ramp patches, as a fraction (0..1).
    pub max_ramp_error: f64,
    pub lines: Vec<String>,
}

/// Dot counts per ink [K, C, M, Y] and level 0..=3, and the number of dots.
type Counts = ([[u64; 4]; 4], u64);

/// Ink counts per image colour, over every pixel whose left and right neighbours have the same colour
/// (Windows dithers with a fixed mask, so only the pixel itself matters; the neighbour rule keeps
/// pixels that straddle a colour edge after resampling out).
fn measure_colours(page: &InkPage, img: &Image, hscale: u32) -> HashMap<[u8; 3], Counts> {
    let mut acc: HashMap<[u8; 3], Counts> = HashMap::new();
    let h = img.height().min(page.height());
    let w = img.width().min(page.width() / hscale);
    for y in 0..h {
        for x in 1..w.saturating_sub(1) {
            let c = img.pixel(x, y);
            if img.pixel(x - 1, y) != c || img.pixel(x + 1, y) != c {
                continue;
            }
            let e = acc.entry(c).or_default();
            for s in 0..hscale {
                for (i, ink) in Ink::ALL.iter().enumerate() {
                    e.0[i][usize::from(page.get(*ink, x * hscale + s, y))] += 1;
                }
                e.1 += 1;
            }
        }
    }
    acc
}

fn fractions(c: &Counts) -> [f64; 12] {
    let mut f = [0f64; 12];
    for (ink, l) in c.0.iter().enumerate() {
        for lvl in 0..3 {
            f[ink * 3 + lvl] = l[lvl + 1] as f64 / c.1.max(1) as f64;
        }
    }
    f
}

fn to_u16(v: f64) -> u16 {
    (v * 65535.0).round().clamp(0.0, 65535.0) as u16
}

/// Measured colours of the job's first page, which is `img` printed at the page origin.
pub fn samples(job: &[u8], img: &Image) -> Result<Vec<Sample>, String> {
    let pages = reassemble(job).map_err(|e| format!("job does not decode: {e:?}"))?;
    let page = pages.first().ok_or("job has no page")?;
    let hscale = if page.width() > MAX_PRINT_WIDTH { 2 } else { 1 };
    let mut out: Vec<Sample> = measure_colours(page, img, hscale)
        .into_iter()
        .filter(|(_, c)| c.1 / u64::from(hscale) >= MIN_SAMPLES)
        .map(|(rgb, c)| Sample {
            rgb,
            fractions: fractions(&c),
            pixels: c.1 / u64::from(hscale),
        })
        .collect();
    out.sort_by_key(|s| s.rgb);
    Ok(out)
}

/// Per ink, the measured split (small, medium, large fractions) as a function of the amount: a
/// pixel-weighted kernel average of the samples, scaled so each point's amount is exact; straight
/// lines across gaps, and towards all-large drops above the largest amount measured.
pub fn split_curves(samples: &[Sample]) -> [Vec<Drops>; 4] {
    std::array::from_fn(|ink| {
        let pts: Vec<(f64, [f64; 3], f64)> = samples
            .iter()
            .map(|s| {
                let f = &s.fractions[ink * 3..ink * 3 + 3];
                (s.amounts()[ink], [f[0], f[1], f[2]], s.weight())
            })
            .filter(|p| p.0 > 0.0)
            .collect();
        let last = SPLIT_POINTS - 1;
        let mut curve: Vec<Option<[f64; 3]>> = vec![None; SPLIT_POINTS];
        curve[0] = Some([0.0; 3]);
        for (i, point) in curve.iter_mut().enumerate().skip(1) {
            let t = i as f64 / last as f64;
            let (mut wsum, mut acc) = (0.0, [0.0; 3]);
            for p in &pts {
                let d = (p.0 - t) / SPLIT_KERNEL;
                if d.abs() < 3.0 {
                    let w = p.2 * (-0.5 * d * d).exp();
                    wsum += w;
                    acc.iter_mut().zip(p.1).for_each(|(a, v)| *a += w * v);
                }
            }
            if wsum > 0.5 {
                let mean = acc.map(|a| a / wsum);
                let vol = (mean[0] + 2.0 * mean[1] + 3.0 * mean[2]) / 3.0;
                if vol > 0.0 {
                    *point = Some(mean.map(|v| v * t / vol));
                }
            }
        }
        if curve[last].is_none() {
            curve[last] = Some([0.0, 0.0, 1.0]);
        }
        // straight lines between measured points
        let mut prev = 0;
        for i in 1..SPLIT_POINTS {
            if let Some(b) = curve[i] {
                let a = curve[prev].unwrap_or_default();
                for (j, point) in curve.iter_mut().enumerate().take(i).skip(prev + 1) {
                    let f = (j - prev) as f64 / (i - prev) as f64;
                    *point = Some(std::array::from_fn(|k| a[k] * (1.0 - f) + b[k] * f));
                }
                prev = i;
            }
        }
        curve
            .into_iter()
            .map(|p| {
                let mut p = p.unwrap_or_default().map(|v| v.max(0.0));
                let sum: f64 = p.iter().sum();
                if sum > 1.0 {
                    p.iter_mut().for_each(|v| *v /= sum);
                }
                let mut d = p.map(to_u16);
                while d.iter().map(|&v| u32::from(v)).sum::<u32>() > 65535 {
                    let k = (0..3).max_by_key(|&k| d[k]).unwrap_or(0);
                    d[k] -= 1;
                }
                d
            })
            .collect()
    })
}

fn grid_index(n: usize, rgb: [u8; 3]) -> Option<usize> {
    let levels: Vec<u8> = (0..n)
        .map(|i| ((i * 255 + (n - 1) / 2) / (n - 1)) as u8)
        .collect();
    match rgb.map(|v| levels.iter().position(|&l| l == v)) {
        [Some(r), Some(g), Some(b)] => Some((r * n + g) * n + b),
        _ => None,
    }
}

/// Σ wᵢ·xᵢ over (node, wᵢ) = target amounts, with a weight.
type Equation = (Vec<(usize, f64)>, [f64; 4], f64);

/// Weighted least squares for the nodes without a value in `known`: each equation is
/// (Σ wᵢ·xᵢ = target, weight); known nodes move to the right-hand side.
fn least_squares(known: &[Option<[f64; 4]>], equations: &[Equation]) -> Option<Vec<[f64; 4]>> {
    let m = known.len();
    let unknown: Vec<usize> = (0..m).filter(|&i| known[i].is_none()).collect();
    let mut row = vec![usize::MAX; m];
    for (k, &i) in unknown.iter().enumerate() {
        row[i] = k;
    }
    let u = unknown.len();
    let mut a = vec![0f64; u * u];
    let mut rhs = vec![[0f64; 4]; u];
    for (terms, target, w) in equations {
        let mut t = *target;
        let mut unk = Vec::with_capacity(terms.len());
        for &(i, wi) in terms {
            match known[i] {
                Some(f) => t.iter_mut().zip(f).for_each(|(x, y)| *x -= wi * y),
                None => unk.push((row[i], wi)),
            }
        }
        for &(i, wi) in &unk {
            for &(j, wj) in &unk {
                a[i * u + j] += w * wi * wj;
            }
            for (r, y) in rhs[i].iter_mut().zip(&t) {
                *r += w * wi * y;
            }
        }
    }
    // keeps the system positive definite even for a node no equation reaches
    for i in 0..u {
        a[i * u + i] += 1e-9;
    }
    let x = cholesky_solve(a, rhs, u)?;
    Some(
        (0..m)
            .map(|i| known[i].unwrap_or_else(|| x[row[i]]))
            .collect(),
    )
}

/// Amounts on the chart grid: printed nodes as measured, the others solved from the colours around
/// them and the smoothness term.
fn chart_amounts(samples: &[Sample]) -> Option<Vec<[f64; 4]>> {
    let n = CHART_GRID;
    let mut known = vec![None; n * n * n];
    for s in samples {
        if let Some(i) = grid_index(n, s.rgb) {
            known[i] = Some(s.amounts());
        }
    }
    let mut eq: Vec<Equation> = samples
        .iter()
        .map(|s| {
            let terms = tetra_weights(n, s.rgb)
                .map(|(i, w)| (i, f64::from(w)))
                .to_vec();
            (terms, s.amounts(), s.weight())
        })
        .collect();
    let idx = |r: usize, g: usize, b: usize| (r * n + g) * n + b;
    for r in 0..n {
        for g in 0..n {
            for b in 0..n {
                let c = idx(r, g, b);
                let axes = [
                    (r > 0 && r + 1 < n).then(|| (idx(r - 1, g, b), idx(r + 1, g, b))),
                    (g > 0 && g + 1 < n).then(|| (idx(r, g - 1, b), idx(r, g + 1, b))),
                    (b > 0 && b + 1 < n).then(|| (idx(r, g, b - 1), idx(r, g, b + 1))),
                ];
                for (lo, hi) in axes.into_iter().flatten() {
                    eq.push((vec![(lo, 1.0), (c, -2.0), (hi, 1.0)], [0.0; 4], SMOOTHNESS));
                }
            }
        }
    }
    least_squares(&known, &eq)
}

/// The table from measured colours: chart-grid amounts, refined on the finer table grid where colours
/// between the chart levels (greys, ramps, mixtures) were measured, and the measured split curves.
pub fn fit_samples(samples: &[Sample]) -> Option<Lut> {
    let coarse = chart_amounts(samples)?;
    let cn = CHART_GRID;
    let n = TABLE_GRID;
    let coarse_at = |rgb: [u8; 3]| -> [f64; 4] {
        let mut out = [0.0; 4];
        for (i, w) in tetra_weights(cn, rgb) {
            out.iter_mut()
                .zip(coarse[i])
                .for_each(|(o, v)| *o += f64::from(w) * v);
        }
        out
    };
    let levels: Vec<u8> = (0..n)
        .map(|i| ((i * 255 + (n - 1) / 2) / (n - 1)) as u8)
        .collect();
    let rgb_of = |i: usize| [levels[i / (n * n)], levels[i / n % n], levels[i % n]];
    // a fine node stays at the coarse value unless a measured colour touches it
    let mut touched = vec![false; n * n * n];
    let eq_samples: Vec<_> = samples
        .iter()
        .map(|s| {
            let terms: Vec<(usize, f64)> = tetra_weights(n, s.rgb)
                .map(|(i, w)| (i, f64::from(w)))
                .to_vec();
            for &(i, w) in &terms {
                if w > 0.0 {
                    touched[i] = true;
                }
            }
            (terms, s.amounts(), s.weight())
        })
        .collect();
    let mut known: Vec<Option<[f64; 4]>> = (0..n * n * n)
        .map(|i| (!touched[i]).then(|| coarse_at(rgb_of(i))))
        .collect();
    for s in samples {
        if grid_index(cn, s.rgb).is_some() {
            if let Some(i) = grid_index(n, s.rgb) {
                known[i] = Some(s.amounts());
            }
        }
    }
    let mut eq = eq_samples;
    for (i, k) in known.iter().enumerate() {
        if k.is_none() {
            eq.push((vec![(i, 1.0)], coarse_at(rgb_of(i)), PRIOR));
        }
    }
    let mut x = least_squares(&known, &eq)?;
    // an ink measured as unused at a node stays unused (no stray dots on white or in pure colours)
    for s in samples {
        if let Some(i) = grid_index(n, s.rgb) {
            for (v, a) in x[i].iter_mut().zip(s.amounts()) {
                if a == 0.0 {
                    *v = 0.0;
                }
            }
        }
    }
    let amounts = x
        .iter()
        .map(|a| a.map(|v| if v < MIN_AMOUNT { 0 } else { to_u16(v) }))
        .collect();
    Lut::new(n, amounts, split_curves(samples))
}

/// A colour-mode table for a coating/quality that Windows was only captured with in Preferred mode:
/// `base` (that coating/quality, Preferred) plus the mode's change measured on the reference
/// coating/quality (`mode` − `pref`, node by node), scaled by the ratio of total ink between `base`
/// and `pref`, capped at the base's ink limit (scaled as the mode changes the reference's limit).
/// Keeps the base's split curves. All three tables must share a grid.
pub fn derive_mode(base: &Lut, pref: &Lut, mode: &Lut) -> Option<Lut> {
    let n = base.grid();
    if pref.grid() != n || mode.grid() != n {
        return None;
    }
    let levels: Vec<u8> = (0..n)
        .map(|i| ((i * 255 + (n - 1) / 2) / (n - 1)) as u8)
        .collect();
    let nodes: Vec<[u8; 3]> = (0..n * n * n)
        .map(|i| [levels[i / (n * n)], levels[i / n % n], levels[i % n]])
        .collect();
    let total = |l: &Lut| -> f64 {
        nodes
            .iter()
            .flat_map(|&c| l.amounts(c))
            .map(f64::from)
            .sum()
    };
    let scale = total(base) / total(pref).max(1.0);
    // ink limit: the base coating's densest node, raised or lowered as the mode changes the
    // reference's densest node (clamping an ink at zero would otherwise add ink)
    let max_total = |l: &Lut| -> f64 {
        nodes
            .iter()
            .map(|&c| l.amounts(c).iter().map(|&v| f64::from(v)).sum::<f64>())
            .fold(0.0, f64::max)
    };
    let limit = max_total(base) * max_total(mode) / max_total(pref).max(1.0);
    let amounts = nodes
        .iter()
        .map(|&c| {
            let (b, p, m) = (base.amounts(c), pref.amounts(c), mode.amounts(c));
            let v: [f64; 4] = std::array::from_fn(|i| {
                (f64::from(b[i]) + scale * (f64::from(m[i]) - f64::from(p[i]))).clamp(0.0, 65535.0)
            });
            let sum: f64 = v.iter().sum();
            let k = if sum > limit { limit / sum } else { 1.0 };
            v.map(|x| (x * k).round().min(65535.0) as u16)
        })
        .collect();
    Lut::new(
        n,
        amounts,
        std::array::from_fn(|i| base.split_curve(i).to_vec()),
    )
}

/// Solves A·x = b for symmetric positive definite A (row-major m×m), 4 right-hand sides.
fn cholesky_solve(mut a: Vec<f64>, mut b: Vec<[f64; 4]>, m: usize) -> Option<Vec<[f64; 4]>> {
    for j in 0..m {
        let mut d = a[j * m + j];
        for k in 0..j {
            d -= a[j * m + k] * a[j * m + k];
        }
        if d <= 0.0 {
            return None;
        }
        let d = d.sqrt();
        a[j * m + j] = d;
        for i in j + 1..m {
            let mut s = a[i * m + j];
            for k in 0..j {
                s -= a[i * m + k] * a[j * m + k];
            }
            a[i * m + j] = s / d;
        }
    }
    for i in 0..m {
        for k in 0..i {
            let l = a[i * m + k];
            let bk = b[k];
            for (x, y) in b[i].iter_mut().zip(bk) {
                *x -= l * y;
            }
        }
        let d = a[i * m + i];
        b[i].iter_mut().for_each(|x| *x /= d);
    }
    for i in (0..m).rev() {
        for k in i + 1..m {
            let l = a[k * m + i];
            let bk = b[k];
            for (x, y) in b[i].iter_mut().zip(bk) {
                *x -= l * y;
            }
        }
        let d = a[i * m + i];
        b[i].iter_mut().for_each(|x| *x /= d);
    }
    Some(b)
}

fn frac_err(a: &[Drops; 4], b: &[f64; 12]) -> f64 {
    a.iter()
        .flatten()
        .zip(b)
        .map(|(x, y)| (f64::from(*x) / 65535.0 - y).abs())
        .fold(0.0, f64::max)
}

/// Fit the table from a job whose first page is `img` printed at the page origin.
pub fn fit(job: &[u8], img: &Image, layout: &Layout) -> Result<(Lut, Report), String> {
    let samples = samples(job, img)?;
    let cube = layout.patches.iter().filter(|p| p.kind == "cube").count();
    if cube != CHART_GRID.pow(3) {
        return Err(format!(
            "the layout has {cube} cube patches, not {}",
            CHART_GRID.pow(3)
        ));
    }
    if samples
        .iter()
        .all(|s| s.fractions.iter().all(|&f| f == 0.0))
    {
        return Err("no ink found where the chart should be (wrong job or image?)".into());
    }
    let lut = fit_samples(&samples).ok_or("fit failed (singular system)")?;
    let by_rgb: HashMap<[u8; 3], &Sample> = samples.iter().map(|s| (s.rgb, s)).collect();
    let mut report = Report::default();
    let direct = layout
        .patches
        .iter()
        .filter(|p| p.kind == "cube" && by_rgb.contains_key(&p.rgb))
        .count();
    report.lines.push(format!(
        "{} colours measured; {direct} of {cube} chart nodes printed, the rest inferred",
        samples.len()
    ));
    for p in &layout.patches {
        let Some(s) = by_rgb.get(&p.rgb) else {
            if p.kind != "cube" {
                report
                    .lines
                    .push(format!("{:8} {:3?}  not measured", p.kind, p.rgb));
            }
            continue;
        };
        if p.kind == "grey" || p.kind.starts_with("ramp_") {
            let e = frac_err(&lut.lookup(p.rgb), &s.fractions);
            report.max_ramp_error = report.max_ramp_error.max(e);
            report
                .lines
                .push(format!("{:8} {:3?}  table error {:.3}", p.kind, p.rgb, e));
        } else if let Some(name) = p.kind.strip_prefix("brand:") {
            report.lines.push(format!(
                "brand {name} {:3?}  measured K,C,M,Y [small,medium,large] = {:?}",
                p.rgb,
                s.fractions.map(to_u16)
            ));
        }
    }
    Ok((lut, report))
}
