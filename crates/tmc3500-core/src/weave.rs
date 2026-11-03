//! Head-pass layout ("weave"): turns a dithered [`InkPage`] into the band commands of one page
//! (`ESC(v`, `ESC($`, `ESC i` + RLE data, CR), with the pass structure of the Windows driver.
//!
//! The head has two staggered nozzle rows per ink (colour codes `c` and `c|0x40`); a block line `l`
//! of a pass at absolute row `y` prints row `y + 2l − 20 (+1 for the second row)`. See the phase 2 plan
//! for the captured pass schedules.
use crate::protocol::writer::Writer;
use crate::raster::{Ink, InkPage};
use crate::rle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeaveMode {
    /// 360×360 dpi. `banding` = banding reduction (feathered pass boundaries).
    Speed { banding: bool },
    /// 720×360 dpi, two horizontal phases.
    Quality1,
    /// 720×360 dpi, two horizontal phases, each row shared by two passes per phase.
    Quality2,
}

/// Rows above a pass's position that its first nozzle line reaches.
const NOZZLE_LEAD: i64 = 20;

#[derive(Debug, Clone, Copy)]
struct Pass {
    y: i64,
    phase: u32,
    /// Block line count for nozzle row 0 and 1.
    lines: [u16; 2],
}

/// Passes on a fixed grid: `y_p = first + p·step`; pass p owns rows `[y_p − lead, y_{p+1} − lead)`
/// (pass 0 from its first nozzle), the last `feather` rows of which are shared with pass p+1.
#[derive(Debug, Clone, Copy)]
struct Stride {
    first: i64,
    step: i64,
    lead: i64,
    feather: i64,
    lines: [u16; 2],
}

const SPEED: Stride = Stride {
    first: 0,
    step: 330,
    lead: 0,
    feather: 10,
    lines: [175, 175],
};
const SPEED_NO_BANDING: Stride = Stride {
    first: 20,
    step: 359,
    lead: 20,
    feather: 0,
    lines: [180, 179],
};
const Q1_PHASE_FIRST: [i64; 2] = [0, 10];
const Q2_LINES: u16 = 176;
const Q2_STEP: i64 = 172;
const Q2_FIRST: [i64; 2] = [96, 10];

