use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use tmc3500_core::status::printer::InkColor;
use tmc3500_fakeprinter::{FakePrinter, SnmpMode};

const BIN: &str = env!("CARGO_BIN_EXE_tmc3500ctl");

struct H {
    dir: tempfile::TempDir,
}

impl H {
    fn new() -> H {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("bin")).unwrap();
        H { dir }
    }
    fn cfg(&self) -> PathBuf {
        self.dir.path().join("tmc3500.toml")
    }
    fn write_cfg(&self, fake: &FakePrinter) {
        std::fs::write(
            self.cfg(),
            format!(
                "[printer]\nhost = \"127.0.0.1\"\n[snmp]\nport = {}\ntimeout_ms = 200\nretries = 0\n",
                fake.snmp_port()
            ),
        )
        .unwrap();
    }
    fn tool(&self, name: &str, body: &str) {
        let p = self.dir.path().join("bin").join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.dir.path().join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(BIN)
            .args(args)
            .env("TMC3500_CONFIG", self.cfg())
            .env("PATH", path)
            .output()
            .unwrap()
    }
    fn file(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.path().join(name)).ok()
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn json(o: &Output) -> serde_json::Value {
    serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}\n{}", out(o), String::from_utf8_lossy(&o.stderr)))
}

#[test]
fn status_human_and_json() {
    let h = H::new();
    let fake = FakePrinter::start_local().unwrap();
    h.write_cfg(&fake);
    fake.set_paper_out(true);
    fake.set_ink(InkColor::Cyan, 4);
    let o = h.run(&["status"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let s = out(&o);
    assert!(s.contains("127.0.0.1"), "{s}");
    assert!(s.contains("media-empty-error"), "{s}");
    assert!(s.contains("out of media"), "{s}");
    assert!(s.contains("cyan"), "{s}");
    let j = json(&h.run(&["status", "--json"]));
    assert_eq!(j["reachable"], true);
    assert_eq!(j["blocking"], true);
    assert!(j["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r == "media-empty-error"));
    assert_eq!(j["device_status"], "down");
    assert_eq!(j["supplies"][1]["color"], "cyan");
    assert_eq!(j["supplies"][1]["percent"], 4);
    assert_eq!(j["supplies"][1]["state"], "low");
}

#[test]
fn printer_flag_overrides_config_and_unreachable_exits_2() {
    let h = H::new();
    let fake = FakePrinter::start_local().unwrap();
    h.write_cfg(&fake);
    fake.set_snmp_mode(SnmpMode::Silent);
    let o = h.run(&["status", "--json", "--printer", "127.0.0.1"]);
    assert_eq!(o.status.code(), Some(2));
    let j = json(&o);
    assert_eq!(j["reachable"], false);
    assert!(j["error"].as_str().unwrap().contains("timeout"));
    let o = h.run(&["status"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("unreachable"));
}

#[test]
fn no_printer_configured() {
    let h = H::new();
    let o = h.run(&["status"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("setup"));
}

#[test]
fn ink_and_counters() {
    let h = H::new();
    let fake = FakePrinter::start_local().unwrap();
    h.write_cfg(&fake);
    fake.set_waste_remaining(7);
    let j = json(&h.run(&["ink", "--json"]));
    let s = j["supplies"].as_array().unwrap();
    assert_eq!(s.len(), 5);
    assert_eq!(s[4]["kind"], "maintenance-box");
    assert_eq!(s[4]["state"], "low");
    let o = h.run(&["ink"]);
    assert!(out(&o).contains("Maintenance Box SJMB3500"), "{}", out(&o));
    let j = json(&h.run(&["counters", "--json"]));
    assert_eq!(j["life_count"], 0);
    assert!(j["sys_descr"].as_str().unwrap().contains("TM-C3500"));
    let o = h.run(&["counters"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(out(&o).contains("life count"), "{}", out(&o));
}

#[test]
fn maintenance_stays_disabled() {
    let h = H::new();
    for cmd in ["nozzle-check", "clean"] {
        let o = h.run(&[cmd]);
        assert_eq!(o.status.code(), Some(3));
        assert!(String::from_utf8_lossy(&o.stderr).contains("not yet verified on the real printer"));
        let o = h.run(&[cmd, "--i-know-this-is-unverified"]);
        assert_eq!(o.status.code(), Some(3));
        assert!(String::from_utf8_lossy(&o.stderr)
            .contains("not implemented: command bytes not captured yet"));
        let j = json(&h.run(&[cmd, "--json"]));
        assert_eq!(j["ok"], false);
        assert_eq!(j["command"], cmd);
    }
    // the flag is hidden from help
    let o = h.run(&["nozzle-check", "--help"]);
    assert!(!out(&o).contains("i-know"));
}

#[test]
fn setup_writes_config_and_prints_lpadmin() {
    let h = H::new();
    h.tool(
        "lpadmin",
        &format!("echo \"$@\" > {}/lpadmin.args", h.dir.path().display()),
    );
    let o = h.run(&["setup", "--printer", "192.0.2.50"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(out(&o).contains(
        "lpadmin -p TM-C3500 -E -v tmc3500://192.0.2.50 -P /usr/share/ppd/tmc3500/tmc3500.ppd -o printer-error-policy=stop-printer"
    ), "{}", out(&o));
    assert!(
        h.file("lpadmin.args").is_none(),
        "lpadmin ran without --apply"
    );
    let c = tmc3500_common::config::Config::load(&h.cfg()).unwrap();
    assert_eq!(c.printer.host.as_deref(), Some("192.0.2.50"));

    let o = h.run(&["setup", "--printer", "192.0.2.51", "--apply", "--json"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let j = json(&o);
    assert_eq!(j["applied"], true);
    assert_eq!(
        h.file("lpadmin.args").unwrap().trim(),
        "-p TM-C3500 -E -v tmc3500://192.0.2.51 -P /usr/share/ppd/tmc3500/tmc3500.ppd -o printer-error-policy=stop-printer"
    );
    // existing settings are kept
    std::fs::write(h.cfg(), "[snmp]\ncommunity = \"s3cret\"\n").unwrap();
    assert_eq!(
        h.run(&["setup", "--printer", "fe80::1"]).status.code(),
        Some(0)
    );
    let c = tmc3500_common::config::Config::load(&h.cfg()).unwrap();
    assert_eq!(c.snmp.community, "s3cret");
    assert_eq!(c.printer.host.as_deref(), Some("fe80::1"));
}

#[test]
fn setup_rejects_bad_input() {
    let h = H::new();
    assert_eq!(
        h.run(&["setup", "--printer", "bad host"]).status.code(),
        Some(1)
    );
    assert_eq!(h.run(&["setup", "--printer=-o x"]).status.code(), Some(1));
    std::fs::write(h.cfg(), "not toml [").unwrap();
    assert_eq!(
        h.run(&["setup", "--printer", "192.0.2.1"]).status.code(),
        Some(1)
    );
    // the broken file was not overwritten
    assert_eq!(h.file("tmc3500.toml").unwrap(), "not toml [");
}

const LPSTAT: &str = "\
TM-C3500-12             alice             1024   Tue 03 Nov 2026 10:00:00 AM UTC
\tStatus: Connection to the printer lost
\tAlerts: job-hold-until-specified
\tqueued for TM-C3500
TM-C3500-13             bob               2048   Tue 03 Nov 2026 10:01:00 AM UTC
\tAlerts: job-incoming
\tqueued for TM-C3500
TM-C3500-2-14           carol             2048   Tue 03 Nov 2026 10:01:00 AM UTC
\tAlerts: job-hold-until-specified
TM-C3500-15             dave              4096   Tue 03 Nov 2026 10:02:00 AM UTC
\tAlerts: job-hold-until-specified,job-printing
";

fn lp_tools(h: &H) {
    let d = h.dir.path().display();
    std::fs::write(h.dir.path().join("lpstat.out"), LPSTAT).unwrap();
    h.tool(
        "lpstat",
        &format!("echo \"$@\" > {d}/lpstat.args; cat {d}/lpstat.out"),
    );
    h.tool("lp", &format!("echo \"$@\" >> {d}/lp.args"));
}

#[test]
fn release_held_jobs_dry_run_and_real() {
    let h = H::new();
    lp_tools(&h);
    let o = h.run(&["release-held-jobs", "--dry-run"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        out(&o).contains("TM-C3500-12") && out(&o).contains("TM-C3500-15"),
        "{}",
        out(&o)
    );
    assert!(!out(&o).contains("TM-C3500-13"));
    assert!(!out(&o).contains("TM-C3500-2-14"));
    assert!(h.file("lp.args").is_none());
    assert_eq!(h.file("lpstat.args").unwrap().trim(), "-l -o TM-C3500");

    let j = json(&h.run(&["release-held-jobs", "--json", "--yes"]));
    assert_eq!(j["held"], serde_json::json!(["TM-C3500-12", "TM-C3500-15"]));
    assert_eq!(
        j["released"],
        serde_json::json!(["TM-C3500-12", "TM-C3500-15"])
    );
    assert_eq!(
        h.file("lp.args").unwrap(),
        "-i TM-C3500-12 -H resume\n-i TM-C3500-15 -H resume\n"
    );
}

#[test]
fn releasing_everything_needs_yes_and_warns_first() {
    let h = H::new();
    lp_tools(&h);
    let o = h.run(&["release-held-jobs"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(h.file("lp.args").is_none(), "released without --yes");
    let all = format!("{}{}", out(&o), String::from_utf8_lossy(&o.stderr));
    assert!(all.contains("--yes") && all.contains("--job"), "{all}");
    assert!(all.contains("may reprint"), "{all}");
    let o = h.run(&["release-held-jobs", "--yes"]);
    assert_eq!(o.status.code(), Some(0));
    let s = out(&o);
    let warn = s.find("may reprint").expect("warning");
    let first = s.find("Released").expect("released");
    assert!(warn < first, "{s}");
}

#[test]
fn release_a_single_job_and_clear_its_marker() {
    let h = H::new();
    lp_tools(&h);
    let state = h.dir.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::write(
        h.cfg(),
        format!("[alerts]\nstate_dir = \"{}\"\n", state.display()),
    )
    .unwrap();
    std::fs::write(state.join("partial-job-15"), "job 15 partially sent").unwrap();
    std::fs::write(state.join("partial-job-12"), "job 12 partially sent").unwrap();
    let o = h.run(&["release-held-jobs", "--job", "15"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(h.file("lp.args").unwrap(), "-i TM-C3500-15 -H resume\n");
    assert!(!state.join("partial-job-15").exists());
    assert!(state.join("partial-job-12").exists());
    // a job that is not held is refused
    let o = h.run(&["release-held-jobs", "--job", "TM-C3500-13"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(h.file("lp.args").unwrap(), "-i TM-C3500-15 -H resume\n");
}

#[test]
fn release_reports_lp_failures() {
    let h = H::new();
    lp_tools(&h);
    h.tool("lp", "echo 'lp: Forbidden' >&2; exit 1");
    let o = h.run(&["release-held-jobs", "--json", "--yes"]);
    assert_eq!(o.status.code(), Some(1));
    let j = json(&o);
    assert_eq!(j["failed"][0]["id"], "TM-C3500-12");
    assert!(j["failed"][0]["error"]
        .as_str()
        .unwrap()
        .contains("Forbidden"));
}

#[test]
fn setup_keeps_the_config_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let h = H::new();
    std::fs::write(h.cfg(), "[snmp]\ncommunity = \"s3cret\"\n").unwrap();
    std::fs::set_permissions(h.cfg(), std::fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(
        h.run(&["setup", "--printer", "192.0.2.7"]).status.code(),
        Some(0)
    );
    let m = std::fs::metadata(h.cfg()).unwrap().permissions().mode();
    assert_eq!(m & 0o777, 0o640);
}

#[test]
fn setup_apply_enables_the_monitor() {
    let h = H::new();
    h.tool("lpadmin", "true");
    h.tool(
        "systemctl",
        &format!("echo \"$@\" >> {}/systemctl.args", h.dir.path().display()),
    );
    assert!(h.file("systemctl.args").is_none());
    let o = h.run(&["setup", "--printer", "192.0.2.52"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(
        h.file("systemctl.args").is_none(),
        "systemctl ran without --apply"
    );
    let o = h.run(&["setup", "--printer", "192.0.2.52", "--apply"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    // enable for boot, and restart so a running monitor picks up a changed printer address
    assert_eq!(
        h.file("systemctl.args").unwrap(),
        "enable tmc3500-monitor.service\nrestart tmc3500-monitor.service\n"
    );
}

#[test]
fn marker_is_removed_before_the_job_is_resumed() {
    // CUPS restarts a resumed job immediately; if the marker still existed, the backend would hold it
    // again (seen with a real cupsd).
    let h = H::new();
    lp_tools(&h);
    let state = h.dir.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::write(
        h.cfg(),
        format!("[alerts]\nstate_dir = \"{}\"\n", state.display()),
    )
    .unwrap();
    std::fs::write(state.join("partial-job-15"), "job 15 partially sent").unwrap();
    h.tool(
        "lp",
        &format!(
            "test -e {s}/partial-job-15 && echo present > {d}/marker.at.lp; echo \"$@\" > {d}/lp.args",
            s = state.display(),
            d = h.dir.path().display()
        ),
    );
    let o = h.run(&["release-held-jobs", "--job", "15"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        h.file("marker.at.lp").is_none(),
        "marker still present when lp ran"
    );
    // if resuming fails, the marker is put back so the job stays protected
    std::fs::write(state.join("partial-job-15"), "job 15 partially sent").unwrap();
    h.tool("lp", "exit 1");
    let o = h.run(&["release-held-jobs", "--job", "15"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        std::fs::read_to_string(state.join("partial-job-15")).unwrap(),
        "job 15 partially sent"
    );
}
