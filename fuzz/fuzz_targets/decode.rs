//! Printer job stream (fake printer input) → tokens, skeleton, reassembled pages.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = tmc3500_core::decode::skeleton(data);
    let _ = tmc3500_core::reassemble::reassemble(data);
    let _ = tmc3500_core::rle::decode(data, data.len() * 3);
});
