//! Simulated TM-C3500 for integration tests.
//!
//! - TCP job port: accepts one connection at a time, reads the job until EOF, validates it with
//!   `tmc3500_core::decode::decode` and records it (accepted, rejected, dropped).
//! - UDP SNMP agent: answers Get/GetNext for the Host Resources / Printer MIB values the driver
//!   reads, from the scripted state.
//! - Fault injection through this in-process API, or text commands ([`FakePrinter::apply_command`],
//!   used by the binary on stdin): paper out (stops reading the socket until cleared), jam, cover
//!   open, offline, ink low/empty, maintenance box, drop the connection after N bytes, slow reads,
//!   refuse connections, garbage or silent SNMP.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;
use tmc3500_core::status::printer::{
    oids, supply_oid, InkColor, SUPPLY_CLASS, SUPPLY_DESCRIPTION, SUPPLY_LEVEL, SUPPLY_MAX_CAPACITY,
};
use tmc3500_core::status::snmp::{
    self, Message, Oid, PduType, Value, VarBind, Version, MAX_MESSAGE,
};

/// Largest job kept in memory.
pub const DEFAULT_MAX_JOB: usize = 256 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnmpMode {
    Normal,
    /// Reply with malformed or mismatched packets.
    Garbage,
    /// Never reply.
    Silent,
    /// Answer, but without any status values (every OID is noSuchInstance).
    NoStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    Accepted,
    Rejected(String),
    /// Connection closed by the fake (drop fault) or reset by the client before EOF.
    Dropped,
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRecord {
    pub data: Vec<u8>,
    pub outcome: JobOutcome,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub bind: std::net::IpAddr,
    /// 0 = any free port.
    pub job_port: u16,
    pub snmp_port: u16,
    pub community: String,
    pub max_job: usize,
    /// Write accepted jobs to `<dir>/job-<n>.prn`.
    pub store_dir: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            bind: [127, 0, 0, 1].into(),
            job_port: 0,
            snmp_port: 0,
            community: "public".into(),
            max_job: DEFAULT_MAX_JOB,
            store_dir: None,
        }
    }
}

#[derive(Debug, Clone)]
struct State {
    paper_out: bool,
    jam: bool,
    cover_open: bool,
    offline: bool,
    /// K, C, M, Y remaining percent.
    inks: [i64; 4],
    /// Maintenance box free space percent.
    waste_remaining: i64,
    paper_out_after: Option<u64>,
    drop_after: Option<u64>,
    slow_read: Option<(usize, Duration)>,
    refuse: bool,
    /// Stop reading the job socket without reporting any error (SNMP shows "printing").
    paused: bool,
    snmp_mode: SnmpMode,
    reply: Vec<u8>,
    jobs: Vec<JobRecord>,
    connections: u64,
    snmp_requests: u64,
    receiving: bool,
    stalled: bool,
    bytes_current: u64,
    garbage_counter: u32,
}

impl Default for State {
    fn default() -> Self {
        State {
            paper_out: false,
            jam: false,
            cover_open: false,
            offline: false,
            inks: [80, 80, 80, 80],
            waste_remaining: 80,
            paper_out_after: None,
            drop_after: None,
            slow_read: None,
            refuse: false,
            paused: false,
            snmp_mode: SnmpMode::Normal,
            reply: Vec::new(),
            jobs: Vec::new(),
            connections: 0,
            snmp_requests: 0,
            receiving: false,
            stalled: false,
            bytes_current: 0,
            garbage_counter: 0,
        }
    }
}

impl State {
    fn blocked(&self) -> bool {
        self.paper_out || self.jam || self.cover_open || self.offline
    }
}

