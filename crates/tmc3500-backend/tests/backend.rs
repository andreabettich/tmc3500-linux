//! The `tmc3500` backend binary against the fake printer.
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use tmc3500_fakeprinter::{FakePrinter, JobOutcome, SnmpMode};

const BIN: &str = env!("CARGO_BIN_EXE_tmc3500");

fn capture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/captures")
            .join(name),
    )
    .unwrap()
}

fn big_job() -> Vec<u8> {
    capture("320_quality_glossy_mode2.prn")
}

struct H {
    dir: tempfile::TempDir,
    fake: FakePrinter,
    job_file: PathBuf,
    extra_backend: String,
}

impl H {
    fn new(job: &[u8]) -> H {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakePrinter::start_local().unwrap();
        let job_file = dir.path().join("job.prn");
        std::fs::write(&job_file, job).unwrap();
        std::fs::create_dir(dir.path().join("alert.d")).unwrap();
        std::fs::create_dir(dir.path().join("state")).unwrap();
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
        H {
            dir,
            fake,
            job_file,
            extra_backend: String::new(),
        }
    }

    fn config(&self) -> PathBuf {
        let p = self.dir.path().join("tmc3500.toml");
        let d = self.dir.path().display();
        let mut backend = String::from(
            "connect_timeout_ms = 300\nio_timeout_ms = 300\nstatus_interval_ms = 50\n\
             retry_initial_ms = 20\nretry_max_ms = 100\nunreachable_hold_after_ms = 600\n\
             stall_timeout_ms = 3000\ndrain_timeout_ms = 2000\n",
        );
        // later keys win: rewrite overridden ones
        for line in self.extra_backend.lines() {
            let key = line.split('=').next().unwrap().trim();
            backend = backend
                .lines()
                .filter(|l| !l.starts_with(key))
                .map(|l| format!("{l}\n"))
                .collect();
            backend.push_str(line);
            backend.push('\n');
        }
        std::fs::write(
            &p,
            format!(
                "[snmp]\ntimeout_ms = 200\nretries = 0\n\n[backend]\n{backend}\n\
                 [alerts]\ndir = \"{d}/alert.d\"\nhook_timeout_ms = 2000\n\
                 repeat_interval_s = 3600\nstate_dir = \"{d}/state\"\n"
            ),
        )
        .unwrap();
        p
    }

    fn uri(&self) -> String {
        format!(
            "tmc3500://127.0.0.1:{}?snmp-port={}",
            self.fake.job_port(),
            self.fake.snmp_port()
        )
    }

    fn cmd(&self, uri: &str) -> Command {
        let mut c = Command::new(BIN);
        c.env("DEVICE_URI", uri)
            .env("TMC3500_CONFIG", self.config())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        c
    }

    fn spawn_file(&self, uri: &str) -> Child {
        self.cmd(uri)
            .args([
                "42",
                "alice",
                "label",
                "1",
                "",
                self.job_file.to_str().unwrap(),
            ])
            .stdin(Stdio::null())
            .spawn()
            .unwrap()
    }

    fn run_file(&self) -> Run {
        finish(self.spawn_file(&self.uri()), Duration::from_secs(20))
    }

