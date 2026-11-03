//! Reader for CUPS raster streams (the input of the filter): sync words `RaSt`/`RaS2`/`RaS3` in either
//! byte order, the 1796-byte page header, and CUPS v2 line/pixel run-length compression.
//! Only chunky 8-bit RGB/sRGB, white/grey and black pages are accepted. Everything is size-checked.
use std::io::Read;

pub const HEADER_LEN: usize = 1796;
/// Limits: wider than any label at 720 dpi, longer than 44 in at 720 dpi.
pub const MAX_WIDTH: u32 = 8192;
pub const MAX_HEIGHT: u32 = 44 * 720;

#[derive(Debug)]
pub enum RasterError {
    Io(std::io::Error),
    BadSync,
    Truncated,
    BadCompression,
    Unsupported(String),
}

impl std::fmt::Display for RasterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "read error: {e}"),
            Self::BadSync => write!(f, "not a CUPS raster stream"),
            Self::Truncated => write!(f, "raster data ends early"),
            Self::BadCompression => write!(f, "corrupt raster compression"),
            Self::Unsupported(s) => write!(f, "unsupported raster: {s}"),
        }
    }
}

impl std::error::Error for RasterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pixels {
    Rgb,
    White,
    Black,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageHeader {
    /// Horizontal and vertical resolution in dpi.
    pub hw_res: [u32; 2],
    pub width: u32,
    pub height: u32,
    /// Media size in points.
    pub page_size_pt: [u32; 2],
    pub num_copies: u32,
    pub page_size_name: String,
    bytes_per_line: usize,
    bytes_per_pixel: usize,
    pixels: Pixels,
}

pub struct Reader<R: Read> {
    r: R,
    swap: bool,
    compressed: bool,
    page: Option<PageHeader>,
    lines_left: u32,
    /// v2: current decoded line and how many more times it repeats.
    line: Vec<u8>,
    repeat: u32,
}

fn read_exact_or_eof<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<bool, RasterError> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) if got == 0 => return Ok(false),
            Ok(0) => return Err(RasterError::Truncated),
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(RasterError::Io(e)),
        }
    }
    Ok(true)
}

fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<(), RasterError> {
    if read_exact_or_eof(r, buf)? || buf.is_empty() {
        Ok(())
    } else {
        Err(RasterError::Truncated)
    }
}

impl<R: Read> Reader<R> {
    pub fn new(mut r: R) -> Result<Self, RasterError> {
        let mut sync = [0u8; 4];
        if !read_exact_or_eof(&mut r, &mut sync)? {
            return Err(RasterError::BadSync);
        }
        let (swap, compressed) = match &sync {
            b"RaSt" | b"RaS3" => (false, false),
            b"tSaR" | b"3SaR" => (true, false),
            b"RaS2" => (false, true),
            b"2SaR" => (true, true),
            _ => return Err(RasterError::BadSync),
        };
        // "swap" here means little-endian fields (the sync word was written byte-reversed)
        Ok(Self {
            r,
            swap,
            compressed,
            page: None,
            lines_left: 0,
            line: Vec::new(),
            repeat: 0,
        })
    }

    fn u32_at(&self, h: &[u8], off: usize) -> u32 {
        let b = [h[off], h[off + 1], h[off + 2], h[off + 3]];
        if self.swap {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }
    }

