use super::*;
use proptest::prelude::*;

fn oid(s: &str) -> Oid {
    Oid::parse(s).unwrap()
}

/// `snmpget -v2c -c public host 1.3.6.1.2.1.1.1.0` with request-id 0x1234, assembled by hand.
const SYSDESCR_GET: &[u8] = &[
    0x30, 0x27, 0x02, 0x01, 0x01, 0x04, 0x06, b'p', b'u', b'b', b'l', b'i', b'c', 0xa0, 0x1a, 0x02,
    0x02, 0x12, 0x34, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x0e, 0x30, 0x0c, 0x06, 0x08, 0x2b,
    0x06, 0x01, 0x02, 0x01, 0x01, 0x01, 0x00, 0x05, 0x00,
];

#[test]
fn encodes_get_request_like_net_snmp() {
    let b = encode_request(
        Version::V2c,
        b"public",
        PduType::GetRequest,
        0x1234,
        &[oid("1.3.6.1.2.1.1.1.0")],
    )
    .unwrap();
    assert_eq!(b, SYSDESCR_GET);
}

#[test]
fn decodes_request() {
    let m = decode(SYSDESCR_GET).unwrap();
    assert_eq!(m.version, Version::V2c);
    assert_eq!(m.community, b"public");
    assert_eq!(m.pdu_type, PduType::GetRequest);
    assert_eq!(m.request_id, 0x1234);
    assert_eq!(m.varbinds.len(), 1);
    assert_eq!(m.varbinds[0].oid, oid("1.3.6.1.2.1.1.1.0"));
    assert_eq!(m.varbinds[0].value, Value::Null);
}

fn response(id: i32, varbinds: Vec<VarBind>) -> Message {
    Message {
        version: Version::V1,
        community: b"public".to_vec(),
        pdu_type: PduType::GetResponse,
        request_id: id,
        error_status: 0,
        error_index: 0,
        varbinds,
    }
}

#[test]
fn response_round_trip_all_value_types() {
    let vbs = vec![
        VarBind::new(oid("1.3.6.1.2.1.25.3.2.1.5.1"), Value::Integer(5)),
        VarBind::new(oid("1.3.6.1.2.1.43.11.1.1.9.1.1"), Value::Integer(-3)),
        VarBind::new(
            oid("1.3.6.1.2.1.43.11.1.1.6.1.1"),
            Value::OctetString(b"Black Ink SJIC22P(K)".to_vec()),
        ),
        VarBind::new(
            oid("1.3.6.1.2.1.1.2.0"),
            Value::ObjectId(oid("1.3.6.1.4.1.1248")),
        ),
        VarBind::new(oid("1.3.6.1.2.1.1.3.0"), Value::TimeTicks(u32::MAX)),
        VarBind::new(oid("1.3.6.1.2.1.2.2.1.10.1"), Value::Counter32(0x8000_0000)),
        VarBind::new(oid("1.3.6.1.2.1.2.2.1.5.1"), Value::Gauge32(100_000_000)),
        VarBind::new(oid("1.3.6.1.2.1.31.1.1.1.6.1"), Value::Counter64(u64::MAX)),
        VarBind::new(
            oid("1.3.6.1.2.1.4.20.1.1.1"),
            Value::IpAddress([10, 0, 0, 7]),
        ),
        VarBind::new(oid("1.3.6.1.2.1.1.9.9"), Value::NoSuchObject),
        VarBind::new(oid("1.3.6.1.2.1.1.9.8"), Value::NoSuchInstance),
        VarBind::new(oid("1.3.6.1.2.1.1.9.7"), Value::EndOfMibView),
        VarBind::new(oid("1.3.6.1.2.1.1.9.6"), Value::Null),
        VarBind::new(
            oid("1.3.6.1.2.1.1.9.5"),
            Value::Integer(i64::from(i32::MIN)),
        ),
    ];
    let m = response(-77, vbs);
    let b = encode(&m).unwrap();
    assert_eq!(decode_response(&b, -77).unwrap(), m);
}

