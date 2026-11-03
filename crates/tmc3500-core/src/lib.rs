//! Pure (no I/O) core of the Epson TM-C3500 Linux driver.
#![forbid(unsafe_code)]

pub mod cupsopts;
pub mod decode;
pub mod error;
pub mod geometry;
pub mod options;
pub mod protocol;
pub mod rle;
pub mod status;
pub mod units;