    fn alerts(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.dir.path().join("alerts.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn alert_names(&self) -> Vec<String> {
        self.alerts()
            .iter()
            .map(|a| a["event"].as_str().unwrap().to_string())
            .collect()
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
    elapsed: Duration,
}

fn finish(mut child: Child, limit: Duration) -> Run {
    let t = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if t.elapsed() > limit {
            child.kill().unwrap();
            let o = child.wait_with_output().unwrap();
            panic!(
                "backend hung; stderr:\n{}",
                String::from_utf8_lossy(&o.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let o: Output = child.wait_with_output().unwrap();
    let r = Run {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        elapsed: t.elapsed(),
    };
    assert!(!r.stderr.contains("panicked"), "{}", r.stderr);
    for line in r.stderr.lines() {
        assert!(
            [
                "ERROR: ",
                "WARNING: ",
                "INFO: ",
                "DEBUG: ",
                "STATE: ",
                "ATTR: ",
                "ALERT: ",
            ]
            .iter()
            .any(|p| line.starts_with(p)),
            "line without CUPS prefix: {line:?}"
        );
    }
    r
}

fn wait_for(what: &str, limit: Duration, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < limit, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn has_line(stderr: &str, line: &str) -> bool {
    stderr.lines().any(|l| l == line)
}

#[test]
fn discovery_without_arguments() {
    let o = Command::new(BIN).output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "network tmc3500 \"Epson TM-C3500\" \"Epson TM-C3500 (tmc3500)\"\n"
    );
}

#[test]
fn wrong_argument_count_fails() {
    let o = Command::new(BIN).args(["1", "2"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).starts_with("ERROR: "));
}

#[test]
fn normal_job_from_file_is_delivered_byte_identical() {
    let job = big_job();
    let h = H::new(&job);
    let r = h.run_file();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(h.fake.accepted_jobs(), vec![job]);
    assert_eq!(h.fake.connection_count(), 1);
    assert!(!r.stderr.contains("ERROR:"), "{}", r.stderr);
    assert!(r.stdout.is_empty());
    assert!(
        r.stderr.contains("ATTR: marker-levels=80,80,80,80,80"),
        "{}",
        r.stderr
    );
    assert!(r.stderr.contains(
        "ATTR: marker-types=ink-cartridge,ink-cartridge,ink-cartridge,ink-cartridge,waste-ink"
    ));
    assert!(h.alerts().is_empty());
}

#[test]
fn normal_job_from_stdin_is_delivered_byte_identical() {
    let job = capture("000_default_t1.prn");
    let h = H::new(&job);
    let mut child = h
        .cmd(&h.uri())
        .args(["7", "bob", "t", "1", "opt=1"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&job).unwrap();
    let r = finish(child, Duration::from_secs(20));
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(h.fake.accepted_jobs(), vec![job]);
}

#[test]
fn paper_out_mid_job_waits_reports_and_completes() {
    let job = big_job();
    let h = H::new(&job);
    h.fake.set_paper_out_after(Some(100_000));
    let child = h.spawn_file(&h.uri());
    wait_for("stall", Duration::from_secs(10), || h.fake.is_stalled());
    std::thread::sleep(Duration::from_millis(500));
    assert!(h.fake.jobs().is_empty());
    h.fake.set_paper_out(false);
    let r = finish(child, Duration::from_secs(20));
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        has_line(&r.stderr, "STATE: +media-empty-error"),
        "{}",
        r.stderr
    );
    assert!(
        has_line(&r.stderr, "STATE: -media-empty-error"),
        "{}",
        r.stderr
    );
    assert_eq!(h.fake.accepted_jobs(), vec![job]);
    assert_eq!(h.fake.connection_count(), 1);
    assert_eq!(h.alert_names(), vec!["media-out"]);
    assert_eq!(h.alerts()[0]["job_id"], "42");
}

#[test]
fn error_before_start_waits_without_connecting() {
    let job = capture("000_default_t1.prn");
    let h = H::new(&job);
    h.fake.set_cover_open(true);
    h.fake.set_jam(true);
    let child = h.spawn_file(&h.uri());
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        h.fake.connection_count(),
        0,
        "connected while the printer was in error"
    );
    h.fake.set_cover_open(false);
    h.fake.set_jam(false);
    let r = finish(child, Duration::from_secs(20));
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        has_line(&r.stderr, "STATE: +cover-open,media-jam-error")
            || (has_line(&r.stderr, "STATE: +cover-open")
                && has_line(&r.stderr, "STATE: +media-jam-error")),
        "{}",
        r.stderr
    );
    assert!(r.stderr.contains("STATE: -cover-open"), "{}", r.stderr);
    assert_eq!(h.fake.accepted_jobs(), vec![job]);
    let names = h.alert_names();
    assert!(names.contains(&"cover-open".to_string()) && names.contains(&"media-jam".to_string()));
}

#[test]
fn connection_dropped_mid_job_holds_and_alerts_without_retry() {
    let h = H::new(&big_job());
    h.fake.set_drop_after(Some(50_000));
    let r = h.run_file();
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert!(r.stderr.contains("ERROR: "), "{}", r.stderr);
    std::thread::sleep(Duration::from_millis(300));
    let jobs = h.fake.jobs();
    assert_eq!(jobs.len(), 1, "{jobs:?}");
    assert_eq!(jobs[0].outcome, JobOutcome::Dropped);
    assert!(h.fake.accepted_jobs().is_empty());
    assert_eq!(h.fake.connection_count(), 1);
    let a = h.alerts();
    assert!(
        a.iter()
            .any(|e| e["event"] == "job-held" && e["job_id"] == "42"),
        "{a:?}"
    );
}

#[test]
fn unreachable_printer_retries_then_holds() {
    let h = H::new(&capture("000_default_t1.prn"));
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let uri = format!(
        "tmc3500://127.0.0.1:{port}?snmp-port={}",
        h.fake.snmp_port()
    );
    let r = finish(h.spawn_file(&uri), Duration::from_secs(20));
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert!(r.elapsed >= Duration::from_millis(600), "{:?}", r.elapsed);
    assert!(r.elapsed < Duration::from_secs(5), "{:?}", r.elapsed);
    let retries = r
        .stderr
        .lines()
        .filter(|l| l.contains("retrying in"))
        .count();
    assert!(retries >= 3, "{}", r.stderr);
    let names = h.alert_names();
    assert!(names.contains(&"unreachable".to_string()), "{names:?}");
    assert!(names.contains(&"job-held".to_string()), "{names:?}");
}

#[test]
fn unreachable_then_reachable_prints() {
    let job = capture("000_default_t1.prn");
    let mut h = H::new(&job);
    h.extra_backend = "unreachable_hold_after_ms = 10000".into();
    h.fake.set_refuse(true);
    std::thread::sleep(Duration::from_millis(50));
    let child = h.spawn_file(&h.uri());
    std::thread::sleep(Duration::from_millis(400));
    h.fake.set_refuse(false);
    let r = finish(child, Duration::from_secs(20));
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(h.fake.accepted_jobs(), vec![job]);
    assert!(r.stderr.contains("retrying in"), "{}", r.stderr);
}

#[test]
fn garbage_snmp_degrades_but_prints() {
    let job = capture("000_default_t1.prn");
    let h = H::new(&job);
    h.fake.set_snmp_mode(SnmpMode::Garbage);
    let r = h.run_file();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stderr.contains("WARNING: "), "{}", r.stderr);
    assert!(r.stderr.contains("SNMP"), "{}", r.stderr);
    assert_eq!(h.fake.accepted_jobs(), vec![job]);
}

#[test]
fn silent_snmp_degrades_but_prints() {
    let job = capture("000_default_t1.prn");
    let h = H::new(&job);
    h.fake.set_snmp_mode(SnmpMode::Silent);
    let r = h.run_file();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stderr.contains("WARNING: "), "{}", r.stderr);
    assert_eq!(h.fake.accepted_jobs(), vec![job]);
}

#[test]
fn stall_without_status_holds_instead_of_hanging() {
    let mut h = H::new(&big_job());
    h.extra_backend = "stall_timeout_ms = 800".into();
    h.fake.set_snmp_mode(SnmpMode::Silent);
    h.fake.set_paper_out_after(Some(100_000));
    let r = h.run_file();
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert!(h.fake.accepted_jobs().is_empty());
    assert!(h.alert_names().contains(&"job-held".to_string()));
}

#[test]
fn slow_printer_still_completes() {
    let job = big_job();
    let h = H::new(&job);
    h.fake
        .set_slow_read(Some((16 * 1024, Duration::from_millis(2))));
    let r = h.run_file();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(h.fake.accepted_jobs(), vec![job]);
}

#[test]
fn back_channel_reply_is_logged() {
    let h = H::new(&capture("000_default_t1.prn"));
    h.fake.set_reply(b"@BDC ST2\r\n".to_vec());
    let r = h.run_file();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stderr.contains("DEBUG: back-channel"), "{}", r.stderr);
}