#[test]
fn oid_parse_and_large_subidentifiers() {
    let o = oid("1.3.6.1.4.1.4294967295.0");
    let m = response(1, vec![VarBind::new(o.clone(), Value::Null)]);
    assert_eq!(decode(&encode(&m).unwrap()).unwrap().varbinds[0].oid, o);
    assert_eq!(o.to_string(), "1.3.6.1.4.1.4294967295.0");
    assert!(Oid::parse("").is_err());
    assert!(Oid::parse("1").is_err());
    assert!(Oid::parse("3.1").is_err());
    assert!(Oid::parse("1.40").is_err());
    assert!(Oid::parse("1.3.x").is_err());
    assert!(Oid::parse("1.3.4294967296").is_err());
    assert!(Oid::parse("1..3").is_err());
    assert!(Oid::parse("2.999.1").is_ok());
    assert!(oid("1.3.6.1.2").starts_with(&oid("1.3.6")));
    assert!(!oid("1.3").starts_with(&oid("1.3.6")));
    assert!(oid("1.3.6.1.2") < oid("1.3.6.2"));
}

#[test]
fn mismatched_request_id_and_wrong_pdu_rejected() {
    let b = encode(&response(5, vec![])).unwrap();
    assert_eq!(
        decode_response(&b, 6),
        Err(SnmpError::RequestIdMismatch {
            expected: 6,
            found: 5
        })
    );
    assert_eq!(
        decode_response(SYSDESCR_GET, 0x1234),
        Err(SnmpError::UnexpectedPdu(0xa0))
    );
}

#[test]
fn rejects_too_large() {
    let mut b = vec![0x30, 0x83, 0x01, 0x00, 0x01];
    b.resize(0x10005, 0);
    assert_eq!(decode(&b), Err(SnmpError::TooLarge));
}

#[test]
fn rejects_deep_nesting_in_values() {
    // varbind value is a constructed type nested 10 deep
    let mut v = vec![0x05, 0x00];
    for _ in 0..10 {
        let mut w = vec![0x30, v.len() as u8];
        w.extend_from_slice(&v);
        v = w;
    }
    let raw = message_with_raw_value(&v);
    assert_eq!(decode(&raw), Err(SnmpError::TooDeep));
    // 2 levels inside a varbind is fine (depth 6 total) and kept as Other
    let ok = message_with_raw_value(&[0x30, 0x04, 0x30, 0x02, 0x05, 0x00]);
    let m = decode(&ok).unwrap();
    assert!(matches!(
        m.varbinds[0].value,
        Value::Other { tag: 0x30, .. }
    ));
}

fn message_with_raw_value(value: &[u8]) -> Vec<u8> {
    let oid = [0x06, 0x02, 0x2b, 0x06];
    let mut vb = oid.to_vec();
    vb.extend_from_slice(value);
    let vb = tlv(0x30, &vb);
    let list = tlv(0x30, &vb);
    let mut pdu = vec![0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00];
    pdu.extend_from_slice(&list);
    let pdu = tlv(0xa2, &pdu);
    let mut msg = vec![0x02, 0x01, 0x00, 0x04, 0x00];
    msg.extend_from_slice(&pdu);
    tlv(0x30, &msg)
}

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let n = content.len();
    if n < 0x80 {
        out.push(n as u8);
    } else {
        out.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]);
    }
    out.extend_from_slice(content);
    out
}

#[test]
fn rejects_bad_lengths() {
    // indefinite length
    assert_eq!(decode(&[0x30, 0x80, 0, 0]), Err(SnmpError::BadLength));
    // length past end
    let mut b = SYSDESCR_GET.to_vec();
    b[1] = 0x28;
    assert_eq!(decode(&b), Err(SnmpError::Truncated));
    // trailing bytes after the message
    let mut b = SYSDESCR_GET.to_vec();
    b.push(0);
    assert_eq!(decode(&b), Err(SnmpError::TrailingBytes));
    // inner length shorter than its contents (varbind list says 0x0d instead of 0x0e)
    let mut b = SYSDESCR_GET.to_vec();
    b[26] = 0x0d;
    assert!(decode(&b).is_err());
    // long form with too many length octets
    assert_eq!(
        decode(&[0x30, 0x85, 0, 0, 0, 0, 1, 0]),
        Err(SnmpError::BadLength)
    );
    assert_eq!(decode(&[]), Err(SnmpError::Truncated));
    assert_eq!(decode(&[0x30]), Err(SnmpError::Truncated));
}

