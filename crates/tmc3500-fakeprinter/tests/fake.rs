use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tmc3500_common::snmp::{ClientError, SnmpClient};
use tmc3500_core::status::printer::{oids, EventKind, InkColor, SupplyKind, Thresholds};
use tmc3500_core::status::snmp::{Oid, Version};
use tmc3500_fakeprinter::{FakePrinter, JobOutcome, SnmpMode};

fn capture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/captures")
        .join(name);
    std::fs::read(p).unwrap()
}

fn job() -> Vec<u8> {
    capture("000_default_t1.prn")
}

fn send(fake: &FakePrinter, data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut s = TcpStream::connect(("127.0.0.1", fake.job_port()))?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    s.write_all(data)?;
    s.shutdown(std::net::Shutdown::Write)?;
    let mut back = Vec::new();
    s.read_to_end(&mut back)?;
    Ok(back)
}

fn client(fake: &FakePrinter) -> SnmpClient {
    SnmpClient::new(
        SocketAddr::from(([127, 0, 0, 1], fake.snmp_port())),
        "public",
        Version::V2c,
        Duration::from_millis(300),
        0,
    )
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn accepts_a_valid_job_byte_identical() {
    let fake = FakePrinter::start_local().unwrap();
    let j = job();
    send(&fake, &j).unwrap();
    wait_for("job", || fake.jobs().len() == 1);
    let rec = &fake.jobs()[0];
    assert_eq!(rec.outcome, JobOutcome::Accepted);
    assert_eq!(rec.data, j);
    assert_eq!(fake.accepted_jobs(), vec![j]);
    assert_eq!(fake.connection_count(), 1);
}

#[test]
fn rejects_garbage_job() {
    let fake = FakePrinter::start_local().unwrap();
    send(&fake, b"\x1b(G\x05\x00\x01").unwrap(); // truncated command
    wait_for("job", || fake.jobs().len() == 1);
    assert!(matches!(fake.jobs()[0].outcome, JobOutcome::Rejected(_)));
    assert!(fake.accepted_jobs().is_empty());
}

#[test]
fn snmp_reports_scripted_state() {
    let fake = FakePrinter::start_local().unwrap();
    let c = client(&fake);
    let st = c.query_full().unwrap();
    let thr = Thresholds::default();
    assert!(st.reasons(&thr).is_empty(), "{:?}", st.reasons(&thr));
    assert_eq!(st.supplies.len(), 5);
    assert_eq!(st.supplies[0].kind, SupplyKind::Ink(InkColor::Black));
    assert_eq!(st.supplies[4].kind, SupplyKind::MaintenanceBox);
    assert!(st.sys_descr.as_deref().unwrap().contains("TM-C3500"));
    assert_eq!(st.life_count, Some(0));

    fake.set_paper_out(true);
    fake.set_ink(InkColor::Cyan, 5);
    fake.set_ink(InkColor::Yellow, 0);
    fake.set_waste_remaining(8);
    fake.set_cover_open(true);
    fake.set_jam(true);
    let st = c.query_status().unwrap();
    let r: Vec<_> = st.reasons(&thr).into_iter().collect();
    for want in [
        "media-empty-error",
        "cover-open",
        "media-jam-error",
        "marker-supply-low-warning",
        "marker-supply-empty-error",
        "marker-waste-almost-full-warning",
    ] {
        assert!(r.contains(&want), "{want} missing in {r:?}");
    }
    let events: Vec<_> = st.conditions(&thr).into_iter().map(|c| c.event).collect();
    assert!(events.contains(&EventKind::MediaOut));
    assert!(events.contains(&EventKind::InkEmpty));

    fake.set_paper_out(false);
    fake.set_cover_open(false);
    fake.set_jam(false);
    fake.set_offline(true);
    let r = c.query_status().unwrap().reasons(&thr);
    assert!(r.contains("offline-report"));
    assert!(!r.contains("media-empty-error"));
}

#[test]
fn snmp_get_next_walks_the_table() {
    let fake = FakePrinter::start_local().unwrap();
    let c = client(&fake);
    let first = c
        .get_next(&Oid::parse("1.3.6.1.2.1.25.3.2.1.5").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(first.oid, Oid::parse(oids::HR_DEVICE_STATUS).unwrap());
    let end = c
        .get_next(&Oid::parse("1.3.6.1.9").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(end.value, tmc3500_core::status::snmp::Value::EndOfMibView);
}

#[test]
fn wrong_community_is_ignored() {
    let fake = FakePrinter::start_local().unwrap();
    let mut c = client(&fake);
    c.community = "wrong".into();
    assert!(matches!(c.query_status(), Err(ClientError::Timeout)));
}

#[test]
fn paper_out_stops_reading_until_cleared() {
    let fake = FakePrinter::start_local().unwrap();
    // make the job larger than any socket buffer so the writer really blocks
    let mut big = Vec::new();
    for _ in 0..40 {
        big.extend_from_slice(&capture("320_quality_glossy_mode2.prn"));
    }
    fake.set_paper_out_after(Some(100_000));
    let data = big.clone();
    let port = fake.job_port();
    let writer = std::thread::spawn(move || {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(&data).unwrap();
        s.shutdown(std::net::Shutdown::Write).unwrap();
        let mut b = Vec::new();
        let _ = s.read_to_end(&mut b);
    });
    wait_for("stall", || fake.is_stalled());
    let st = client(&fake).query_status().unwrap();
    assert!(st
        .reasons(&Thresholds::default())
        .contains("media-empty-error"));
    let got = fake.bytes_received_current();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        fake.bytes_received_current(),
        got,
        "kept reading while out of paper"
    );
    assert!(got >= 100_000 && got < big.len() as u64);
    fake.set_paper_out(false);
    writer.join().unwrap();
    wait_for("job", || fake.jobs().len() == 1);
    assert_eq!(fake.accepted_jobs(), vec![big]);
}

#[test]
fn drop_after_n_bytes() {
    let fake = FakePrinter::start_local().unwrap();
    fake.set_drop_after(Some(1000));
    let j = capture("320_quality_glossy_mode2.prn");
    let r = send(&fake, &j);
    assert!(r.is_err() || r.unwrap().is_empty());
    wait_for("record", || fake.jobs().len() == 1);
    let rec = &fake.jobs()[0];
    assert_eq!(rec.outcome, JobOutcome::Dropped);
    assert_eq!(rec.data.len(), 1000);
}

#[test]
fn refuse_connections_then_accept_again() {
    let fake = FakePrinter::start_local().unwrap();
    fake.set_refuse(true);
    wait_for("refusal", || {
        TcpStream::connect(("127.0.0.1", fake.job_port())).is_err()
    });
    fake.set_refuse(false);
    wait_for("listening", || {
        TcpStream::connect(("127.0.0.1", fake.job_port())).is_ok()
    });
    send(&fake, &job()).unwrap();
    wait_for("job", || !fake.accepted_jobs().is_empty());
}

#[test]
fn slow_reads_still_deliver() {
    let fake = FakePrinter::start_local().unwrap();
    fake.set_slow_read(Some((16 * 1024, Duration::from_millis(5))));
    let j = capture("320_quality_glossy_mode2.prn");
    let t = Instant::now();
    send(&fake, &j).unwrap();
    wait_for("job", || fake.jobs().len() == 1);
    assert!(t.elapsed() >= Duration::from_millis(100));
    assert_eq!(fake.accepted_jobs(), vec![j]);
}

#[test]
fn garbage_and_silent_snmp() {
    let fake = FakePrinter::start_local().unwrap();
    let c = client(&fake);
    fake.set_snmp_mode(SnmpMode::Garbage);
    for _ in 0..4 {
        assert!(c.query_status().is_err());
    }
    fake.set_snmp_mode(SnmpMode::Silent);
    assert!(matches!(c.query_status(), Err(ClientError::Timeout)));
    fake.set_snmp_mode(SnmpMode::Normal);
    assert!(c.query_status().is_ok());
    assert!(fake.snmp_request_count() >= 6);
}

#[test]
fn back_channel_reply_is_sent_after_the_job() {
    let fake = FakePrinter::start_local().unwrap();
    fake.set_reply(b"@BDC ST2\r\n".to_vec());
    let back = send(&fake, &job()).unwrap();
    assert_eq!(back, b"@BDC ST2\r\n");
}

#[test]
fn line_commands() {
    let fake = FakePrinter::start_local().unwrap();
    for cmd in [
        "paper-out on",
        "jam on",
        "cover on",
        "offline on",
        "ink cyan 3",
        "waste 2",
        "drop-after 10",
        "drop-after off",
        "paper-out-after 100",
        "slow-read 1024 5",
        "slow-read off",
        "refuse off",
        "snmp garbage",
        "snmp normal",
        "jobs",
        "status",
    ] {
        assert!(fake.apply_command(cmd).is_ok(), "{cmd}");
    }
    for bad in ["", "ink pink 3", "ink cyan 300", "paper-out maybe", "fly"] {
        assert!(fake.apply_command(bad).is_err(), "{bad}");
    }
    let r = client(&fake)
        .query_status()
        .unwrap()
        .reasons(&Thresholds::default());
    assert!(r.contains("media-empty-error") && r.contains("media-jam-error"));
}