struct Shared {
    state: Mutex<State>,
    stop: AtomicBool,
    opts: Options,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panicking test thread must not wedge the fake: ignore poisoning.
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

pub struct FakePrinter {
    shared: Arc<Shared>,
    job_addr: SocketAddr,
    snmp_addr: SocketAddr,
    threads: Vec<JoinHandle<()>>,
}

fn color_index(c: InkColor) -> usize {
    match c {
        InkColor::Black => 0,
        InkColor::Cyan => 1,
        InkColor::Magenta => 2,
        InkColor::Yellow => 3,
    }
}

fn parse_color(s: &str) -> Option<InkColor> {
    Some(match s {
        "black" | "k" | "K" => InkColor::Black,
        "cyan" | "c" | "C" => InkColor::Cyan,
        "magenta" | "m" | "M" => InkColor::Magenta,
        "yellow" | "y" | "Y" => InkColor::Yellow,
        _ => return None,
    })
}

fn parse_on_off(s: Option<&str>) -> Result<bool, String> {
    match s {
        Some("on" | "1" | "true") => Ok(true),
        Some("off" | "0" | "false") => Ok(false),
        other => Err(format!("expected on/off, got {other:?}")),
    }
}

fn parse_opt_u64(s: Option<&str>) -> Result<Option<u64>, String> {
    match s {
        Some("off") => Ok(None),
        Some(n) => n.parse().map(Some).map_err(|_| format!("bad number {n:?}")),
        None => Err("missing value".into()),
    }
}

fn parse_percent(s: Option<&str>) -> Result<i64, String> {
    match s.and_then(|s| s.parse::<i64>().ok()) {
        Some(v) if (0..=100).contains(&v) => Ok(v),
        _ => Err(format!("expected 0..=100, got {s:?}")),
    }
}

impl FakePrinter {
    /// Start on 127.0.0.1 with free ports.
    pub fn start_local() -> std::io::Result<FakePrinter> {
        FakePrinter::start(Options::default())
    }

