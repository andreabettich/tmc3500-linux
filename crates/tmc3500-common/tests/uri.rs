use tmc3500_common::uri::DeviceUri;

#[test]
fn parses_host_and_port() {
    let u = DeviceUri::parse("tmc3500://192.0.2.5").unwrap();
    assert_eq!((u.host.as_str(), u.port), ("192.0.2.5", 9100));
    assert_eq!(u.authority(), "192.0.2.5:9100");
    let u = DeviceUri::parse("tmc3500://printer.lan:9101/").unwrap();
    assert_eq!((u.host.as_str(), u.port), ("printer.lan", 9101));
    let u = DeviceUri::parse("tmc3500://[fe80::1]:9100").unwrap();
    assert_eq!(u.host, "fe80::1");
    assert_eq!(u.authority(), "[fe80::1]:9100");
    assert_eq!(u.snmp_authority(161), "[fe80::1]:161");
}

#[test]
fn parses_query() {
    let u = DeviceUri::parse("tmc3500://h:1234?snmp-port=1161&community=pub%20lic").unwrap();
    assert_eq!(u.port, 1234);
    assert_eq!(u.snmp_port, Some(1161));
    assert_eq!(u.community.as_deref(), Some("pub lic"));
}

#[test]
fn rejects_bad_uris() {
    for bad in [
        "",
        "socket://h",
        "tmc3500://",
        "tmc3500://h:0",
        "tmc3500://h:65536",
        "tmc3500://h:x",
        "tmc3500://h h",
        "tmc3500://user@h",
        "tmc3500://h?snmp-port=0",
        "tmc3500://h?bogus=1",
        "tmc3500://h?community=%zz",
        "tmc3500://[::1",
        "tmc3500://h/path",
        "tmc3500://-h",
    ] {
        assert!(DeviceUri::parse(bad).is_err(), "{bad:?} accepted");
    }
    let long = format!("tmc3500://{}", "a".repeat(300));
    assert!(DeviceUri::parse(&long).is_err());
}
