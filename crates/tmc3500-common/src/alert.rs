//! Alert dispatch: each event is serialised as JSON and passed on stdin to every executable in the
//! alert directory, with a timeout per hook. Repeats of the same event key are suppressed for a
//! configurable interval; the last-sent times live in a small JSON state file.
//!
//! Alerts are best effort and never fatal: problems are returned as warnings for the caller to log.
use crate::time::{rfc3339, unix_now};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tmc3500_core::status::printer::{Condition, EventKind};

/// Most hooks run per event.
pub const MAX_HOOKS: usize = 64;
/// Largest state file read.
const MAX_STATE: u64 = 1024 * 1024;
/// Most keys kept in the state file.
const MAX_KEYS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Event name, e.g. `ink-low` (see `EventKind::name`).
    pub event: String,
    /// Repeat-suppression key, e.g. `ink-low:cyan`.
    pub key: String,
    /// `info`, `warning` or `error`.
    pub severity: String,
    /// Printer address.
    pub printer: String,
    pub message: String,
    /// RFC 3339 UTC.
    pub time: String,
    /// Unix seconds.
    pub timestamp: u64,
    /// Emitting program: `backend`, `monitor`, ...
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub job_id: Option<String>,
    /// Current CUPS printer-state-reasons, if known.
    #[serde(default)]
    pub reasons: Vec<String>,
}

