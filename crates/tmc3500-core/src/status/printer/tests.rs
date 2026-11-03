use super::*;
use crate::status::snmp::{Oid, Value, VarBind};
use proptest::prelude::*;

fn vb(oid: &str, value: Value) -> VarBind {
    VarBind::new(Oid::parse(oid).unwrap(), value)
}

fn s(x: &str) -> Value {
    Value::OctetString(x.as_bytes().to_vec())
}

const THR: Thresholds = Thresholds {
    ink_low_percent: 10,
    waste_near_full_percent: 90,
};

/// A healthy printer: 4 inks and the maintenance box.
fn healthy() -> Vec<VarBind> {
    let mut v = vec![
        vb(oids::HR_DEVICE_STATUS, Value::Integer(2)),
        vb(oids::HR_PRINTER_STATUS, Value::Integer(3)),
        vb(
            oids::HR_PRINTER_DETECTED_ERROR_STATE,
            Value::OctetString(vec![0, 0]),
        ),
    ];
    let names = [
        "Black Ink SJIC22P(K)",
        "Cyan Ink SJIC22P(C)",
        "Magenta Ink SJIC22P(M)",
        "Yellow Ink SJIC22P(Y)",
        "Maintenance Box SJMB3500",
    ];
    for (i, n) in names.iter().enumerate() {
        let x = i as u32 + 1;
        v.push(VarBind::new(supply_oid(SUPPLY_DESCRIPTION, x), s(n)));
        v.push(VarBind::new(
            supply_oid(SUPPLY_MAX_CAPACITY, x),
            Value::Integer(100),
        ));
        v.push(VarBind::new(
            supply_oid(SUPPLY_LEVEL, x),
            Value::Integer(80),
        ));
    }
    v
}

fn with(mut v: Vec<VarBind>, extra: Vec<VarBind>) -> Vec<VarBind> {
    for e in extra {
        v.retain(|x| x.oid != e.oid);
        v.push(e);
    }
    v
}

fn errs(b: &[u8]) -> VarBind {
    vb(
        oids::HR_PRINTER_DETECTED_ERROR_STATE,
        Value::OctetString(b.to_vec()),
    )
}

#[test]
fn healthy_printer_has_no_reasons() {
    let st = PrinterStatus::from_varbinds(&healthy());
    assert_eq!(st.device, Some(DeviceStatus::Running));
    assert_eq!(st.printer, Some(PrinterState::Idle));
    assert!(st.reasons(&THR).is_empty());
    assert!(!st.is_blocking(&THR));
    assert!(st.conditions(&THR).is_empty());
    assert_eq!(st.supplies.len(), 5);
    assert_eq!(st.supplies[0].kind, SupplyKind::Ink(InkColor::Black));
    assert_eq!(st.supplies[1].kind, SupplyKind::Ink(InkColor::Cyan));
    assert_eq!(st.supplies[2].kind, SupplyKind::Ink(InkColor::Magenta));
    assert_eq!(st.supplies[3].kind, SupplyKind::Ink(InkColor::Yellow));
    assert_eq!(st.supplies[4].kind, SupplyKind::MaintenanceBox);
    assert_eq!(st.supplies[0].percent_remaining(), Some(80));
}

#[test]
fn every_error_bit_decodes() {
    let all = [
        (ErrorBit::LowPaper, [0x80, 0]),
        (ErrorBit::NoPaper, [0x40, 0]),
        (ErrorBit::LowToner, [0x20, 0]),
        (ErrorBit::NoToner, [0x10, 0]),
        (ErrorBit::DoorOpen, [0x08, 0]),
        (ErrorBit::Jammed, [0x04, 0]),
        (ErrorBit::Offline, [0x02, 0]),
        (ErrorBit::ServiceRequested, [0x01, 0]),
        (ErrorBit::InputTrayMissing, [0, 0x80]),
        (ErrorBit::OutputTrayMissing, [0, 0x40]),
        (ErrorBit::MarkerSupplyMissing, [0, 0x20]),
        (ErrorBit::OutputNearFull, [0, 0x10]),
        (ErrorBit::OutputFull, [0, 0x08]),
        (ErrorBit::InputTrayEmpty, [0, 0x04]),
        (ErrorBit::OverduePreventMaint, [0, 0x02]),
    ];
    assert_eq!(all.len(), ErrorBit::ALL.len());
    for (bit, bytes) in all {
        let e = DetectedErrors::from_octets(&bytes);
        assert!(e.contains(bit), "{bit:?}");
        assert_eq!(e.iter().collect::<Vec<_>>(), vec![bit]);
    }
    // one octet only, and extra octets beyond the second are ignored
    assert!(DetectedErrors::from_octets(&[0x40]).contains(ErrorBit::NoPaper));
    assert!(DetectedErrors::from_octets(&[0, 0, 0xff, 0xff]).is_empty());
    assert!(DetectedErrors::from_octets(&[]).is_empty());
    // bit 15 (unassigned) is ignored
    assert!(DetectedErrors::from_octets(&[0, 0x01]).is_empty());
}

