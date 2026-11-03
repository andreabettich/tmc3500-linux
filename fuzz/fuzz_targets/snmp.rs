//! SNMP reply (UDP from the network) → message → printer status; Epson back-channel reply.
#![no_main]
use libfuzzer_sys::fuzz_target;
use tmc3500_core::status::{epson, printer::PrinterStatus, snmp};

fuzz_target!(|data: &[u8]| {
    if let Ok(m) = snmp::decode(data) {
        let s = PrinterStatus::from_varbinds(&m.varbinds);
        let _ = format!("{s:?}");
        let _ = snmp::encode(&m);
    }
    let _ = snmp::decode_response(data, 1);
    let _ = epson::parse_reply(data);
});
