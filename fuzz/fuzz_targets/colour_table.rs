//! Colour table file → lookups.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(lut) = tmc3500_core::color::Lut::parse(data) {
        for v in [0u8, 1, 31, 32, 128, 254, 255] {
            let _ = lut.lookup([v, 255 - v, v / 2]);
        }
    }
});