fn reasons_for(extra: Vec<VarBind>) -> Vec<&'static str> {
    PrinterStatus::from_varbinds(&with(healthy(), extra))
        .reasons(&THR)
        .into_iter()
        .collect()
}

#[test]
fn error_bits_map_to_cups_reasons() {
    assert_eq!(
        reasons_for(vec![errs(&[0x40, 0])]),
        vec!["media-empty-error"]
    );
    assert_eq!(
        reasons_for(vec![errs(&[0, 0x04])]),
        vec!["media-empty-error"]
    );
    assert_eq!(reasons_for(vec![errs(&[0x04, 0])]), vec!["media-jam-error"]);
    assert_eq!(reasons_for(vec![errs(&[0x08, 0])]), vec!["cover-open"]);
    assert_eq!(reasons_for(vec![errs(&[0x02, 0])]), vec!["offline-report"]);
    assert_eq!(
        reasons_for(vec![errs(&[0x20, 0])]),
        vec!["marker-supply-low-warning"]
    );
    assert_eq!(
        reasons_for(vec![errs(&[0x10, 0])]),
        vec!["marker-supply-empty-error"]
    );
    assert_eq!(reasons_for(vec![errs(&[0x01, 0])]), vec!["other-error"]);
    assert_eq!(
        reasons_for(vec![errs(&[0x80, 0])]),
        vec!["media-low-report"]
    );
    assert_eq!(
        reasons_for(vec![errs(&[0x4c, 0])]),
        vec!["cover-open", "media-empty-error", "media-jam-error"]
    );
}

#[test]
fn device_down_without_cause_is_other_error() {
    let r = reasons_for(vec![vb(oids::HR_DEVICE_STATUS, Value::Integer(5))]);
    assert_eq!(r, vec!["other-error"]);
    // down with a known cause only reports the cause
    let r = reasons_for(vec![
        vb(oids::HR_DEVICE_STATUS, Value::Integer(5)),
        errs(&[0x40, 0]),
    ]);
    assert_eq!(r, vec!["media-empty-error"]);
}

fn level(x: u32, l: i64) -> VarBind {
    VarBind::new(supply_oid(SUPPLY_LEVEL, x), Value::Integer(l))
}

#[test]
fn supply_levels_map_to_reasons_and_conditions() {
    let st = PrinterStatus::from_varbinds(&with(healthy(), vec![level(2, 9), level(4, 0)]));
    let r: Vec<_> = st.reasons(&THR).into_iter().collect();
    assert_eq!(
        r,
        vec!["marker-supply-empty-error", "marker-supply-low-warning"]
    );
    assert!(st.is_blocking(&THR));
    let keys: Vec<_> = st.conditions(&THR).into_iter().map(|c| c.key).collect();
    assert_eq!(keys, vec!["ink-low:cyan", "ink-empty:yellow"]);

    // maintenance box: Epson style (level = remaining capacity)
    let st = PrinterStatus::from_varbinds(&with(healthy(), vec![level(5, 10)]));
    assert_eq!(
        st.reasons(&THR).into_iter().collect::<Vec<_>>(),
        vec!["marker-waste-almost-full-warning"]
    );
    assert!(!st.is_blocking(&THR));
    let c = st.conditions(&THR);
    assert_eq!(c[0].event, EventKind::MaintenanceBoxNearFull);
    let st = PrinterStatus::from_varbinds(&with(healthy(), vec![level(5, 0)]));
    assert_eq!(
        st.reasons(&THR).into_iter().collect::<Vec<_>>(),
        vec!["marker-waste-full-error"]
    );
    assert_eq!(st.conditions(&THR)[0].key, "maintenance-box-full");

    // receptacleThatIsFilled: level = fill amount
    let st = PrinterStatus::from_varbinds(&with(
        healthy(),
        vec![
            VarBind::new(supply_oid(SUPPLY_CLASS, 5), Value::Integer(4)),
            level(5, 95),
        ],
    ));
    assert_eq!(st.supplies[4].percent_remaining(), Some(5));
    assert_eq!(
        st.reasons(&THR).into_iter().collect::<Vec<_>>(),
        vec!["marker-waste-almost-full-warning"]
    );
}

