use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};
use tmc3500_common::alert::{Alerter, Event, HookOutcome};
use tmc3500_core::status::printer::EventKind;

fn hook(dir: &Path, name: &str, body: &str, mode: u32) {
    let p = dir.join(name);
    fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(mode)).unwrap();
}

fn setup() -> (tempfile::TempDir, Alerter) {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("alert.d")).unwrap();
    fs::create_dir(d.path().join("state")).unwrap();
    let a = Alerter::new(
        d.path().join("alert.d"),
        Duration::from_millis(500),
        Duration::from_secs(3600),
        Some(d.path().join("state/alerts.json")),
    );
    (d, a)
}

fn ev(kind: EventKind, key: &str) -> Event {
    Event::new(kind, key, "192.0.2.5", "test message", "test")
}

#[test]
fn event_json_shape() {
    let mut e = ev(EventKind::InkLow, "ink-low:cyan");
    e.job_id = Some("42".into());
    e.reasons = vec!["marker-supply-low-warning".into()];
    let v: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
    assert_eq!(v["event"], "ink-low");
    assert_eq!(v["key"], "ink-low:cyan");
    assert_eq!(v["severity"], "warning");
    assert_eq!(v["printer"], "192.0.2.5");
    assert_eq!(v["source"], "test");
    assert_eq!(v["job_id"], "42");
    assert_eq!(v["reasons"][0], "marker-supply-low-warning");
    assert!(v["time"].as_str().unwrap().ends_with('Z'));
    assert!(v["timestamp"].as_u64().unwrap() > 1_700_000_000);
}

#[test]
fn runs_executable_hooks_with_json_on_stdin() {
    let (d, a) = setup();
    let out = d.path().join("out");
    let ad = d.path().join("alert.d");
    hook(
        &ad,
        "10-a",
        &format!("cat > {}/a.json", out.display()),
        0o755,
    );
    hook(
        &ad,
        "20-b",
        &format!("echo \"$TMC3500_EVENT\" > {}/b.txt", out.display()),
        0o755,
    );
    hook(
        &ad,
        "30-not-exec",
        &format!("touch {}/c", out.display()),
        0o644,
    );
    hook(
        &ad,
        "40-world-writable",
        &format!("touch {}/d", out.display()),
        0o777,
    );
    hook(&ad, ".hidden", &format!("touch {}/e", out.display()), 0o755);
    hook(
        &ad,
        "50-backup~",
        &format!("touch {}/f", out.display()),
        0o755,
    );
    fs::create_dir(&out).unwrap();
    let r = a.emit(&ev(EventKind::MediaOut, "media-out"));
    assert!(!r.suppressed);
    assert_eq!(r.hooks.len(), 2, "{r:?}");
    assert!(
        r.hooks.iter().all(|h| h.outcome == HookOutcome::Ok),
        "{r:?}"
    );
    let j: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.join("a.json")).unwrap()).unwrap();
    assert_eq!(j["event"], "media-out");
    assert_eq!(
        fs::read_to_string(out.join("b.txt")).unwrap().trim(),
        "media-out"
    );
    for f in ["c", "d", "e", "f"] {
        assert!(!out.join(f).exists(), "{f} ran");
    }
    assert!(
        r.warnings.iter().any(|w| w.contains("world-writable")),
        "{r:?}"
    );
}

#[test]
fn slow_hook_is_killed_after_timeout() {
    let (d, a) = setup();
    hook(&d.path().join("alert.d"), "slow", "sleep 10", 0o755);
    hook(&d.path().join("alert.d"), "fail", "exit 3", 0o755);
    let t = Instant::now();
    let r = a.emit(&ev(EventKind::MediaJam, "media-jam"));
    assert!(t.elapsed() < Duration::from_secs(3));
    let outcomes: Vec<_> = r.hooks.iter().map(|h| h.outcome.clone()).collect();
    assert_eq!(
        outcomes,
        vec![HookOutcome::Failed(3), HookOutcome::TimedOut]
    );
}

#[test]
fn repeats_are_suppressed_until_cleared() {
    let (d, a) = setup();
    let count = d.path().join("count");
    hook(
        &d.path().join("alert.d"),
        "h",
        &format!("echo x >> {}", count.display()),
        0o755,
    );
    assert!(!a.emit(&ev(EventKind::InkLow, "ink-low:cyan")).suppressed);
    assert!(a.emit(&ev(EventKind::InkLow, "ink-low:cyan")).suppressed);
    // other keys are independent
    assert!(!a.emit(&ev(EventKind::InkLow, "ink-low:black")).suppressed);
    // state persists across instances
    let b = Alerter::new(
        d.path().join("alert.d"),
        Duration::from_millis(500),
        Duration::from_secs(3600),
        Some(d.path().join("state/alerts.json")),
    );
    assert!(b.emit(&ev(EventKind::InkLow, "ink-low:cyan")).suppressed);
    b.clear("ink-low:cyan");
    assert!(!a.emit(&ev(EventKind::InkLow, "ink-low:cyan")).suppressed);
    assert_eq!(fs::read_to_string(&count).unwrap().lines().count(), 3);
}

#[test]
fn zero_interval_never_suppresses_and_corrupt_state_is_ignored() {
    let (d, _) = setup();
    let state = d.path().join("state/alerts.json");
    fs::write(&state, "{not json").unwrap();
    let a = Alerter::new(
        d.path().join("alert.d"),
        Duration::from_millis(500),
        Duration::ZERO,
        Some(state),
    );
    let r = a.emit(&ev(EventKind::CoverOpen, "cover-open"));
    assert!(!r.suppressed);
    assert!(r.warnings.iter().any(|w| w.contains("state")), "{r:?}");
    assert!(!a.emit(&ev(EventKind::CoverOpen, "cover-open")).suppressed);
}

#[test]
fn missing_alert_dir_and_unwritable_state_are_not_fatal() {
    let d = tempfile::tempdir().unwrap();
    let a = Alerter::new(
        d.path().join("nope"),
        Duration::from_millis(500),
        Duration::from_secs(60),
        Some(d.path().join("no/such/dir/alerts.json")),
    );
    let r = a.emit(&ev(EventKind::Unreachable, "unreachable"));
    assert!(!r.suppressed);
    assert!(r.hooks.is_empty());
    assert!(!r.warnings.is_empty());
}