fn kill_term(pid: u32) {
    let st = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .unwrap();
    assert!(st.success());
}

#[test]
fn sigterm_while_waiting_exits_cleanly() {
    let h = H::new(&capture("000_default_t1.prn"));
    h.fake.set_paper_out(true);
    let child = h.spawn_file(&h.uri());
    std::thread::sleep(Duration::from_millis(400));
    kill_term(child.id());
    let r = finish(child, Duration::from_secs(5));
    assert_eq!(r.code, 5, "{}", r.stderr);
    assert!(r.elapsed < Duration::from_secs(2));
    assert!(h.fake.jobs().is_empty());
}

#[test]
fn sigterm_mid_job_exits_cleanly() {
    let h = H::new(&big_job());
    h.fake.set_paper_out_after(Some(100_000));
    let child = h.spawn_file(&h.uri());
    wait_for("stall", Duration::from_secs(10), || h.fake.is_stalled());
    kill_term(child.id());
    let r = finish(child, Duration::from_secs(5));
    assert_eq!(r.code, 5, "{}", r.stderr);
    assert!(r.stderr.contains("cancel"), "{}", r.stderr);
}

#[test]
fn invalid_config_or_uri_stops_the_queue() {
    let h = H::new(&capture("000_default_t1.prn"));
    let bad = h.dir.path().join("bad.toml");
    std::fs::write(&bad, "[snmp]\nbogus = 1\n").unwrap();
    let r = finish(
        h.cmd(&h.uri())
            .env("TMC3500_CONFIG", &bad)
            .args(["1", "u", "t", "1", "", h.job_file.to_str().unwrap()])
            .spawn()
            .unwrap(),
        Duration::from_secs(5),
    );
    assert_eq!(r.code, 4, "{}", r.stderr);
    let r = finish(
        h.cmd("tmc3500://bad host")
            .args(["1", "u", "t", "1", "", h.job_file.to_str().unwrap()])
            .spawn()
            .unwrap(),
        Duration::from_secs(5),
    );
    assert_eq!(r.code, 4, "{}", r.stderr);
    assert!(h.fake.jobs().is_empty());
}

