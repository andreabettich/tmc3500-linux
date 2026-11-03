//! One page from CUPS raster to printer band commands: resolution mapping, colour conversion,
//! dither, layout (rotate, position, media saving) and weave.
use crate::color::Converter;
use crate::cupsraster::{PageHeader, RasterError, Reader};
use crate::dither::dither_row;
use crate::layout::{media_saving, print_area, transform};
use crate::options::{JobConfig, Quality};
use crate::protocol::job::Page;
use crate::raster::InkPage;
use crate::weave::{weave, WeaveMode};
use std::io::Read;

pub fn weave_mode(cfg: &JobConfig) -> WeaveMode {
    match cfg.quality {
        Quality::Speed => WeaveMode::Speed {
            banding: cfg.banding_reduction,
        },
        Quality::Quality1 => WeaveMode::Quality1,
        Quality::Quality2 => WeaveMode::Quality2,
    }
}

/// Render the current page of `rd` (header `h`) into a printer page.
pub fn render_page<R: Read>(
    rd: &mut Reader<R>,
    h: &PageHeader,
    cfg: &JobConfig,
    conv: &Converter,
) -> Result<Page, RasterError> {
    let area = print_area(cfg);
    let mut ink = InkPage::new(area.width, area.height)
        .ok_or_else(|| RasterError::Unsupported("print area too large".into()))?;
    // device grid: 360 dpi rows, 360 or 720 dpi columns
    let (dev_h, dev_v) = (u64::from(360 * area.hscale), 360u64);
    let (src_h, src_v) = (u64::from(h.hw_res[0]), u64::from(h.hw_res[1]));
    let same_cols = src_h == dev_h;
    // A raster larger than the printable area (CUPS renders the whole media) is centred on it.
    let off_x = (u64::from(h.width) * dev_h / src_h).saturating_sub(u64::from(area.width)) / 2;
    let off_y = (u64::from(h.height) * dev_v / src_v).saturating_sub(u64::from(area.height)) / 2;
    let mut src = vec![0u8; h.width as usize * 3];
    let mut dev = vec![255u8; area.width as usize * 3];
    for sy in 0..u64::from(h.height) {
        rd.read_line_rgb(&mut src)?;
        // device rows whose centre falls in this source row
        let t0 = (sy * dev_v).div_ceil(src_v).max(off_y) - off_y;
        let t1 =
            (((sy + 1) * dev_v).div_ceil(src_v).max(off_y) - off_y).min(u64::from(area.height));
        if t0 >= t1 {
            continue;
        }
        if same_cols {
            let from = (off_x as usize * 3).min(src.len());
            let n = (src.len() - from).min(dev.len());
            dev[..n].copy_from_slice(&src[from..from + n]);
        } else {
            for tx in 0..u64::from(area.width) {
                let sx = ((tx + off_x) * src_h / dev_h) as usize;
                let px = src.get(sx * 3..sx * 3 + 3).unwrap_or(&[255, 255, 255]);
                let d = tx as usize * 3;
                dev[d..d + 3].copy_from_slice(px);
            }
        }
        for ty in t0..t1 {
            dither_row(&mut ink, ty as u32, &dev, conv);
        }
    }
    let ink = transform(&ink, cfg);
    let (ink, length) = media_saving(ink, cfg);
    let blank = ink.is_blank();
    Ok(Page {
        raster: weave(&ink, weave_mode(cfg), area.row_offset),
        blank,
        length,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{decode, Kind};

    /// Uncompressed little-endian v3 raster with one RGB page.
    fn raster(width: u32, height: u32, res: [u32; 2], px: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let mut out = b"3SaR".to_vec();
        let mut h = vec![0u8; crate::cupsraster::HEADER_LEN];
        for (off, v) in [
            (276, res[0]),
            (280, res[1]),
            (372, width),
            (376, height),
            (384, 8),
            (388, 24),
            (392, width * 3),
            (400, 1),
        ] {
            h[off..off + 4].copy_from_slice(&u32::to_le_bytes(v));
        }
        out.extend_from_slice(&h);
        for y in 0..height {
            for x in 0..width {
                out.extend_from_slice(&px(x, y));
            }
        }
        out
    }

    fn render(cfg: &JobConfig, data: &[u8]) -> Page {
        let mut rd = Reader::new(data).unwrap();
        let h = rd.next_page().unwrap().unwrap();
        let conv = Converter::new(&cfg.color, cfg.black);
        render_page(&mut rd, &h, cfg, &conv).unwrap()
    }

    fn black_rect(x: u32, y: u32) -> [u8; 3] {
        if (90..450).contains(&x) && (90..180).contains(&y) {
            [0, 0, 0]
        } else {
            [255, 255, 255]
        }
    }

    fn block_extents(b: &[u8]) -> Vec<(u32, u16)> {
        let mut x = 0;
        let mut out = vec![];
        for t in decode(b).unwrap() {
            match t.kind {
                Kind::Paren { cmd: b'$', payload } => {
                    x = u32::from_le_bytes(payload.try_into().unwrap())
                }
                Kind::Raster { bytes_per_line, .. } => {
                    out.push((x, bytes_per_line));
                    x = 0;
                }
                _ => {}
            }
        }
        out
    }

    #[test]
    fn black_rect_at_360_dpi_gives_capture_000_blocks() {
        let cfg = JobConfig::default();
        let p = render(&cfg, &raster(1475, 2477, [360, 360], black_rect));
        assert!(!p.blank);
        assert_eq!(block_extents(&p.raster), vec![(80, 94); 8]);
    }

    #[test]
    fn other_resolutions_are_rescaled_to_the_device_grid() {
        // 180 dpi source: every pixel becomes 2x2 dots
        let cfg = JobConfig::default();
        let p = render(
            &cfg,
            &raster(738, 1239, [180, 180], |x, y| black_rect(2 * x, 2 * y)),
        );
        assert_eq!(block_extents(&p.raster), vec![(80, 94); 8]);
        // Quality: 720x360 device grid from a 360 dpi source
        let cfg = JobConfig::from_cups("TmQuality=Quality1", 1).unwrap();
        let p = render(&cfg, &raster(1475, 2477, [360, 360], black_rect));
        let e = block_extents(&p.raster);
        assert!(e.contains(&(160, 94)) && e.contains(&(161, 94)), "{e:?}");
    }

    #[test]
    fn white_page_is_blank_and_has_no_bands() {
        let cfg = JobConfig::default();
        let p = render(&cfg, &raster(100, 100, [360, 360], |_, _| [255, 255, 255]));
        assert!(p.blank && p.raster.is_empty());
    }

    #[test]
    fn full_media_raster_is_centred_on_the_printable_area() {
        // CUPS rasterises the whole 108 mm media (1531 dots); the head prints the centred 1475.
        let cfg = JobConfig::default();
        let p = render(
            &cfg,
            &raster(1531, 2477, [360, 360], |x, y| {
                black_rect(x.wrapping_sub(28), y)
            }),
        );
        assert_eq!(block_extents(&p.raster), vec![(80, 94); 8]);
        // borderless off: the printable area starts 21 rows down, so the raster is cropped by 21
        let cfg = JobConfig::from_cups("TmBorderless=False", 1).unwrap();
        let p = render(
            &cfg,
            &raster(1475, 2477, [360, 360], |x, y| {
                black_rect(x, y.wrapping_sub(21))
            }),
        );
        let q = render(&cfg, &raster(1475, 2435, [360, 360], black_rect));
        assert_eq!(p.raster, q.raster);
    }

    #[test]
    fn media_saving_sets_page_length() {
        let cfg = JobConfig::from_cups(
            "PageSize=Custom.4.25x6.88in TmMediaForm=ContinuousLabel TmMediaSaving=LowerUpper",
            1,
        )
        .unwrap();
        let p = render(&cfg, &raster(1475, 2477, [360, 360], black_rect));
        assert_eq!(p.length, Some(212));
    }
}
