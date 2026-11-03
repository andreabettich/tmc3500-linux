//! `tmc3500ctl`: admin CLI for the TM-C3500 driver. Every command supports `--json`.
//!
//! Exit codes: 0 ok, 1 error, 2 printer unreachable (or usage error from argument parsing),
//! 3 maintenance command disabled.
#![forbid(unsafe_code)]

mod lpstat;

use clap::{Args, Parser, Subcommand};
use serde_json::{json, Value};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tmc3500_common::alert::write_atomic;
use tmc3500_common::config::Config;
use tmc3500_common::snmp::{resolve, SnmpClient};
use tmc3500_common::time::rfc3339;
use tmc3500_common::uri::DeviceUri;
use tmc3500_core::status::printer::{
    reason_message, LevelState, PrinterStatus, Supply, SupplyKind, Thresholds,
};

const DEFAULT_PPD: &str = "/usr/share/ppd/tmc3500/tmc3500.ppd";
const EXIT_ERROR: i32 = 1;
const EXIT_UNREACHABLE: i32 = 2;
const EXIT_DISABLED: i32 = 3;
/// Timeout for lpstat / lp / lpadmin.
const TOOL_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest tool output read.
const MAX_TOOL_OUTPUT: usize = 16 * 1024 * 1024;

#[derive(Parser, Debug)]
#[command(name = "tmc3500ctl", version, about = "Epson TM-C3500 admin tool")]
struct Cli {
    /// JSON output.
    #[arg(long, global = true)]
    json: bool,
    /// Config file (default: $TMC3500_CONFIG or /etc/tmc3500/tmc3500.toml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Debug)]
struct Target {
    /// Printer address (default: [printer] host from the config).
    #[arg(long)]
    printer: Option<String>,
}

#[derive(Args, Debug)]
struct Maint {
    #[command(flatten)]
    target: Target,
    #[arg(long = "i-know-this-is-unverified", hide = true)]
    unverified: bool,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Printer state, reasons and supplies.
    Status(Target),
    /// Ink and maintenance box levels.
    Ink(Target),
    /// Page/usage counters reported over SNMP.
    Counters(Target),
    /// Print a nozzle check pattern (disabled until verified on the real printer).
    NozzleCheck(Maint),
    /// Head cleaning (disabled until verified on the real printer).
    Clean(Maint),
    /// Write the printer address to the config and print (or run) the lpadmin command.
    Setup {
        /// Printer IP address or host name.
        #[arg(long)]
        printer: String,
        /// CUPS queue name (default: [printer] queue, "TM-C3500").
        #[arg(long)]
        queue: Option<String>,
        #[arg(long, default_value = DEFAULT_PPD)]
        ppd: PathBuf,
        /// Run lpadmin (otherwise it is only printed).
        #[arg(long)]
        apply: bool,
    },
    /// Release jobs held on the queue (e.g. after a lost connection) with `lp -i <id> -H resume`.
    ReleaseHeldJobs {
        /// CUPS queue name (default: [printer] queue).
        #[arg(long)]
        queue: Option<String>,
        /// Only list the held jobs.
        #[arg(long)]
        dry_run: bool,
        /// Release only this job (`TM-C3500-12` or `12`; repeatable).
        #[arg(long = "job", value_name = "ID")]
        jobs: Vec<String>,
        /// Release every held job on the queue.
        #[arg(long)]
        yes: bool,
    },
}

struct Ctx {
    json: bool,
    cfg: Config,
    cfg_path: PathBuf,
}

impl Ctx {
    fn fail(&self, code: i32, msg: &str, extra: Value) -> i32 {
        if self.json {
            let mut v = json!({ "ok": false, "error": msg });
            if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
                o.extend(e);
            }
            println!("{v}");
        }
        eprintln!("tmc3500ctl: {msg}");
        code
    }

    fn print(&self, v: &Value, human: impl FnOnce() -> String) {
        if self.json {
            println!("{v}");
        } else {
            print!("{}", human());
        }
    }
}

fn main() {
    let cli = Cli::parse();
    std::process::exit(run(cli));
}