#[test]
fn missing_job_file_fails() {
    let h = H::new(b"");
    let r = finish(
        h.cmd(&h.uri())
            .args(["1", "u", "t", "1", "", "/nonexistent/job.prn"])
            .spawn()
            .unwrap(),
        Duration::from_secs(5),
    );
    assert_eq!(r.code, 1, "{}", r.stderr);
}

/// A TCP peer that accepts the job connection but never reads (exp1 of the review).
fn non_reading_listener() -> (u16, std::thread::JoinHandle<()>) {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let t = std::thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        std::thread::sleep(Duration::from_secs(8));
        drop(s);
    });
    (port, t)
}

#[test]
fn job_stuck_in_local_send_buffer_is_not_reported_printed() {
    // SNMP says idle and OK, but the peer never reads: data stays in our send queue.
    let h = H::new(&big_job());
    let (port, _t) = non_reading_listener();
    let uri = format!(
        "tmc3500://127.0.0.1:{port}?snmp-port={}",
        h.fake.snmp_port()
    );
    let r = finish(h.spawn_file(&uri), Duration::from_secs(20));
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert!(
        !r.stderr.contains("assuming the job was received"),
        "{}",
        r.stderr
    );
}

#[test]
fn snmp_reply_without_status_values_is_not_ok() {
    // The printer took the bytes into its buffer but never finished; SNMP answers but has no
    // status values: that must count as "status unknown", not as "printer OK".
    let h = H::new(&capture("000_default_t1.prn"));
    h.fake.set_snmp_mode(SnmpMode::NoStatus);
    h.fake.set_paused(true);
    let r = h.run_file();
    assert_eq!(r.code, 3, "{}", r.stderr);
}

#[test]
fn sigterm_after_partial_send_marks_the_job_and_a_restart_is_held() {
    // cupsd also sends SIGTERM on shutdown/restart and later restarts the job from the start
    let h = H::new(&big_job());
    h.fake.set_paper_out_after(Some(100_000));
    let child = h.spawn_file(&h.uri());
    wait_for("stall", Duration::from_secs(10), || h.fake.is_stalled());
    kill_term(child.id());
    let r = finish(child, Duration::from_secs(5));
    assert_eq!(r.code, 5, "{}", r.stderr);
    assert!(r.stderr.contains("ERROR: "), "{}", r.stderr);
    assert!(h.dir.path().join("state/partial-job-42").exists());
    assert!(
        h.alert_names().contains(&"job-failed".to_string()),
        "{:?}",
        h.alert_names()
    );
    h.fake.set_paper_out(false);
    wait_for("fake idle", Duration::from_secs(5), || {
        !h.fake.is_receiving()
    });
    let before = h.fake.connection_count();
    // the same job id comes back: held, nothing sent
    let r = h.run_file();
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert!(r.stderr.contains("partially sent"), "{}", r.stderr);
    assert_eq!(h.fake.connection_count(), before);
    assert!(h.alert_names().contains(&"job-held".to_string()));
}

