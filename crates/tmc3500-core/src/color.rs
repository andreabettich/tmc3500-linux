//! RGB → ink amounts (K, C, M, Y; 0..=65535) → drop fractions.
//!
//! The conversion uses a colour table fitted to Windows colour-chart captures (`Lut`, one per
//! coating/quality/mode); without one, a provisional formula with grey-component replacement and the
//! Windows "rich black". Brightness and the ink profile are applied on top, both calibrated against
//! Windows captures; K-only black bypasses the table.
use crate::options::{BlackMode, ColorMode, ColorSettings, Gamma};

/// Fractions of dots (0..=65535 = 100 %) that get a small, medium and large drop.
pub type Drops = [u16; 3];

/// Provisional mapping of an ink amount (0..=65535 = all large drops) to drop fractions: every dot
/// gets one of two neighbouring drop sizes.
pub fn amount_to_drops(a: u16) -> Drops {
    let v = u32::from(a) * 3;
    let (lo, frac) = (v / 65535, (v % 65535) as u16);
    match lo {
        0 => [frac, 0, 0],
        1 => [65535 - frac, frac, 0],
        2 => [0, 65535 - frac, frac],
        _ => [0, 0, 65535],
    }
}

/// Points of a split curve: amounts 0, 1/384, …, 1 (the provisional kinks at 1/3 and 2/3 are points).
pub const SPLIT_POINTS: usize = 385;

/// The provisional split (`amount_to_drops`) as a curve.
pub fn provisional_split() -> Vec<Drops> {
    (0..SPLIT_POINTS)
        .map(|i| amount_to_drops(((i * 65535) / (SPLIT_POINTS - 1)) as u16))
        .collect()
}

/// Colour table in two stages, like the Windows driver:
/// - an n×n×n grid of RGB nodes at the chart levels round(i·255/(n−1)), each holding an ink amount
///   per ink (K, C, M, Y; 65535 = every dot a large drop), tetrahedral interpolation in between;
/// - per ink, a split curve from amount to the fractions of small, medium and large drops
///   (`SPLIT_POINTS` points, linear in between).
///
/// The amount is (small + 2·medium + 3·large) / 3: the Windows captures show each ink's split is a
/// function of it alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lut {
    n: usize,
    levels: Vec<u8>,
    amounts: Vec<[u16; 4]>,
    split: [Vec<Drops>; 4],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LutError(pub &'static str);

impl std::fmt::Display for LutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bad colour table: {}", self.0)
    }
}

impl std::error::Error for LutError {}

const LUT_MAGIC: &[u8; 7] = b"TMCLUT2";

fn drop_sum(d: &Drops) -> u32 {
    d.iter().map(|&f| u32::from(f)).sum()
}

impl Lut {
    /// `amounts` in r-major, b-minor order. `None` if a size is wrong or a split point exceeds 100 %.
    pub fn new(n: usize, amounts: Vec<[u16; 4]>, split: [Vec<Drops>; 4]) -> Option<Self> {
        if !(2..=33).contains(&n) || amounts.len() != n * n * n {
            return None;
        }
        if split
            .iter()
            .any(|c| c.len() != SPLIT_POINTS || c.iter().any(|d| drop_sum(d) > 65535))
        {
            return None;
        }
        Some(Self {
            n,
            levels: grid_levels(n),
            amounts,
            split,
        })
    }

    /// Table from drop fractions per node, split with the provisional curve (tests and tools).
    pub fn from_nodes(n: usize, nodes: Vec<[Drops; 4]>) -> Option<Self> {
        if nodes.iter().flatten().any(|d| drop_sum(d) > 65535) {
            return None;
        }
        let amounts = nodes
            .iter()
            .map(|node| {
                node.map(|[s, m, l]| {
                    ((u32::from(s) + 2 * u32::from(m) + 3 * u32::from(l) + 1) / 3) as u16
                })
            })
            .collect();
        Self::new(n, amounts, std::array::from_fn(|_| provisional_split()))
    }

