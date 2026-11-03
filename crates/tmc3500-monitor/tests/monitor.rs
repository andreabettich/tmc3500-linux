use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tmc3500_common::alert::Alerter;
use tmc3500_core::status::printer::{
    oids, supply_oid, EventKind, PrinterStatus, Thresholds, SUPPLY_DESCRIPTION, SUPPLY_LEVEL,
    SUPPLY_MAX_CAPACITY,
};
use tmc3500_core::status::snmp::{Oid, Value, VarBind};
use tmc3500_fakeprinter::{FakePrinter, SnmpMode};
use tmc3500_monitor::{render_metrics, Monitor, Poll};

const BIN: &str = env!("CARGO_BIN_EXE_tmc3500-monitor");

struct H {
    dir: tempfile::TempDir,
}

impl H {
    fn new() -> H {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("alert.d")).unwrap();
        std::fs::create_dir(dir.path().join("state")).unwrap();
        std::fs::create_dir(dir.path().join("metrics")).unwrap();
        let hook = dir.path().join("alert.d/10-record");
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\ncat >> {}\n",
                dir.path().join("alerts.jsonl").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        H { dir }
    }
    fn alerter(&self) -> Alerter {
        Alerter::new(
            self.dir.path().join("alert.d"),
            Duration::from_secs(2),
            Duration::from_secs(3600),
            Some(self.dir.path().join("state/alerts.json")),
        )
    }
    fn events(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("alerts.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).unwrap();
                v["event"].as_str().unwrap().to_string()
            })
            .collect()
    }
    fn textfile(&self) -> PathBuf {
        self.dir.path().join("metrics/tmc3500.prom")
    }
    fn config(&self, fake: &FakePrinter) -> PathBuf {
        let p = self.dir.path().join("c.toml");
        let d = self.dir.path().display();
        std::fs::write(
            &p,
            format!(
                "[printer]\nhost = \"127.0.0.1\"\n[snmp]\nport = {}\ntimeout_ms = 200\nretries = 0\n\
                 [alerts]\ndir = \"{d}/alert.d\"\nstate_dir = \"{d}/state\"\n\
                 [monitor]\ninterval_ms = 60000\ntextfile = \"{}\"\n",
                fake.snmp_port(),
                self.textfile().display()
            ),
        )
        .unwrap();
        p
    }
}

fn vb(o: &str, v: Value) -> VarBind {
    VarBind::new(Oid::parse(o).unwrap(), v)
}

fn status(err_bits: u8, cyan: i64) -> PrinterStatus {
    let mut v = vec![
        vb(oids::HR_DEVICE_STATUS, Value::Integer(2)),
        vb(oids::HR_PRINTER_STATUS, Value::Integer(3)),
        vb(
            oids::HR_PRINTER_DETECTED_ERROR_STATE,
            Value::OctetString(vec![err_bits, 0]),
        ),
    ];
    for (i, (name, lvl)) in [("Black SJIC22P(K)", 70), ("Cyan SJIC22P(C)", cyan)]
        .iter()
        .enumerate()
    {
        let x = i as u32 + 1;
        v.push(VarBind::new(
            supply_oid(SUPPLY_DESCRIPTION, x),
            Value::OctetString(name.as_bytes().to_vec()),
        ));
        v.push(VarBind::new(
            supply_oid(SUPPLY_MAX_CAPACITY, x),
            Value::Integer(100),
        ));
        v.push(VarBind::new(
            supply_oid(SUPPLY_LEVEL, x),
            Value::Integer(*lvl),
        ));
    }
    PrinterStatus::from_varbinds(&v)
}

#[test]
fn transitions_produce_events() {
    let h = H::new();
    let mut m = Monitor::new(
        "192.0.2.9".into(),
        Some("TM-C3500".into()),
        h.alerter(),
        Thresholds::default(),
    );
    let ok = status(0, 50);
    let names =
        |v: Vec<tmc3500_common::alert::Event>| v.into_iter().map(|e| e.event).collect::<Vec<_>>();
    // first poll, healthy: nothing
    assert!(names(m.step(&Ok(ok.clone()))).is_empty());
    // paper out + cyan low
    let e = names(m.step(&Ok(status(0x40, 5))));
    assert_eq!(e, vec!["state-change", "media-out", "ink-low"]);
    // same again: suppressed
    assert!(names(m.step(&Ok(status(0x40, 5)))).is_empty());
    // unreachable
    assert_eq!(names(m.step(&Err("timeout".into()))), vec!["unreachable"]);
    assert!(names(m.step(&Err("timeout".into()))).is_empty());
    // back, paper still out: recovered (media-out still suppressed, it never cleared)
    assert_eq!(names(m.step(&Ok(status(0x40, 5)))), vec!["recovered"]);
    // paper back: state change
    assert_eq!(names(m.step(&Ok(status(0, 5)))), vec!["state-change"]);
    // paper out again: alerts again because the condition cleared in between
    let e = names(m.step(&Ok(status(0x40, 5))));
    assert_eq!(e, vec!["state-change", "media-out"]);
    // second outage alerts again
    assert_eq!(names(m.step(&Err("timeout".into()))), vec!["unreachable"]);
    assert_eq!(h.events().len(), 9);
    let _ = EventKind::Recovered;
}

