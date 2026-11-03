//! CUPS backend `tmc3500`. See the library documentation for the behaviour.
#![forbid(unsafe_code)]

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn main() {
    let cancel = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        if let Err(e) = signal_hook::flag::register(sig, Arc::clone(&cancel)) {
            eprintln!("WARNING: cannot install signal handler: {e}");
        }
    }
    let code = tmc3500_backend::run(std::env::args_os().collect(), cancel);
    std::process::exit(code);
}
