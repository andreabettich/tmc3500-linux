//! The shipped systemd unit keeps the hardening required by spec section 6.
#[test]
fn unit_has_required_hardening() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packaging/systemd/tmc3500-monitor.service");
    let u = std::fs::read_to_string(p).unwrap();
    let lines: Vec<&str> = u.lines().map(str::trim).collect();
    for want in [
        "ExecStart=/usr/bin/tmc3500-monitor",
        "NoNewPrivileges=yes",
        "ProtectSystem=strict",
        "PrivateTmp=yes",
        "User=tmc3500",
        "StateDirectory=tmc3500",
        "ReadWritePaths=-/var/lib/prometheus/node-exporter",
        "CapabilityBoundingSet=",
    ] {
        assert!(lines.contains(&want), "missing {want}");
    }
    // IPAddressAllow is site specific: only as a commented example
    assert!(lines
        .iter()
        .any(|l| l.starts_with('#') && l.contains("IPAddressAllow=")));
    assert!(!lines.iter().any(|l| l.starts_with("IPAddressAllow=")));
}
