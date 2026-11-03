//! Byte-exact comparison with the Windows driver captures.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tmc3500_core::decode::{decode, skeleton, Kind};
use tmc3500_core::options::JobConfig;
use tmc3500_core::protocol::job::{encode_job, JobMeta, Page};
use tmc3500_core::rle;

#[derive(Deserialize)]
struct Golden {
    captures: BTreeMap<String, Entry>,
}

#[derive(Deserialize, Default)]
struct Entry {
    #[serde(default)]
    options: String,
    copies: Option<u32>,
    skip: Option<String>,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
}

struct ManifestRow {
    image: String,
    copies: u32,
    pages: usize,
}

fn manifest() -> BTreeMap<String, ManifestRow> {
    let text = std::fs::read_to_string(root().join("captures/manifest.csv")).unwrap();
    text.lines()
        .skip(1)
        .map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            (
                f[1].to_string(),
                ManifestRow {
                    image: f[2].into(),
                    copies: f[3].parse().unwrap(),
                    pages: f[4].parse().unwrap(),
                },
            )
        })
        .collect()
}

fn meta_from_capture(b: &[u8]) -> JobMeta {
    let toks = decode(b).unwrap();
    let ti = toks
        .iter()
        .find_map(|t| match t.kind {
            Kind::Remote {
                cmd: [b'T', b'I'],
                payload,
            } => Some(payload),
            _ => None,
        })
        .unwrap();
    let jh = toks
        .iter()
        .find_map(|t| match t.kind {
            Kind::Remote {
                cmd: [b'J', b'H'],
                payload,
            } => Some(payload),
            _ => None,
        })
        .unwrap();
    JobMeta {
        year: u16::from_be_bytes([ti[1], ti[2]]),
        month: ti[3],
        day: ti[4],
        hour: ti[5],
        minute: ti[6],
        second: ti[7],
        job_id: u32::from_le_bytes([jh[2], jh[3], jh[4], jh[5]]),
        host_id: jh[6..12].try_into().unwrap(),
    }
}

/// Human-readable first difference, for failure messages.
fn first_diff(a: &[u8], b: &[u8]) -> String {
    let i = a
        .iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let s = i.saturating_sub(16);
    format!(
        "at skeleton offset {i:#x}: ours {:02x?} vs windows {:02x?}",
        &a[s..(i + 16).min(a.len())],
        &b[s..(i + 16).min(b.len())]
    )
}

#[test]
fn every_capture_has_a_golden_entry() {
    let g: Golden =
        toml::from_str(&std::fs::read_to_string(root().join("golden.toml")).unwrap()).unwrap();
    for name in manifest().keys() {
        assert!(
            g.captures.contains_key(name),
            "capture {name} missing from golden.toml"
        );
    }
}

#[test]
fn command_skeleton_matches_windows_byte_for_byte() {
    let g: Golden =
        toml::from_str(&std::fs::read_to_string(root().join("golden.toml")).unwrap()).unwrap();
    let m = manifest();
    let mut failures = Vec::new();
    let mut checked = 0;
    for (name, e) in &g.captures {
        if let Some(reason) = &e.skip {
            eprintln!("skip {name}: {reason}");
            continue;
        }
        if e.options.contains("TmMediaSaving") {
            // page length depends on the printed area: covered by the filter golden test
            eprintln!("skip {name}: media saving needs raster data");
            continue;
        }
        let row = &m[name];
        let capture = std::fs::read(root().join(format!("captures/{name}.prn"))).unwrap();
        let cfg = match JobConfig::from_cups(&e.options, e.copies.unwrap_or(row.copies)) {
            Ok(c) => c,
            Err(err) => {
                failures.push(format!("{name}: options error {err}"));
                continue;
            }
        };
        let blank = row.image == "t4_blank.png";
        let pages = vec![
            Page {
                raster: vec![],
                blank,
                length: None,
            };
            row.pages
        ];
        let ours = encode_job(&cfg, &meta_from_capture(&capture), &pages);
        let (a, b) = (skeleton(&ours).unwrap(), skeleton(&capture).unwrap());
        if a != b {
            failures.push(format!("{name}: {}", first_diff(&a, &b)));
        }
        checked += 1;
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} captures differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(checked >= 82, "only {checked} captures checked");
}

#[test]
fn rle_reproduces_every_captured_raster_block() {
    let mut blocks = 0;
    for entry in std::fs::read_dir(root().join("captures")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("prn") {
            continue;
        }
        let b = std::fs::read(&path).unwrap();
        for t in decode(&b).unwrap() {
            if let Kind::Raster {
                compressed: true,
                bytes_per_line,
                lines,
                data,
                ..
            } = t.kind
            {
                let n = usize::from(bytes_per_line) * usize::from(lines);
                let raw = rle::decode(data, n).unwrap();
                assert_eq!(
                    rle::encode(&raw, usize::from(bytes_per_line)),
                    data,
                    "{} @ {:#x}",
                    path.display(),
                    t.offset
                );
                blocks += 1;
            }
        }
    }
    assert!(blocks > 1500, "only {blocks} raster blocks found");
}
