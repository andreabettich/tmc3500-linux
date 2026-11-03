//! Pure (no I/O) core of the Epson TM-C3500 Linux driver.
#![forbid(unsafe_code)]

pub mod cupsopts;
pub mod error;
pub mod geometry;
pub mod options;
pub mod rle;
pub mod units;
