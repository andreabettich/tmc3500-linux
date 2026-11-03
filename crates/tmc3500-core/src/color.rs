//! RGB → ink amounts (K, C, M, Y; 0..=65535).
//!
//! PROVISIONAL: a simple model with grey-component replacement and the Windows "rich black". Phase 3
//! replaces the core conversion with 3-D tables fitted to Windows colour-chart captures; the option
//! handling (gamma, brightness, ink adjustment, K-only black, colour correction off) stays here.
use crate::options::{BlackMode, ColorMode, ColorSettings, Gamma};

/// Converts one RGB pixel to ink amounts `[K, C, M, Y]`.
#[derive(Debug, Clone)]
pub struct Converter {
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
            darkness,
            rich_black: black == BlackMode::Rich,
            cmy_scale,
            k_scale,
        }
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
}
