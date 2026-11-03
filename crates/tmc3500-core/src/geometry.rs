//! Page geometry in printer units (1/360 in), as sent in ESC(C / ESC(S / ESC(l / ESC(c and SN.
use crate::options::{JobConfig, MediaForm, MediaSize};
use crate::units::{dots_to_mm10_ceil, mm10_to_dots};

/// Left/right liner edge added to the width of label forms.
pub const EDGE_DOTS: u32 = 28;
/// Widest paper width (108 mm). Wider label+edge widths shift the left margin
/// (an assumption that fits both captured cases; see spec section 3).
pub const MAX_PAPER_DOTS: u32 = 1531;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    /// ESC(C page length (label + gap).
    pub page_length: u32,
    /// ESC(S width and height.
    pub width: u32,
    pub height: u32,
    /// ESC(l left margin.
    pub left: u32,
    /// ESC(c top and bottom margins.
    pub top: i32,
    pub bottom: u32,
    /// SN media block width/length in 0.1 mm (length 0xffff = continuous).
    pub sn_width: u16,
    pub sn_length: u16,
}

pub fn geometry(cfg: &JobConfig) -> Geometry {
    let (w_mm10, l_mm10) = cfg.media.mm10();
    let media_w = mm10_to_dots(w_mm10);
    let media_l = mm10_to_dots(l_mm10);
    let width = if cfg.form.is_label() {
        media_w + 2 * EDGE_DOTS
    } else {
        media_w
    };
    let left = if width > MAX_PAPER_DOTS {
        width - MAX_PAPER_DOTS
    } else {
        EDGE_DOTS
    };
    let gap = match (cfg.form.is_die_cut(), cfg.media) {
        (false, _) => 0,
        (true, MediaSize::Label108x1748) => 43,
        (true, MediaSize::Custom { .. }) => 42,
    };
    let height = if cfg.form == MediaForm::DieCutBlackMark {
        media_l + gap
    } else {
        media_l
    };
    let to_u16 = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
    Geometry {
        page_length: media_l + gap,
        width,
        height,
        left,
        top: -20,
        bottom: media_l.saturating_sub(if cfg.borderless { 0 } else { 21 }),
        sn_width: to_u16(dots_to_mm10_ceil(width)),
        sn_length: if cfg.form.is_continuous() {
            0xffff
        } else {
            to_u16(dots_to_mm10_ceil(media_l))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(opts: &str) -> Geometry {
        geometry(&JobConfig::from_cups(opts, 1).unwrap())
    }

    /// (options, page_length, width, height, left, bottom, sn_width, sn_length), all from the captures.
    #[test]
    fn matches_every_captured_geometry() {
        let c = "PageSize=Custom.4.25x6.88in";
        let cases: &[(&str, u32, u32, u32, u32, u32, u16, u16)] = &[
            ("", 2520, 1587, 2477, 56, 2477, 1120, 1748), // 000
            ("TmBorderless=False", 2520, 1587, 2477, 56, 2456, 1120, 1748), // 110
            (c, 2519, 1587, 2477, 56, 2477, 1120, 1748),  // 210
            (
                &format!("{c} TmMediaForm=DieCutBlackMark"),
                2519,
                1587,
                2519,
                56,
                2477,
                1120,
                1748,
            ), // 301
            (
                &format!("{c} TmMediaForm=ContinuousLabel"),
                2477,
                1587,
                2477,
                56,
                2477,
                1120,
                0xffff,
            ), // 302
            (
                &format!("{c} TmMediaForm=ContinuousPaper"),
                2477,
                1531,
                2477,
                28,
                2477,
                1081,
                0xffff,
            ), // 303
            (
                &format!("{c} TmMediaForm=ContinuousPaperBlackMark"),
                2477,
                1531,
                2477,
                28,
                2477,
                1081,
                0xffff,
            ), // 304
            (
                "PageSize=Custom.2.00x6.88in",
                2519,
                776,
                2477,
                28,
                2477,
                548,
                1748,
            ), // 330
            (
                "PageSize=Custom.4.25x3.00in",
                1122,
                1587,
                1080,
                56,
                1080,
                1120,
                762,
            ), // 331
            (
                "PageSize=Custom.1.97x1.18in",
                467,
                765,
                425,
                28,
                425,
                540,
                300,
            ), // 332
            (
                "PageSize=Custom.4.02x5.98in",
                2195,
                1503,
                2153,
                28,
                2153,
                1061,
                1520,
            ), // 333
            (
                "PageSize=Custom.1.00x0.31in",
                155,
                416,
                113,
                28,
                113,
                294,
                80,
            ), // 334
            (
                "PageSize=Custom.4.25x12.00in",
                4362,
                1587,
                4320,
                56,
                4320,
                1120,
                3048,
            ), // 335
        ];
        for &(o, pl, w, h, l, b, snw, snl) in cases {
            let got = g(o);
            assert_eq!(
                (
                    got.page_length,
                    got.width,
                    got.height,
                    got.left,
                    got.top,
                    got.bottom,
                    got.sn_width,
                    got.sn_length
                ),
                (pl, w, h, l, -20, b, snw, snl),
                "options {o:?}"
            );
        }
    }
}
