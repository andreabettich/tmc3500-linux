//! CUPS backend `tmc3500` for the Epson TM-C3500.
//!
//! Behaviour (spec section 5):
//! - before sending: query SNMP and wait while the printer is in error, reporting `STATE:` lines;
//! - printer unreachable before the job starts: retry with exponential backoff; after
//!   `unreachable_hold_after_ms` hold the job (exit 3) and alert;
//! - paper/ink out mid-job: keep the connection open, keep reporting, resume when cleared;
//! - connection lost mid-job: hold the job (exit 3) and alert `job-held`; never resend;
//! - every socket operation has a timeout; the back-channel is size-capped;
//! - SNMP unavailable: print anyway, WARNING logged (status reporting degraded);
//! - SIGTERM (job cancelled): stop, close the connection, exit 5.
#![forbid(unsafe_code)]

pub mod log;
pub mod reasons;

use log::Log;
use reasons::StateReporter;
use std::io::{ErrorKind, Read, Write as _};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tmc3500_common::alert::{Alerter, ConditionTracker, Delivery, Event, HookOutcome};
use tmc3500_common::config::Config;
use tmc3500_common::cups::*;
use tmc3500_common::markers;
use tmc3500_common::snmp::{resolve, SnmpClient};
use tmc3500_common::uri::DeviceUri;
use tmc3500_core::status::epson::{self, EpsonReply};
use tmc3500_core::status::printer::{EventKind, PrinterState, PrinterStatus, Thresholds};

pub const DISCOVERY: &str = "network tmc3500 \"Epson TM-C3500\" \"Epson TM-C3500 (tmc3500)\"";
/// Most back-channel bytes kept (more are read and discarded).
pub const MAX_BACK_CHANNEL: usize = 64 * 1024;
/// Name of the shared alert state file in the state directory.
pub const ALERT_STATE_FILE: &str = "alerts.json";
const CHUNK: usize = 64 * 1024;
const MIN_OP_TIMEOUT: Duration = Duration::from_millis(10);

/// Parsed CUPS backend arguments (`job-id user title copies options [file]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobArgs {
    pub job_id: String,
    pub user: String,
    pub title: String,
    pub copies: u32,
    pub options: String,
    pub file: Option<std::path::PathBuf>,
}