    pub fn parse(b: &[u8]) -> Result<Self, LutError> {
        let rest = b.strip_prefix(&LUT_MAGIC[..]).ok_or(LutError("magic"))?;
        let (&n, data) = rest.split_first().ok_or(LutError("truncated"))?;
        let n = usize::from(n);
        let grid = n * n * n * 8;
        if !(2..=33).contains(&n) || data.len() != grid + 4 * SPLIT_POINTS * 6 {
            return Err(LutError("size"));
        }
        let u16s = |d: &[u8]| -> Vec<u16> {
            d.as_chunks::<2>()
                .0
                .iter()
                .map(|v| u16::from_le_bytes(*v))
                .collect()
        };
        let amounts = u16s(&data[..grid]).as_chunks::<4>().0.to_vec();
        let curves = u16s(&data[grid..]);
        let points = curves.as_chunks::<3>().0;
        let split = std::array::from_fn(|ink| {
            points[ink * SPLIT_POINTS..(ink + 1) * SPLIT_POINTS].to_vec()
        });
        Self::new(n, amounts, split).ok_or(LutError("split above 100 %"))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = LUT_MAGIC.to_vec();
        out.push(self.n as u8);
        let values = self
            .amounts
            .iter()
            .flatten()
            .chain(self.split.iter().flatten().flatten());
        for v in values {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    /// Grid levels per axis.
    pub fn grid(&self) -> usize {
        self.n
    }

    /// The split curve of one ink (`SPLIT_POINTS` points).
    pub fn split_curve(&self, ink: usize) -> &[Drops] {
        &self.split[ink]
    }

    /// Ink amounts `[K, C, M, Y]` (65535 = every dot a large drop).
    pub fn amounts(&self, rgb: [u8; 3]) -> [u16; 4] {
        let mut out = [0f32; 4];
        for (i, w) in weights_on(&self.levels, rgb) {
            for (o, &v) in out.iter_mut().zip(&self.amounts[i]) {
                *o += w * f32::from(v);
            }
        }
        out.map(|x| x.round().clamp(0.0, 65535.0) as u16)
    }

    /// Drop fractions of one ink for an amount.
    pub fn split(&self, ink: usize, amount: u16) -> Drops {
        let c = &self.split[ink];
        let x = u32::from(amount) * (SPLIT_POINTS as u32 - 1);
        let (i, f) = ((x / 65535) as usize, (x % 65535) as f32 / 65535.0);
        let (a, b) = (c[i], c[(i + 1).min(SPLIT_POINTS - 1)]);
        let mut d: Drops = std::array::from_fn(|k| {
            (f32::from(a[k]) * (1.0 - f) + f32::from(b[k]) * f).round() as u16
        });
        // interpolating two points at most 100 % stays within 100 % up to rounding
        while drop_sum(&d) > 65535 {
            let k = (0..3).max_by_key(|&k| d[k]).unwrap_or(0);
            d[k] -= 1;
        }
        d
    }

    /// Drop fractions `[small, medium, large]` per ink `[K, C, M, Y]`.
    pub fn lookup(&self, rgb: [u8; 3]) -> [Drops; 4] {
        let a = self.amounts(rgb);
        std::array::from_fn(|ink| self.split(ink, a[ink]))
    }
}

/// Grid levels round(i·255/(n−1)).
fn grid_levels(n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| ((i * 255 + (n - 1) / 2) / (n - 1)) as u8)
        .collect()
}

/// Segment index and position (0..=1) of an 8-bit value on the level grid.
fn locate(levels: &[u8], v: u8) -> (usize, f32) {
    let n = levels.len();
    let i = levels.partition_point(|&l| l <= v).clamp(1, n - 1) - 1;
    let (a, b) = (levels[i], levels[i + 1]);
    (
        i,
        f32::from(v.saturating_sub(a)) / f32::from(b - a).max(1.0),
    )
}

/// The four (node index, weight) pairs of tetrahedral interpolation on an n×n×n grid (node index
/// r-major, b-minor). `Lut::lookup` is the weighted sum of these nodes.
pub fn tetra_weights(n: usize, rgb: [u8; 3]) -> [(usize, f32); 4] {
    weights_on(&grid_levels(n), rgb)
}

fn weights_on(levels: &[u8], rgb: [u8; 3]) -> [(usize, f32); 4] {
    let n = levels.len();
    let [(ri, fr), (gi, fg), (bi, fb)] = rgb.map(|v| locate(levels, v));
    let idx = |(r, g, b): (usize, usize, usize)| ((ri + r) * n + gi + g) * n + bi + b;
    // tetrahedron containing the point: walk from c000 to c111 along the largest fractions first
    let mut steps = [(fr, (1, 0, 0)), (fg, (0, 1, 0)), (fb, (0, 0, 1))];
    steps.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out = [(0, 0f32); 4];
    let mut pos = (0, 0, 0);
    let mut last_f = 1.0;
    for (k, (f, (dr, dg, db))) in steps.into_iter().enumerate() {
        out[k] = (idx(pos), last_f - f);
        pos = (pos.0 + dr, pos.1 + dg, pos.2 + db);
        last_f = f;
    }
    out[3] = (idx(pos), last_f);
    out
}

/// Ink profile factor for a slider step (0 … −6) at an ink amount (0..=1), measured from the
/// Windows driver (captures 404-407): about −4.7 % per step, a little less for dense ink.
fn ink_factor(step: i8, amount: f32) -> f32 {
    1.0 + f32::from(step) * (0.055 - 0.022 * amount)
}

/// Brightness as a tone curve on each input channel (Windows captures 408/409; validated on 724).
fn brightness_curve(b: i8) -> [u8; 256] {
    std::array::from_fn(|v| {
        let x = v as f32 / 255.0;
        let y = match b {
            0 => x,
            b if b > 0 => 1.0 - (1.0 - x).powf(1.0 + 0.245 * f32::from(b)),
            b => x.powf(1.0 - 0.315 * f32::from(b)),
        };
        (y * 255.0).round().clamp(0.0, 255.0) as u8
    })
}

/// Converts one RGB pixel to ink amounts and drop fractions.
#[derive(Debug, Clone)]
pub struct Converter {
    /// Fitted table; `None` = provisional formula.
    lut: Option<Lut>,
    /// Provisional formula: 8-bit value → darkness 0..=1 (0 = white).
    darkness: [f32; 256],
    rich_black: bool,
    /// Brightness tone curve applied to the input.
    tone: [u8; 256],
    /// Ink profile steps (0 … −6): all inks, and black again.
    bkcmy: i8,
    bk: i8,
}

impl Converter {
    pub fn new(s: &ColorSettings, black: BlackMode) -> Self {
        let exponent = match (s.mode, s.gamma) {
            (ColorMode::Off, _) | (_, Gamma::G22) => 1.0,
            (_, Gamma::G18) => 1.8 / 2.2,
        };
        let mut darkness = [0f32; 256];
        for (v, d) in darkness.iter_mut().enumerate() {
            *d = 1.0 - (v as f32 / 255.0).powf(exponent);
        }
        // without TmInkAdjust the adjustment values are ignored, as in the Windows dialog
        let (b, bkcmy, bk) = if s.ink_adjust {
            (s.brightness, s.ink_bkcmy, s.ink_bk)
        } else {
            (0, 0, 0)
        };
        Self {
            lut: None,
            darkness,
            rich_black: black == BlackMode::Rich,
            tone: brightness_curve(b),
            bkcmy,
            bk,
        }
    }

