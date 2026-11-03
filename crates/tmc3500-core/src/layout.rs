//! Placing the page image on the media: printable area, position adjustment, rotate 180, media saving.
//! Numbers come from the captures (see the phase 2 plan).
use crate::options::{JobConfig, MediaSaving, Quality};
use crate::raster::{Ink, InkPage};
use crate::units::mm10_to_dots;

/// Widest printable band of the head, in 1/360 in (104 mm).
pub const MAX_PRINT_WIDTH: u32 = 1475;
/// Top and bottom margin with "borderless" off, in rows.
pub const BORDER_ROWS: u32 = 21;
/// Shortest page length with media saving (both captures: 212 dots ≈ 15 mm).
pub const MIN_SAVED_LENGTH: u32 = 212;

/// Printable area in device pixels (columns are 1/720 in in Quality modes, `hscale` = 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrintArea {
    pub width: u32,
    pub height: u32,
    /// Rows between the top of the page and the first printable row.
    pub row_offset: u32,
    pub hscale: u32,
}

pub fn print_area(cfg: &JobConfig) -> PrintArea {
    let (w_mm10, l_mm10) = cfg.media.mm10();
    let hscale = if cfg.quality == Quality::Speed { 1 } else { 2 };
    let width = mm10_to_dots(w_mm10).min(MAX_PRINT_WIDTH) * hscale;
    let length = mm10_to_dots(l_mm10);
    let (height, row_offset) = if cfg.borderless {
        (length, 0)
    } else {
        (length.saturating_sub(2 * BORDER_ROWS).max(1), BORDER_ROWS)
    };
    PrintArea {
        width,
        height,
        row_offset,
        hscale,
    }
}

fn signed_dots(mm10: i32) -> i64 {
    let d = i64::from(mm10_to_dots(mm10.unsigned_abs()));
    if mm10 < 0 {
        -d
    } else {
        d
    }
}

/// Apply rotate 180 and position adjustment. Content moved outside the area is clipped.
pub fn transform(page: &InkPage, cfg: &JobConfig) -> InkPage {
    let a = print_area(cfg);
    if !cfg.rotate180
        && cfg.pos_adj_h == 0
        && cfg.pos_adj_v == 0
        && page.width() == a.width
        && page.height() == a.height
    {
        return page.clone();
    }
    let dx = signed_dots(cfg.pos_adj_h) * i64::from(a.hscale);
    let dy = signed_dots(cfg.pos_adj_v);
    let (w, h) = (i64::from(page.width()), i64::from(page.height()));
    let mut out = InkPage::new(a.width, a.height).expect("print area within raster limits");
    for y in 0..page.height() {
        if !page.row_has_ink(y) {
            continue;
        }
        let ty = if cfg.rotate180 {
            h - 1 - i64::from(y)
        } else {
            i64::from(y)
        } + dy;
        if ty < 0 || ty >= i64::from(a.height) {
            continue;
        }
        for x in 0..page.width() {
            let tx = if cfg.rotate180 {
                w - 1 - i64::from(x)
            } else {
                i64::from(x)
            } + dx;
            if tx < 0 || tx >= i64::from(a.width) {
                continue;
            }
            for ink in Ink::ALL {
                let v = page.get(ink, x, y);
                if v != 0 {
                    out.set(ink, tx as u32, ty as u32, v);
                }
            }
        }
    }
    out
}

