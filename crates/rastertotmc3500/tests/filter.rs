//! End-to-end: CUPS raster in, printer job out, compared with the Windows captures.
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tmc3500_core::decode::{decode, skeleton, Kind};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
}

/// Uncompressed little-endian CUPS raster (v3), 8-bit RGB pages.
fn raster(pages: &[(u32, u32, [u32; 2], Vec<u8>)]) -> Vec<u8> {
    let mut out = b"3SaR".to_vec();
    for (w, h, res, rgb) in pages {
        let mut hd = vec![0u8; 1796];
        for (off, v) in [
            (276, res[0]),
            (280, res[1]),
            (340, 1),
            (352, 306),
            (356, 495),
            (372, *w),
            (376, *h),
            (384, 8),
            (388, 24),
            (392, w * 3),
            (400, 1),
        ] {
            hd[off..off + 4].copy_from_slice(&u32::to_le_bytes(v));
        }
        hd[1732..1748].copy_from_slice(b"Label108x174.8mm"); // cupsPageSizeName, as CUPS writes it
        out.extend_from_slice(&hd);
        out.extend_from_slice(rgb);
    }
    out
}

/// A full 108 × 174.8 mm page (1475 × 2477 at 360 dpi) with a kit image at the top-left corner.
fn page_with_image(name: &str) -> (u32, u32, [u32; 2], Vec<u8>) {
    let dec =
        png::Decoder::new(std::fs::File::open(root().join(format!("images/{name}.png"))).unwrap());
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    let (w, h) = (1475u32, 2477u32);
    let mut rgb = vec![255u8; (w * h * 3) as usize];
    for y in 0..info.height {
        let src = &buf[(y * info.width * 3) as usize..((y + 1) * info.width * 3) as usize];
        let d = (y * w * 3) as usize;
        rgb[d..d + src.len()].copy_from_slice(src);
    }
    (w, h, [360, 360], rgb)
}

fn run(args: &[&str], input: &[u8]) -> (i32, Vec<u8>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rastertotmc3500"))
        .args(args)
        .env_remove("PPD")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // feed stdin from a thread: the filter writes output while it is still reading
    let mut stdin = child.stdin.take().unwrap();
    let data = input.to_vec();
    let feeder = std::thread::spawn(move || {
        let _ = stdin.write_all(&data); // the filter may stop reading early
    });
    let out = child.wait_with_output().unwrap();
    feeder.join().unwrap();
    (
        out.status.code().unwrap_or(-1),
        out.stdout,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Skeleton with the job-specific TI / JH payloads blanked.
fn masked_skeleton(b: &[u8]) -> Vec<u8> {
    let mut s = skeleton(b).unwrap();
    for (name, len) in [(&b"TI\x08\x00"[..], 8), (&b"JH\x0c\x00"[..], 12)] {
        if let Some(p) = s.windows(4).position(|w| w == name) {
            for b in &mut s[p + 4..p + 4 + len] {
                *b = 0;
            }
        }
    }
    s
}

/// (y, x, colour, lines, bytes per line) of every raster block.
fn structure(b: &[u8]) -> Vec<(u32, u32, u8, u16, u16)> {
    let (mut y, mut x) = (0, 0);
    let mut out = vec![];
    for t in decode(b).unwrap() {
        match t.kind {
            Kind::Paren { cmd: b'C', .. } => y = 0,
            Kind::Paren { cmd: b'v', payload } => {
                y += u32::from_le_bytes(payload.try_into().unwrap())
            }
            Kind::Paren { cmd: b'$', payload } => {
                x = u32::from_le_bytes(payload.try_into().unwrap())
            }
            Kind::Raster {
                color,
                bytes_per_line,
                lines,
                ..
            } => {
                out.push((y, x, color, lines, bytes_per_line));
                x = 0;
            }
            _ => {}
        }
    }
    out
}

#[test]
fn default_t1_job_matches_capture_000() {
    let (code, out, err) = run(
        &["7", "user", "t1", "1", "PageSize=Label108x174.8mm"],
        &raster(&[page_with_image("t1_black_rect")]),
    );
    assert_eq!(code, 0, "{err}");
    let win = std::fs::read(root().join("captures/000_default_t1.prn")).unwrap();
    assert_eq!(masked_skeleton(&out), masked_skeleton(&win));
    assert_eq!(structure(&out), structure(&win));
    assert!(err.contains("PAGE: 1 1"), "{err}");
}

#[test]
fn t2_bars_match_capture_002() {
    let (code, out, err) = run(
        &["7", "user", "t2", "1", ""],
        &raster(&[page_with_image("t2_cmyk_bars")]),
    );
    assert_eq!(code, 0, "{err}");
    let win = std::fs::read(root().join("captures/002_default_t2.prn")).unwrap();
    assert_eq!(structure(&out), structure(&win));
}

#[test]
fn copies_and_collate() {
    let pages = raster(&[
        page_with_image("t1_black_rect"),
        page_with_image("t1_black_rect"),
    ]);
    let count = |b: &[u8], cmd: &[u8; 2]| {
        decode(b)
            .unwrap()
            .iter()
            .filter(|t| matches!(t.kind, Kind::Remote { cmd: c, .. } if &c == cmd))
            .count()
    };
    let ff = |b: &[u8]| {
        decode(b)
            .unwrap()
            .iter()
            .filter(|t| t.kind == Kind::Data)
            .map(|t| t.raw.iter().filter(|&&c| c == 0x0c).count())
            .sum::<usize>()
    };
    let (code, out, _) = run(&["7", "u", "t", "3", ""], &pages);
    assert_eq!((code, count(&out, b"PM"), ff(&out)), (0, 1, 6));
    let (code, out, _) = run(&["7", "u", "t", "3", "Collate=True"], &pages);
    assert_eq!((code, count(&out, b"PM"), ff(&out)), (0, 3, 6));
    let win = std::fs::read(root().join("captures/104_collate_on_copies3.prn")).unwrap();
    assert_eq!(masked_skeleton(&out), masked_skeleton(&win));
}

#[test]
fn bad_input_and_options_fail_cleanly() {
    let (code, out, err) = run(&["7", "u", "t", "1", ""], b"garbage");
    assert_eq!(code, 1);
    assert!(out.is_empty() && err.contains("ERROR:"), "{err}");
    let (code, _, err) = run(
        &["7", "u", "t", "1", "TmCoating=Shiny"],
        &raster(&[page_with_image("t4_blank")]),
    );
    assert_eq!(code, 1);
    assert!(err.contains("ERROR:") && err.contains("TmCoating"), "{err}");
    let (code, _, err) = run(&["7", "u"], b"");
    assert_eq!(code, 1, "{err}");
}
