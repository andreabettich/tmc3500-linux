//! Shipped example config and alert hooks.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tmc3500_common::config::Config;

fn packaging() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging")
}

#[test]
fn example_config_matches_defaults() {
    let c = Config::load(&packaging().join("tmc3500.toml")).unwrap();
    assert_eq!(c, Config::default());
}

fn hooks() -> Vec<PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(packaging().join("alert.d"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    v.sort();
    v
}

#[test]
fn hooks_are_executable_valid_shell() {
    let h = hooks();
    assert_eq!(h.len(), 3);
    for p in h {
        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "{}", p.display());
        let ok = Command::new("sh").arg("-n").arg(&p).status().unwrap();
        assert!(ok.success(), "{}", p.display());
    }
}

/// Run a hook with fake tools; returns what the fake tool recorded.
fn run_hook(name: &str, env_file: &str, tool_var: &str) -> String {
    let d = tempfile::tempdir().unwrap();
    let rec = d.path().join("rec");
    let tool = d.path().join("tool");
    // Write the fake tool from a child process: if this (multi-threaded) test process held the
    // write fd, a concurrent fork could inherit it and exec would fail with ETXTBSY.
    let script = format!(
        "#!/bin/sh\necho \"ARGS: $*\" > {r}\ncat >> {r}\n",
        r = rec.display()
    );
    let ok = Command::new("sh")
        .arg("-c")
        .arg("printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"")
        .arg("sh")
        .arg(&script)
        .arg(&tool)
        .status()
        .unwrap();
    assert!(ok.success());
    let envf = d.path().join("alert.env");
    std::fs::write(&envf, env_file).unwrap();
    let mut c = Command::new(packaging().join("alert.d").join(name))
        .env("TMC3500_EVENT", "media-out")
        .env("TMC3500_SEVERITY", "error")
        .env("TMC3500_ALERT_ENV", &envf)
        .env(tool_var, &tool)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    // an unconfigured hook exits without reading stdin: EPIPE is fine
    let _ = c
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"event\":\"media-out\"}\n");
    assert!(c.wait().unwrap().success());
    std::fs::read_to_string(rec).unwrap_or_default()
}

#[test]
fn syslog_hook() {
    let r = run_hook("50-syslog", "", "LOGGER");
    assert!(
        r.contains("-t tmc3500 -p user.err -- media-out: {\"event\":\"media-out\"}"),
        "{r}"
    );
}

#[test]
fn mail_hook() {
    // msmtp: an SMTP client that needs no setuid/setgid (works under NoNewPrivileges=yes)
    assert_eq!(run_hook("60-mail", "", "MSMTP"), "");
    let r = run_hook(
        "60-mail",
        "MAIL_TO=ops@example.com\nMAIL_FROM=p@example.com\n",
        "MSMTP",
    );
    assert!(r.contains("ARGS: --read-envelope-from -t"), "{r}");
    assert!(r.contains("To: ops@example.com"), "{r}");
    assert!(r.contains("Subject: [tmc3500] error: media-out"), "{r}");
    assert!(r.contains("{\"event\":\"media-out\"}"), "{r}");
}

#[test]
fn mail_hook_does_not_rely_on_setuid_sendmail() {
    let hook = std::fs::read_to_string(packaging().join("alert.d/60-mail")).unwrap();
    assert!(!hook.contains("/usr/sbin/sendmail"));
    let unit =
        std::fs::read_to_string(packaging().join("systemd/tmc3500-monitor.service")).unwrap();
    assert!(!unit.contains("postfix"));
}

#[test]
fn webhook_hook() {
    assert_eq!(run_hook("70-webhook", "", "CURL"), "");
    let r = run_hook(
        "70-webhook",
        "WEBHOOK_URL=https://hooks.example.com/x\n",
        "CURL",
    );
    assert!(r.contains("https://hooks.example.com/x"), "{r}");
    assert!(r.contains("--data-binary @-"), "{r}");
    assert!(r.contains("{\"event\":\"media-out\"}"), "{r}");
}
