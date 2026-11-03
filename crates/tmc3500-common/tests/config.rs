use std::time::Duration;
use tmc3500_common::config::{Config, ConfigError, SnmpVersion};

#[test]
fn missing_file_gives_defaults() {
    let d = tempfile::tempdir().unwrap();
    let c = Config::load(&d.path().join("nope.toml")).unwrap();
    assert_eq!(c, Config::default());
    assert_eq!(c.printer.port, 9100);
    assert_eq!(c.snmp.port, 161);
    assert_eq!(c.snmp.community, "public");
    assert_eq!(c.snmp.version, SnmpVersion::V2c);
    assert_eq!(c.monitor.interval(), Duration::from_secs(30));
    assert_eq!(c.alerts.dir.to_str(), Some("/etc/tmc3500/alert.d"));
    assert_eq!(c.alerts.state_dir.to_str(), Some("/var/lib/tmc3500"));
    assert_eq!(c.printer.queue, "TM-C3500");
}

#[test]
fn full_file_parses() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("c.toml");
    std::fs::write(
        &p,
        r#"
[printer]
host = "192.0.2.10"
port = 9101
queue = "Labels"

[snmp]
enabled = true
port = 1161
community = "secret"
version = "1"
timeout_ms = 300
retries = 0

[backend]
connect_timeout_ms = 100
io_timeout_ms = 200
status_interval_ms = 50
retry_initial_ms = 10
retry_max_ms = 40
unreachable_hold_after_ms = 500
stall_timeout_ms = 1000
drain_timeout_ms = 700

[alerts]
dir = "/tmp/a"
hook_timeout_ms = 900
repeat_interval_s = 60
state_dir = "/tmp/s"

[monitor]
interval_ms = 1000
textfile = "/tmp/t.prom"

[thresholds]
ink_low_percent = 15
waste_near_full_percent = 85
"#,
    )
    .unwrap();
    let c = Config::load(&p).unwrap();
    assert_eq!(c.printer.host.as_deref(), Some("192.0.2.10"));
    assert_eq!(c.printer.queue, "Labels");
    assert_eq!(c.snmp.version, SnmpVersion::V1);
    assert_eq!(c.snmp.timeout(), Duration::from_millis(300));
    assert_eq!(c.backend.drain_timeout(), Duration::from_millis(700));
    assert_eq!(c.alerts.repeat_interval(), Duration::from_secs(60));
    assert_eq!(
        c.monitor.textfile.as_deref().and_then(|p| p.to_str()),
        Some("/tmp/t.prom")
    );
    assert_eq!(c.thresholds().ink_low_percent, 15);
}

#[test]
fn unknown_keys_bad_values_and_huge_files_rejected() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("c.toml");
    std::fs::write(&p, "[snmp]\ncomunity = \"x\"\n").unwrap();
    assert!(matches!(Config::load(&p), Err(ConfigError::Parse(_))));
    std::fs::write(&p, "[thresholds]\nink_low_percent = 101\n").unwrap();
    assert!(matches!(Config::load(&p), Err(ConfigError::Invalid(_))));
    std::fs::write(&p, "[backend]\nio_timeout_ms = 0\n").unwrap();
    assert!(matches!(Config::load(&p), Err(ConfigError::Invalid(_))));
    std::fs::write(&p, "[printer]\nport = 0\n").unwrap();
    assert!(matches!(Config::load(&p), Err(ConfigError::Invalid(_))));
    std::fs::write(&p, vec![b'#'; 2 * 1024 * 1024]).unwrap();
    assert!(matches!(Config::load(&p), Err(ConfigError::TooLarge)));
    std::fs::write(&p, [0xff, 0xfe, 0x00]).unwrap();
    assert!(Config::load(&p).is_err());
    // a directory is not a config file
    assert!(Config::load(d.path()).is_err());
}

#[test]
fn round_trips_through_toml() {
    let mut c = Config::default();
    c.printer.host = Some("printer.lan".into());
    let s = c.to_toml().unwrap();
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("c.toml");
    std::fs::write(&p, s).unwrap();
    assert_eq!(Config::load(&p).unwrap(), c);
}