#[test]
fn special_level_values() {
    let mk = |max: i64, lvl: i64| Supply {
        index: 1,
        description: "Black".into(),
        kind: SupplyKind::Ink(InkColor::Black),
        class: None,
        max_capacity: Some(max),
        level: Some(lvl),
    };
    assert_eq!(mk(100, -1).percent_remaining(), None);
    assert_eq!(mk(100, -2).percent_remaining(), None);
    assert_eq!(mk(100, -3).percent_remaining(), None);
    assert_eq!(mk(100, -3).level_state(&THR), LevelState::Ok);
    assert_eq!(mk(100, -2).level_state(&THR), LevelState::Unknown);
    assert_eq!(mk(0, 5).percent_remaining(), None);
    assert_eq!(mk(-2, 5).percent_remaining(), None);
    assert_eq!(mk(100, 150).percent_remaining(), Some(100));
    assert_eq!(mk(3, 1).percent_remaining(), Some(33));
    assert_eq!(mk(i64::MAX, i64::MAX).percent_remaining(), Some(100));
    assert_eq!(mk(100, 10).level_state(&THR), LevelState::Low);
    assert_eq!(mk(100, 11).level_state(&THR), LevelState::Ok);
    assert_eq!(mk(100, 0).level_state(&THR), LevelState::Empty);
}

#[test]
fn supply_classification() {
    let k = |d: &str| SupplyKind::classify(d);
    assert_eq!(k("SJIC22P(K)"), SupplyKind::Ink(InkColor::Black));
    assert_eq!(k("black ink"), SupplyKind::Ink(InkColor::Black));
    assert_eq!(k("SJIC22P(C)"), SupplyKind::Ink(InkColor::Cyan));
    assert_eq!(k("SJIC22P(M)"), SupplyKind::Ink(InkColor::Magenta));
    assert_eq!(k("SJIC22P(Y)"), SupplyKind::Ink(InkColor::Yellow));
    assert_eq!(k("SJMB3500"), SupplyKind::MaintenanceBox);
    assert_eq!(k("Waste Ink Box"), SupplyKind::MaintenanceBox);
    assert_eq!(k("Fuser"), SupplyKind::Other);
}

#[test]
fn missing_and_mistyped_values_are_tolerated() {
    let st = PrinterStatus::from_varbinds(&[
        vb(oids::HR_DEVICE_STATUS, Value::NoSuchObject),
        vb(oids::HR_PRINTER_STATUS, s("idle")),
        vb(oids::HR_PRINTER_DETECTED_ERROR_STATE, Value::Integer(4)),
        VarBind::new(supply_oid(SUPPLY_LEVEL, 3), Value::NoSuchInstance),
        VarBind::new(supply_oid(SUPPLY_LEVEL, 99), Value::Integer(5)),
    ]);
    assert_eq!(st.device, None);
    assert_eq!(st.printer, None);
    assert!(st.errors.is_empty());
    assert!(st.supplies.is_empty());
    assert_eq!(st.device_status_or_unknown(), DeviceStatus::Unknown);
    assert!(st.reasons(&THR).is_empty());
}

#[test]
fn marker_attributes_for_cups() {
    let st = PrinterStatus::from_varbinds(&with(healthy(), vec![level(2, -3)]));
    let a = st.marker_attrs();
    assert_eq!(
        a,
        vec![
            "marker-colors=#000000,#00FFFF,#FF00FF,#FFFF00,none".to_string(),
            "marker-levels=80,-1,80,80,80".to_string(),
            "marker-names=\"Black Ink SJIC22P(K)\",\"Cyan Ink SJIC22P(C)\",\"Magenta Ink SJIC22P(M)\",\"Yellow Ink SJIC22P(Y)\",\"Maintenance Box SJMB3500\"".to_string(),
            "marker-types=ink-cartridge,ink-cartridge,ink-cartridge,ink-cartridge,waste-ink".to_string(),
        ]
    );
    // hostile description is sanitised
    let st = PrinterStatus::from_varbinds(&with(
        healthy(),
        vec![VarBind::new(
            supply_oid(SUPPLY_DESCRIPTION, 1),
            Value::OctetString(b"Bl\"a\\ck\nATTR: x\xff".to_vec()),
        )],
    ));
    let names = &st.marker_attrs()[2];
    assert!(
        names.starts_with("marker-names=\"Bl\\\"a\\\\ck?ATTR: x?\""),
        "{names}"
    );
    assert!(!names.contains('\n'));
    assert!(PrinterStatus::default().marker_attrs().is_empty());
}

