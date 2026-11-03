//! Dithered page in device resolution: four ink planes of 2-bit drop sizes (0 = none, 1..3 = small,
//! medium, large), packed four dots per byte, most significant bits first (the printer's own order).

/// Largest page we accept: 2 × 1475 columns (Quality, 720 dpi) and 44 in at 360 dpi.
pub const MAX_WIDTH: u32 = 2 * 1475;
pub const MAX_HEIGHT: u32 = 44 * 360;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    K,
    C,
    M,
    Y,
}

impl Ink {
    /// Plane order in [`InkPage`].
    pub const ALL: [Ink; 4] = [Ink::K, Ink::C, Ink::M, Ink::Y];
    /// Order in which the Windows driver sends the colours of a pass.
    pub const SEND_ORDER: [Ink; 4] = [Ink::C, Ink::M, Ink::Y, Ink::K];

    /// `ESC i` colour code of the first nozzle row (the second row adds 0x40).
    pub fn code(self) -> u8 {
        match self {
            Ink::K => 0,
            Ink::C => 2,
            Ink::M => 1,
            Ink::Y => 4,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InkPage {
    width: u32,
    height: u32,
    stride: usize,
    planes: [Vec<u8>; 4],
}

impl InkPage {
    /// An empty page, or `None` if the size is zero or above the limits.
    pub fn new(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
            return None;
        }
        let stride = (width as usize).div_ceil(4);
        let n = stride * height as usize;
        Some(Self {
            width,
            height,
            stride,
            planes: [vec![0; n], vec![0; n], vec![0; n], vec![0; n]],
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Set one dot; coordinates outside the page are ignored.
    pub fn set(&mut self, ink: Ink, x: u32, y: u32, level: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = y as usize * self.stride + x as usize / 4;
        let shift = 6 - 2 * (x % 4);
        let b = &mut self.planes[ink.index()][i];
        *b = (*b & !(3 << shift)) | ((level & 3) << shift);
    }

    /// Drop size at a dot (0 outside the page).
    pub fn get(&self, ink: Ink, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        let b = self.planes[ink.index()][y as usize * self.stride + x as usize / 4];
        (b >> (6 - 2 * (x % 4))) & 3
    }

    /// Packed row of one plane.
    pub fn row(&self, ink: Ink, y: u32) -> &[u8] {
        let s = y as usize * self.stride;
        &self.planes[ink.index()][s..s + self.stride]
    }

    pub fn row_has_ink(&self, y: u32) -> bool {
        Ink::ALL
            .iter()
            .any(|&i| self.row(i, y).iter().any(|&b| b != 0))
    }

    /// First and last row (inclusive) that carry any ink.
    pub fn inked_rows(&self) -> Option<(u32, u32)> {
        let first = (0..self.height).find(|&y| self.row_has_ink(y))?;
        let last = (first..self.height).rev().find(|&y| self.row_has_ink(y))?;
        Some((first, last))
    }

    pub fn is_blank(&self) -> bool {
        self.planes.iter().all(|p| p.iter().all(|&b| b == 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_and_packing() {
        let mut p = InkPage::new(10, 3).unwrap();
        p.set(Ink::C, 0, 1, 3);
        p.set(Ink::C, 5, 1, 2);
        p.set(Ink::C, 99, 1, 3); // ignored
        assert_eq!(p.get(Ink::C, 0, 1), 3);
        assert_eq!(p.get(Ink::C, 5, 1), 2);
        assert_eq!(p.get(Ink::K, 0, 1), 0);
        assert_eq!(p.row(Ink::C, 1), &[0xc0, 0x20, 0]);
        assert_eq!(p.inked_rows(), Some((1, 1)));
        assert!(!p.is_blank());
    }

    #[test]
    fn rejects_bad_sizes() {
        assert!(InkPage::new(0, 1).is_none());
        assert!(InkPage::new(MAX_WIDTH + 1, 1).is_none());
        assert!(InkPage::new(1, MAX_HEIGHT + 1).is_none());
        assert_eq!(InkPage::new(4, 4).unwrap().inked_rows(), None);
    }
}