fn run(cli: Cli) -> i32 {
    let cfg_path = cli.config.clone().unwrap_or_else(Config::path_from_env);
    let loaded = Config::load(&cfg_path);
    let mut ctx = Ctx {
        json: cli.json,
        cfg: Config::default(),
        cfg_path: cfg_path.clone(),
    };
    match loaded {
        Ok(c) => ctx.cfg = c,
        Err(e) => {
            return ctx.fail(
                EXIT_ERROR,
                &format!("{}: {e}", cfg_path.display()),
                json!({}),
            )
        }
    }
    match cli.cmd {
        Cmd::Status(t) => status(&ctx, &t, View::Status),
        Cmd::Ink(t) => status(&ctx, &t, View::Ink),
        Cmd::Counters(t) => status(&ctx, &t, View::Counters),
        Cmd::NozzleCheck(m) => maintenance(&ctx, "nozzle-check", &m),
        Cmd::Clean(m) => maintenance(&ctx, "clean", &m),
        Cmd::Setup {
            printer,
            queue,
            ppd,
            apply,
        } => setup(&mut ctx, &printer, queue, &ppd, apply),
        Cmd::ReleaseHeldJobs {
            queue,
            dry_run,
            jobs,
            yes,
        } => release(&ctx, queue, dry_run, &jobs, yes),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Status,
    Ink,
    Counters,
}

fn bracket(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn query(ctx: &Ctx, t: &Target) -> Result<(String, PrinterStatus), (i32, String, Value)> {
    let host = t
        .printer
        .clone()
        .or_else(|| ctx.cfg.printer.host.clone())
        .ok_or((
            EXIT_ERROR,
            "no printer configured: run `tmc3500ctl setup --printer <ip>` or pass --printer".into(),
            json!({}),
        ))?;
    let auth = format!("{}:{}", bracket(&host), ctx.cfg.snmp.port);
    let unreachable = |e: String| {
        (
            EXIT_UNREACHABLE,
            format!("printer {host} unreachable: {e}"),
            json!({ "printer": host, "reachable": false, "error": e }),
        )
    };
    let addr = resolve(&auth).map_err(|e| unreachable(e.to_string()))?;
    let c = &ctx.cfg.snmp;
    let client = SnmpClient::new(addr, &c.community, c.version.wire(), c.timeout(), c.retries);
    let st = client
        .query_full()
        .map_err(|e| unreachable(e.to_string()))?;
    Ok((host, st))
}

fn level_state(s: &Supply, thr: &Thresholds) -> &'static str {
    match s.level_state(thr) {
        LevelState::Ok => "ok",
        LevelState::Low => "low",
        LevelState::Empty => "empty",
        LevelState::Unknown => "unknown",
    }
}

fn supply_json(s: &Supply, thr: &Thresholds) -> Value {
    let (kind, color) = match s.kind {
        SupplyKind::Ink(c) => ("ink", Some(c.name())),
        SupplyKind::MaintenanceBox => ("maintenance-box", None),
        SupplyKind::Other => ("other", None),
    };
    json!({
        "index": s.index,
        "description": s.description,
        "kind": kind,
        "color": color,
        "percent": s.percent_remaining(),
        "state": level_state(s, thr),
    })
}

fn supplies_text(st: &PrinterStatus, thr: &Thresholds) -> String {
    let mut out = String::new();
    for s in &st.supplies {
        let name = match s.kind {
            SupplyKind::Ink(c) => c.name(),
            SupplyKind::MaintenanceBox => "maintenance box",
            SupplyKind::Other => "other",
        };
        let pct = s
            .percent_remaining()
            .map_or("  ?".to_string(), |p| format!("{p:3}%"));
        let free = if s.kind == SupplyKind::MaintenanceBox {
            " free"
        } else {
            ""
        };
        let flag = match s.level_state(thr) {
            LevelState::Low if s.kind == SupplyKind::MaintenanceBox => "  NEAR FULL",
            LevelState::Empty if s.kind == SupplyKind::MaintenanceBox => "  FULL",
            LevelState::Low => "  LOW",
            LevelState::Empty => "  EMPTY",
            _ => "",
        };
        out.push_str(&format!(
            "  {name:<16}{pct}{free}  {}{flag}\n",
            s.description
        ));
    }
    if st.supplies.is_empty() {
        out.push_str("  (no supply information)\n");
    }
    out
}

fn status(ctx: &Ctx, t: &Target, view: View) -> i32 {
    let thr = ctx.cfg.thresholds();
    let (host, st) = match query(ctx, t) {
        Ok(v) => v,
        Err((code, msg, extra)) => return ctx.fail(code, &msg, extra),
    };
    let reasons: Vec<&str> = st.reasons(&thr).into_iter().collect();
    let supplies: Vec<Value> = st.supplies.iter().map(|s| supply_json(s, &thr)).collect();
    let v = match view {
        View::Status => json!({
            "printer": host,
            "reachable": true,
            "sys_descr": st.sys_descr,
            "device_status": st.device.map(|d| d.name()),
            "printer_status": st.printer.map(|p| p.name()),
            "blocking": st.is_blocking(&thr),
            "reasons": reasons,
            "errors": st.errors.iter().map(|e| e.name()).collect::<Vec<_>>(),
            "supplies": supplies,
        }),
        View::Ink => json!({ "printer": host, "reachable": true, "supplies": supplies }),
        View::Counters => json!({
            "printer": host,
            "reachable": true,
            "sys_descr": st.sys_descr,
            "uptime_seconds": st.uptime_ticks.map(|t| t / 100),
            "life_count": st.life_count,
            "power_on_count": st.power_on_count,
        }),
    };
    ctx.print(&v, || match view {
        View::Status => {
            let mut o = format!(
                "Printer:  {host}{}\n",
                st.sys_descr
                    .as_deref()
                    .map_or(String::new(), |d| format!(" ({d})"))
            );
            o.push_str(&format!(
                "State:    device {}, printer {}\n",
                st.device.map_or("?", |d| d.name()),
                st.printer.map_or("?", |p| p.name())
            ));
            o.push_str(&format!(
                "Ready:    {}\n",
                if st.is_blocking(&thr) { "no" } else { "yes" }
            ));
            if reasons.is_empty() {
                o.push_str("Reasons:  none\n");
            } else {
                o.push_str("Reasons:\n");
                for r in &reasons {
                    o.push_str(&format!("  {r:<34}{}\n", reason_message(r)));
                }
            }
            o.push_str("Supplies:\n");
            o.push_str(&supplies_text(&st, &thr));
            o
        }
        View::Ink => format!("Supplies of {host}:\n{}", supplies_text(&st, &thr)),
        View::Counters => {
            let opt = |v: Option<u64>| v.map_or("n/a".to_string(), |v| v.to_string());
            format!(
                "Printer:        {host}\nsysDescr:       {}\nuptime:         {} s\nlife count:     {}\npower-on count: {}\n",
                st.sys_descr.as_deref().unwrap_or("n/a"),
                opt(st.uptime_ticks.map(|t| t / 100)),
                opt(st.life_count),
                opt(st.power_on_count),
            )
        }
    });
    0
}

fn maintenance(ctx: &Ctx, name: &str, m: &Maint) -> i32 {
    let _ = &m.target;
    let msg = if m.unverified {
        format!("{name}: not implemented: command bytes not captured yet")
    } else {
        format!(
            "{name}: not yet verified on the real printer; maintenance commands stay disabled until phase 5"
        )
    };
    ctx.fail(EXIT_DISABLED, &msg, json!({ "command": name }))
}

fn read_capped<R: std::io::Read>(r: Option<R>) -> String {
    let mut buf = Vec::new();
    if let Some(r) = r {
        let _ = r.take(MAX_TOOL_OUTPUT as u64).read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Run a tool without a shell, with a timeout and capped output.
fn run_tool(prog: &str, args: &[String]) -> Result<String, String> {
    let mut child = Command::new(prog)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {prog}: {e}"))?;
    // Read output on threads so a chatty tool cannot block on a full pipe.
    let so = child.stdout.take();
    let se = child.stderr.take();
    let t_out = std::thread::spawn(move || read_capped(so));
    let t_err = std::thread::spawn(move || read_capped(se));
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > TOOL_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{prog} timed out"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("{prog}: {e}")),
        }
    };
    let out = t_out.join().unwrap_or_default();
    let err = t_err.join().unwrap_or_default();
    if status.success() {
        Ok(out)
    } else {
        let e = err.trim();
        Err(format!(
            "{prog} failed ({status}){}",
            if e.is_empty() {
                String::new()
            } else {
                format!(": {e}")
            }
        ))
    }
}

fn valid_queue(q: &str) -> bool {
    !q.is_empty()
        && q.len() <= 127
        && q.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'.')
}

