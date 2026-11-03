use std::net::UdpSocket;
use std::thread;
use std::time::{Duration, Instant};
use tmc3500_common::snmp::{ClientError, SnmpClient};
use tmc3500_core::status::snmp::{self, Message, Oid, PduType, Value, VarBind, Version};

enum Mode {
    Echo,
    Garbage,
    WrongIdThenRight,
    Silent,
    V1NoSuchName,
}

fn responder(mode: Mode) -> (u16, thread::JoinHandle<()>) {
    let s = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = s.local_addr().unwrap().port();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let h = thread::spawn(move || {
        let mut buf = [0u8; 65536];
        for _ in 0..4 {
            let Ok((n, from)) = s.recv_from(&mut buf) else {
                return;
            };
            let req = snmp::decode(&buf[..n]).unwrap();
            let mut resp = Message {
                pdu_type: PduType::GetResponse,
                varbinds: req
                    .varbinds
                    .iter()
                    .map(|v| VarBind::new(v.oid.clone(), Value::Integer(42)))
                    .collect(),
                ..req.clone()
            };
            match mode {
                Mode::Echo => {}
                Mode::Garbage => {
                    s.send_to(b"\x30\x03garbage", from).unwrap();
                    continue;
                }
                Mode::Silent => continue,
                Mode::WrongIdThenRight => {
                    let mut wrong = resp.clone();
                    wrong.request_id = wrong.request_id.wrapping_add(1);
                    s.send_to(&snmp::encode(&wrong).unwrap(), from).unwrap();
                }
                Mode::V1NoSuchName => {
                    if req.varbinds.len() == 2 {
                        resp.error_status = 2;
                        resp.error_index = 2;
                    }
                }
            }
            s.send_to(&snmp::encode(&resp).unwrap(), from).unwrap();
        }
    });
    (port, h)
}

fn client(port: u16) -> SnmpClient {
    SnmpClient::new(
        format!("127.0.0.1:{port}").parse().unwrap(),
        "public",
        Version::V2c,
        Duration::from_millis(200),
        1,
    )
}

fn oids() -> Vec<Oid> {
    vec![
        Oid::parse("1.3.6.1.2.1.1.1.0").unwrap(),
        Oid::parse("1.3.6.1.2.1.1.3.0").unwrap(),
    ]
}

#[test]
fn get_returns_values() {
    let (port, _h) = responder(Mode::Echo);
    let v = client(port).get(&oids()).unwrap();
    assert_eq!(v.len(), 2);
    assert_eq!(v[1].value, Value::Integer(42));
}

#[test]
fn stale_reply_with_other_request_id_is_ignored() {
    let (port, _h) = responder(Mode::WrongIdThenRight);
    assert_eq!(client(port).get(&oids()).unwrap().len(), 2);
}

#[test]
fn garbage_reply_is_an_error_not_a_panic() {
    let (port, _h) = responder(Mode::Garbage);
    let t = Instant::now();
    let e = client(port).get(&oids()).unwrap_err();
    assert!(matches!(e, ClientError::InvalidReply(_)), "{e:?}");
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[test]
fn silence_times_out_after_retries() {
    let (port, _h) = responder(Mode::Silent);
    let t = Instant::now();
    let e = client(port).get(&oids()).unwrap_err();
    assert!(matches!(e, ClientError::Timeout), "{e:?}");
    let el = t.elapsed();
    assert!(
        el >= Duration::from_millis(400) && el < Duration::from_secs(2),
        "{el:?}"
    );
}

#[test]
fn v1_no_such_name_drops_the_offending_oid() {
    let (port, _h) = responder(Mode::V1NoSuchName);
    let mut c = client(port);
    c.version = Version::V1;
    let v = c.get(&oids()).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].oid, oids()[0]);
}

#[test]
fn query_status_parses_printer_status() {
    let (port, _h) = responder(Mode::Echo);
    let st = client(port).query_status().unwrap();
    // hrDeviceStatus 42 is not a valid value but must not fail the query
    assert!(st.device.is_some());
}