#[test]
fn metrics_text() {
    let st = status(0x40, 5);
    let snap = Ok(st);
    let poll = |snap| Poll {
        printer: "192.0.2.9",
        queue: "TM-C3500",
        snap,
    };
    let t = render_metrics(&[poll(&snap)], &Thresholds::default(), 1_700_000_000);
    let p = "printer=\"192.0.2.9\",queue=\"TM-C3500\"";
    for want in [
        "# TYPE tmc3500_up gauge".to_string(),
        format!("tmc3500_up{{{p}}} 1"),
        format!("tmc3500_last_poll_timestamp_seconds{{{p}}} 1700000000"),
        format!("tmc3500_device_status{{{p}}} 2"),
        format!("tmc3500_state_reason{{{p},reason=\"media-empty-error\"}} 1"),
        format!("tmc3500_blocking{{{p}}} 1"),
        format!("tmc3500_marker_level_percent{{{p},index=\"2\",marker=\"cyan\",description=\"Cyan SJIC22P(C)\"}} 5"),
    ] {
        assert!(t.lines().any(|l| l == want), "missing {want:?} in\n{t}");
    }
    let down = Err("timeout".into());
    let t = render_metrics(
        &[Poll {
            printer: "p\"x",
            queue: "Q",
            snap: &down,
        }],
        &Thresholds::default(),
        5,
    );
    assert!(
        t.contains("tmc3500_up{printer=\"p\\\"x\",queue=\"Q\"} 0"),
        "{t}"
    );
    assert!(!t.contains("tmc3500_device_status"));
}

#[test]
fn once_against_fake_writes_metrics_and_alerts() {
    let h = H::new();
    let fake = FakePrinter::start_local().unwrap();
    fake.set_paper_out(true);
    let o = Command::new(BIN)
        .args(["--once"])
        .env("TMC3500_CONFIG", h.config(&fake))
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let t = std::fs::read_to_string(h.textfile()).unwrap();
    assert!(
        t.contains("tmc3500_up{printer=\"127.0.0.1\",queue=\"TM-C3500\"} 1"),
        "{t}"
    );
    assert!(t.contains("reason=\"media-empty-error\""), "{t}");
    assert_eq!(h.events(), vec!["media-out"]);
    // no temp files left behind
    let n = std::fs::read_dir(h.dir.path().join("metrics"))
        .unwrap()
        .count();
    assert_eq!(n, 1);
}

#[test]
fn once_unreachable() {
    let h = H::new();
    let fake = FakePrinter::start_local().unwrap();
    fake.set_snmp_mode(SnmpMode::Silent);
    let o = Command::new(BIN)
        .args(["--once"])
        .env("TMC3500_CONFIG", h.config(&fake))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    let t = std::fs::read_to_string(h.textfile()).unwrap();
    assert!(
        t.contains("tmc3500_up{printer=\"127.0.0.1\",queue=\"TM-C3500\"} 0"),
        "{t}"
    );
    assert_eq!(h.events(), vec!["unreachable"]);
}

#[test]
fn sigterm_stops_the_loop_promptly() {
    let h = H::new();
    let fake = FakePrinter::start_local().unwrap();
    let mut child = Command::new(BIN)
        .env("TMC3500_CONFIG", h.config(&fake))
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let t = Instant::now();
    while !h.textfile().exists() {
        assert!(t.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    let st = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(st.success());
    let t = Instant::now();
    let code = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s.code();
        }
        assert!(t.elapsed() < Duration::from_secs(2), "monitor did not stop");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(code, Some(0));
}

#[test]
fn no_printer_configured_is_an_error() {
    let h = H::new();
    let p = h.dir.path().join("empty.toml");
    std::fs::write(&p, "").unwrap();
    let o = Command::new(BIN)
        .args(["--once"])
        .env("TMC3500_CONFIG", &p)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("setup"));
}

#[test]
fn textfile_mode_is_configurable() {
    use std::os::unix::fs::PermissionsExt;
    let h = H::new();
    let fake = FakePrinter::start_local().unwrap();
    let cfg = h.config(&fake);
    let text = std::fs::read_to_string(&cfg).unwrap();
    std::fs::write(&cfg, format!("{text}textfile_mode = 0o640\n")).unwrap();
    let o = Command::new(BIN)
        .args(["--once"])
        .env("TMC3500_CONFIG", &cfg)
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let m = std::fs::metadata(h.textfile())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(m & 0o777, 0o640);
}