#[test]
fn copies_are_sent_for_file_jobs_only() {
    let job = capture("000_default_t1.prn");
    let h = H::new(&job);
    let r = finish(
        h.cmd(&h.uri())
            .args([
                "42",
                "alice",
                "label",
                "3",
                "",
                h.job_file.to_str().unwrap(),
            ])
            .stdin(Stdio::null())
            .spawn()
            .unwrap(),
        Duration::from_secs(20),
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        h.fake.accepted_jobs(),
        vec![job.clone(), job.clone(), job.clone()]
    );
    // from stdin (filter output) copies are already in the job: send once
    let mut child = h
        .cmd(&h.uri())
        .args(["43", "alice", "label", "3", ""])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&job).unwrap();
    let r = finish(child, Duration::from_secs(20));
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(h.fake.accepted_jobs().len(), 4);
}

#[test]
fn copy_failure_holds_without_resending_completed_copies() {
    let job = big_job();
    let h = H::new(&job);
    // the second connection is dropped part-way
    let fake = &h.fake;
    fake.set_drop_nth(Some((2, 50_000)));
    let child = h
        .cmd(&h.uri())
        .args([
            "42",
            "alice",
            "label",
            "3",
            "",
            h.job_file.to_str().unwrap(),
        ])
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let r = finish(child, Duration::from_secs(20));
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert_eq!(fake.accepted_jobs().len(), 1);
    assert_eq!(fake.connection_count(), 2);
    assert!(r.stderr.contains("copy 2 of 3"), "{}", r.stderr);
}

#[test]
fn slow_alert_hooks_do_not_delay_the_job() {
    let job = capture("000_default_t1.prn");
    let h = H::new(&job);
    let slow = h.dir.path().join("alert.d/20-slow");
    std::fs::write(&slow, "#!/bin/sh\ncat >/dev/null\nsleep 1.5\n").unwrap();
    std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755)).unwrap();
    h.fake
        .set_ink(tmc3500_core::status::printer::InkColor::Cyan, 5);
    let t = Instant::now();
    let child = h.spawn_file(&h.uri());
    wait_for("job delivered", Duration::from_secs(10), || {
        h.fake.jobs().len() == 1
    });
    assert!(
        t.elapsed() < Duration::from_millis(1200),
        "job waited for the hook: {:?}",
        t.elapsed()
    );
    let r = finish(child, Duration::from_secs(20));
    assert_eq!(r.code, 0, "{}", r.stderr);
    // the alert was still delivered before exit
    assert_eq!(h.alert_names(), vec!["ink-low"]);
}

#[test]
fn busy_printer_reporting_printing_is_not_a_stall() {
    // larger than the socket buffers, so the backend's writes really block
    let mut job = Vec::new();
    for _ in 0..16 {
        job.extend_from_slice(&big_job());
    }
    let mut h = H::new(&job);
    h.extra_backend = "stall_timeout_ms = 800".into();
    h.fake.set_paused(true);
    let child = h.spawn_file(&h.uri());
    std::thread::sleep(Duration::from_millis(1500));
    h.fake.set_paused(false);
    let r = finish(child, Duration::from_secs(30));
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(h.fake.accepted_jobs().len(), 1);
}

#[test]
fn hold_after_everything_was_delivered_says_so() {
    // small job: fully acknowledged into the printer's buffer, then the printer goes silent
    let mut h = H::new(&capture("000_default_t1.prn"));
    h.extra_backend = "stall_timeout_ms = 800".into();
    h.fake.set_snmp_mode(SnmpMode::Silent);
    h.fake.set_paused(true);
    let r = h.run_file();
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert!(
        r.stderr.contains("bytes were delivered to the printer"),
        "{}",
        r.stderr
    );
    assert!(
        r.stderr
            .contains("do not release it unless output is missing"),
        "{}",
        r.stderr
    );
}

#[test]
fn unreachable_limit_counts_from_the_first_failed_connect() {
    // a long wait for the printer (cover open) must not use up the unreachable budget
    let h = H::new(&capture("000_default_t1.prn"));
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    h.fake.set_cover_open(true);
    let uri = format!(
        "tmc3500://127.0.0.1:{port}?snmp-port={}",
        h.fake.snmp_port()
    );
    let t = Instant::now();
    let child = h.spawn_file(&uri);
    std::thread::sleep(Duration::from_millis(900));
    h.fake.set_cover_open(false);
    let r = finish(child, Duration::from_secs(20));
    assert_eq!(r.code, 3, "{}", r.stderr);
    let retries = r
        .stderr
        .lines()
        .filter(|l| l.contains("retrying in"))
        .count();
    assert!(retries >= 3, "{}", r.stderr);
    assert!(
        t.elapsed() >= Duration::from_millis(1400),
        "{:?}",
        t.elapsed()
    );
}
