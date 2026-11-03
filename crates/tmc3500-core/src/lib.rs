//! Pure (no I/O) core of the Epson TM-C3500 Linux driver.
#![forbid(unsafe_code)]

pub mod color;
pub mod cupsopts;
pub mod decode;
pub mod dither;
pub mod error;
pub mod geometry;
pub mod layout;
pub mod options;
pub mod protocol;
pub mod raster;
pub mod rle;
pub mod units;
pub mod weave;