    pub fn start(opts: Options) -> std::io::Result<FakePrinter> {
        let listener = TcpListener::bind((opts.bind, opts.job_port))?;
        let job_addr = listener.local_addr()?;
        let udp = UdpSocket::bind((opts.bind, opts.snmp_port))?;
        let snmp_addr = udp.local_addr()?;
        udp.set_read_timeout(Some(Duration::from_millis(50)))?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            stop: AtomicBool::new(false),
            opts,
        });
        let s1 = shared.clone();
        let s2 = shared.clone();
        let threads = vec![
            std::thread::spawn(move || job_loop(s1, listener, job_addr)),
            std::thread::spawn(move || snmp_loop(s2, udp)),
        ];
        Ok(FakePrinter {
            shared,
            job_addr,
            snmp_addr,
            threads,
        })
    }

    pub fn job_port(&self) -> u16 {
        self.job_addr.port()
    }

    pub fn snmp_port(&self) -> u16 {
        self.snmp_addr.port()
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.shared.lock())
    }

    pub fn set_paper_out(&self, on: bool) {
        self.with(|s| s.paper_out = on)
    }
    pub fn set_jam(&self, on: bool) {
        self.with(|s| s.jam = on)
    }
    pub fn set_cover_open(&self, on: bool) {
        self.with(|s| s.cover_open = on)
    }
    pub fn set_offline(&self, on: bool) {
        self.with(|s| s.offline = on)
    }
    /// Ink remaining in percent (0 = empty).
    pub fn set_ink(&self, c: InkColor, percent: i64) {
        self.with(|s| s.inks[color_index(c)] = percent)
    }
    /// Maintenance box free space in percent (0 = full).
    pub fn set_waste_remaining(&self, percent: i64) {
        self.with(|s| s.waste_remaining = percent)
    }
    /// Run out of paper after reading this many bytes of the current/next job (one-shot).
    pub fn set_paper_out_after(&self, n: Option<u64>) {
        self.with(|s| s.paper_out_after = n)
    }
    /// Close job connections after reading this many bytes (until cleared).
    pub fn set_drop_after(&self, n: Option<u64>) {
        self.with(|s| s.drop_after = n)
    }
    /// Read at most `chunk` bytes, then sleep `delay`.
    pub fn set_slow_read(&self, v: Option<(usize, Duration)>) {
        self.with(|s| s.slow_read = v.map(|(c, d)| (c.max(1), d)))
    }
    /// Close the listening socket (connections are refused) until cleared.
    /// Stop reading job data without any error state (a busy or hung printer).
    pub fn set_paused(&self, on: bool) {
        self.with(|s| s.paused = on)
    }
    pub fn set_refuse(&self, on: bool) {
        self.with(|s| s.refuse = on)
    }
    pub fn set_snmp_mode(&self, m: SnmpMode) {
        self.with(|s| s.snmp_mode = m)
    }
    /// Bytes sent back on the job connection after a job is accepted.
    pub fn set_reply(&self, b: Vec<u8>) {
        self.with(|s| s.reply = b)
    }

    pub fn jobs(&self) -> Vec<JobRecord> {
        self.with(|s| s.jobs.clone())
    }
    pub fn accepted_jobs(&self) -> Vec<Vec<u8>> {
        self.with(|s| {
            s.jobs
                .iter()
                .filter(|j| j.outcome == JobOutcome::Accepted)
                .map(|j| j.data.clone())
                .collect()
        })
    }
    pub fn connection_count(&self) -> u64 {
        self.with(|s| s.connections)
    }
    pub fn snmp_request_count(&self) -> u64 {
        self.with(|s| s.snmp_requests)
    }
    /// True while a job connection is open but not being read (paper out, jam, ...).
    pub fn is_stalled(&self) -> bool {
        self.with(|s| s.stalled)
    }
    /// True while a job connection is open.
    pub fn is_receiving(&self) -> bool {
        self.with(|s| s.receiving)
    }
    /// Bytes read so far on the current (or last) job connection.
    pub fn bytes_received_current(&self) -> u64 {
        self.with(|s| s.bytes_current)
    }

    /// Apply a text command (used by the binary's stdin control). Returns a reply line.
    pub fn apply_command(&self, line: &str) -> Result<String, String> {
        let mut it = line.split_whitespace();
        let cmd = it.next().ok_or("empty command")?;
        let a1 = it.next();
        let a2 = it.next();
        match cmd {
            "paper-out" => self.set_paper_out(parse_on_off(a1)?),
            "jam" => self.set_jam(parse_on_off(a1)?),
            "cover" => self.set_cover_open(parse_on_off(a1)?),
            "offline" => self.set_offline(parse_on_off(a1)?),
            "refuse" => self.set_refuse(parse_on_off(a1)?),
            "pause" => self.set_paused(parse_on_off(a1)?),
            "ink" => {
                let c = a1.and_then(parse_color).ok_or("unknown ink colour")?;
                self.set_ink(c, parse_percent(a2)?)
            }
            "waste" => self.set_waste_remaining(parse_percent(a1)?),
            "drop-after" => self.set_drop_after(parse_opt_u64(a1)?),
            "paper-out-after" => self.set_paper_out_after(parse_opt_u64(a1)?),
            "slow-read" => match (a1, a2) {
                (Some("off"), _) => self.set_slow_read(None),
                (Some(c), Some(ms)) => {
                    let c = c.parse::<usize>().map_err(|_| "bad chunk size")?;
                    let ms = ms.parse::<u64>().map_err(|_| "bad delay")?;
                    self.set_slow_read(Some((c, Duration::from_millis(ms))))
                }
                _ => return Err("usage: slow-read <chunk> <ms> | off".into()),
            },
            "snmp" => self.set_snmp_mode(match a1 {
                Some("normal") => SnmpMode::Normal,
                Some("garbage") => SnmpMode::Garbage,
                Some("silent") => SnmpMode::Silent,
                Some("no-status") => SnmpMode::NoStatus,
                _ => return Err("usage: snmp normal|garbage|silent".into()),
            }),
            "jobs" => {
                return Ok(self
                    .jobs()
                    .iter()
                    .enumerate()
                    .map(|(i, j)| format!("job {i}: {} bytes, {:?}", j.data.len(), j.outcome))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "status" => return Ok(self.with(|s| format!("{s:?}"))),
            other => return Err(format!("unknown command {other:?}")),
        }
        Ok("ok".into())
    }
}

impl Drop for FakePrinter {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

fn job_loop(sh: Arc<Shared>, listener: TcpListener, addr: SocketAddr) {
    let mut listener = Some(listener);
    let mut n_accepted = 0usize;
    while !sh.stop.load(Ordering::SeqCst) {
        if sh.lock().refuse {
            listener = None;
            std::thread::sleep(POLL);
            continue;
        }
        if listener.is_none() {
            match TcpListener::bind(addr) {
                Ok(l) => listener = Some(l),
                Err(_) => {
                    std::thread::sleep(POLL);
                    continue;
                }
            }
        }
        let Some(l) = &listener else { continue };
        if l.set_nonblocking(true).is_err() {
            std::thread::sleep(POLL);
            continue;
        }
        match l.accept() {
            Ok((stream, _)) => {
                handle_job(&sh, stream, &mut n_accepted);
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(POLL),
            Err(_) => std::thread::sleep(POLL),
        }
    }
}

fn record(sh: &Shared, data: Vec<u8>, outcome: JobOutcome) {
    let mut s = sh.lock();
    s.receiving = false;
    s.stalled = false;
    s.jobs.push(JobRecord { data, outcome });
}

fn handle_job(sh: &Shared, mut stream: TcpStream, n_accepted: &mut usize) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));
    {
        let mut s = sh.lock();
        s.connections += 1;
        s.receiving = true;
        s.bytes_current = 0;
    }
    let mut data: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if sh.stop.load(Ordering::SeqCst) {
            record(sh, data, JobOutcome::Dropped);
            return;
        }
        let (chunk, delay) = {
            let mut s = sh.lock();
            if s.blocked() || s.paused {
                s.stalled = true;
                drop(s);
                std::thread::sleep(POLL);
                continue;
            }
            s.stalled = false;
            if let Some(n) = s.drop_after {
                if data.len() as u64 >= n {
                    drop(s);
                    // Closing with unread data makes the kernel send RST, like a printer reset.
                    drop(stream);
                    record(sh, data, JobOutcome::Dropped);
                    return;
                }
            }
            let mut chunk = buf.len();
            let mut delay = None;
            if let Some((c, d)) = s.slow_read {
                chunk = chunk.min(c);
                delay = Some(d);
            }
            let have = data.len() as u64;
            for limit in [s.drop_after, s.paper_out_after].into_iter().flatten() {
                if limit > have {
                    chunk = chunk.min((limit - have).min(usize::MAX as u64) as usize);
                }
            }
            (chunk.max(1), delay)
        };
        match stream.read(&mut buf[..chunk]) {
            Ok(0) => break,
            Ok(n) => {
                data.extend_from_slice(&buf[..n]);
                let mut s = sh.lock();
                s.bytes_current = data.len() as u64;
                if let Some(p) = s.paper_out_after {
                    if data.len() as u64 >= p {
                        s.paper_out = true;
                        s.paper_out_after = None;
                    }
                }
                drop(s);
                if data.len() > sh.opts.max_job {
                    drop(stream);
                    record(sh, Vec::new(), JobOutcome::TooLarge);
                    return;
                }
                if let Some(d) = delay {
                    std::thread::sleep(d);
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => {
                record(sh, data, JobOutcome::Dropped);
                return;
            }
        }
    }
    let outcome = match tmc3500_core::decode::decode(&data) {
        Ok(_) => JobOutcome::Accepted,
        Err(e) => JobOutcome::Rejected(format!("offset {}: {}", e.offset, e.reason)),
    };
    if outcome == JobOutcome::Accepted {
        if let Some(dir) = &sh.opts.store_dir {
            let _ = std::fs::write(dir.join(format!("job-{n_accepted}.prn")), &data);
        }
        *n_accepted += 1;
        let reply = sh.lock().reply.clone();
        if !reply.is_empty() {
            let _ = stream.write_all(&reply);
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
    record(sh, data, outcome);
}

fn device_values(s: &State, accepted: u32) -> BTreeMap<Oid, Value> {
    let mut m = BTreeMap::new();
    let mut put = |o: Oid, v: Value| {
        m.insert(o, v);
    };
    let oid = |s: &str| Oid::parse(s).unwrap_or_else(|_| Oid::zero_dot_zero());
    let mut hi = 0u8;
    let lo = 0u8;
    if s.paper_out {
        hi |= 0x40;
    }
    if s.cover_open {
        hi |= 0x08;
    }
    if s.jam {
        hi |= 0x04;
    }
    if s.offline {
        hi |= 0x02;
    }
    if s.inks.iter().any(|&l| l > 0 && l <= 10) {
        hi |= 0x20;
    }
    if s.inks.contains(&0) {
        hi |= 0x10;
    }
    let error = s.blocked() || s.inks.contains(&0) || s.waste_remaining == 0;
    let warning = hi != 0 || s.waste_remaining <= 10;
    let device = if error {
        5
    } else if warning {
        3
    } else {
        2
    };
    let printer = if error {
        1
    } else if s.receiving {
        4
    } else {
        3
    };
    put(
        oid(oids::SYS_DESCR),
        Value::OctetString(b"EPSON TM-C3500 (tmc3500-fakeprinter)".to_vec()),
    );
    put(oid(oids::SYS_UPTIME), Value::TimeTicks(12345));
    put(oid(oids::HR_DEVICE_STATUS), Value::Integer(device));
    put(oid(oids::HR_PRINTER_STATUS), Value::Integer(printer));
    put(
        oid(oids::HR_PRINTER_DETECTED_ERROR_STATE),
        Value::OctetString(vec![hi, lo]),
    );
    let names = [
        "Black Ink SJIC22P(K)",
        "Cyan Ink SJIC22P(C)",
        "Magenta Ink SJIC22P(M)",
        "Yellow Ink SJIC22P(Y)",
        "Maintenance Box SJMB3500",
    ];
    for (i, name) in names.iter().enumerate() {
        let x = i as u32 + 1;
        let level = s.inks.get(i).copied().unwrap_or(s.waste_remaining);
        put(supply_oid(SUPPLY_CLASS, x), Value::Integer(3));
        put(
            supply_oid(SUPPLY_DESCRIPTION, x),
            Value::OctetString(name.as_bytes().to_vec()),
        );
        put(supply_oid(SUPPLY_MAX_CAPACITY, x), Value::Integer(100));
        put(supply_oid(SUPPLY_LEVEL, x), Value::Integer(level));
    }
    put(oid(oids::PRT_MARKER_LIFE_COUNT), Value::Counter32(accepted));
    put(oid(oids::PRT_MARKER_POWER_ON_COUNT), Value::Counter32(0));
    m
}

fn answer(req: &Message, table: &BTreeMap<Oid, Value>) -> Message {
    let mut resp = Message {
        pdu_type: PduType::GetResponse,
        error_status: 0,
        error_index: 0,
        ..req.clone()
    };
    for (i, vb) in req.varbinds.iter().enumerate() {
        let found = match req.pdu_type {
            PduType::GetNextRequest => table
                .range((
                    std::ops::Bound::Excluded(vb.oid.clone()),
                    std::ops::Bound::Unbounded,
                ))
                .next()
                .map(|(o, v)| VarBind::new(o.clone(), v.clone())),
            _ => table
                .get(&vb.oid)
                .map(|v| VarBind::new(vb.oid.clone(), v.clone())),
        };
        match (found, req.version) {
            (Some(f), _) => resp.varbinds[i] = f,
            (None, Version::V1) => {
                resp.error_status = 2; // noSuchName
                resp.error_index = i as i64 + 1;
                resp.varbinds = req.varbinds.clone();
                return resp;
            }
            (None, Version::V2c) => {
                resp.varbinds[i].value = if req.pdu_type == PduType::GetNextRequest {
                    Value::EndOfMibView
                } else {
                    Value::NoSuchInstance
                }
            }
        }
    }
    resp
}

fn garbage(n: u32, req: &Message) -> Vec<u8> {
    match n % 4 {
        0 => b"\x30\x03garbage-not-snmp".to_vec(),
        1 => {
            // valid but truncated
            let mut r = snmp::encode(&answer(req, &BTreeMap::new())).unwrap_or_default();
            r.truncate(r.len() / 2);
            r
        }
        2 => {
            // pseudo-random bytes
            let mut x = 0x9e37_79b9u32 ^ n;
            (0..64)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x as u8
                })
                .collect()
        }
        _ => {
            // indefinite length / nesting bomb
            let mut v = vec![0x30, 0x80];
            v.extend(std::iter::repeat_n(0x30, 100));
            v
        }
    }
}

fn snmp_loop(sh: Arc<Shared>, sock: UdpSocket) {
    let mut buf = vec![0u8; MAX_MESSAGE];
    while !sh.stop.load(Ordering::SeqCst) {
        let (n, from) = match sock.recv_from(&mut buf) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Ok(req) = snmp::decode(&buf[..n]) else {
            continue;
        };
        let (mode, table, gc) = {
            let mut s = sh.lock();
            s.snmp_requests += 1;
            s.garbage_counter = s.garbage_counter.wrapping_add(1);
            let accepted = s
                .jobs
                .iter()
                .filter(|j| j.outcome == JobOutcome::Accepted)
                .count() as u32;
            (s.snmp_mode, device_values(&s, accepted), s.garbage_counter)
        };
        if req.community != sh.opts.community.as_bytes() || req.pdu_type == PduType::GetResponse {
            continue;
        }
        let out = match mode {
            SnmpMode::Silent => continue,
            SnmpMode::NoStatus => match snmp::encode(&answer(&req, &BTreeMap::new())) {
                Ok(b) => b,
                Err(_) => continue,
            },
            SnmpMode::Garbage => garbage(gc, &req),
            SnmpMode::Normal => match snmp::encode(&answer(&req, &table)) {
                Ok(b) => b,
                Err(_) => continue,
            },
        };
        let _ = sock.send_to(&out, from);
    }
}
