//! Ink amounts → 2-bit drop sizes with a stable ordered (threshold-matrix) dither.
//!
//! Each dot gets one of two neighbouring drop sizes, so flat areas are smooth, solids (100 %) are
//! all large drops with no holes, and nothing is placed where the amount is zero: bar edges stay
//! exactly where the image puts them.
use crate::color::Converter;
use crate::raster::{Ink, InkPage};

/// 16×16 Bayer threshold, 0..=255.
fn bayer16(x: u32, y: u32) -> u32 {
    let mut v = 0;
    for i in 0..4 {
        let (bx, by) = ((x >> i) & 1, (y >> i) & 1);
        v |= ((bx ^ by) << (2 * (3 - i) + 1)) | (by << (2 * (3 - i)));
    }
    v
}

/// Drop size (0..=3) for an ink amount at a dot position.
pub fn level(amount: u16, x: u32, y: u32) -> u8 {
    let v = u32::from(amount) * 3 * 256 / 65535;
    let (lo, frac) = (v / 256, v % 256);
    if lo >= 3 {
        return 3;
    }
    (lo + u32::from(frac > bayer16(x, y))) as u8
}

/// Dither one row of 8-bit RGB pixels (3 bytes each) into page row `y`.
pub fn dither_row(page: &mut InkPage, y: u32, rgb: &[u8], conv: &Converter) {
    let n = (rgb.len() / 3).min(page.width() as usize);
    for (x, px) in rgb.as_chunks::<3>().0.iter().take(n).enumerate() {
        if *px == [255, 255, 255] {
            continue;
        }
        let amounts = conv.ink(*px);
        for (i, (&ink, &a)) in Ink::ALL.iter().zip(amounts.iter()).enumerate() {
            if a == 0 {
                continue;
            }
            // shift the matrix per ink so the inks' patterns don't coincide
            let o = 5 * i as u32;
            page.set(ink, x as u32, y, level(a, x as u32 + o, y + 2 * o));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::JobConfig;

    fn conv() -> Converter {
        let c = JobConfig::default();
        Converter::new(&c.color, c.black)
    }

    #[test]
    fn levels_cover_the_full_range() {
        assert_eq!(level(0, 3, 5), 0);
        assert_eq!(level(65535, 3, 5), 3);
        for x in 0..16 {
            for y in 0..16 {
                assert_eq!(level(65535, x, y), 3);
                assert_eq!(level(0, x, y), 0);
            }
        }
    }

    #[test]
    fn mean_drop_size_tracks_the_amount() {
        let mut prev = 0.0;
        for a in (0..=65535u32).step_by(4096) {
            let sum: u32 = (0..16)
                .flat_map(|x| (0..16).map(move |y| (x, y)))
                .map(|(x, y)| u32::from(level(a as u16, x, y)))
                .sum();
            let mean = f64::from(sum) / 256.0;
            assert!(
                (mean - 3.0 * f64::from(a) / 65535.0).abs() < 0.07,
                "amount {a}: mean {mean}"
            );
            assert!(mean >= prev);
            prev = mean;
        }
    }

    #[test]
    fn a_solid_bar_has_exact_edges() {
        let mut page = InkPage::new(64, 8).unwrap();
        let mut rgb = vec![255u8; 64 * 3];
        for x in 20..30 {
            rgb[x * 3..x * 3 + 3].copy_from_slice(&[0, 0, 0]);
        }
        let c = conv();
        for y in 0..8 {
            dither_row(&mut page, y, &rgb, &c);
        }
        for y in 0..8 {
            for x in 0..64 {
                let k = page.get(Ink::K, x, y);
                if (20..30).contains(&x) {
                    assert_eq!(k, 3, "bar dot {x},{y}");
                } else {
                    assert_eq!(k, 0, "stray dot {x},{y}");
                }
            }
        }
    }

    #[test]
    fn white_row_leaves_page_blank() {
        let mut page = InkPage::new(10, 1).unwrap();
        dither_row(&mut page, 0, &[255u8; 30], &conv());
        assert!(page.is_blank());
    }
}
