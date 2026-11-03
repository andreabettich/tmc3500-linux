//! CUPS raster stream (filter input) → full page pipeline.
#![no_main]
use libfuzzer_sys::fuzz_target;
use tmc3500_core::color::Converter;
use tmc3500_core::cupsraster::Reader;
use tmc3500_core::options::JobConfig;
use tmc3500_core::pipeline::render_page;

fuzz_target!(|data: &[u8]| {
    // small media keeps each iteration fast; the reader's own limits are still exercised
    let cfg = JobConfig::from_cups("PageSize=Custom.1x0.5in", 1).unwrap();
    let conv = Converter::new(&cfg.color, cfg.black);
    let Ok(mut rd) = Reader::new(data) else { return };
    for _ in 0..4 {
        match rd.next_page() {
            Ok(Some(h)) => {
                if render_page(&mut rd, &h, &cfg, &conv).is_err() {
                    return;
                }
            }
            _ => return,
        }
    }
});
