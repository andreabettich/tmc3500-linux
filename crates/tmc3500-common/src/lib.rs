//! Shared runtime code (I/O) for the TM-C3500 backend, monitor and CLI.
#![forbid(unsafe_code)]

pub mod alert;
pub mod config;
pub mod cups;
pub mod net;
pub mod snmp;
pub mod time;
pub mod uri;