#[test]
fn rejects_bad_version_and_integers() {
    let mut b = SYSDESCR_GET.to_vec();
    b[4] = 3; // SNMPv3 is not supported
    assert_eq!(decode(&b), Err(SnmpError::BadVersion(3)));
    // empty INTEGER
    let v = message_with_raw_value(&[0x02, 0x00]);
    assert_eq!(decode(&v), Err(SnmpError::BadInteger));
    // 10-byte INTEGER
    let v = message_with_raw_value(&[0x02, 0x0a, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(decode(&v), Err(SnmpError::BadInteger));
    // negative Counter32
    let v = message_with_raw_value(&[0x41, 0x01, 0xff]);
    assert_eq!(decode(&v), Err(SnmpError::BadInteger));
    // Counter32 above u32
    let v = message_with_raw_value(&[0x41, 0x05, 0x01, 0, 0, 0, 0]);
    assert_eq!(decode(&v), Err(SnmpError::BadInteger));
    // non-minimal but in range is tolerated
    let v = message_with_raw_value(&[0x02, 0x03, 0x00, 0x00, 0x05]);
    assert_eq!(decode(&v).unwrap().varbinds[0].value, Value::Integer(5));
    // multi-byte tag
    let v = message_with_raw_value(&[0x1f, 0x01, 0x00]);
    assert!(decode(&v).is_err());
    // bad OID: sub-identifier with leading 0x80
    let v = message_with_raw_value(&[0x06, 0x02, 0x80, 0x01]);
    assert_eq!(decode(&v), Err(SnmpError::BadOid));
    // IpAddress with 3 bytes
    let v = message_with_raw_value(&[0x40, 0x03, 1, 2, 3]);
    assert!(decode(&v).is_err());
}

#[test]
fn encode_rejects_oversized_message() {
    let big = Value::OctetString(vec![0; MAX_MESSAGE]);
    let m = response(1, vec![VarBind::new(oid("1.3.6"), big)]);
    assert_eq!(encode(&m), Err(SnmpError::TooLarge));
}

#[test]
fn too_many_varbinds_rejected() {
    let vbs = (0..(MAX_VARBINDS as u32 + 1))
        .map(|i| VarBind::new(oid("1.3.6").child(i), Value::Null))
        .collect();
    assert_eq!(encode(&response(1, vbs)), Err(SnmpError::TooManyVarBinds));
}

fn arb_oid() -> impl Strategy<Value = Oid> {
    (
        0u32..3,
        0u32..40,
        proptest::collection::vec(any::<u32>(), 0..20),
    )
        .prop_map(|(a, b, rest)| {
            let mut v = vec![a, b];
            v.extend(rest);
            Oid::from_vec(v).unwrap()
        })
}

fn arb_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<i32>().prop_map(|i| Value::Integer(i64::from(i))),
        proptest::collection::vec(any::<u8>(), 0..300).prop_map(Value::OctetString),
        Just(Value::Null),
        arb_oid().prop_map(Value::ObjectId),
        any::<[u8; 4]>().prop_map(Value::IpAddress),
        any::<u32>().prop_map(Value::Counter32),
        any::<u32>().prop_map(Value::Gauge32),
        any::<u32>().prop_map(Value::TimeTicks),
        any::<u64>().prop_map(Value::Counter64),
        Just(Value::NoSuchObject),
        Just(Value::NoSuchInstance),
        Just(Value::EndOfMibView),
    ]
}

proptest! {
    #[test]
    fn decode_never_panics(b in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = decode(&b);
    }

    #[test]
    fn decode_never_panics_on_mutated_valid(idx in 0usize..41, byte in any::<u8>()) {
        let mut b = SYSDESCR_GET.to_vec();
        b[idx] = byte;
        let _ = decode(&b);
    }

    #[test]
    fn round_trip(id in any::<i32>(), vbs in proptest::collection::vec((arb_oid(), arb_value()), 0..20)) {
        let m = response(id, vbs.into_iter().map(|(o, v)| VarBind::new(o, v)).collect());
        let b = encode(&m).unwrap();
        prop_assert_eq!(decode_response(&b, id).unwrap(), m);
    }
}
