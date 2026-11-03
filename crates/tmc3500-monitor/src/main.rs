//! `tmc3500-monitor`: idle status poller (systemd service).
#![forbid(unsafe_code)]

use clap::Parser;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tmc3500_common::alert::{write_atomic_exact_mode, Alerter};
use tmc3500_common::config::Config;
use tmc3500_common::snmp::{resolve, SnmpClient};
use tmc3500_common::time::unix_now;
use tmc3500_monitor::{render_metrics, Monitor, Snapshot};

/// Shared alert state file name (same as the backend, so repeats are suppressed across both).
const ALERT_STATE_FILE: &str = "alerts.json";

#[derive(Parser, Debug)]
#[command(
    version,
    about = "TM-C3500 status monitor (SNMP polling, alerts, Prometheus textfile)"
)]
struct Args {
    /// Config file (default: $TMC3500_CONFIG or /etc/tmc3500/tmc3500.toml).
    #[arg(long)]
    config: Option<PathBuf>,
    /// Printer address (overrides [printer] host).
    #[arg(long)]
    printer: Option<String>,
    /// Poll interval in milliseconds (overrides [monitor] interval_ms).
    #[arg(long)]
    interval_ms: Option<u64>,
    /// Poll once and exit (exit code 2 if the printer is unreachable).
    #[arg(long)]
    once: bool,
}

fn log(level: &str, msg: &str) {
    let msg: String = msg
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    eprintln!("{level}: {msg}");
}

fn main() {
    let a = Args::parse();
    std::process::exit(run(a));
}

fn run(a: Args) -> i32 {
    let stop = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        if let Err(e) = signal_hook::flag::register(sig, Arc::clone(&stop)) {
            log("WARNING", &format!("cannot install signal handler: {e}"));
        }
    }
    let path = a.config.clone().unwrap_or_else(Config::path_from_env);
    let cfg = match Config::load(&path) {
        Ok(c) => c,
        Err(e) => {
            log("ERROR", &format!("{}: {e}", path.display()));
            return 1;
        }
    };
    let Some(host) = a.printer.clone().or(cfg.printer.host.clone()) else {
        log(
            "ERROR",
            "no printer configured: set [printer] host or run `tmc3500ctl setup --printer <ip>`",
        );
        return 1;
    };
    if !cfg.snmp.enabled {
        log(
            "ERROR",
            "SNMP is disabled in the configuration; nothing to monitor",
        );
        return 1;
    }
    let authority = if host.contains(':') {
        format!("[{host}]:{}", cfg.snmp.port)
    } else {
        format!("{host}:{}", cfg.snmp.port)
    };
    let interval = a
        .interval_ms
        .map(Duration::from_millis)
        .unwrap_or_else(|| cfg.monitor.interval())
        .max(Duration::from_millis(100));
    let alerter = Alerter::from_config(&cfg.alerts, ALERT_STATE_FILE);
    let thr = cfg.thresholds();
    let mut mon = Monitor::new(host.clone(), alerter, thr);
    log(
        "INFO",
        &format!(
            "monitoring {authority} every {:.1} s",
            interval.as_secs_f64()
        ),
    );
    loop {
        let started = Instant::now();
        let snap: Snapshot = resolve(&authority)
            .map(|addr| {
                SnmpClient::new(
                    addr,
                    &cfg.snmp.community,
                    cfg.snmp.version.wire(),
                    cfg.snmp.timeout(),
                    cfg.snmp.retries,
                )
            })
            .and_then(|c| c.query_full())
            .map_err(|e| e.to_string());
        if let Err(e) = &snap {
            log("WARNING", &format!("{host}: {e}"));
        }
        for ev in mon.step(&snap) {
            log("INFO", &format!("alert {}: {}", ev.event, ev.message));
        }
        for w in &mon.warnings {
            log("WARNING", w);
        }
        if let Some(tf) = &cfg.monitor.textfile {
            let text = render_metrics(&host, &snap, &thr, unix_now());
            if let Err(e) = write_atomic_exact_mode(tf, text.as_bytes(), cfg.monitor.textfile_mode)
            {
                log("WARNING", &format!("cannot write {}: {e}", tf.display()));
            }
        }
        if a.once {
            return if snap.is_ok() { 0 } else { 2 };
        }
        while started.elapsed() < interval {
            if stop.load(Ordering::SeqCst) {
                log("INFO", "stopping");
                return 0;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if stop.load(Ordering::SeqCst) {
            log("INFO", "stopping");
            return 0;
        }
    }
}