/// Deterministic per-dot value in 0..1024 used to share rows between passes.
fn hash(x: u32, r: i64) -> u32 {
    let mut h = (u64::from(x) << 32) ^ (r as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;
    (h & 1023) as u32
}

impl Stride {
    fn y(&self, first: i64, p: i64) -> i64 {
        first + p * self.step
    }

    /// Pass index (within this grid) that prints row r at column x.
    fn owner(&self, first: i64, r: i64, x: u32) -> Option<i64> {
        if r < first - NOZZLE_LEAD {
            return None;
        }
        let p = (r - first + self.lead).div_euclid(self.step).max(0);
        let feather_start = self.y(first, p + 1) - self.lead - self.feather;
        if self.feather > 0 && r >= feather_start {
            let k = r - feather_start;
            // share kept by pass p: (2(f−k)−1)/(2f), from 95 % down to 5 % for f = 10
            let keep = (2 * (self.feather - k) - 1) * 1024 / (2 * self.feather);
            if i64::from(hash(x, r)) >= keep {
                return Some(p + 1);
            }
        }
        Some(p)
    }
}

/// Quality2: y of the pass that prints row r at column x. Same-phase passes covering r (at most 3,
/// ascending y) share it with triangular weights over their nozzle span.
fn q2_owner(r: i64, x: u32) -> Option<i64> {
    let ph = (x % 2) as usize;
    let span = 2 * i64::from(Q2_LINES);
    let weight = |y: i64| {
        let top = y - NOZZLE_LEAD;
        if r < top || r >= top + span {
            0
        } else {
            (r - top + 1).min(top + span - r)
        }
    };
    let mut cand = [(0i64, 0i64); 4];
    let mut n = 0;
    if ph == 0 && weight(0) > 0 {
        cand[n] = (0, weight(0));
        n += 1;
    }
    let f = Q2_FIRST[ph];
    let k_hi = (r - f + NOZZLE_LEAD).div_euclid(Q2_STEP);
    let k_lo = (r - f + NOZZLE_LEAD - span + 1).div_euclid(Q2_STEP).max(0);
    for k in k_lo.max(0)..=k_hi {
        let y = f + k * Q2_STEP;
        let w = weight(y);
        if w > 0 && n < cand.len() {
            cand[n] = (y, w);
            n += 1;
        }
    }
    let total: i64 = cand[..n].iter().map(|&(_, w)| w).sum();
    if total == 0 {
        return None;
    }
    let mut t = i64::from(hash(x, r)) * total / 1024;
    for &(y, w) in &cand[..n] {
        if t < w {
            return Some(y);
        }
        t -= w;
    }
    None
}

struct Plan {
    mode: WeaveMode,
    passes: Vec<Pass>,
}

impl Plan {
    fn new(mode: WeaveMode, last_row: i64) -> Self {
        let mut passes = Vec::new();
        let grid = |passes: &mut Vec<Pass>, first: i64, step: i64, phase: u32, lines: [u16; 2]| {
            let mut y = first;
            while y - NOZZLE_LEAD <= last_row {
                passes.push(Pass { y, phase, lines });
                y += step;
            }
        };
        match mode {
            WeaveMode::Speed { banding } => {
                let s = if banding { SPEED } else { SPEED_NO_BANDING };
                grid(&mut passes, s.first, s.step, 0, s.lines);
            }
            WeaveMode::Quality1 => {
                for ph in 0..2 {
                    grid(
                        &mut passes,
                        Q1_PHASE_FIRST[ph as usize],
                        SPEED.step,
                        ph,
                        SPEED.lines,
                    );
                }
            }
            WeaveMode::Quality2 => {
                // phase 0 has an extra first pass at the page top
                passes.push(Pass {
                    y: 0,
                    phase: 0,
                    lines: [Q2_LINES; 2],
                });
                for ph in 0..2 {
                    grid(
                        &mut passes,
                        Q2_FIRST[ph as usize],
                        Q2_STEP,
                        ph,
                        [Q2_LINES; 2],
                    );
                }
            }
        }
        passes.sort_by_key(|p| (p.y, p.phase));
        Plan { mode, passes }
    }

    /// Does pass `pi` print absolute row r at device column x?
    fn prints(&self, pi: usize, r: i64, x: u32) -> bool {
        let pass = self.passes[pi];
        match self.mode {
            WeaveMode::Speed { banding } => {
                let s = if banding { SPEED } else { SPEED_NO_BANDING };
                s.owner(s.first, r, x).map(|p| s.y(s.first, p)) == Some(pass.y)
            }
            WeaveMode::Quality1 => {
                let ph = x % 2;
                let first = Q1_PHASE_FIRST[ph as usize];
                ph == pass.phase
                    && SPEED.owner(first, r, x).map(|p| SPEED.y(first, p)) == Some(pass.y)
            }
            WeaveMode::Quality2 => x % 2 == pass.phase && q2_owner(r, x) == Some(pass.y),
        }
    }
}

/// Build the band commands for a page. `row_offset` moves the page down (21 with borderless off).
pub fn weave(page: &InkPage, mode: WeaveMode, row_offset: u32) -> Vec<u8> {
    let mut w = Writer::new();
    let Some((_, last)) = page.inked_rows() else {
        return w.into_bytes();
    };
    // The pass grid moves with the page: plan in page rows, add the offset when positioning.
    let offset = i64::from(row_offset);
    let plan = Plan::new(mode, i64::from(last));
    let quality = !matches!(mode, WeaveMode::Speed { .. });
    let (step, phases) = if quality { (2u32, 2u32) } else { (1, 1) };
    let mut cur_y = 0i64;
    for (pi, pass) in plan.passes.iter().enumerate() {
        let ph = pass.phase % phases;
        let ndots = (page.width().saturating_sub(ph)).div_ceil(step) as usize;
        let mut blocks = Vec::new();
        for ink in Ink::SEND_ORDER {
            for nr in 0..2usize {
                let lines = usize::from(pass.lines[nr]);
                let mut levels = vec![0u8; lines * ndots];
                let (mut lo, mut hi) = (usize::MAX, 0usize);
                for l in 0..lines {
                    let r = pass.y + 2 * l as i64 - NOZZLE_LEAD + nr as i64;
                    if r < 0 || r >= i64::from(page.height()) || !page.row_has_ink(r as u32) {
                        continue;
                    }
                    for k in 0..ndots {
                        let x = k as u32 * step + ph;
                        let v = page.get(ink, x, r as u32);
                        if v != 0 && plan.prints(pi, r, x) {
                            levels[l * ndots + k] = v;
                            lo = lo.min(k);
                            hi = hi.max(k);
                        }
                    }
                }
                if lo == usize::MAX {
                    continue;
                }
                let start = lo / 16 * 16;
                let end = (hi + 1).div_ceil(8) * 8;
                let bpl = (end - start) / 4;
                let mut packed = vec![0u8; bpl * lines];
                for l in 0..lines {
                    for k in start..end.min(ndots) {
                        let v = levels[l * ndots + k];
                        let i = k - start;
                        packed[l * bpl + i / 4] |= v << (6 - 2 * (i % 4));
                    }
                }
                let x = if quality {
                    2 * start as u32 + ph
                } else {
                    start as u32
                };
                blocks.push((ink.code() | (nr as u8 * 0x40), x, bpl, lines, packed));
            }
        }
        if blocks.is_empty() {
            continue;
        }
        let y = pass.y + offset;
        if y > cur_y {
            w.paren(b'v', &((y - cur_y) as u32).to_le_bytes());
            cur_y = y;
        }
        for (code, x, bpl, lines, packed) in blocks {
            if x != 0 {
                w.paren(b'$', &x.to_le_bytes());
            }
            w.bytes(&[0x1b, b'i', code, 1, 2]);
            w.bytes(&(bpl as u16).to_le_bytes());
            w.bytes(&(lines as u16).to_le_bytes());
            w.bytes(&rle::encode(&packed, bpl));
            w.bytes(&[0x0d]);
        }
    }
    w.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{decode, Kind};

    /// Rebuild the page from the band commands: every dot must come back exactly once.
    fn reassemble(b: &[u8], w: u32, h: u32, quality: bool, offset: i64) -> (InkPage, usize) {
        let mut p = InkPage::new(w, h).unwrap();
        let (mut y, mut x) = (0i64, 0u32);
        let mut dots = 0;
        for t in decode(b).unwrap() {
            match t.kind {
                Kind::Paren { cmd: b'v', payload } => {
                    y += i64::from(u32::from_le_bytes(payload.try_into().unwrap()))
                }
                Kind::Paren { cmd: b'$', payload } => {
                    x = u32::from_le_bytes(payload.try_into().unwrap())
                }
                Kind::Raster {
                    color,
                    bytes_per_line,
                    lines,
                    data,
                    ..
                } => {
                    let raw = rle::decode(data, usize::from(bytes_per_line) * usize::from(lines))
                        .unwrap();
                    let ink = Ink::ALL
                        .into_iter()
                        .find(|i| i.code() == color & 0x3f)
                        .unwrap();
                    let nr = i64::from(color >> 6);
                    for l in 0..usize::from(lines) {
                        for k in 0..usize::from(bytes_per_line) * 4 {
                            let v = (raw[l * usize::from(bytes_per_line) + k / 4]
                                >> (6 - 2 * (k % 4)))
                                & 3;
                            if v == 0 {
                                continue;
                            }
                            let col = if quality {
                                x + 2 * k as u32
                            } else {
                                x + k as u32
                            };
                            let r = y + 2 * l as i64 - 20 + nr - offset;
                            assert_eq!(
                                p.get(ink, col, r as u32),
                                0,
                                "dot printed twice at {col},{r}"
                            );
                            p.set(ink, col, r as u32, v);
                            dots += 1;
                        }
                    }
                    x = 0;
                }
                _ => {}
            }
        }
        (p, dots)
    }

    fn sample_page(w: u32, h: u32) -> InkPage {
        let mut p = InkPage::new(w, h).unwrap();
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 7 + y * 13) % 5) as u8;
                if v < 4 && (x + y) % 3 != 0 {
                    p.set(Ink::ALL[((x + y) % 4) as usize], x, y, v);
                }
            }
        }
        p
    }

    /// The original per-dot implementation of the Quality2 owner (walks every pass), kept as the
    /// oracle for the O(1) version.
    fn reference_q2_prints(plan: &Plan, pi: usize, r: i64, x: u32) -> bool {
        let pass = plan.passes[pi];
        let ph = x % 2;
        if ph != pass.phase {
            return false;
        }
        let span = 2 * i64::from(Q2_LINES);
        let weight = |p: &Pass| {
            let top = p.y - NOZZLE_LEAD;
            if r < top || r >= top + span {
                0
            } else {
                (r - top + 1).min(top + span - r)
            }
        };
        let covering: Vec<(i64, i64)> = plan
            .passes
            .iter()
            .filter(|p| p.phase == ph)
            .map(|p| (p.y, weight(p)))
            .filter(|&(_, w)| w > 0)
            .collect();
        let total: i64 = covering.iter().map(|&(_, w)| w).sum();
        if total == 0 {
            return false;
        }
        let mut t = i64::from(hash(x, r)) * total / 1024;
        for (y, w) in covering {
            if t < w {
                return y == pass.y;
            }
            t -= w;
        }
        false
    }

    #[test]
    fn quality2_owner_matches_reference() {
        let plan = Plan::new(WeaveMode::Quality2, 3000);
        for r in (0..3000).step_by(3) {
            for x in 0..40 {
                for pi in 0..plan.passes.len() {
                    assert_eq!(
                        plan.prints(pi, r, x),
                        reference_q2_prints(&plan, pi, r, x),
                        "r={r} x={x} pass={pi}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_dot_is_printed_exactly_once_in_every_mode() {
        for (mode, quality, offset) in [
            (WeaveMode::Speed { banding: true }, false, 0),
            (WeaveMode::Speed { banding: true }, false, 21),
            (WeaveMode::Speed { banding: false }, false, 0),
            (WeaveMode::Quality1, true, 0),
            (WeaveMode::Quality2, true, 0),
        ] {
            let page = sample_page(203, 1100);
            let (back, _) = reassemble(
                &weave(&page, mode, offset as u32),
                203,
                1100,
                quality,
                offset,
            );
            assert_eq!(back, page, "{mode:?} offset {offset}");
        }
    }

    #[test]
    fn blank_page_has_no_band_commands() {
        assert!(weave(&InkPage::new(100, 100).unwrap(), WeaveMode::Quality2, 0).is_empty());
    }
}