/// Media saving: returns the (possibly shifted) page and the shortened page length, if any.
pub fn media_saving(page: InkPage, cfg: &JobConfig) -> (InkPage, Option<u32>) {
    let Some((first, last)) = page.inked_rows() else {
        return match cfg.saving {
            MediaSaving::None => (page, None),
            _ => (page, Some(MIN_SAVED_LENGTH)),
        };
    };
    match cfg.saving {
        MediaSaving::None => (page, None),
        MediaSaving::Lower => (page, Some((last + 1).max(MIN_SAVED_LENGTH))),
        MediaSaving::LowerUpper => {
            let mut out = InkPage::new(page.width(), page.height()).expect("same size as input");
            for y in first..=last {
                for x in 0..page.width() {
                    for ink in Ink::ALL {
                        let v = page.get(ink, x, y);
                        if v != 0 {
                            out.set(ink, x, y - first, v);
                        }
                    }
                }
            }
            (out, Some((last - first + 1).max(MIN_SAVED_LENGTH)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(o: &str) -> JobConfig {
        JobConfig::from_cups(o, 1).unwrap()
    }

    fn rect(area: &PrintArea, x0: u32, y0: u32, x1: u32, y1: u32) -> InkPage {
        let mut p = InkPage::new(area.width, area.height).unwrap();
        for y in y0..y1 {
            for x in x0..x1 {
                p.set(Ink::K, x, y, 3);
            }
        }
        p
    }

    /// (first col, last col, first row, last row) with ink.
    fn extent(p: &InkPage) -> (u32, u32, u32, u32) {
        let (r0, r1) = p.inked_rows().unwrap();
        let cols: Vec<u32> = (0..p.width())
            .filter(|&x| (r0..=r1).any(|y| p.get(Ink::K, x, y) != 0))
            .collect();
        (cols[0], *cols.last().unwrap(), r0, r1)
    }

    #[test]
    fn printable_area_matches_captures() {
        assert_eq!(
            print_area(&cfg("")),
            PrintArea {
                width: 1475,
                height: 2477,
                row_offset: 0,
                hscale: 1
            }
        );
        assert_eq!(print_area(&cfg("TmQuality=Quality1")).width, 2950);
        assert_eq!(
            print_area(&cfg("TmBorderless=False")),
            PrintArea {
                width: 1475,
                height: 2435,
                row_offset: 21,
                hscale: 1
            }
        );
        assert_eq!(
            print_area(&cfg("PageSize=Custom.1.00x0.31in")),
            PrintArea {
                width: 360,
                height: 113,
                row_offset: 0,
                hscale: 1
            }
        );
        assert_eq!(
            print_area(&cfg(
                "PageSize=Custom.4.25x6.88in TmMediaForm=ContinuousPaper"
            ))
            .width,
            1475
        );
    }

    #[test]
    fn rotate_180_matches_capture_101() {
        let c = cfg("TmRotate180=True");
        let a = print_area(&c);
        let out = transform(&rect(&a, 90, 90, 450, 180), &c);
        assert_eq!(extent(&out), (1025, 1384, 2297, 2386));
    }

    #[test]
    fn position_adjustment_shifts_and_clips_like_106_to_109() {
        for (o, want) in [
            ("TmPosAdjV=0.50in", (90, 449, 270, 359)),
            ("TmPosAdjV=-0.25in", (90, 449, 0, 88)),
            ("TmPosAdjH=0.50in", (270, 629, 90, 179)),
            ("TmPosAdjH=-0.25in", (0, 358, 90, 179)),
        ] {
            let c = cfg(o);
            let a = print_area(&c);
            assert_eq!(
                extent(&transform(&rect(&a, 90, 90, 450, 180), &c)),
                want,
                "{o}"
            );
        }
        // Quality: horizontal shift in 1/720 in
        let c = cfg("TmQuality=Quality1 TmPosAdjH=0.50in");
        let a = print_area(&c);
        assert_eq!(extent(&transform(&rect(&a, 180, 90, 900, 180), &c)).0, 540);
    }

    #[test]
    fn media_saving_matches_305_to_308() {
        let base = "PageSize=Custom.4.25x6.88in TmMediaForm=ContinuousLabel";
        let c = cfg(&format!("{base} TmMediaSaving=Lower"));
        let a = print_area(&c);
        let (p, len) = media_saving(rect(&a, 90, 90, 450, 180), &c);
        assert_eq!((extent(&p).2, len), (90, Some(212)));
        let c = cfg(&format!("{base} TmMediaSaving=LowerUpper"));
        let (p, len) = media_saving(rect(&a, 90, 90, 450, 180), &c);
        assert_eq!((extent(&p).2, extent(&p).3, len), (0, 89, Some(212)));
        let (_, len) = media_saving(rect(&a, 90, 0, 450, 300), &c);
        assert_eq!(len, Some(300));
        let (_, len) = media_saving(InkPage::new(a.width, a.height).unwrap(), &c);
        assert_eq!(len, Some(MIN_SAVED_LENGTH));
        assert_eq!(media_saving(rect(&a, 0, 0, 1, 1), &cfg(base)).1, None);
    }
}