fn setup(ctx: &mut Ctx, printer: &str, queue: Option<String>, ppd: &Path, apply: bool) -> i32 {
    let uri = format!("tmc3500://{}", bracket(printer));
    if printer.starts_with('-') || DeviceUri::parse(&uri).is_err() {
        return ctx.fail(
            EXIT_ERROR,
            &format!("invalid printer address {printer:?}"),
            json!({}),
        );
    }
    let mut cfg = ctx.cfg.clone();
    cfg.printer.host = Some(printer.to_string());
    if let Some(q) = queue {
        cfg.printer.queue = q;
    }
    if !valid_queue(&cfg.printer.queue) {
        return ctx.fail(EXIT_ERROR, "invalid queue name", json!({}));
    }
    let ppd_s = ppd.to_string_lossy().into_owned();
    if ppd_s.starts_with('-') || ppd_s.chars().any(char::is_control) {
        return ctx.fail(EXIT_ERROR, "invalid PPD path", json!({}));
    }
    if let Err(e) = cfg.validate() {
        return ctx.fail(EXIT_ERROR, &e.to_string(), json!({}));
    }
    let text = match cfg.to_toml() {
        Ok(t) => format!(
            "# Written by tmc3500ctl setup on {}\n{t}",
            rfc3339(tmc3500_common::time::unix_now())
        ),
        Err(e) => return ctx.fail(EXIT_ERROR, &e.to_string(), json!({})),
    };
    if let Some(dir) = ctx.cfg_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = write_atomic(&ctx.cfg_path, text.as_bytes()) {
        return ctx.fail(
            EXIT_ERROR,
            &format!(
                "cannot write {}: {e} (run as root?)",
                ctx.cfg_path.display()
            ),
            json!({}),
        );
    }
    ctx.cfg = cfg;
    let args: Vec<String> = [
        "-p",
        &ctx.cfg.printer.queue,
        "-E",
        "-v",
        &uri,
        "-P",
        &ppd_s,
        "-o",
        "printer-error-policy=stop-printer",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let cmdline = format!("lpadmin {}", args.join(" "));
    let mut applied = false;
    if apply {
        if let Err(e) = run_tool("lpadmin", &args) {
            return ctx.fail(EXIT_ERROR, &e, json!({ "command": args }));
        }
        applied = true;
        // The monitor needs the printer address, so it is enabled only now; restart (not just
        // start) so a monitor that is already running picks up a changed address.
        for action in ["enable", "restart"] {
            if let Err(e) = run_tool(
                "systemctl",
                &[action.into(), "tmc3500-monitor.service".into()],
            ) {
                eprintln!("WARNING: systemctl {action} tmc3500-monitor.service failed ({e}); start it by hand");
                break;
            }
        }
    }
    let v = json!({
        "ok": true,
        "printer": printer,
        "config": ctx.cfg_path,
        "uri": uri,
        "command": std::iter::once("lpadmin".to_string()).chain(args.iter().cloned()).collect::<Vec<_>>(),
        "applied": applied,
    });
    ctx.print(&v, || {
        let mut o = format!("Wrote {}\n", ctx.cfg_path.display());
        if applied {
            o.push_str(&format!("Ran: {cmdline}\n"));
        } else {
            o.push_str(&format!(
                "Create the CUPS queue with (or rerun with --apply):\n{cmdline}\n"
            ));
        }
        o
    });
    0
}

const REPRINT_WARNING: &str = "Warning: a released job prints again from the start, so it may reprint labels that were already printed. Check the printer output first.";

fn release(ctx: &Ctx, queue: Option<String>, dry_run: bool, jobs: &[String], yes: bool) -> i32 {
    let queue = queue.unwrap_or_else(|| ctx.cfg.printer.queue.clone());
    if !valid_queue(&queue) {
        return ctx.fail(EXIT_ERROR, "invalid queue name", json!({}));
    }
    let out = match run_tool("lpstat", &["-l".into(), "-o".into(), queue.clone()]) {
        Ok(o) => o,
        Err(e) => return ctx.fail(EXIT_ERROR, &e, json!({ "queue": queue })),
    };
    let held = lpstat::held_jobs(&out, &queue);
    let mut targets = Vec::new();
    for j in jobs {
        let id = if !j.is_empty() && j.bytes().all(|c| c.is_ascii_digit()) {
            format!("{queue}-{j}")
        } else {
            j.clone()
        };
        if !held.contains(&id) {
            return ctx.fail(
                EXIT_ERROR,
                &format!(
                    "{j} is not a held job on {queue} (held: {})",
                    held.join(", ")
                ),
                json!({ "queue": queue, "held": held }),
            );
        }
        if !targets.contains(&id) {
            targets.push(id);
        }
    }
    if jobs.is_empty() {
        targets = held.clone();
    }
    if !dry_run && jobs.is_empty() && !yes && !held.is_empty() {
        return ctx.fail(
            EXIT_ERROR,
            &format!(
                "held jobs on {queue}: {}. {REPRINT_WARNING} Release one with --job <id>, or all with --yes.",
                held.join(", ")
            ),
            json!({ "queue": queue, "held": held, "warning": REPRINT_WARNING }),
        );
    }
    if !ctx.json && !dry_run && !targets.is_empty() {
        // shown before anything is released
        println!("{REPRINT_WARNING}");
    }
    let mut released = Vec::new();
    let mut failed = Vec::new();
    let mut warnings = Vec::new();
    if !dry_run {
        for id in &targets {
            // Remove the partial-send marker BEFORE resuming: CUPS restarts the job at once, and a
            // marker still present would make the backend hold it again.
            let num = id.rsplit('-').next().unwrap_or(id).to_string();
            let state_dir = &ctx.cfg.alerts.state_dir;
            let marker = tmc3500_common::markers::read_marker(state_dir, &num);
            if marker.is_some() {
                if let Err(e) = tmc3500_common::markers::remove_marker(state_dir, &num) {
                    failed.push(json!({ "id": id, "error": format!("cannot remove the partial-send marker: {e}") }));
                    continue;
                }
            }
            match run_tool(
                "lp",
                &["-i".into(), id.clone(), "-H".into(), "resume".into()],
            ) {
                Ok(_) => {
                    if !ctx.json {
                        println!("Released {id}");
                    }
                    released.push(id.clone());
                }
                Err(e) => {
                    // not resumed: put the marker back so the job stays protected
                    if let Some(m) = &marker {
                        if let Err(we) = tmc3500_common::markers::write_marker(state_dir, &num, m) {
                            warnings.push(format!(
                                "cannot restore the partial-send marker of {id}: {we}"
                            ));
                        }
                    }
                    failed.push(json!({ "id": id, "error": e }));
                }
            }
        }
    }
    let ok = failed.is_empty();
    let v = json!({
        "ok": ok,
        "queue": queue,
        "dry_run": dry_run,
        "held": held,
        "released": released,
        "failed": failed,
        "warning": REPRINT_WARNING,
        "warnings": warnings,
    });
    if ctx.json {
        println!("{v}");
    } else if held.is_empty() {
        println!("No held jobs on {queue}");
    } else if dry_run {
        println!(
            "Held jobs on {queue} (dry run, nothing released): {}",
            held.join(", ")
        );
    }
    for w in &warnings {
        eprintln!("tmc3500ctl: {w}");
    }
    if !ok {
        for f in &failed {
            eprintln!(
                "tmc3500ctl: cannot release {}: {}",
                f["id"].as_str().unwrap_or("?"),
                f["error"].as_str().unwrap_or("?")
            );
        }
        return EXIT_ERROR;
    }
    0
}
