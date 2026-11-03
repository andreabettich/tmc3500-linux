//! `tmc3500-fakeprinter`: run the simulated printer. Faults can be set with flags at start and
//! changed at run time by typing commands on stdin (`help` lists them).
#![forbid(unsafe_code)]

use clap::Parser;
use std::io::BufRead;
use std::path::PathBuf;
use std::time::Duration;
use tmc3500_fakeprinter::{FakePrinter, Options};

#[derive(Parser, Debug)]
#[command(version, about = "Simulated Epson TM-C3500 for tests")]
struct Args {
    /// Address to bind.
    #[arg(long, default_value = "127.0.0.1")]
    bind: std::net::IpAddr,
    /// TCP job port.
    #[arg(long, default_value_t = 9100)]
    job_port: u16,
    /// UDP SNMP port (161 needs privileges; use e.g. 1161 and `?snmp-port=1161` in the URI).
    #[arg(long, default_value_t = 161)]
    snmp_port: u16,
    #[arg(long, default_value = "public")]
    community: String,
    /// Store accepted jobs as job-<n>.prn in this directory.
    #[arg(long)]
    store_dir: Option<PathBuf>,
    /// Start out of paper.
    #[arg(long)]
    paper_out: bool,
    #[arg(long)]
    jam: bool,
    #[arg(long)]
    cover_open: bool,
    #[arg(long)]
    offline: bool,
    /// Ink level, e.g. `--ink cyan=5` (repeatable).
    #[arg(long, value_name = "COLOR=PERCENT")]
    ink: Vec<String>,
    /// Maintenance box free space in percent.
    #[arg(long)]
    waste: Option<u8>,
    /// Run out of paper after N bytes of a job.
    #[arg(long)]
    paper_out_after: Option<u64>,
    /// Drop job connections after N bytes.
    #[arg(long)]
    drop_after: Option<u64>,
    /// Slow reads: `CHUNK:MS`.
    #[arg(long)]
    slow_read: Option<String>,
    /// Refuse job connections.
    #[arg(long)]
    refuse: bool,
    /// SNMP behaviour: normal, garbage, silent.
    #[arg(long, default_value = "normal")]
    snmp: String,
}

const HELP: &str = "commands: paper-out on|off, jam on|off, cover on|off, offline on|off, \
ink <color> <0-100>, waste <0-100>, drop-after <n>|off, paper-out-after <n>|off, \
slow-read <chunk> <ms>|off, refuse on|off, snmp normal|garbage|silent, jobs, status, quit";

fn main() {
    let a = Args::parse();
    let fake = match FakePrinter::start(Options {
        bind: a.bind,
        job_port: a.job_port,
        snmp_port: a.snmp_port,
        community: a.community.clone(),
        store_dir: a.store_dir.clone(),
        ..Options::default()
    }) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("tmc3500-fakeprinter: cannot start: {e}");
            std::process::exit(1);
        }
    };
    let mut startup = vec![
        format!("paper-out {}", on(a.paper_out)),
        format!("jam {}", on(a.jam)),
        format!("cover {}", on(a.cover_open)),
        format!("offline {}", on(a.offline)),
        format!("refuse {}", on(a.refuse)),
        format!("snmp {}", a.snmp),
    ];
    for i in &a.ink {
        startup.push(format!("ink {}", i.replace('=', " ")));
    }
    if let Some(w) = a.waste {
        startup.push(format!("waste {w}"));
    }
    if let Some(n) = a.paper_out_after {
        startup.push(format!("paper-out-after {n}"));
    }
    if let Some(n) = a.drop_after {
        startup.push(format!("drop-after {n}"));
    }
    if let Some(s) = &a.slow_read {
        startup.push(format!("slow-read {}", s.replace(':', " ")));
    }
    for c in &startup {
        if let Err(e) = fake.apply_command(c) {
            eprintln!("tmc3500-fakeprinter: {c}: {e}");
            std::process::exit(2);
        }
    }
    eprintln!(
        "tmc3500-fakeprinter: job port {}, SNMP port {} ({})",
        fake.job_port(),
        fake.snmp_port(),
        HELP
    );
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        match line {
            "" => continue,
            "quit" | "exit" => break,
            "help" => println!("{HELP}"),
            _ => match fake.apply_command(line) {
                Ok(r) => println!("{r}"),
                Err(e) => println!("error: {e}"),
            },
        }
    }
    // stdin closed (e.g. started in the background): keep serving until killed
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }
}

fn on(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}
