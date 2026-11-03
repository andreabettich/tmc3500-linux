//! RGB → ink amounts (K, C, M, Y; 0..=65535).
//!
//! PROVISIONAL: a simple model with grey-component replacement and the Windows "rich black". Phase 3
//! replaces the core conversion with 3-D tables fitted to Windows colour-chart captures; the option
//! handling (gamma, brightness, ink adjustment, K-only black, colour correction off) stays here.
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

/// 3-D colour table: n×n×n RGB nodes at the chart levels round(i·255/(n−1)), each holding the drop
/// fractions of the four inks; tetrahedral interpolation in between.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lut {
    n: usize,
    levels: Vec<u8>,
    nodes: Vec<[Drops; 4]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LutError(pub &'static str);

impl std::fmt::Display for LutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bad colour table: {}", self.0)
    }
}

impl std::error::Error for LutError {}

const LUT_MAGIC: &[u8; 7] = b"TMCLUT1";

impl Lut {
    /// `nodes` in r-major, b-minor order. `None` if the size is wrong or an ink's fractions exceed 100 %.
    pub fn from_nodes(n: usize, nodes: Vec<[Drops; 4]>) -> Option<Self> {
        if !(2..=33).contains(&n) || nodes.len() != n * n * n {
            return None;
        }
        if nodes
            .iter()
            .flatten()
            .any(|d| d.iter().map(|&f| u32::from(f)).sum::<u32>() > 65535)
        {
            return None;
        }
        let levels = (0..n)
            .map(|i| ((i * 255 + (n - 1) / 2) / (n - 1)) as u8)
            .collect();
        Some(Self { n, levels, nodes })
    }