fn clean(s: &str, max: usize) -> String {
    s.chars()
        .take(max)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// `None` for discovery mode (no arguments); `Err` for a wrong argument count.
pub fn parse_args(args: &[std::ffi::OsString]) -> Result<Option<JobArgs>, String> {
    match args.len() {
        0 | 1 => Ok(None),
        6 | 7 => {
            let s = |i: usize| args[i].to_string_lossy().into_owned();
            Ok(Some(JobArgs {
                job_id: clean(&s(1), 32),
                user: clean(&s(2), 64),
                title: clean(&s(3), 128),
                // CUPS socket-backend convention: copies only apply to a file argument; jobs
                // from a filter on stdin already contain their copies.
                copies: if args.len() == 7 {
                    s(4).trim().parse::<u32>().map_or(1, |c| c.clamp(1, 9999))
                } else {
                    1
                },
                options: s(5),
                file: args.get(6).map(std::path::PathBuf::from),
            }))
        }
        _ => Err("usage: tmc3500 job-id user title copies options [file]".into()),
    }
}

/// Retry delay for attempt `n` (0-based): `initial * 2^n`, capped at `max`.
pub fn backoff(initial: Duration, max: Duration, n: u32) -> Duration {
    let factor = 1u32.checked_shl(n.min(20)).unwrap_or(u32::MAX);
    initial.saturating_mul(factor).min(max)
}

/// Why sending stopped.
#[derive(Debug)]
enum SendError {
    Cancelled {
        sent: u64,
    },
    /// Connection reset/closed by the printer.
    Lost {
        sent: u64,
        err: String,
    },
    /// No progress for `stall_timeout` while the printer reported no error (or status unknown).
    Stalled {
        sent: u64,
    },
    Input {
        sent: u64,
        err: std::io::Error,
    },
}

enum ConnectError {
    Cancelled,
    Unreachable(String),
}

struct Backend {
    cfg: Config,
    uri: DeviceUri,
    job: JobArgs,
    log: Log,
    cancel: Arc<AtomicBool>,
    snmp: Option<SnmpClient>,
    snmp_ok: Option<bool>,
    next_poll: Option<Instant>,
    last_blocking: Option<bool>,
    last_status: Option<PrinterStatus>,
    reporter: StateReporter,
    alerter: Alerter,
    tracker: ConditionTracker,
    thr: Thresholds,
    stdin: Option<Box<dyn Read>>,
    /// (current copy, number of copies).
    copies: (u32, u32),
    /// Bytes of completed copies.
    prior: u64,
}

/// Process environment the backend reads.
#[derive(Debug, Clone)]
pub struct Env {
    /// `DEVICE_URI`.
    pub device_uri: Option<String>,
    /// `$TMC3500_CONFIG` or the default config path.
    pub config_path: std::path::PathBuf,
}

impl Env {
    pub fn from_process() -> Env {
        Env {
            device_uri: std::env::var("DEVICE_URI").ok(),
            config_path: Config::path_from_env(),
        }
    }
}

/// Entry point used by `main`: returns the process exit code.
pub fn run(args: Vec<std::ffi::OsString>, cancel: Arc<AtomicBool>) -> i32 {
    run_with(
        args,
        cancel,
        &Env::from_process(),
        Box::new(std::io::stdin()),
    )
}

/// [`run`] with explicit environment and stdin (for tests).
pub fn run_with(
    args: Vec<std::ffi::OsString>,
    cancel: Arc<AtomicBool>,
    env: &Env,
    stdin: Box<dyn Read>,
) -> i32 {
    let log = Log::stderr();
    let job = match parse_args(&args) {
        Ok(None) => {
            println!("{DISCOVERY}");
            return CUPS_BACKEND_OK;
        }
        Ok(Some(j)) => j,
        Err(e) => {
            log.error(&e);
            return CUPS_BACKEND_FAILED;
        }
    };
    let cfg_path = env.config_path.clone();
    let cfg = match Config::load(&cfg_path) {
        Ok(c) => c,
        Err(e) => {
            log.error(&format!("configuration {}: {e}", cfg_path.display()));
            return CUPS_BACKEND_STOP;
        }
    };
    log.debug(&format!("configuration {}", cfg_path.display()));
    let uri_s = env
        .device_uri
        .clone()
        .or_else(|| args.first().map(|a| a.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let uri = match DeviceUri::parse(&uri_s) {
        Ok(u) => u,
        Err(e) => {
            log.error(&e.to_string());
            return CUPS_BACKEND_STOP;
        }
    };
    let snmp = if cfg.snmp.enabled {
        match resolve(&uri.snmp_authority(cfg.snmp.port)) {
            Ok(addr) => Some(SnmpClient::new(
                addr,
                uri.community.as_deref().unwrap_or(&cfg.snmp.community),
                cfg.snmp.version.wire(),
                cfg.snmp.timeout(),
                cfg.snmp.retries,
            )),
            Err(e) => {
                log.warning(&format!(
                    "SNMP status unavailable ({e}); printing without status reporting"
                ));
                None
            }
        }
    } else {
        log.debug("SNMP disabled in configuration");
        None
    };
    let alerter = Alerter::from_config(&cfg.alerts, ALERT_STATE_FILE);
    let thr = cfg.thresholds();
    let mut b = Backend {
        cfg,
        uri,
        job,
        log,
        cancel,
        snmp,
        snmp_ok: None,
        next_poll: None,
        last_blocking: None,
        last_status: None,
        reporter: StateReporter::default(),
        alerter,
        tracker: ConditionTracker::default(),
        thr,
        stdin: Some(stdin),
        copies: (1, 1),
        prior: 0,
    };
    b.run_job()
}

impl Backend {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Sleep `d`, waking early on cancel. Returns true if cancelled.
    fn sleep(&self, d: Duration) -> bool {
        let end = Instant::now() + d;
        loop {
            if self.cancelled() {
                return true;
            }
            let now = Instant::now();
            if now >= end {
                return false;
            }
            std::thread::sleep((end - now).min(Duration::from_millis(20)));
        }
    }

    fn printer_name(&self) -> String {
        self.uri.authority()
    }

    fn log_delivery(&self, ev: &Event, d: &Delivery) {
        for w in &d.warnings {
            self.log.warning(&format!("alert: {w}"));
        }
        if d.suppressed {
            self.log
                .debug(&format!("alert {} suppressed (repeat)", ev.key));
            return;
        }
        self.log.debug(&format!(
            "alert {} sent to {} hook(s)",
            ev.key,
            d.hooks.len()
        ));
        for h in &d.hooks {
            if h.outcome != HookOutcome::Ok {
                self.log.warning(&format!(
                    "alert hook {} for {}: {:?}",
                    h.hook.display(),
                    ev.event,
                    h.outcome
                ));
            }
        }
    }

    fn alert(&self, kind: EventKind, key: &str, message: &str) {
        let mut ev = Event::new(kind, key, &self.printer_name(), message, "backend");
        ev.job_id = Some(self.job.job_id.clone());
        ev.reasons = self.reporter.current().to_vec();
        let d = self.alerter.emit(&ev);
        self.log_delivery(&ev, &d);
    }

    /// Query SNMP now. Returns whether the printer is in a blocking error (None = unknown).
    fn poll_status(&mut self) -> Option<bool> {
        let interval = self.cfg.backend.status_interval();
        let Some(client) = &self.snmp else {
            self.next_poll = Some(Instant::now() + interval);
            return None;
        };
        let res = client.query_status().and_then(|st| {
            if st.has_core_status() {
                Ok(st)
            } else {
                Err(tmc3500_common::snmp::ClientError::Io(
                    "reply has no printer status values".into(),
                ))
            }
        });
        match res {
            Ok(st) => {
                if self.snmp_ok == Some(false) {
                    self.log.info("SNMP status available again");
                }
                self.snmp_ok = Some(true);
                self.next_poll = Some(Instant::now() + interval);
                self.reporter
                    .set_printer_reasons(&self.log, &st.reasons(&self.thr));
                self.reporter.attrs(&self.log, st.marker_attrs());
                let conditions = st.conditions(&self.thr);
                let job_id = self.job.job_id.clone();
                let reasons = self.reporter.current().to_vec();
                for (ev, d) in self.tracker.observe(
                    &self.alerter,
                    &conditions,
                    &self.uri.authority(),
                    "backend",
                    |e| {
                        e.job_id = Some(job_id.clone());
                        e.reasons = reasons.clone();
                    },
                ) {
                    self.log_delivery(&ev, &d);
                }
                let blocking = st.is_blocking(&self.thr);
                self.last_blocking = Some(blocking);
                self.last_status = Some(st);
                Some(blocking)
            }
            Err(e) => {
                if self.snmp_ok != Some(false) {
                    self.log.warning(&format!(
                        "SNMP status unavailable ({e}); printing continues without status reporting"
                    ));
                } else {
                    self.log.debug(&format!("SNMP: {e}"));
                }
                self.snmp_ok = Some(false);
                // poll a dead agent less often so it does not slow the job down
                self.next_poll = Some(Instant::now() + interval.saturating_mul(10));
                self.last_blocking = None;
                self.last_status = None;
                None
            }
        }
    }

    /// Poll if the status interval has elapsed, else return the last known blocking state.
    fn poll_if_due(&mut self) -> Option<bool> {
        match self.next_poll {
            Some(t) if Instant::now() < t => self.last_blocking,
            _ => self.poll_status(),
        }
    }

    /// Wait while the printer reports a blocking error. Returns false if cancelled.
    fn wait_until_ready(&mut self) -> bool {
        let mut announced = false;
        while self.poll_status() == Some(true) {
            if !announced {
                self.log
                    .info("Waiting for the printer to become ready before sending the job");
                announced = true;
            }
            if self.sleep(self.cfg.backend.status_interval()) {
                return false;
            }
        }
        if announced {
            self.log.info("Printer ready");
        }
        true
    }

    fn try_connect(&self) -> Result<TcpStream, String> {
        let auth = self.uri.authority();
        let addrs: Vec<_> = auth
            .to_socket_addrs()
            .map_err(|e| format!("cannot resolve {auth}: {e}"))?
            .collect();
        let mut last = format!("no address for {auth}");
        for a in addrs {
            match TcpStream::connect_timeout(&a, self.cfg.backend.connect_timeout()) {
                Ok(s) => return Ok(s),
                Err(e) => last = format!("{a}: {e}"),
            }
        }
        Err(last)
    }

    fn connect(&mut self, start: Instant) -> Result<TcpStream, ConnectError> {
        let bc = self.cfg.backend.clone();
        let mut attempt = 0u32;
        loop {
            if self.cancelled() {
                return Err(ConnectError::Cancelled);
            }
            match self.try_connect() {
                Ok(s) => {
                    self.reporter
                        .set_own(&self.log, "connecting-to-device", false);
                    return Ok(s);
                }
                Err(e) => {
                    let elapsed = start.elapsed();
                    let limit = bc.unreachable_hold_after();
                    if elapsed >= limit {
                        return Err(ConnectError::Unreachable(e));
                    }
                    let wait =
                        backoff(bc.retry_initial(), bc.retry_max(), attempt).min(limit - elapsed);
                    self.reporter
                        .set_own(&self.log, "connecting-to-device", true);
                    self.log.info(&format!(
                        "Printer {} unreachable ({e}); retrying in {:.1} s",
                        self.uri.authority(),
                        wait.as_secs_f64()
                    ));
                    attempt = attempt.saturating_add(1);
                    if self.sleep(wait) {
                        return Err(ConnectError::Cancelled);
                    }
                }
            }
        }
    }

    fn op_timeout(&self) -> Duration {
        self.cfg
            .backend
            .io_timeout()
            .min(self.cfg.backend.status_interval())
            .max(MIN_OP_TIMEOUT)
    }

    /// Send `pending` and then the rest of `input`; then wait for the printer to close.
    fn send(
        &mut self,
        stream: &mut TcpStream,
        input: &mut dyn Read,
        pending: &mut Vec<u8>,
    ) -> Result<u64, SendError> {
        let t = self.op_timeout();
        let lost = |sent: u64, e: &dyn std::fmt::Display| SendError::Lost {
            sent,
            err: e.to_string(),
        };
        stream
            .set_write_timeout(Some(t))
            .and_then(|_| stream.set_read_timeout(Some(t)))
            .map_err(|e| lost(0, &e))?;
        let mut total: u64 = 0;
        let mut stall_since = Instant::now();
        let mut stalled_logged = false;
        let stall_timeout = self.cfg.backend.stall_timeout();
        loop {
            if pending.is_empty() {
                pending.resize(CHUNK, 0);
                let n = loop {
                    match input.read(pending) {
                        Ok(n) => break n,
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(e) => {
                            pending.clear();
                            return Err(SendError::Input {
                                sent: total,
                                err: e,
                            });
                        }
                    }
                };
                pending.truncate(n);
                if n == 0 {
                    break;
                }
            }
            let mut off = 0;
            while off < pending.len() {
                if self.cancelled() {
                    pending.drain(..off);
                    return Err(SendError::Cancelled { sent: total });
                }
                match stream.write(&pending[off..]) {
                    Ok(0) => {
                        pending.drain(..off);
                        return Err(lost(total, &"connection closed"));
                    }
                    Ok(k) => {
                        off += k;
                        total += k as u64;
                        stall_since = Instant::now();
                        if stalled_logged {
                            self.log.info("Printer is accepting data again");
                            stalled_logged = false;
                        }
                    }
                    Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                        if !stalled_logged {
                            self.log.info("Printer is not accepting data; waiting");
                            stalled_logged = true;
                        }
                        if self.poll_if_due() == Some(true) {
                            stall_since = Instant::now();
                        } else if stall_since.elapsed() >= stall_timeout {
                            pending.drain(..off);
                            return Err(SendError::Stalled { sent: total });
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) => {
                        pending.drain(..off);
                        return Err(lost(total, &e));
                    }
                }
            }
            pending.clear();
        }
        self.log.info(&format!(
            "Sent {total} bytes; waiting for the printer to finish"
        ));
        if let Err(e) = stream.shutdown(Shutdown::Write) {
            return Err(lost(total, &e));
        }
        self.drain(stream, total)
    }

    /// After EOF: read the back-channel until the printer closes the connection.
    fn drain(&mut self, stream: &mut TcpStream, total: u64) -> Result<u64, SendError> {
        let mut back: Vec<u8> = Vec::new();
        let mut discarded = 0usize;
        let mut buf = vec![0u8; 4096];
        let mut quiet = Instant::now();
        let mut last_queue: Option<u64> = None;
        let drain_timeout = self.cfg.backend.drain_timeout();
        let stall_timeout = self.cfg.backend.stall_timeout();
        let result = loop {
            if self.cancelled() {
                break Err(SendError::Cancelled { sent: total });
            }
            match stream.read(&mut buf) {
                Ok(0) => break Ok(total),
                Ok(n) => {
                    let room = MAX_BACK_CHANNEL.saturating_sub(back.len());
                    back.extend_from_slice(&buf[..n.min(room)]);
                    discarded += n.saturating_sub(room);
                    quiet = Instant::now();
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    let blocking = self.poll_if_due();
                    let printing = self
                        .last_status
                        .as_ref()
                        .is_some_and(|s| s.printer == Some(PrinterState::Printing));
                    // Bytes still in our send queue were never received by the printer.
                    let queue = tmc3500_common::net::send_queue(stream);
                    if let (Some(q), Some(prev)) = (queue, last_queue) {
                        if q < prev {
                            quiet = Instant::now();
                        }
                    }
                    last_queue = queue;
                    if blocking == Some(true) || printing {
                        quiet = Instant::now();
                    } else if blocking == Some(false)
                        && queue == Some(0)
                        && quiet.elapsed() >= drain_timeout
                    {
                        self.log.warning(
                            "Printer did not close the connection; all data was acknowledged and the status is OK, assuming the job was received",
                        );
                        break Ok(total);
                    } else if quiet.elapsed() >= stall_timeout {
                        break Err(SendError::Stalled { sent: total });
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => {
                    break Err(SendError::Lost {
                        sent: total,
                        err: e.to_string(),
                    })
                }
            }
        };
        if !back.is_empty() {
            self.log_back_channel(&back, discarded);
        }
        result
    }

    fn log_back_channel(&self, back: &[u8], discarded: usize) {
        let hex: String = back
            .iter()
            .take(256)
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        self.log.debug(&format!(
            "back-channel: {} bytes ({discarded} discarded): {hex}",
            back.len()
        ));
        match epson::parse_reply(back) {
            EpsonReply::Parsed(_) => {}
            EpsonReply::Unrecognised { len } => self.log.debug(&format!(
                "back-channel: Epson reply format not known yet ({len} bytes ignored)"
            )),
            EpsonReply::TooLarge { len } => self
                .log
                .debug(&format!("back-channel: reply too large ({len} bytes)")),
        }
    }

    /// Final status update after the job: report the current state, or clear stale errors.
    fn final_status(&mut self) {
        if self.poll_status().is_none() {
            self.reporter.clear_blocking(&self.log);
        }
    }

    fn hold(&mut self, kind: EventKind, key: &str, msg: &str) -> i32 {
        self.log.error(msg);
        if kind != EventKind::JobHeld {
            self.alert(kind, key, msg);
        }
        self.alert(
            EventKind::JobHeld,
            &format!("job-held:{}", self.job.job_id),
            msg,
        );
        CUPS_BACKEND_HOLD
    }

    /// "Job 42" or "Job 42 (copy 2 of 3)".
    fn job_label(&self) -> String {
        if self.copies.1 > 1 {
            format!(
                "Job {} (copy {} of {})",
                self.job.job_id, self.copies.0, self.copies.1
            )
        } else {
            format!("Job {}", self.job.job_id)
        }
    }

    fn open_input(&mut self) -> Result<Box<dyn Read>, String> {
        match &self.job.file {
            Some(p) => std::fs::File::open(p)
                .map(|f| Box::new(f) as Box<dyn Read>)
                .map_err(|e| format!("cannot open job file {}: {e}", p.display())),
            None => Ok(self
                .stdin
                .take()
                .unwrap_or_else(|| Box::new(std::io::empty()))),
        }
    }

    fn run_job(&mut self) -> i32 {
        let state_dir = self.cfg.alerts.state_dir.clone();
        if let Some(m) = markers::read_marker(&state_dir, &self.job.job_id) {
            let msg = format!(
                "Job {} was already partially sent to the printer in an earlier attempt ({}); held instead of printing it again. Check the printer output; if labels are missing release it with `tmc3500ctl release-held-jobs --job <id>` (which clears the marker), otherwise cancel it.",
                self.job.job_id,
                m.trim()
            );
            return self.hold(EventKind::JobHeld, "", &msg);
        }
        self.log.info(&format!(
            "Printing job {} to {}{}",
            self.job.job_id,
            self.uri.redacted(),
            if self.job.copies > 1 {
                format!(" ({} copies)", self.job.copies)
            } else {
                String::new()
            }
        ));
        let n = self.job.copies.max(1);
        for copy in 1..=n {
            self.copies = (copy, n);
            let input = match self.open_input() {
                Ok(i) => i,
                Err(e) if self.prior == 0 => {
                    self.log.error(&e);
                    return CUPS_BACKEND_FAILED;
                }
                Err(e) => {
                    self.mark_partial(0, &e);
                    let msg = format!(
                        "{}: {e}; earlier copies were printed. Held and NOT reprinted automatically.",
                        self.job_label()
                    );
                    return self.hold(EventKind::JobHeld, "", &msg);
                }
            };
            match self.deliver(input) {
                Ok(total) => {
                    self.prior += total;
                    if n > 1 {
                        self.log
                            .info(&format!("{} sent ({total} bytes)", self.job_label()));
                    }
                }
                Err(code) => return code,
            }
        }
        self.log.info(&format!("Job sent ({} bytes)", self.prior));
        if let Err(e) = markers::remove_marker(&state_dir, &self.job.job_id) {
            self.log
                .warning(&format!("cannot remove partial-send marker: {e}"));
        }
        self.final_status();
        CUPS_BACKEND_OK
    }

    /// Deliver one copy. `Err` carries the exit code.
    fn deliver(&mut self, mut input: Box<dyn Read>) -> Result<u64, i32> {
        let start = Instant::now();
        let mut pending: Vec<u8> = Vec::new();
        loop {
            if !self.wait_until_ready() {
                return Err(self.cancelled_exit(0));
            }
            let mut stream = match self.connect(start) {
                Ok(s) => s,
                Err(ConnectError::Cancelled) => return Err(self.cancelled_exit(0)),
                Err(ConnectError::Unreachable(e)) => {
                    let secs = self.cfg.backend.unreachable_hold_after().as_secs_f64();
                    if self.prior > 0 {
                        self.mark_partial(0, "printer unreachable between copies");
                    }
                    let msg = format!(
                        "Printer {} unreachable for {secs:.1} s ({e}); {} held. Release it with `tmc3500ctl release-held-jobs` once the printer is back.",
                        self.uri.authority(),
                        self.job_label()
                    );
                    return Err(self.hold(EventKind::Unreachable, "unreachable", &msg));
                }
            };
            let _ = self.alerter.clear("unreachable");
            self.log
                .info(&format!("Connected to {}", self.uri.authority()));
            let res = self.send(&mut stream, input.as_mut(), &mut pending);
            match res {
                Ok(total) => return Ok(total),
                Err(SendError::Cancelled { sent }) => {
                    let _ = stream.shutdown(Shutdown::Both);
                    return Err(self.cancelled_exit(sent));
                }
                Err(SendError::Input { sent, err: e }) if sent + self.prior > 0 => {
                    let _ = stream.shutdown(Shutdown::Both);
                    self.mark_partial(sent, &format!("job data unreadable: {e}"));
                    let msg = format!(
                        "Cannot read job data after sending {sent} bytes ({e}); the printer may have printed part of the job. {} held and NOT reprinted automatically.",
                        self.job_label()
                    );
                    return Err(self.hold(EventKind::JobHeld, "", &msg));
                }
                Err(SendError::Input { err: e, .. }) => {
                    let _ = stream.shutdown(Shutdown::Both);
                    self.log.error(&format!("cannot read job data: {e}"));
                    self.alert(
                        EventKind::JobFailed,
                        &format!("job-failed:{}", self.job.job_id),
                        &format!("job {} failed: cannot read job data: {e}", self.job.job_id),
                    );
                    return Err(CUPS_BACKEND_FAILED);
                }
                Err(SendError::Lost { sent: 0, err }) => {
                    // Nothing was written: the printer cannot have printed anything. Reconnect.
                    self.log.warning(&format!(
                        "Connection closed by the printer before any data was sent ({err}); reconnecting"
                    ));
                    if self.sleep(self.cfg.backend.retry_initial()) {
                        return Err(self.cancelled_exit(0));
                    }
                    if start.elapsed() >= self.cfg.backend.unreachable_hold_after() {
                        if self.prior > 0 {
                            self.mark_partial(0, "printer keeps closing the connection");
                        }
                        let msg = format!(
                            "Printer {} keeps closing the connection ({err}); {} held",
                            self.uri.authority(),
                            self.job_label()
                        );
                        return Err(self.hold(EventKind::Unreachable, "unreachable", &msg));
                    }
                }
                Err(SendError::Lost { sent, err }) => {
                    self.mark_partial(sent, &format!("connection lost: {err}"));
                    let msg = format!(
                        "Connection to the printer lost after sending {sent} bytes ({err}); the printer may have printed part of the job. {} held and NOT reprinted automatically: check the printer output, then release it with `tmc3500ctl release-held-jobs` or cancel it.",
                        self.job_label()
                    );
                    return Err(self.hold(EventKind::JobHeld, "", &msg));
                }
                Err(SendError::Stalled { sent }) => {
                    let _ = stream.shutdown(Shutdown::Both);
                    if sent + self.prior > 0 {
                        self.mark_partial(sent, "printer stalled");
                    }
                    let msg = format!(
                        "Printer stopped accepting data after {sent} bytes without reporting an error. {} held and NOT reprinted automatically: check the printer, then release it with `tmc3500ctl release-held-jobs` or cancel it.",
                        self.job_label()
                    );
                    return Err(self.hold(EventKind::JobHeld, "", &msg));
                }
            }
        }
    }

    /// Record that `sent` bytes of this job reached the printer (see [`markers`]).
    fn mark_partial(&self, sent: u64, why: &str) {
        let done = if self.copies.0 > 1 {
            format!(
                ", copies 1..{} of {} completed",
                self.copies.0 - 1,
                self.copies.1
            )
        } else {
            String::new()
        };
        let text = format!(
            "job {} partially sent ({} bytes{done}) at {}: {why}\n",
            self.job.job_id,
            sent + self.prior,
            tmc3500_common::time::rfc3339(tmc3500_common::time::unix_now())
        );
        if let Err(e) = markers::write_marker(&self.cfg.alerts.state_dir, &self.job.job_id, &text) {
            self.log.error(&format!(
                "cannot write partial-send marker in {}: {e}; a restarted job will NOT be detected as a duplicate",
                self.cfg.alerts.state_dir.display()
            ));
        }
    }

    fn cancelled_exit(&mut self, sent: u64) -> i32 {
        if sent + self.prior > 0 {
            let msg = format!(
                "{} stopped (SIGTERM) after {} bytes were sent: the printer may have printed part of it. If CUPS restarts this job it will be held, not reprinted.",
                self.job_label(),
                sent + self.prior
            );
            self.log.error(&msg);
            self.mark_partial(sent, "stopped by SIGTERM");
            self.alert(
                EventKind::JobFailed,
                &format!("job-failed:{}", self.job.job_id),
                &msg,
            );
        }
        self.log.info(&format!(
            "Job {} cancelled (SIGTERM) after {sent} bytes sent",
            self.job.job_id
        ));
        self.reporter
            .set_own(&self.log, "connecting-to-device", false);
        CUPS_BACKEND_CANCEL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn args() {
        assert_eq!(parse_args(&os(&["tmc3500"])), Ok(None));
        assert_eq!(parse_args(&[]), Ok(None));
        assert!(parse_args(&os(&["x", "1", "2"])).is_err());
        let j = parse_args(&os(&["uri", "42", "alice", "t\nitle", "x", "o=1"]))
            .unwrap()
            .unwrap();
        assert_eq!(j.job_id, "42");
        assert_eq!(j.title, "t itle");
        assert_eq!(j.copies, 1);
        assert_eq!(j.file, None);
        let j = parse_args(&os(&["uri", "1", "u", "t", "3", "", "/f"]))
            .unwrap()
            .unwrap();
        assert_eq!(j.copies, 3);
        assert_eq!(j.file.as_deref(), Some(std::path::Path::new("/f")));
    }

    /// Yields `data`, then fails after `fail_after` bytes.
    struct FailingReader {
        data: Vec<u8>,
        pos: usize,
        fail_after: usize,
    }

    impl Read for FailingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.fail_after {
                return Err(std::io::Error::other("filter crashed"));
            }
            let n = buf
                .len()
                .min(self.fail_after - self.pos)
                .min(self.data.len() - self.pos);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    #[test]
    fn input_error_after_partial_send_holds_and_marks() {
        let fake = tmc3500_fakeprinter::FakePrinter::start_local().unwrap();
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("state")).unwrap();
        let cfg = d.path().join("c.toml");
        std::fs::write(
            &cfg,
            format!(
                "[snmp]\ntimeout_ms = 200\n[backend]\nstatus_interval_ms = 50\nio_timeout_ms = 300\n\
                 [alerts]\ndir = \"{0}/none\"\nstate_dir = \"{0}/state\"\n",
                d.path().display()
            ),
        )
        .unwrap();
        let env = Env {
            device_uri: Some(format!(
                "tmc3500://127.0.0.1:{}?snmp-port={}",
                fake.job_port(),
                fake.snmp_port()
            )),
            config_path: cfg,
        };
        let input = FailingReader {
            data: vec![0u8; 1 << 20],
            pos: 0,
            fail_after: 300_000,
        };
        let code = run_with(
            os(&["tmc3500", "42", "u", "t", "1", ""]),
            Arc::new(AtomicBool::new(false)),
            &env,
            Box::new(input),
        );
        assert_eq!(code, CUPS_BACKEND_HOLD);
        assert!(d.path().join("state/partial-job-42").exists());
    }

    #[test]
    fn copies_only_with_a_file_argument() {
        let c = |v: &[&str]| parse_args(&os(v)).unwrap().unwrap().copies;
        assert_eq!(c(&["u", "1", "u", "t", "3", ""]), 1);
        assert_eq!(c(&["u", "1", "u", "t", "3", "", "/f"]), 3);
        assert_eq!(c(&["u", "1", "u", "t", "0", "", "/f"]), 1);
        assert_eq!(c(&["u", "1", "u", "t", "x", "", "/f"]), 1);
        assert_eq!(c(&["u", "1", "u", "t", "100000", "", "/f"]), 9999);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let ms = Duration::from_millis;
        let d: Vec<_> = (0..6).map(|n| backoff(ms(20), ms(100), n)).collect();
        assert_eq!(d, vec![ms(20), ms(40), ms(80), ms(100), ms(100), ms(100)]);
        assert_eq!(backoff(ms(1), Duration::MAX, 1000), ms(1 << 20));
    }
}
