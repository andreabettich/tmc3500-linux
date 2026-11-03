#![forbid(unsafe_code)]
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let env = rastertotmc3500::Env {
        ppd: std::env::var("PPD")
            .ok()
            .and_then(|p| rastertotmc3500::read_ppd(&p)),
        unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        host_id: rastertotmc3500::host_id(
            &std::fs::read_to_string("/etc/machine-id").unwrap_or_default(),
        ),
    };
    let stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    let code = if args.len() == 7 {
        match std::fs::File::open(&args[6]) {
            Ok(f) => rastertotmc3500::run(
                &args,
                std::io::BufReader::new(f),
                stdout.lock(),
                &mut stderr,
                &env,
            ),
            Err(e) => {
                eprintln!("ERROR: cannot open {}: {e}", args[6]);
                1
            }
        }
    } else {
        rastertotmc3500::run(
            &args,
            std::io::BufReader::new(std::io::stdin().lock()),
            stdout.lock(),
            &mut stderr,
            &env,
        )
    };
    std::process::exit(code);
}