    /// Converter using a fitted table (ink adjustment and brightness are applied on top).
    /// With K-only black the table is not used (tables are fitted from Windows' rich black): the
    /// K-only formula keeps barcodes and text pure black.
    pub fn with_lut(s: &ColorSettings, black: BlackMode, lut: Lut) -> Self {
        Self {
            lut: (black == BlackMode::Rich).then_some(lut),
            ..Self::new(s, black)
        }
    }

    /// True when a fitted colour table is in use.
    pub fn uses_table(&self) -> bool {
        self.lut.is_some()
    }

    /// Ink amounts `[K, C, M, Y]` (65535 = every dot a large drop), after brightness and ink profile.
    pub fn amounts(&self, rgb: [u8; 3]) -> [u16; 4] {
        let rgb = rgb.map(|v| self.tone[usize::from(v)]);
        let base = match &self.lut {
            Some(lut) => lut.amounts(rgb),
            None => self.provisional(rgb),
        };
        if self.bkcmy == 0 && self.bk == 0 {
            return base;
        }
        std::array::from_fn(|i| {
            let a = f32::from(base[i]) / 65535.0;
            let mut f = ink_factor(self.bkcmy, a);
            if i == 0 {
                f *= ink_factor(self.bk, a);
            }
            (f32::from(base[i]) * f).round().clamp(0.0, 65535.0) as u16
        })
    }