    /// Advance to the next page; `None` at the end of the stream. Unread lines are skipped.
    pub fn next_page(&mut self) -> Result<Option<PageHeader>, RasterError> {
        if let Some(h) = self.page.clone() {
            let mut skip = vec![0u8; h.bytes_per_line];
            while self.lines_left > 0 {
                self.read_line_raw(&mut skip)?;
            }
        }
        let mut h = vec![0u8; HEADER_LEN];
        if !read_exact_or_eof(&mut self.r, &mut h)? {
            self.page = None;
            return Ok(None);
        }
        let g = |off| self.u32_at(&h, off);
        let (width, height) = (g(372), g(376));
        let (bits_color, bits_pixel, bpl) = (g(384), g(388), g(392));
        let (order, space) = (g(396), g(400));
        let hw_res = [g(276), g(280)];
        let bad = |s: String| Err(RasterError::Unsupported(s));
        if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
            return bad(format!("page size {width}x{height} pixels"));
        }
        if hw_res.iter().any(|&r| !(72..=2880).contains(&r)) {
            return bad(format!("resolution {}x{}", hw_res[0], hw_res[1]));
        }
        if bits_color != 8 || order != 0 {
            return bad(format!(
                "{bits_color} bits per colour, colour order {order}"
            ));
        }
        let pixels = match (space, bits_pixel) {
            (1 | 19, 24) => Pixels::Rgb,
            (0 | 18, 8) => Pixels::White,
            (3, 8) => Pixels::Black,
            _ => {
                return bad(format!(
                    "colour space {space} with {bits_pixel} bits per pixel"
                ))
            }
        };
        let bytes_per_pixel = bits_pixel as usize / 8;
        if bpl as usize != width as usize * bytes_per_pixel {
            return bad(format!("{bpl} bytes per line for width {width}"));
        }
        let name_raw = &h[1732..1796];
        let name_len = name_raw.iter().position(|&c| c == 0).unwrap_or(64);
        let page_size_name = String::from_utf8_lossy(&name_raw[..name_len]).into_owned();
        let header = PageHeader {
            hw_res,
            width,
            height,
            page_size_pt: [g(352), g(356)],
            num_copies: g(340),
            page_size_name,
            bytes_per_line: bpl as usize,
            bytes_per_pixel,
            pixels,
        };
        self.page = Some(header.clone());
        self.lines_left = height;
        self.repeat = 0;
        self.line = vec![0; bpl as usize];
        Ok(Some(header))
    }

    fn read_line_raw(&mut self, out: &mut [u8]) -> Result<(), RasterError> {
        if self.lines_left == 0 {
            return Err(RasterError::Truncated);
        }
        self.lines_left -= 1;
        if !self.compressed {
            return read_full(&mut self.r, out);
        }
        if self.repeat > 0 {
            self.repeat -= 1;
            out.copy_from_slice(&self.line);
            return Ok(());
        }
        let bpp = self.page.as_ref().map_or(1, |p| p.bytes_per_pixel);
        let mut b = [0u8; 1];
        read_full(&mut self.r, &mut b)?;
        self.repeat = u32::from(b[0]);
        let mut pos = 0;
        while pos < self.line.len() {
            read_full(&mut self.r, &mut b)?;
            let c = usize::from(b[0]);
            if c & 128 != 0 {
                let n = (257 - c) * bpp;
                if pos + n > self.line.len() {
                    return Err(RasterError::BadCompression);
                }
                read_full(&mut self.r, &mut self.line[pos..pos + n])?;
                pos += n;
            } else {
                let n = (c + 1) * bpp;
                if pos + n > self.line.len() {
                    return Err(RasterError::BadCompression);
                }
                let mut px = vec![0u8; bpp];
                read_full(&mut self.r, &mut px)?;
                for chunk in self.line[pos..pos + n].chunks_mut(bpp) {
                    chunk.copy_from_slice(&px);
                }
                pos += n;
            }
        }
        out.copy_from_slice(&self.line);
        Ok(())
    }

    /// Read the next line of the current page as 8-bit RGB (`out.len()` must be width × 3).
    pub fn read_line_rgb(&mut self, out: &mut [u8]) -> Result<(), RasterError> {
        let Some(h) = self.page.clone() else {
            return Err(RasterError::Truncated);
        };
        if out.len() != h.width as usize * 3 {
            return Err(RasterError::Unsupported("output buffer size".into()));
        }
        if h.pixels == Pixels::Rgb {
            return self.read_line_raw(out);
        }
        let mut raw = vec![0u8; h.bytes_per_line];
        self.read_line_raw(&mut raw)?;
        for (px, &v) in out.chunks_mut(3).zip(raw.iter()) {
            let g = if h.pixels == Pixels::Black {
                255 - v
            } else {
                v
            };
            px.copy_from_slice(&[g, g, g]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Minimal CUPS raster writer for tests.
    struct W {
        be: bool,
        out: Vec<u8>,
    }

    impl W {
        fn new(sync: &[u8; 4], be: bool) -> Self {
            let mut out = sync.to_vec();
            if !be {
                out.reverse();
            }
            W { be, out }
        }
        fn header(
            &mut self,
            width: u32,
            height: u32,
            space: u32,
            bpp: u32,
            res: [u32; 2],
            name: &str,
        ) {
            let mut h = vec![0u8; HEADER_LEN];
            let mut put = |off: usize, v: u32| {
                let b = if self.be {
                    v.to_be_bytes()
                } else {
                    v.to_le_bytes()
                };
                h[off..off + 4].copy_from_slice(&b);
            };
            put(276, res[0]);
            put(280, res[1]);
            put(340, 1);
            put(352, 306);
            put(356, 495);
            put(372, width);
            put(376, height);
            put(384, 8);
            put(388, bpp);
            put(392, width * bpp / 8);
            put(396, 0);
            put(400, space);
            put(420, bpp / 8);
            h[1732..1732 + name.len()].copy_from_slice(name.as_bytes());
            self.out.extend_from_slice(&h);
        }
    }

    fn rgb_lines(width: u32, height: u32) -> Vec<Vec<u8>> {
        (0..height)
            .map(|y| {
                (0..width)
                    .flat_map(|x| {
                        [
                            (x * 7 + y) as u8,
                            (y * 3) as u8,
                            if x % 5 == 0 { 255 } else { 0 },
                        ]
                    })
                    .collect()
            })
            .collect()
    }

    /// CUPS v2 compression of whole pages, with line repeats.
    fn compress(lines: &[Vec<u8>], bpp: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let mut rep = 1;
            while i + rep < lines.len() && lines[i + rep] == lines[i] && rep < 256 {
                rep += 1;
            }
            out.push((rep - 1) as u8);
            let px: Vec<&[u8]> = lines[i].chunks(bpp).collect();
            let mut p = 0;
            while p < px.len() {
                let mut run = 1;
                while p + run < px.len() && px[p + run] == px[p] && run < 128 {
                    run += 1;
                }
                if run > 1 {
                    out.push((run - 1) as u8);
                    out.extend_from_slice(px[p]);
                    p += run;
                } else {
                    let mut lit = 1;
                    while p + lit < px.len() && lit < 128 && px[p + lit] != px[p + lit - 1] {
                        lit += 1;
                    }
                    out.push((257 - lit) as u8);
                    for q in &px[p..p + lit] {
                        out.extend_from_slice(q);
                    }
                    p += lit;
                }
            }
            i += rep;
        }
        out
    }

    type Pages = Vec<(PageHeader, Vec<Vec<u8>>)>;

    fn read_all(b: &[u8]) -> Result<Pages, RasterError> {
        let mut r = Reader::new(b)?;
        let mut pages = Vec::new();
        while let Some(h) = r.next_page()? {
            let mut lines = Vec::new();
            for _ in 0..h.height {
                let mut l = vec![0u8; h.width as usize * 3];
                r.read_line_rgb(&mut l)?;
                lines.push(l);
            }
            pages.push((h, lines));
        }
        Ok(pages)
    }

    #[test]
    fn reads_uncompressed_v1_and_v3_in_both_byte_orders() {
        for (sync, be) in [
            (b"RaSt", true),
            (b"RaSt", false),
            (b"RaS3", true),
            (b"RaS3", false),
        ] {
            let mut w = W::new(sync, be);
            let lines = rgb_lines(5, 3);
            for _ in 0..2 {
                w.header(5, 3, 1, 24, [360, 360], "Label108x174.8mm");
                for l in &lines {
                    w.out.extend_from_slice(l);
                }
            }
            let pages = read_all(&w.out).unwrap();
            assert_eq!(pages.len(), 2);
            assert_eq!(pages[0].1, lines);
            assert_eq!(pages[1].0.hw_res, [360, 360]);
            assert_eq!(pages[1].0.page_size_pt, [306, 495]);
            assert_eq!(pages[1].0.page_size_name, "Label108x174.8mm");
        }
    }

    #[test]
    fn reads_compressed_v2() {
        for be in [true, false] {
            let mut w = W::new(b"RaS2", be);
            let mut lines = rgb_lines(300, 4);
            lines.insert(1, lines[0].clone());
            lines.insert(1, lines[0].clone());
            lines.push(vec![255; 900]);
            w.header(300, lines.len() as u32, 19, 24, [720, 360], "");
            w.out.extend_from_slice(&compress(&lines, 3));
            let pages = read_all(&w.out).unwrap();
            assert_eq!(pages[0].1, lines);
        }
    }

    #[test]
    fn expands_grey_and_black_to_rgb() {
        let mut w = W::new(b"RaS3", false);
        w.header(2, 1, 18, 8, [360, 360], "");
        w.out.extend_from_slice(&[0, 200]);
        w.header(2, 1, 3, 8, [360, 360], "");
        w.out.extend_from_slice(&[255, 0]);
        let pages = read_all(&w.out).unwrap();
        assert_eq!(pages[0].1[0], vec![0, 0, 0, 200, 200, 200]);
        assert_eq!(pages[1].1[0], vec![0, 0, 0, 255, 255, 255]);
    }

    #[test]
    fn rejects_bad_streams() {
        assert!(matches!(
            Reader::new(&b"XXXX"[..]),
            Err(RasterError::BadSync)
        ));
        let mut w = W::new(b"RaS3", false);
        w.header(5, 3, 1, 24, [360, 360], "");
        w.out.extend_from_slice(&[0; 20]); // truncated page data
        assert!(read_all(&w.out).is_err());
        let mut w = W::new(b"RaS3", false);
        w.header(100_000, 3, 1, 24, [360, 360], "");
        assert!(matches!(read_all(&w.out), Err(RasterError::Unsupported(_))));
        let mut w = W::new(b"RaS3", false);
        w.header(5, 3, 6, 32, [360, 360], ""); // CMYK
        assert!(matches!(read_all(&w.out), Err(RasterError::Unsupported(_))));
        let mut w = W::new(b"RaS3", false);
        w.header(5, 3, 1, 24, [0, 360], "");
        assert!(matches!(read_all(&w.out), Err(RasterError::Unsupported(_))));
        let mut w = W::new(b"RaS2", false);
        w.header(4, 1, 1, 24, [360, 360], "");
        w.out.extend_from_slice(&[0, 10, 1, 2, 3]); // run of 11 pixels in a 4-pixel line
        assert!(read_all(&w.out).is_err());
    }

    proptest! {
        #[test]
        fn never_panics_on_garbage(tail in proptest::collection::vec(any::<u8>(), 0..4000), v in 0usize..3) {
            let mut w = W::new([b"RaSt", b"RaS2", b"RaS3"][v], false);
            w.header(7, 5, 1, 24, [360, 360], "");
            w.out.extend_from_slice(&tail);
            let _ = read_all(&w.out);
            let _ = read_all(&tail);
        }
    }
}
