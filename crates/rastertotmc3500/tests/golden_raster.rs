//! Every Windows capture, rebuilt from its source image through the filter: the command skeleton
//! (with TI/JH masked) and the raster block structure (pass positions, offsets, colours, widths)
//! must match. Printed with the provisional colours: block edges follow where sparse dots fall,
//! which depends on the colour table and the dither (ours differs from Windows' by design), and
//! with the fitted tables 9 captures move an edge by 16-64 dots. Colour accuracy is checked
//! against the Windows chart captures in crates/xtask/tests/fit.rs.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tmc3500_core::decode::{decode, skeleton, Kind};
use tmc3500_core::layout::print_area;
use tmc3500_core::options::JobConfig;

#[derive(Deserialize)]
struct Golden {
    captures: BTreeMap<String, Entry>,
}

#[derive(Deserialize)]
struct Entry {
    #[serde(default)]
    options: String,
    copies: Option<u32>,
    skip: Option<String>,
    raster: Option<String>,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
}

fn image(name: &str) -> (u32, u32, Vec<u8>) {
    let dec =
        png::Decoder::new(std::fs::File::open(root().join(format!("images/{name}"))).unwrap());
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    (info.width, info.height, buf)
}

/// CUPS raster (v3 LE) of `pages` identical pages covering the print area, image at the top left.
fn raster(cfg: &JobConfig, img: &(u32, u32, Vec<u8>), pages: usize, landscape: bool) -> Vec<u8> {
    let a = print_area(cfg);
    let (w, h) = (a.width, a.height);
    let (iw, ih, px) = img;
    let mut rgb = vec![255u8; (w * h * 3) as usize];
    for y in 0..*ih {
        for x in 0..*iw {
            // landscape: the page is turned 90°: image (x, y) lands at (W/hscale - 1 - y, x)
            let (tx, ty) = if landscape {
                (w / a.hscale - 1 - y, x)
            } else {
                (x, y)
            };
            if ty >= h {
                continue;
            }
            let s = ((y * iw + x) * 3) as usize;
            for k in 0..a.hscale {
                let dx = tx * a.hscale + k;
                if dx < w {
                    let d = ((ty * w + dx) * 3) as usize;
                    rgb[d..d + 3].copy_from_slice(&px[s..s + 3]);
                }
            }
        }
    }
    let mut out = b"3SaR".to_vec();
    for _ in 0..pages {
        let mut hd = vec![0u8; 1796];
        for (off, v) in [
            (276, 360 * a.hscale),
            (280, 360),
            (340, 1),
            (372, w),
            (376, h),
            (384, 8),
            (388, 24),
            (392, w * 3),
            (400, 1),
        ] {
            hd[off..off + 4].copy_from_slice(&u32::to_le_bytes(v));
        }
        out.extend_from_slice(&hd);
        out.extend_from_slice(&rgb);
    }
    out
}

fn masked_skeleton(b: &[u8]) -> Vec<u8> {
    let mut s = skeleton(b).unwrap();
    for (name, len) in [(&b"TI\x08\x00"[..], 8), (&b"JH\x0c\x00"[..], 12)] {
        if let Some(p) = s.windows(4).position(|w| w == name) {
            s[p + 4..p + 4 + len].fill(0);
        }
    }
    s
}

type Block = (u32, u32, u8, u16, u16);

/// Per page: (y, x, colour, lines, bytes per line) of every raster block.
fn structure(b: &[u8]) -> Vec<Vec<Block>> {
    let (mut y, mut x) = (0, 0);
    let mut pages = vec![];
    for t in decode(b).unwrap() {
        match t.kind {
            Kind::Paren { cmd: b'C', .. } => {
                y = 0;
                pages.push(vec![]);
            }
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
                pages
                    .last_mut()
                    .unwrap()
                    .push((y, x, color, lines, bytes_per_line));
                x = 0;
            }
            _ => {}
        }
    }
    pages
}

fn first_diff(a: &[u8], b: &[u8]) -> String {
    let i = a
        .iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let s = i.saturating_sub(12);
    format!(
        "skeleton @{i:#x}: ours {:02x?} windows {:02x?}",
        &a[s..(i + 12).min(a.len())],
        &b[s..(i + 12).min(b.len())]
    )
}

#[test]
fn filter_output_matches_every_capture() {
    let g: Golden =
        toml::from_str(&std::fs::read_to_string(root().join("golden.toml")).unwrap()).unwrap();
    let manifest = std::fs::read_to_string(root().join("captures/manifest.csv")).unwrap();
    let rows: BTreeMap<String, (String, u32, usize)> = manifest
        .lines()
        .skip(1)
        .map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            (
                f[1].to_string(),
                (
                    f[2].to_string(),
                    f[3].parse().unwrap(),
                    f[4].parse().unwrap(),
                ),
            )
        })
        .collect();
    let env = rastertotmc3500::Env {
        ppd: None,
        colour_dir: None,
        unix_secs: 0,
        host_id: rastertotmc3500::FALLBACK_HOST_ID,
    };
    let (mut failures, mut checked, mut structured) = (vec![], 0, 0);
    for (name, e) in &g.captures {
        if e.skip.is_some() {
            continue;
        }
        let (img, mcopies, pages) = &rows[name];
        let copies = e.copies.unwrap_or(*mcopies);
        let options = if e.options.contains("PageSize=") {
            e.options.clone()
        } else {
            format!("PageSize=Label108x174.8mm {}", e.options)
        };
        let cfg = JobConfig::from_cups(&options, copies).unwrap();
        let ras = raster(
            &cfg,
            &image(img),
            *pages,
            e.options.contains("orientation-requested=4"),
        );
        let args: Vec<String> = [
            "rastertotmc3500",
            "1",
            "u",
            "t",
            &copies.to_string(),
            &options,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mut out = Vec::new();
        let mut log = Vec::new();
        let code = rastertotmc3500::run(&args, &ras[..], &mut out, &mut log, &env);
        if code != 0 {
            failures.push(format!(
                "{name}: exit {code}: {}",
                String::from_utf8_lossy(&log)
            ));
            continue;
        }
        let win = std::fs::read(root().join(format!("captures/{name}.prn"))).unwrap();
        let (a, b) = (masked_skeleton(&out), masked_skeleton(&win));
        if a != b {
            failures.push(format!("{name}: {}", first_diff(&a, &b)));
        }
        checked += 1;
        if e.raster.as_deref() == Some("skip") {
            continue;
        }
        let (sa, sb) = (structure(&out), structure(&win));
        if sa != sb {
            let p = sa.iter().zip(&sb).position(|(x, y)| x != y).unwrap_or(0);
            failures.push(format!(
                "{name}: raster structure differs on page {p}: ours {:?} windows {:?}",
                sa.get(p).map(|v| &v[..v.len().min(10)]),
                sb.get(p).map(|v| &v[..v.len().min(10)])
            ));
        }
        structured += 1;
    }
    assert!(
        failures.is_empty(),
        "{} captures differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        checked >= 86 && structured >= 75,
        "checked {checked}, structure {structured}"
    );
}
