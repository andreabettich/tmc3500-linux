//! The PPD must offer exactly the options the driver understands, with the Windows defaults.
use std::collections::BTreeMap;
use std::path::PathBuf;
use tmc3500_core::options::JobConfig;

fn ppd() -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/ppd/tmc3500.ppd"),
    )
    .unwrap()
}

/// option key → (default, choices)
fn ui_options(text: &str) -> BTreeMap<String, (String, Vec<String>)> {
    let mut out = BTreeMap::new();
    let mut cur: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("*OpenUI *") {
            let key = rest.split(['/', ':']).next().unwrap().to_string();
            out.insert(key.clone(), (String::new(), vec![]));
            cur = Some(key);
        } else if line.starts_with("*CloseUI:") {
            let key = cur.take().expect("CloseUI without OpenUI");
            assert!(line.ends_with(&format!("*{key}")), "{line}");
        } else if let Some(key) = &cur {
            if let Some(d) = line.strip_prefix(&format!("*Default{key}: ")) {
                out.get_mut(key).unwrap().0 = d.trim().to_string();
            } else if let Some(c) = line.strip_prefix(&format!("*{key} ")) {
                out.get_mut(key)
                    .unwrap()
                    .1
                    .push(c.split(['/', ':']).next().unwrap().to_string());
            }
        }
    }
    assert!(cur.is_none(), "unclosed OpenUI");
    out
}

const DRIVER_KEYS: &[&str] = &[
    "TmMediaForm",
    "TmMediaSaving",
    "TmCoating",
    "TmQuality",
    "TmBandingReduction",
    "TmAfterPrint",
    "TmCutInterval",
    "TmNotify",
    "TmBorderless",
    "TmBidirectional",
    "TmRotate180",
    "TmSkipBlank",
    "TmInkAdjust",
    "TmVacuum",
    "TmPosAdjH",
    "TmPosAdjV",
    "TmBlack",
    "TmColorMode",
    "TmGamma",
    "TmInkBkCMY",
    "TmInkBk",
    "TmBrightness",
];

#[test]
fn ppd_is_well_formed() {
    let text = ppd();
    assert!(text.starts_with("*PPD-Adobe: \"4.3\""));
    assert!(text.is_ascii());
    assert!(text.lines().all(|l| l.len() <= 255));
    assert!(text.contains("*cupsFilter2: \"application/vnd.cups-raster application/vnd.cups-raw 100 rastertotmc3500\""));
    assert!(text.contains("*cupsManualCopies: False"));
    for (key, (default, choices)) in ui_options(&text) {
        assert!(
            choices.contains(&default),
            "{key}: default {default} not among choices"
        );
    }
}

#[test]
fn every_driver_option_is_offered_and_every_choice_parses() {
    let opts = ui_options(&ppd());
    for key in DRIVER_KEYS {
        assert!(opts.contains_key(*key), "{key} missing from PPD");
    }
    for (key, (_, choices)) in &opts {
        if key.starts_with("Tm") {
            assert!(
                DRIVER_KEYS.contains(&key.as_str()),
                "PPD offers unknown option {key}"
            );
        }
        for c in choices {
            let r = JobConfig::from_cups(&format!("{key}={c}"), 1);
            assert!(r.is_ok(), "{key}={c}: {r:?}");
        }
    }
}

#[test]
fn ppd_defaults_are_the_windows_defaults() {
    let defaults = rastertotmc3500::ppd_defaults(&ppd());
    assert_eq!(
        JobConfig::from_cups(&defaults, 1).unwrap(),
        JobConfig::default()
    );
}

#[test]
fn rejected_combinations_are_constrained_in_the_ppd() {
    let text = ppd();
    let opts = ui_options(&text);
    let keys = ["TmCoating", "TmQuality", "TmMediaForm", "TmAfterPrint"];
    for (i, a) in keys.iter().enumerate() {
        for b in &keys[i + 1..] {
            for ca in &opts[*a].1 {
                for cb in &opts[*b].1 {
                    if JobConfig::from_cups(&format!("{a}={ca} {b}={cb}"), 1).is_err() {
                        for line in [
                            format!("*UIConstraints: *{a} {ca} *{b} {cb}"),
                            format!("*UIConstraints: *{b} {cb} *{a} {ca}"),
                        ] {
                            assert!(text.lines().any(|l| l == line), "missing `{line}`");
                        }
                    }
                }
            }
        }
    }
}