impl Event {
    pub fn new(kind: EventKind, key: &str, printer: &str, message: &str, source: &str) -> Event {
        let now = unix_now();
        Event {
            event: kind.name().to_string(),
            key: key.to_string(),
            severity: kind.severity().name().to_string(),
            printer: printer.to_string(),
            message: message.to_string(),
            time: rfc3339(now),
            timestamp: now,
            source: source.to_string(),
            job_id: None,
            reasons: Vec::new(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookOutcome {
    Ok,
    /// Non-zero exit status (-1 if killed by a signal).
    Failed(i32),
    TimedOut,
    SpawnError(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRun {
    pub hook: PathBuf,
    pub outcome: HookOutcome,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delivery {
    /// True if the event was suppressed as a repeat (no hook ran).
    pub suppressed: bool,
    pub hooks: Vec<HookRun>,
    /// Non-fatal problems (missing dir, skipped hooks, state file errors).
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Alerter {
    pub dir: PathBuf,
    pub hook_timeout: Duration,
    pub repeat_interval: Duration,
    pub state_file: Option<PathBuf>,
    /// In-process record of sent keys: suppression keeps working when the state file cannot be
    /// read or written (shared between clones).
    memory: Arc<Mutex<HashMap<String, Instant>>>,
    /// False after a failed state-file write.
    save_ok: Arc<AtomicBool>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    sent: BTreeMap<String, u64>,
}

impl Alerter {
    pub fn new(
        dir: PathBuf,
        hook_timeout: Duration,
        repeat_interval: Duration,
        state_file: Option<PathBuf>,
    ) -> Self {
        Alerter {
            dir,
            hook_timeout,
            repeat_interval,
            state_file,
            memory: Arc::default(),
            save_ok: Arc::new(AtomicBool::new(true)),
        }
    }

    fn mem(&self) -> std::sync::MutexGuard<'_, HashMap<String, Instant>> {
        self.memory.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// From the `[alerts]` config section; the state file is `<state_dir>/<state_name>`.
    pub fn from_config(cfg: &crate::config::AlertCfg, state_name: &str) -> Self {
        Alerter::new(
            cfg.dir.clone(),
            cfg.hook_timeout(),
            cfg.repeat_interval(),
            Some(cfg.state_dir.join(state_name)),
        )
    }

    fn load_state(&self, warnings: &mut Vec<String>) -> State {
        let Some(p) = &self.state_file else {
            return State::default();
        };
        let mut buf = Vec::new();
        match std::fs::File::open(p) {
            Ok(f) => {
                if let Err(e) = f.take(MAX_STATE).read_to_end(&mut buf) {
                    warnings.push(format!("alert state {}: {e}", p.display()));
                    return State::default();
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return State::default(),
            Err(e) => {
                warnings.push(format!("alert state {}: {e}", p.display()));
                return State::default();
            }
        }
        serde_json::from_slice(&buf).unwrap_or_else(|e| {
            warnings.push(format!(
                "alert state {} is corrupt, ignored: {e}",
                p.display()
            ));
            State::default()
        })
    }

    fn save_state(&self, st: &State, warnings: &mut Vec<String>) {
        let Some(p) = &self.state_file else { return };
        match write_atomic_mode(
            p,
            serde_json::to_string(st).unwrap_or_default().as_bytes(),
            0o660,
        ) {
            Ok(()) => self.save_ok.store(true, Ordering::SeqCst),
            Err(e) => {
                self.save_ok.store(false, Ordering::SeqCst);
                warnings.push(format!("cannot write alert state {}: {e}", p.display()));
            }
        }
    }

    /// Forget `key`, so its next occurrence is sent immediately (call when a condition clears).
    pub fn clear(&self, key: &str) -> Vec<String> {
        self.mem().remove(key);
        let mut w = Vec::new();
        let mut st = self.load_state(&mut w);
        if st.sent.remove(key).is_some() {
            self.save_state(&st, &mut w);
        }
        w
    }

    /// Send `ev` to every hook unless the same key was sent within the repeat interval.
    pub fn emit(&self, ev: &Event) -> Delivery {
        let mut d = Delivery::default();
        let mut st = self.load_state(&mut d.warnings);
        // The state file decides while it works; otherwise fall back to the in-process record.
        let file_usable = self.state_file.is_some()
            && d.warnings.is_empty()
            && self.save_ok.load(Ordering::SeqCst);
        let now = unix_now();
        if !self.repeat_interval.is_zero() {
            let file_recent = st.sent.get(&ev.key).is_some_and(|&last| {
                now.saturating_sub(last) < self.repeat_interval.as_secs().max(1)
            });
            let mem_recent = self
                .mem()
                .get(&ev.key)
                .is_some_and(|t| t.elapsed() < self.repeat_interval);
            if file_recent || (!file_usable && mem_recent) {
                d.suppressed = true;
                return d;
            }
        }
        self.mem().insert(ev.key.clone(), Instant::now());
        st.sent.insert(ev.key.clone(), now);
        if st.sent.len() > MAX_KEYS {
            // drop the oldest entries
            let mut v: Vec<_> = st.sent.iter().map(|(k, t)| (*t, k.clone())).collect();
            v.sort();
            for (_, k) in v.iter().take(st.sent.len() - MAX_KEYS) {
                st.sent.remove(k);
            }
        }
        self.save_state(&st, &mut d.warnings);
        let json = ev.to_json();
        for hook in self.hooks(&mut d.warnings) {
            let outcome = run_hook(&hook, ev, &json, self.hook_timeout);
            d.hooks.push(HookRun { hook, outcome });
        }
        d
    }

    /// Executable hooks in name order. The directory and each hook must be owned by root or the
    /// running user and not be group/world-writable; symlinks, hidden files and editor/dpkg
    /// leftovers are skipped.
    pub fn hooks(&self, warnings: &mut Vec<String>) -> Vec<PathBuf> {
        let euid = effective_uid();
        match std::fs::symlink_metadata(&self.dir) {
            Ok(md) => {
                if let Some(why) = untrusted(&md, euid) {
                    warnings.push(format!("alert dir {} ignored: {why}", self.dir.display()));
                    return Vec::new();
                }
            }
            Err(e) => {
                warnings.push(format!("alert dir {}: {e}", self.dir.display()));
                return Vec::new();
            }
        }
        let rd = match std::fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) => {
                warnings.push(format!("alert dir {}: {e}", self.dir.display()));
                return Vec::new();
            }
        };
        let mut v: Vec<PathBuf> = Vec::new();
        for entry in rd.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.')
                || name.ends_with('~')
                || name.contains(".dpkg-")
                || name.ends_with(".disabled")
            {
                continue;
            }
            let path = entry.path();
            let Ok(md) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if md.file_type().is_symlink() {
                warnings.push(format!("skipping symlinked alert hook {}", path.display()));
                continue;
            }
            if !md.is_file() || md.permissions().mode() & 0o111 == 0 {
                continue;
            }
            if let Some(why) = untrusted(&md, euid) {
                warnings.push(format!("skipping alert hook {}: {why}", path.display()));
                continue;
            }
            v.push(path);
        }
        v.sort();
        if v.len() > MAX_HOOKS {
            warnings.push(format!(
                "more than {MAX_HOOKS} alert hooks, extra ones skipped"
            ));
            v.truncate(MAX_HOOKS);
        }
        v
    }
}

/// Hooks and the hook directory must be owned by root or by the user running the program.
pub fn owner_allowed(uid: u32, euid: u32) -> bool {
    uid == 0 || uid == euid
}

/// The effective uid of this process (owner of `/proc/self`); `u32::MAX` if unknown, which
/// then only allows root-owned hooks.
pub fn effective_uid() -> u32 {
    std::fs::metadata("/proc/self").map_or(u32::MAX, |m| m.uid())
}

fn untrusted(md: &std::fs::Metadata, euid: u32) -> Option<String> {
    if md.permissions().mode() & 0o022 != 0 {
        return Some("group/world-writable".into());
    }
    if !owner_allowed(md.uid(), euid) {
        return Some(format!(
            "owned by uid {} (not root or uid {euid})",
            md.uid()
        ));
    }
    None
}

fn run_hook(path: &Path, ev: &Event, json: &str, timeout: Duration) -> HookOutcome {
    let mut child = match Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("TMC3500_EVENT", &ev.event)
        .env("TMC3500_EVENT_KEY", &ev.key)
        .env("TMC3500_SEVERITY", &ev.severity)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return HookOutcome::SpawnError(e.to_string()),
    };
    if let Some(mut stdin) = child.stdin.take() {
        // The JSON is far below the pipe buffer size; a hook that does not read is fine (EPIPE).
        let _ = stdin.write_all(json.as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return match status.code() {
                    Some(0) => HookOutcome::Ok,
                    Some(c) => HookOutcome::Failed(c),
                    None => HookOutcome::Failed(-1),
                }
            }
            Ok(None) => {}
            Err(e) => return HookOutcome::SpawnError(e.to_string()),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return HookOutcome::TimedOut;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Turns successive condition snapshots into alerts: every current condition is offered to the
/// alerter (repeat suppression decides), and the key of a condition that disappeared is cleared so
/// it alerts immediately if it comes back.
#[derive(Debug, Default, Clone)]
pub struct ConditionTracker {
    previous: Vec<String>,
}

impl ConditionTracker {
    /// Returns the deliveries that were not suppressed.
    pub fn observe(
        &mut self,
        alerter: &Alerter,
        conditions: &[Condition],
        printer: &str,
        source: &str,
        mut decorate: impl FnMut(&mut Event),
    ) -> Vec<(Event, Delivery)> {
        let mut out = Vec::new();
        for c in conditions {
            let mut ev = Event::new(c.event, &c.key, printer, &c.message, source);
            decorate(&mut ev);
            let d = alerter.emit(&ev);
            if !d.suppressed {
                out.push((ev, d));
            }
        }
        for k in &self.previous {
            if !conditions.iter().any(|c| &c.key == k) {
                alerter.clear(k);
            }
        }
        self.previous = conditions.iter().map(|c| c.key.clone()).collect();
        out
    }

    /// Keys seen in the last snapshot.
    pub fn current(&self) -> &[String] {
        &self.previous
    }
}

static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// An unpredictable suffix for temporary file names.
fn random_suffix() -> String {
    let mut b = [0u8; 8];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_ok();
    if !ok {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        b = (t ^ (u64::from(std::process::id()) << 32) ^ n.wrapping_mul(0x9e37_79b9_7f4a_7c15))
            .to_le_bytes();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Write `data` to `path` atomically: a new temp file with an unpredictable name is created with
/// `O_EXCL` (a planted symlink cannot redirect the write), then renamed over `path`. An existing
/// file keeps its mode (and owner, when permitted); a new one gets `0644`.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    write_atomic_mode(path, data, 0o644)
}

/// Like [`write_atomic`], with the mode used when `path` does not exist yet.
pub fn write_atomic_mode(path: &Path, data: &[u8], new_mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let existing = std::fs::symlink_metadata(path).ok().filter(|m| m.is_file());
    let mode = existing
        .as_ref()
        .map_or(new_mode, |m| m.permissions().mode() & 0o7777);
    let mut last_err = None;
    for _ in 0..8 {
        let tmp = dir.join(format!(".{name}.{}.tmp", random_suffix()));
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp);
        let mut f = match f {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                last_err = Some(e);
                continue;
            }
            Err(e) => return Err(e),
        };
        let res = (|| {
            f.write_all(data)?;
            if let Some(m) = &existing {
                // best effort: only root can give the file away
                let _ = std::os::unix::fs::fchown(&f, Some(m.uid()), Some(m.gid()));
            }
            f.set_permissions(std::fs::Permissions::from_mode(mode))?;
            f.sync_all()?;
            std::fs::rename(&tmp, path)
        })();
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        return res;
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("cannot create temp file")))
}