    /// Drop-size fractions `[small, medium, large]` per ink `[K, C, M, Y]`.
    pub fn drops(&self, rgb: [u8; 3]) -> [Drops; 4] {
        let a = self.amounts(rgb);
        match &self.lut {
            Some(lut) => std::array::from_fn(|i| lut.split(i, a[i])),
            None => a.map(amount_to_drops),
        }
    }

    /// Provisional formula (no table): grey-component replacement and the Windows rich black.
    fn provisional(&self, rgb: [u8; 3]) -> [u16; 4] {
        let [c, m, y] = rgb.map(|v| self.darkness[usize::from(v)]);
        let k = c.min(m).min(y);
        // under-colour added to the CMY of dark neutrals (Windows prints black as rich black)
        let under = if self.rich_black {
            0.55 * k * k * k
        } else {
            0.0
        };
        let q = |a: f32| (a.clamp(0.0, 1.0) * 65535.0).round() as u16;
        [q(k), q(c - k + under), q(m - k + under), q(y - k + under)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::JobConfig;

    fn conv(o: &str) -> Converter {
        let c = JobConfig::from_cups(o, 1).unwrap();
        Converter::new(&c.color, c.black)
    }

    #[test]
    fn white_is_no_ink_and_black_is_rich_black() {
        let c = conv("");
        assert_eq!(c.amounts([255, 255, 255]), [0, 0, 0, 0]);
        let [k, cy, m, y] = c.amounts([0, 0, 0]);
        assert_eq!(k, 65535);
        assert!(cy > 20000 && m > 20000 && y > 20000, "rich black adds CMY");
    }

    #[test]
    fn k_only_black_has_no_colour_ink_on_neutrals() {
        let c = conv("TmBlack=KOnly");
        assert_eq!(c.amounts([0, 0, 0]), [65535, 0, 0, 0]);
        let [k, cy, m, y] = c.amounts([128, 128, 128]);
        assert!(k > 0 && cy == 0 && m == 0 && y == 0);
    }

    #[test]
    fn primaries_use_their_inks() {
        let c = conv("");
        assert_eq!(c.amounts([0, 255, 255]), [0, 65535, 0, 0]);
        assert_eq!(c.amounts([255, 0, 255]), [0, 0, 65535, 0]);
        assert_eq!(c.amounts([255, 255, 0]), [0, 0, 0, 65535]);
    }

    #[test]
    fn grey_ramp_is_monotonic() {
        let c = conv("");
        let mut prev = 0u32;
        for v in (0..=255u8).rev() {
            let total: u32 = c.amounts([v, v, v]).iter().map(|&a| u32::from(a)).sum();
            assert!(
                total >= prev,
                "ink must not decrease as grey darkens (v={v})"
            );
            prev = total;
        }
    }

    #[test]
    fn brightness_and_ink_adjust_reduce_ink() {
        let base: u32 = conv("")
            .amounts([100, 100, 100])
            .iter()
            .map(|&a| u32::from(a))
            .sum();
        let bright: u32 = conv("TmInkAdjust=True TmBrightness=2")
            .amounts([100, 100, 100])
            .iter()
            .map(|&a| u32::from(a))
            .sum();
        let less: u32 = conv("TmInkAdjust=True TmInkBkCMY=-6")
            .amounts([100, 100, 100])
            .iter()
            .map(|&a| u32::from(a))
            .sum();
        assert!(bright < base && less < base);
        // without TmInkAdjust the adjustment values are ignored, as in the Windows dialog
        let ignored: u32 = conv("TmInkBkCMY=-6")
            .amounts([100, 100, 100])
            .iter()
            .map(|&a| u32::from(a))
            .sum();
        assert_eq!(ignored, base);
    }

    fn node_lut(n: usize, f: impl Fn(usize, usize, usize) -> [Drops; 4]) -> Lut {
        let mut nodes = Vec::new();
        for r in 0..n {
            for g in 0..n {
                for b in 0..n {
                    nodes.push(f(r, g, b));
                }
            }
        }
        Lut::from_nodes(n, nodes).unwrap()
    }

    #[test]
    fn provisional_drops_follow_the_amount() {
        assert_eq!(amount_to_drops(0), [0, 0, 0]);
        assert_eq!(amount_to_drops(65535), [0, 0, 65535]);
        let [s, m, l] = amount_to_drops(32768); // 1.5 drop units: half small, half medium
        assert!(l == 0 && (s as i32 - m as i32).abs() <= 3 && s + m > 65530);
    }

    /// Table with amounts `f(r, g, b)` per ink and the provisional split.
    fn amount_lut(n: usize, f: impl Fn(usize, usize, usize) -> [u16; 4]) -> Lut {
        let mut amounts = Vec::new();
        for r in 0..n {
            for g in 0..n {
                for b in 0..n {
                    amounts.push(f(r, g, b));
                }
            }
        }
        Lut::new(n, amounts, std::array::from_fn(|_| provisional_split())).unwrap()
    }

    #[test]
    fn lut_amounts_are_exact_at_the_chart_levels_and_linear_between() {
        let lut = amount_lut(9, |r, g, b| {
            [(r * 6000) as u16, (g * 6000) as u16, (b * 6000) as u16, 0]
        });
        let lv = |i: usize| (i * 255 + 4) / 8; // chart levels round(i*255/8)
        for (r, g, b) in [(0, 0, 0), (8, 8, 8), (3, 5, 1), (1, 7, 4)] {
            let got = lut.amounts([lv(r) as u8, lv(g) as u8, lv(b) as u8]);
            assert_eq!(
                got,
                [(r * 6000) as u16, (g * 6000) as u16, (b * 6000) as u16, 0]
            );
        }
        // halfway between red levels 1 and 2 (32 and 64): 9000
        assert_eq!(lut.amounts([48, 0, 0])[0], 9000);
    }

    #[test]
    fn lut_splits_each_amount_with_the_ink_curve() {
        // ink 0: provisional split; ink 1: all small drops up to amount 1/3, a custom curve
        let mut split: [Vec<Drops>; 4] = std::array::from_fn(|_| provisional_split());
        split[1] = (0..SPLIT_POINTS)
            .map(|i| [((i * 65535) / (SPLIT_POINTS - 1)) as u16 / 3, 0, 0])
            .collect();
        let lut = Lut::new(2, vec![[65535 / 2, 65535, 0, 0]; 8], split).unwrap();
        let d = lut.lookup([0, 0, 0]);
        let want = amount_to_drops(65535 / 2);
        assert!(
            d[0].iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 2),
            "{:?}",
            d[0]
        );
        assert_eq!(d[1], [21845, 0, 0]);
        assert_eq!(d[2], [0, 0, 0]);
    }

    #[test]
    fn tetrahedral_weights_reproduce_the_lookup() {
        let f = |i: usize| {
            let (r, g, b) = (i / 81, i / 9 % 9, i % 9);
            [
                (r * 977 % 4000) as u16,
                (g * 3001 % 5000) as u16,
                (b * 7) as u16,
                65535,
            ]
        };
        let lut = amount_lut(9, |r, g, b| f((r * 9 + g) * 9 + b));
        for rgb in [
            [0, 0, 0],
            [255, 255, 255],
            [48, 200, 7],
            [131, 12, 250],
            [33, 33, 34],
        ] {
            let w = tetra_weights(9, rgb);
            assert!((w.iter().map(|x| x.1).sum::<f32>() - 1.0).abs() < 1e-5);
            let want = lut.amounts(rgb);
            for (ink, &want) in want.iter().enumerate() {
                let got: f32 = w.iter().map(|&(i, x)| x * f32::from(f(i)[ink])).sum();
                assert!((got - f32::from(want)).abs() <= 1.0, "{rgb:?}");
            }
        }
    }

    #[test]
    fn from_nodes_keeps_drops_on_the_provisional_split() {
        // all large, all medium (both on the provisional curve), and nothing
        let lut = node_lut(2, |r, g, _| match (r, g) {
            (0, 0) => [[0, 0, 65535]; 4],
            (0, 1) => [[0, 65535, 0]; 4],
            _ => [[0; 3]; 4],
        });
        assert_eq!(lut.lookup([0, 0, 0])[1], [0, 0, 65535]);
        assert_eq!(lut.lookup([0, 255, 0])[1], [0, 65535, 0]);
        assert_eq!(lut.lookup([255, 0, 0])[1], [0, 0, 0]);
    }

    #[test]
    fn lut_round_trips_and_rejects_corrupt_data() {
        let lut = amount_lut(3, |r, g, b| [(r + g + b) as u16 * 100, 7, 9, 65535]);
        let bytes = lut.to_bytes();
        assert!(bytes.starts_with(b"TMCLUT2"));
        assert_eq!(Lut::parse(&bytes).unwrap(), lut);
        assert!(Lut::parse(&bytes[..bytes.len() - 1]).is_err());
        assert!(Lut::parse(b"TMCLUT2").is_err());
        let mut bad = bytes.clone();
        bad[7] = 1; // grid size 1
        assert!(Lut::parse(&bad).is_err());
        let mut v1 = bytes.clone();
        v1[6] = b'1';
        assert!(Lut::parse(&v1).is_err(), "old format");
        // a split point above 100 %
        let mut split: [Vec<Drops>; 4] = std::array::from_fn(|_| provisional_split());
        split[2][100] = [40000, 30000, 0];
        assert!(Lut::new(2, vec![[0; 4]; 8], split).is_none());
        assert!(Lut::new(
            2,
            vec![[0; 4]; 7],
            std::array::from_fn(|_| provisional_split())
        )
        .is_none());
    }

    #[test]
    fn converter_uses_the_table() {
        let lut = node_lut(2, |r, _, _| {
            if r == 0 {
                [[0, 0, 65535]; 4]
            } else {
                [[0; 3]; 4]
            }
        });
        let c = JobConfig::default();
        let conv = Converter::with_lut(&c.color, c.black, lut);
        assert_eq!(conv.drops([0, 0, 0])[1], [0, 0, 65535]);
        assert_eq!(conv.drops([255, 0, 0]), [[0; 3]; 4]);
    }

    fn table_conv(opts: &str, lut: Lut) -> Converter {
        let c = JobConfig::from_cups(opts, 1).unwrap();
        Converter::with_lut(&c.color, c.black, lut)
    }

    #[test]
    fn ink_profile_scales_amounts_like_windows() {
        // Windows (captures 404-407): factor 1 + step·(0.055 − 0.022·amount); BkCMY scales all four
        // inks, Bk scales black again
        let lut = amount_lut(2, |_, _, _| [32768; 4]);
        let c = table_conv("TmInkAdjust=True TmInkBkCMY=-3", lut.clone());
        assert_eq!(c.amounts([0, 0, 0]), [28443; 4]);
        let c = table_conv("TmInkAdjust=True TmInkK=-3", lut.clone());
        assert_eq!(c.amounts([0, 0, 0]), [28443, 32768, 32768, 32768]);
        let c = table_conv("TmInkAdjust=True TmInkBkCMY=-3 TmInkK=-3", lut.clone());
        assert_eq!(c.amounts([0, 0, 0]), [24688, 28443, 28443, 28443]);
        // the adjusted amount is split into drop sizes by the table's curve
        assert_eq!(c.drops([0, 0, 0])[1], lut.split(1, 28443));
        // ignored without TmInkAdjust, as in the Windows dialog
        let c = table_conv("TmInkBkCMY=-3", lut);
        assert_eq!(c.amounts([0, 0, 0]), [32768; 4]);
    }

    #[test]
    fn brightness_is_a_tone_curve_on_the_input() {
        // Windows (captures 408/409): +b → 1 − (1 − v)^(1 + 0.245·b); −b → v^(1 + 0.315·b)
        let lut = amount_lut(9, |r, g, b| {
            [(r * 8000) as u16, (g * 8000) as u16, (b * 8000) as u16, 0]
        });
        let c = table_conv("TmInkAdjust=True TmBrightness=2", lut.clone());
        assert_eq!(c.amounts([200, 200, 200]), lut.amounts([229, 229, 229]));
        let c = table_conv("TmInkAdjust=True TmBrightness=-2", lut.clone());
        assert_eq!(c.amounts([200, 200, 200]), lut.amounts([172, 172, 172]));
        assert_eq!(
            c.amounts([255, 0, 255]),
            lut.amounts([255, 0, 255]),
            "ends fixed"
        );
    }

    #[test]
    fn k_only_black_is_kept_when_a_table_is_installed() {
        // tables are fitted from Windows' rich black; K-only (barcodes) must not silently use them
        let lut = node_lut(2, |_, _, _| [[0, 0, 30000]; 4]);
        let c = JobConfig::from_cups("TmBlack=KOnly", 1).unwrap();
        let conv = Converter::with_lut(&c.color, c.black, lut);
        assert_eq!(
            conv.drops([0, 0, 0]),
            [[0, 0, 65535], [0; 3], [0; 3], [0; 3]]
        );
        assert!(!conv.uses_table());
    }
}
