//! Length units. Lengths are kept in 0.1 mm (`mm10`, the Windows driver's internal unit) and
//! converted to dots (1/360 inch, the printer's command unit).

/// 0.1 mm → 1/360 inch, rounding half up (as the Windows driver does).
pub fn mm10_to_dots(mm10: u32) -> u32 {
    let v = (u64::from(mm10) * 360 + 127) / 254;
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// 1/360 inch → 0.1 mm, rounding up (used for the REMOTE1 `SN` media block).
pub fn dots_to_mm10_ceil(dots: u32) -> u32 {
    let v = (u64::from(dots) * 254 + 359) / 360;
    u32::try_from(v).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mm10_to_dots_matches_windows_driver() {
        // (mm10, dots) observed in the captures' driver log and ESC(S/ESC(C values
        for (mm10, dots) in [
            (1080, 1531),
            (1748, 2477),
            (508, 720),
            (500, 709),
            (300, 425),
            (1021, 1447),
            (1519, 2153),
            (254, 360),
            (80, 113),
            (3048, 4320),
            (762, 1080),
        ] {
            assert_eq!(mm10_to_dots(mm10), dots, "mm10={mm10}");
        }
    }

    #[test]
    fn dots_to_mm10_ceil_matches_sn_block() {
        for (dots, mm10) in [
            (1587, 1120),
            (1531, 1081),
            (776, 548),
            (765, 540),
            (1503, 1061),
            (416, 294),
            (2477, 1748),
            (1080, 762),
            (425, 300),
            (2153, 1520),
            (113, 80),
            (4320, 3048),
        ] {
            assert_eq!(dots_to_mm10_ceil(dots), mm10, "dots={dots}");
        }
    }

    #[test]
    fn conversions_saturate_instead_of_overflowing() {
        assert_eq!(mm10_to_dots(u32::MAX), u32::MAX);
        assert_eq!(dots_to_mm10_ceil(u32::MAX), 3_030_338_036);
    }
}
