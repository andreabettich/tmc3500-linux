//! REMOTE1 `SN` media block (26 bytes, checksum = 16-bit sum of bytes 2..24).
use crate::geometry::Geometry;
use crate::options::JobConfig;

pub fn sn_payload(g: &Geometry, cfg: &JobConfig) -> [u8; 26] {
    let mut p = [0xffu8; 26];
    p[0] = 0x00;
    p[1] = 0x40;
    p[4..6].copy_from_slice(&g.sn_width.to_le_bytes());
    p[6..8].copy_from_slice(&g.sn_length.to_le_bytes());
    p[8] = if cfg.form.is_label() {
        cfg.coating.sn_code()
    } else {
        2
    };
    p[9] = 0x00;
    p[18] = cfg.vacuum.sn_byte();
    let sum = p[2..24]
        .iter()
        .fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
    p[24..26].copy_from_slice(&sum.to_le_bytes());
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::geometry;

    fn sn(o: &str) -> [u8; 26] {
        let c = JobConfig::from_cups(o, 1).unwrap();
        sn_payload(&geometry(&c), &c)
    }

    #[test]
    fn matches_captured_blocks() {
        let hex = |s: &str| -> Vec<u8> {
            s.split(' ')
                .map(|x| u8::from_str_radix(x, 16).unwrap())
                .collect()
        };
        assert_eq!(
            sn("").to_vec(),
            hex("00 40 ff ff 60 04 d4 06 03 00 ff ff ff ff ff ff ff ff ff ff ff ff ff ff 31 11")
        );
        assert_eq!(
            sn("TmVacuum=0").to_vec(),
            hex("00 40 ff ff 60 04 d4 06 03 00 ff ff ff ff ff ff ff ff 1e ff ff ff ff ff 50 10")
        );
        assert_eq!(
            sn("PageSize=Custom.4.25x6.88in TmMediaForm=ContinuousPaper").to_vec(),
            hex("00 40 ff ff 39 04 ff ff 02 00 ff ff ff ff ff ff ff ff ff ff ff ff ff ff 2d 12")
        );
        assert_eq!(
            sn("PageSize=Custom.1.97x1.18in").to_vec(),
            hex("00 40 ff ff 1c 02 2c 01 03 00 ff ff ff ff ff ff ff ff ff ff ff ff ff ff 3e 10")
        );
        assert_eq!(
            sn("TmCoating=Glossy").to_vec(),
            hex("00 40 ff ff 60 04 d4 06 06 00 ff ff ff ff ff ff ff ff ff ff ff ff ff ff 34 11")
        );
    }
}
