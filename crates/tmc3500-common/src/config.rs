//! `/etc/tmc3500/tmc3500.toml` (path overridable with `TMC3500_CONFIG`). A missing file means
//! built-in defaults; an unreadable, oversized or invalid file is an error (never silently ignored).
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tmc3500_core::status::printer::Thresholds;
use tmc3500_core::status::snmp::Version;

pub const DEFAULT_PATH: &str = "/etc/tmc3500/tmc3500.toml";
pub const ENV_PATH: &str = "TMC3500_CONFIG";
/// Largest config file accepted.
pub const MAX_SIZE: u64 = 1024 * 1024;
/// Longest timeout or interval accepted in the file (one week).
const MAX_MS: u64 = 7 * 24 * 3600 * 1000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub printer: PrinterCfg,
    pub snmp: SnmpCfg,
    pub backend: BackendCfg,
    pub alerts: AlertCfg,
    pub monitor: MonitorCfg,
    #[serde(rename = "thresholds")]
    pub thresholds_cfg: ThresholdCfg,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PrinterCfg {
    /// Printer address for the monitor and `tmc3500ctl` (the backend uses the CUPS device URI).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub port: u16,
    /// CUPS queue name.
    pub queue: String,
}

impl Default for PrinterCfg {
    fn default() -> Self {
        PrinterCfg {
            host: None,
            port: 9100,
            queue: "TM-C3500".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnmpVersion {
    #[serde(rename = "1")]
    V1,
    #[serde(rename = "2c")]
    V2c,
}

impl SnmpVersion {
    pub fn wire(self) -> Version {
        match self {
            SnmpVersion::V1 => Version::V1,
            SnmpVersion::V2c => Version::V2c,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SnmpCfg {
    pub enabled: bool,
    pub port: u16,
    pub community: String,
    pub version: SnmpVersion,
    pub timeout_ms: u64,
    pub retries: u32,
}

impl Default for SnmpCfg {
    fn default() -> Self {
        SnmpCfg {
            enabled: true,
            port: 161,
            community: "public".into(),
            version: SnmpVersion::V2c,
            timeout_ms: 1500,
            retries: 2,
        }
    }
}

impl SnmpCfg {
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BackendCfg {
    /// TCP connect timeout per attempt.
    pub connect_timeout_ms: u64,
    /// Timeout of a single socket read/write.
    pub io_timeout_ms: u64,
    /// How often status is polled while waiting or stalled.
    pub status_interval_ms: u64,
    /// First retry delay when the printer is unreachable (doubles up to `retry_max_ms`).
    pub retry_initial_ms: u64,
    pub retry_max_ms: u64,
    /// Hold the job after the printer has been unreachable this long before the job started.
    pub unreachable_hold_after_ms: u64,
    /// Hold the job when the printer accepts no data for this long while reporting no error.
    pub stall_timeout_ms: u64,
    /// After sending everything, wait this long for the printer to close the connection.
    pub drain_timeout_ms: u64,
}

impl Default for BackendCfg {
    fn default() -> Self {
        BackendCfg {
            connect_timeout_ms: 10_000,
            io_timeout_ms: 10_000,
            status_interval_ms: 5_000,
            retry_initial_ms: 2_000,
            retry_max_ms: 60_000,
            unreachable_hold_after_ms: 600_000,
            stall_timeout_ms: 300_000,
            drain_timeout_ms: 60_000,
        }
    }
}

impl BackendCfg {
    pub fn connect_timeout(&self) -> Duration {
        Duration::from_millis(self.connect_timeout_ms)
    }
    pub fn io_timeout(&self) -> Duration {
        Duration::from_millis(self.io_timeout_ms)
    }
    pub fn status_interval(&self) -> Duration {
        Duration::from_millis(self.status_interval_ms)
    }
    pub fn retry_initial(&self) -> Duration {
        Duration::from_millis(self.retry_initial_ms)
    }
    pub fn retry_max(&self) -> Duration {
        Duration::from_millis(self.retry_max_ms)
    }
    pub fn unreachable_hold_after(&self) -> Duration {
        Duration::from_millis(self.unreachable_hold_after_ms)
    }
    pub fn stall_timeout(&self) -> Duration {
        Duration::from_millis(self.stall_timeout_ms)
    }
    pub fn drain_timeout(&self) -> Duration {
        Duration::from_millis(self.drain_timeout_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AlertCfg {
    pub dir: PathBuf,
    pub hook_timeout_ms: u64,
    /// The same event key is not sent again within this interval (0 = never suppress).
    pub repeat_interval_s: u64,
    pub state_dir: PathBuf,
}

impl Default for AlertCfg {
    fn default() -> Self {
        AlertCfg {
            dir: "/etc/tmc3500/alert.d".into(),
            hook_timeout_ms: 10_000,
            repeat_interval_s: 3600,
            state_dir: "/var/lib/tmc3500".into(),
        }
    }
}

impl AlertCfg {
    pub fn hook_timeout(&self) -> Duration {
        Duration::from_millis(self.hook_timeout_ms)
    }
    pub fn repeat_interval(&self) -> Duration {
        Duration::from_secs(self.repeat_interval_s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MonitorCfg {
    pub interval_ms: u64,
    /// Prometheus textfile (node_exporter textfile collector). Unset = no metrics file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub textfile: Option<PathBuf>,
}

impl Default for MonitorCfg {
    fn default() -> Self {
        MonitorCfg {
            interval_ms: 30_000,
            textfile: Some("/var/lib/prometheus/node-exporter/tmc3500.prom".into()),
        }
    }
}

impl MonitorCfg {
    pub fn interval(&self) -> Duration {
        Duration::from_millis(self.interval_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ThresholdCfg {
    pub ink_low_percent: u8,
    pub waste_near_full_percent: u8,
}

impl Default for ThresholdCfg {
    fn default() -> Self {
        let t = Thresholds::default();
        ThresholdCfg {
            ink_low_percent: t.ink_low_percent,
            waste_near_full_percent: t.waste_near_full_percent,
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(PathBuf, std::io::Error),
    TooLarge,
    Parse(String),
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(p, e) => write!(f, "cannot read {}: {e}", p.display()),
            Self::TooLarge => write!(f, "config file larger than {MAX_SIZE} bytes"),
            Self::Parse(e) => write!(f, "invalid config: {e}"),
            Self::Invalid(e) => write!(f, "invalid config value: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Config path: `$TMC3500_CONFIG` if set and non-empty, else [`DEFAULT_PATH`].
    pub fn path_from_env() -> PathBuf {
        match std::env::var_os(ENV_PATH) {
            Some(p) if !p.is_empty() => PathBuf::from(p),
            _ => PathBuf::from(DEFAULT_PATH),
        }
    }

    /// Load from [`Config::path_from_env`].
    pub fn load_default() -> Result<(Config, PathBuf), ConfigError> {
        let p = Self::path_from_env();
        Self::load(&p).map(|c| (c, p))
    }

    /// Load `path`; a missing file gives the defaults.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        use std::io::Read;
        let f = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(ConfigError::Io(path.into(), e)),
        };
        let mut buf = Vec::new();
        f.take(MAX_SIZE + 1)
            .read_to_end(&mut buf)
            .map_err(|e| ConfigError::Io(path.into(), e))?;
        if buf.len() as u64 > MAX_SIZE {
            return Err(ConfigError::TooLarge);
        }
        let text = String::from_utf8(buf).map_err(|_| ConfigError::Parse("not UTF-8".into()))?;
        let c: Config = toml::from_str(&text).map_err(|e| ConfigError::Parse(e.to_string()))?;
        c.validate()?;
        Ok(c)
    }

    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string_pretty(self).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    pub fn thresholds(&self) -> Thresholds {
        Thresholds {
            ink_low_percent: self.thresholds_cfg.ink_low_percent,
            waste_near_full_percent: self.thresholds_cfg.waste_near_full_percent,
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let bad = |s: String| Err(ConfigError::Invalid(s));
        if self.printer.port == 0 {
            return bad("printer.port must be 1..=65535".into());
        }
        if self.snmp.port == 0 {
            return bad("snmp.port must be 1..=65535".into());
        }
        if self.snmp.community.len() > 255 {
            return bad("snmp.community longer than 255 bytes".into());
        }
        if self.snmp.retries > 10 {
            return bad("snmp.retries must be 0..=10".into());
        }
        if self.printer.queue.is_empty()
            || !self
                .printer
                .queue
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'.')
        {
            return bad("printer.queue must be a CUPS queue name (A-Z a-z 0-9 - _ .)".into());
        }
        let b = &self.backend;
        for (name, v) in [
            ("snmp.timeout_ms", self.snmp.timeout_ms),
            ("backend.connect_timeout_ms", b.connect_timeout_ms),
            ("backend.io_timeout_ms", b.io_timeout_ms),
            ("backend.status_interval_ms", b.status_interval_ms),
            ("backend.retry_initial_ms", b.retry_initial_ms),
            ("backend.retry_max_ms", b.retry_max_ms),
            (
                "backend.unreachable_hold_after_ms",
                b.unreachable_hold_after_ms,
            ),
            ("backend.stall_timeout_ms", b.stall_timeout_ms),
            ("backend.drain_timeout_ms", b.drain_timeout_ms),
            ("alerts.hook_timeout_ms", self.alerts.hook_timeout_ms),
            ("monitor.interval_ms", self.monitor.interval_ms),
        ] {
            if v == 0 || v > MAX_MS {
                return bad(format!("{name} must be 1..={MAX_MS}"));
            }
        }
        if self.alerts.repeat_interval_s > MAX_MS / 1000 {
            return bad("alerts.repeat_interval_s too large".into());
        }
        let t = &self.thresholds_cfg;
        if t.ink_low_percent > 100 || t.waste_near_full_percent > 100 {
            return bad("thresholds must be 0..=100".into());
        }
        Ok(())
    }
}