#[test]
fn metrics_for_several_printers_declare_each_metric_once() {
    let up = Ok(status(0x40, 5));
    let down = Err("timeout".to_string());
    let t = render_metrics(
        &[
            Poll {
                printer: "192.0.2.1",
                queue: "TM-C3500-1",
                snap: &up,
            },
            Poll {
                printer: "192.0.2.2",
                queue: "TM-C3500-2",
                snap: &down,
            },
            Poll {
                printer: "192.0.2.3",
                queue: "TM-C3500-3",
                snap: &up,
            },
        ],
        &Thresholds::default(),
        7,
    );
    for want in [
        "tmc3500_up{printer=\"192.0.2.1\",queue=\"TM-C3500-1\"} 1",
        "tmc3500_up{printer=\"192.0.2.2\",queue=\"TM-C3500-2\"} 0",
        "tmc3500_up{printer=\"192.0.2.3\",queue=\"TM-C3500-3\"} 1",
        "tmc3500_blocking{printer=\"192.0.2.3\",queue=\"TM-C3500-3\"} 1",
    ] {
        assert!(t.lines().any(|l| l == want), "missing {want:?} in\n{t}");
    }
    // the text format needs one HELP/TYPE per metric and all its samples together
    let mut seen: Vec<&str> = Vec::new();
    let mut current = "";
    for l in t.lines() {
        if let Some(rest) = l.strip_prefix("# TYPE ") {
            let name = rest.split(' ').next().unwrap();
            assert!(!seen.contains(&name), "{name} declared twice:\n{t}");
            seen.push(name);
            current = name;
        } else if !l.starts_with('#') {
            let name = l.split('{').next().unwrap();
            assert_eq!(name, current, "sample outside its family:\n{t}");
        }
    }
    assert!(!t.contains("printer=\"192.0.2.2\",queue=\"TM-C3500-2\"} 2"));
}

#[test]
fn once_polls_every_configured_printer() {
    let h = H::new();
    let a = FakePrinter::start_local().unwrap();
    // a second loopback address, so the two printers have different hosts
    let b = FakePrinter::start(tmc3500_fakeprinter::Options {
        bind: "127.0.0.2".parse().unwrap(),
        ..Default::default()
    })
    .unwrap();
    let p = h.dir.path().join("multi.toml");
    let d = h.dir.path().display();
    let write = |extra: &str| {
        std::fs::write(
            &p,
            format!(
                "[snmp]\ntimeout_ms = 200\nretries = 0\n\
                 [alerts]\ndir = \"{d}/alert.d\"\nstate_dir = \"{d}/state\"\n\
                 [monitor]\ninterval_ms = 60000\ntextfile = \"{}\"\n\
                 [[printers]]\nqueue = \"TM-C3500-1\"\nhost = \"127.0.0.1\"\nsnmp_port = {}\n\
                 [[printers]]\nqueue = \"TM-C3500-2\"\nhost = \"127.0.0.2\"\nsnmp_port = {}\n{extra}",
                h.textfile().display(),
                a.snmp_port(),
                b.snmp_port()
            ),
        )
        .unwrap()
    };
    write("");
    let once = || {
        Command::new(BIN)
            .args(["--once"])
            .env("TMC3500_CONFIG", &p)
            .output()
            .unwrap()
    };
    // both out of paper: one alert per printer
    a.set_paper_out(true);
    b.set_paper_out(true);
    let o = once();
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let t = std::fs::read_to_string(h.textfile()).unwrap();
    assert!(
        t.contains("tmc3500_up{printer=\"127.0.0.1\",queue=\"TM-C3500-1\"} 1"),
        "{t}"
    );
    assert!(
        t.contains("tmc3500_up{printer=\"127.0.0.2\",queue=\"TM-C3500-2\"} 1"),
        "{t}"
    );
    let raw = std::fs::read_to_string(h.dir.path().join("alerts.jsonl")).unwrap();
    let ev: Vec<serde_json::Value> = raw
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let mut queues: Vec<&str> = ev
        .iter()
        .filter(|e| e["event"] == "media-out")
        .map(|e| e["queue"].as_str().unwrap())
        .collect();
    queues.sort();
    assert_eq!(queues, vec!["TM-C3500-1", "TM-C3500-2"], "{raw}");
    // one printer unreachable: exit 2, the other still reported
    b.set_snmp_mode(SnmpMode::Silent);
    let o = once();
    assert_eq!(o.status.code(), Some(2));
    let t = std::fs::read_to_string(h.textfile()).unwrap();
    assert!(t.contains("queue=\"TM-C3500-1\"} 1"), "{t}");
    assert!(
        t.contains("tmc3500_up{printer=\"127.0.0.2\",queue=\"TM-C3500-2\"} 0"),
        "{t}"
    );
}