    pub fn parse(b: &[u8]) -> Result<Self, LutError> {
        let rest = b.strip_prefix(&LUT_MAGIC[..]).ok_or(LutError("magic"))?;
        let (&n, data) = rest.split_first().ok_or(LutError("truncated"))?;
        let n = usize::from(n);
        if !(2..=33).contains(&n) || data.len() != n * n * n * 24 {
            return Err(LutError("size"));
        }
        let mut nodes = Vec::with_capacity(n * n * n);
        for node in data.as_chunks::<24>().0 {
            let mut d = [[0u16; 3]; 4];
            for (i, v) in node.as_chunks::<2>().0.iter().enumerate() {
                d[i / 3][i % 3] = u16::from_le_bytes([v[0], v[1]]);
            }
            nodes.push(d);
        }
        Self::from_nodes(n, nodes).ok_or(LutError("fractions above 100 %"))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = LUT_MAGIC.to_vec();
        out.push(self.n as u8);
        for v in self.nodes.iter().flatten().flatten() {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    fn node(&self, r: usize, g: usize, b: usize) -> &[Drops; 4] {
        &self.nodes[(r * self.n + g) * self.n + b]
    }

    /// Segment index and position (0..=1) of an 8-bit value on the level grid.
    fn locate(&self, v: u8) -> (usize, f32) {
        let i = self
            .levels
            .partition_point(|&l| l <= v)
            .clamp(1, self.n - 1)
            - 1;
        let (a, b) = (self.levels[i], self.levels[i + 1]);
        (
            i,
            f32::from(v.saturating_sub(a)) / f32::from(b - a).max(1.0),
        )
    }

    pub fn lookup(&self, rgb: [u8; 3]) -> [Drops; 4] {
        let [(ri, fr), (gi, fg), (bi, fb)] = rgb.map(|v| self.locate(v));
        let c = |dr, dg, db| self.node(ri + dr, gi + dg, bi + db);
        // tetrahedron containing the point: walk from c000 to c111 along the largest fractions first
        let mut steps = [(fr, (1, 0, 0)), (fg, (0, 1, 0)), (fb, (0, 0, 1))];
        steps.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut out = [[0f32; 3]; 4];
        let mut pos = (0, 0, 0);
        let mut prev = c(0, 0, 0);
        let acc = |w: f32, node: &[Drops; 4], out: &mut [[f32; 3]; 4]| {
            for (o, n) in out.iter_mut().zip(node) {
                for (x, &v) in o.iter_mut().zip(n) {
                    *x += w * f32::from(v);
                }
            }
        };
        let mut last_f = 1.0;
        for (f, (dr, dg, db)) in steps {
            acc(last_f - f, prev, &mut out);
            pos = (pos.0 + dr, pos.1 + dg, pos.2 + db);
            prev = c(pos.0, pos.1, pos.2);
            last_f = f;
        }
        acc(last_f, prev, &mut out);
        out.map(|ink| ink.map(|x| x.round().clamp(0.0, 65535.0) as u16))
    }
}

/// Converts one RGB pixel to ink amounts `[K, C, M, Y]`.
#[derive(Debug, Clone)]
pub struct Converter {
    /// Fitted table; `None` = provisional formula.
    lut: Option<Lut>,
    /// Input transfer curve: 8-bit value → darkness 0..=1 (0 = white).
    darkness: [f32; 256],
    rich_black: bool,
    /// Multipliers for colour inks and black (ink adjustment, brightness).
    cmy_scale: f32,
    k_scale: f32,
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
        let (mut cmy_scale, mut k_scale) = (1.0f32, 1.0f32);
        if s.ink_adjust {
            let bright = 1.0 - 0.08 * f32::from(s.brightness);
            cmy_scale = (1.0 + 0.05 * f32::from(s.ink_bkcmy)) * bright;
            k_scale = (1.0 + 0.05 * f32::from(s.ink_bk)) * bright;
        }
        Self {
            lut: None,
            darkness,
            rich_black: black == BlackMode::Rich,
            cmy_scale,
            k_scale,
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

    /// Drop-size fractions `[small, medium, large]` per ink `[K, C, M, Y]`.
    pub fn drops(&self, rgb: [u8; 3]) -> [Drops; 4] {
        let Some(lut) = &self.lut else {
            return self.ink(rgb).map(amount_to_drops);
        };
        let mut d = lut.lookup(rgb);
        for (i, ink) in d.iter_mut().enumerate() {
            let scale = if i == 0 { self.k_scale } else { self.cmy_scale };
            if scale != 1.0 {
                for f in ink.iter_mut() {
                    *f = (f32::from(*f) * scale).round().clamp(0.0, 65535.0) as u16;
                }
            }
        }
        d
    }

    pub fn ink(&self, rgb: [u8; 3]) -> [u16; 4] {
        let [c, m, y] = rgb.map(|v| self.darkness[usize::from(v)]);
        let k = c.min(m).min(y);
        // under-colour added to the CMY of dark neutrals (Windows prints black as rich black)
        let under = if self.rich_black {
            0.55 * k * k * k
        } else {
            0.0
        };
        let q = |a: f32, scale: f32| ((a * scale).clamp(0.0, 1.0) * 65535.0).round() as u16;
        [
            q(k, self.k_scale),
            q(c - k + under, self.cmy_scale),
            q(m - k + under, self.cmy_scale),
            q(y - k + under, self.cmy_scale),
        ]
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
        assert_eq!(c.ink([255, 255, 255]), [0, 0, 0, 0]);
        let [k, cy, m, y] = c.ink([0, 0, 0]);
        assert_eq!(k, 65535);
        assert!(cy > 20000 && m > 20000 && y > 20000, "rich black adds CMY");
    }

    #[test]
    fn k_only_black_has_no_colour_ink_on_neutrals() {
        let c = conv("TmBlack=KOnly");
        assert_eq!(c.ink([0, 0, 0]), [65535, 0, 0, 0]);
        let [k, cy, m, y] = c.ink([128, 128, 128]);
        assert!(k > 0 && cy == 0 && m == 0 && y == 0);
    }

    #[test]
    fn primaries_use_their_inks() {
        let c = conv("");
        assert_eq!(c.ink([0, 255, 255]), [0, 65535, 0, 0]);
        assert_eq!(c.ink([255, 0, 255]), [0, 0, 65535, 0]);
        assert_eq!(c.ink([255, 255, 0]), [0, 0, 0, 65535]);
    }

    #[test]
    fn grey_ramp_is_monotonic() {
        let c = conv("");
        let mut prev = 0u32;
        for v in (0..=255u8).rev() {
            let total: u32 = c.ink([v, v, v]).iter().map(|&a| u32::from(a)).sum();
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
            .ink([100, 100, 100])
            .iter()
            .map(|&a| u32::from(a))
            .sum();
        let bright: u32 = conv("TmInkAdjust=True TmBrightness=2")
            .ink([100, 100, 100])
            .iter()
            .map(|&a| u32::from(a))
            .sum();
        let less: u32 = conv("TmInkAdjust=True TmInkBkCMY=-6")
            .ink([100, 100, 100])
            .iter()
            .map(|&a| u32::from(a))
            .sum();
        assert!(bright < base && less < base);
        // without TmInkAdjust the adjustment values are ignored, as in the Windows dialog
        let ignored: u32 = conv("TmInkBkCMY=-6")
            .ink([100, 100, 100])
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

    #[test]
    fn lut_is_exact_at_the_chart_levels_and_linear_between() {
        let lut = node_lut(9, |r, g, b| {
            [[(r * 1000) as u16, (g * 1000) as u16, (b * 1000) as u16]; 4]
        });
        let lv = |i: usize| (i * 255 + 4) / 8; // chart levels round(i*255/8)
        for (r, g, b) in [(0, 0, 0), (8, 8, 8), (3, 5, 1), (1, 7, 4)] {
            let got = lut.lookup([lv(r) as u8, lv(g) as u8, lv(b) as u8]);
            assert_eq!(
                got[2],
                [(r * 1000) as u16, (g * 1000) as u16, (b * 1000) as u16]
            );
        }
        // halfway between red levels 1 and 2 (32 and 64): 1500
        assert_eq!(lut.lookup([48, 0, 0])[0][0], 1500);
    }

    #[test]
    fn lut_round_trips_and_rejects_corrupt_data() {
        let lut = node_lut(3, |r, g, b| [[(r + g + b) as u16 * 100, 7, 9]; 4]);
        let bytes = lut.to_bytes();
        assert_eq!(Lut::parse(&bytes).unwrap(), lut);
        assert!(Lut::parse(&bytes[..bytes.len() - 1]).is_err());
        assert!(Lut::parse(b"TMCLUT1").is_err());
        let mut bad = bytes.clone();
        bad[7] = 1; // grid size 1
        assert!(Lut::parse(&bad).is_err());
        assert!(
            Lut::from_nodes(2, vec![[[40000, 30000, 0]; 4]; 8]).is_none(),
            "fractions above 100 %"
        );
    }

    #[test]
    fn converter_uses_the_table_and_ink_adjustment_scales_it() {
        let lut = node_lut(2, |r, _, _| {
            if r == 0 {
                [[0, 0, 60000]; 4]
            } else {
                [[0; 3]; 4]
            }
        });
        let c = JobConfig::default();
        let conv = Converter::with_lut(&c.color, c.black, lut.clone());
        assert_eq!(conv.drops([0, 0, 0])[1], [0, 0, 60000]);
        let c = JobConfig::from_cups("TmInkAdjust=True TmInkBkCMY=-2", 1).unwrap();
        let conv = Converter::with_lut(&c.color, c.black, lut);
        assert_eq!(conv.drops([0, 0, 0])[1], [0, 0, 54000]);
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