#[test]
fn status_oids_cover_the_mib_values() {
    let o = status_oids();
    for want in [
        oids::HR_DEVICE_STATUS,
        oids::HR_PRINTER_STATUS,
        oids::HR_PRINTER_DETECTED_ERROR_STATE,
        "1.3.6.1.2.1.43.11.1.1.6.1.1",
        "1.3.6.1.2.1.43.11.1.1.8.1.5",
        "1.3.6.1.2.1.43.11.1.1.9.1.5",
    ] {
        assert!(o.contains(&Oid::parse(want).unwrap()), "{want}");
    }
    assert!(o.len() <= crate::status::snmp::MAX_VARBINDS);
}

#[test]
fn event_names() {
    assert_eq!(EventKind::InkLow.name(), "ink-low");
    assert_eq!(EventKind::MaintenanceBoxFull.name(), "maintenance-box-full");
    assert_eq!(EventKind::JobHeld.name(), "job-held");
    assert_eq!(EventKind::CutterError.name(), "cutter-error");
    for e in EventKind::ALL {
        assert_eq!(EventKind::from_name(e.name()), Some(*e));
    }
}

#[test]
fn epson_extension_merges() {
    let mut st = PrinterStatus::from_varbinds(&healthy());
    st.merge_epson(crate::status::epson::EpsonStatus {
        cutter_error: true,
        reasons: vec![],
    });
    assert!(st.reasons(&THR).contains("other-error"));
    assert_eq!(st.conditions(&THR)[0].event, EventKind::CutterError);
}

fn arb_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<i64>().prop_map(Value::Integer),
        proptest::collection::vec(any::<u8>(), 0..64).prop_map(Value::OctetString),
        Just(Value::Null),
        any::<u32>().prop_map(Value::Gauge32),
    ]
}

proptest! {
    #[test]
    fn arbitrary_values_never_panic(vals in proptest::collection::vec((0usize..40, arb_value()), 0..40)) {
        let oids = status_oids();
        let vbs: Vec<_> = vals
            .into_iter()
            .map(|(i, v)| VarBind::new(oids[i % oids.len()].clone(), v))
            .collect();
        let st = PrinterStatus::from_varbinds(&vbs);
        let _ = st.reasons(&THR);
        let _ = st.conditions(&THR);
        let _ = st.marker_attrs();
        for sp in &st.supplies {
            if let Some(p) = sp.percent_remaining() {
                prop_assert!(p <= 100);
            }
        }
    }
}

#[test]
fn oid_constants_are_valid() {
    for o in [
        oids::SYS_DESCR,
        oids::SYS_UPTIME,
        oids::HR_DEVICE_STATUS,
        oids::HR_PRINTER_STATUS,
        oids::HR_PRINTER_DETECTED_ERROR_STATE,
        oids::PRT_MARKER_SUPPLIES_ENTRY,
        oids::PRT_MARKER_LIFE_COUNT,
        oids::PRT_MARKER_POWER_ON_COUNT,
    ] {
        assert!(Oid::parse(o).is_ok(), "{o}");
    }
}

#[test]
fn core_status_presence() {
    assert!(PrinterStatus::from_varbinds(&healthy()).has_core_status());
    assert!(!PrinterStatus::from_varbinds(&[]).has_core_status());
    // supplies alone are not enough to call the printer OK
    let only_supplies: Vec<_> = healthy().into_iter().skip(3).collect();
    assert!(!PrinterStatus::from_varbinds(&only_supplies).has_core_status());
    let mistyped = with(
        healthy(),
        vec![vb(
            oids::HR_PRINTER_DETECTED_ERROR_STATE,
            Value::NoSuchInstance,
        )],
    );
    assert!(!PrinterStatus::from_varbinds(&mistyped).has_core_status());
}
