//! Typed printer status from standard MIB values (Host Resources MIB, Printer MIB), mapped to CUPS
//! `printer-state-reasons` keywords and alert conditions. Pure; never panics.
use super::epson::EpsonStatus;
use super::snmp::{Oid, Value, VarBind};
use std::collections::BTreeSet;

/// OIDs (dotted strings) of the values read.
pub mod oids {
    pub const SYS_DESCR: &str = "1.3.6.1.2.1.1.1.0";
    pub const SYS_UPTIME: &str = "1.3.6.1.2.1.1.3.0";
    pub const HR_DEVICE_STATUS: &str = "1.3.6.1.2.1.25.3.2.1.5.1";
    pub const HR_PRINTER_STATUS: &str = "1.3.6.1.2.1.25.3.5.1.1.1";
    pub const HR_PRINTER_DETECTED_ERROR_STATE: &str = "1.3.6.1.2.1.25.3.5.1.2.1";
    /// prtMarkerSuppliesEntry (column and index are appended: `.<col>.1.<x>`).
    pub const PRT_MARKER_SUPPLIES_ENTRY: &str = "1.3.6.1.2.1.43.11.1.1";
    /// prtMarkerLifeCount for marker 1.
    pub const PRT_MARKER_LIFE_COUNT: &str = "1.3.6.1.2.1.43.10.2.1.4.1.1";
    /// prtMarkerPowerOnCount for marker 1.
    pub const PRT_MARKER_POWER_ON_COUNT: &str = "1.3.6.1.2.1.43.10.2.1.5.1.1";
}

/// prtMarkerSuppliesClass column.
pub const SUPPLY_CLASS: u32 = 4;
/// prtMarkerSuppliesDescription column.
pub const SUPPLY_DESCRIPTION: u32 = 6;
/// prtMarkerSuppliesMaxCapacity column.
pub const SUPPLY_MAX_CAPACITY: u32 = 8;
/// prtMarkerSuppliesLevel column.
pub const SUPPLY_LEVEL: u32 = 9;
/// Supply indices queried (1..=MAX_SUPPLIES). The TM-C3500 has 5 (4 inks + maintenance box).
pub const MAX_SUPPLIES: u32 = 8;
/// Longest supply description kept.
pub const MAX_DESCRIPTION: usize = 128;

fn oid(s: &str) -> Oid {
    // Only called with the constants above, which are valid (checked by a unit test).
    Oid::parse(s).unwrap_or_else(|_| Oid::zero_dot_zero())
}

/// OID of a prtMarkerSupplies column for supply `index` (hrDeviceIndex 1).
pub fn supply_oid(column: u32, index: u32) -> Oid {
    oid(oids::PRT_MARKER_SUPPLIES_ENTRY)
        .child(column)
        .child(1)
        .child(index)
}

/// Every OID needed for [`PrinterStatus::from_varbinds`].
pub fn status_oids() -> Vec<Oid> {
    let mut v = vec![
        oid(oids::HR_DEVICE_STATUS),
        oid(oids::HR_PRINTER_STATUS),
        oid(oids::HR_PRINTER_DETECTED_ERROR_STATE),
    ];
    for x in 1..=MAX_SUPPLIES {
        for col in [
            SUPPLY_CLASS,
            SUPPLY_DESCRIPTION,
            SUPPLY_MAX_CAPACITY,
            SUPPLY_LEVEL,
        ] {
            v.push(supply_oid(col, x));
        }
    }
    v
}

/// OIDs for counters (`tmc3500ctl counters`, monitor metrics).
pub fn counter_oids() -> Vec<Oid> {
    vec![
        oid(oids::SYS_DESCR),
        oid(oids::SYS_UPTIME),
        oid(oids::PRT_MARKER_LIFE_COUNT),
        oid(oids::PRT_MARKER_POWER_ON_COUNT),
    ]
}

/// hrDeviceStatus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStatus {
    Unknown,
    Running,
    Warning,
    Testing,
    Down,
    Invalid(i64),
}

impl DeviceStatus {
    pub fn from_i64(v: i64) -> Self {
        match v {
            1 => Self::Unknown,
            2 => Self::Running,
            3 => Self::Warning,
            4 => Self::Testing,
            5 => Self::Down,
            v => Self::Invalid(v),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Running => "running",
            Self::Warning => "warning",
            Self::Testing => "testing",
            Self::Down => "down",
            Self::Invalid(_) => "invalid",
        }
    }

    /// The MIB value (Invalid keeps the raw value).
    pub fn value(self) -> i64 {
        match self {
            Self::Unknown => 1,
            Self::Running => 2,
            Self::Warning => 3,
            Self::Testing => 4,
            Self::Down => 5,
            Self::Invalid(v) => v,
        }
    }
}

/// hrPrinterStatus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrinterState {
    Other,
    Unknown,
    Idle,
    Printing,
    Warmup,
    Invalid(i64),
}

impl PrinterState {
    pub fn from_i64(v: i64) -> Self {
        match v {
            1 => Self::Other,
            2 => Self::Unknown,
            3 => Self::Idle,
            4 => Self::Printing,
            5 => Self::Warmup,
            v => Self::Invalid(v),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Other => "other",
            Self::Unknown => "unknown",
            Self::Idle => "idle",
            Self::Printing => "printing",
            Self::Warmup => "warmup",
            Self::Invalid(_) => "invalid",
        }
    }

    pub fn value(self) -> i64 {
        match self {
            Self::Other => 1,
            Self::Unknown => 2,
            Self::Idle => 3,
            Self::Printing => 4,
            Self::Warmup => 5,
            Self::Invalid(v) => v,
        }
    }
}

/// Bits of hrPrinterDetectedErrorState (bit 0 = most significant bit of the first octet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ErrorBit {
    LowPaper = 0,
    NoPaper,
    LowToner,
    NoToner,
    DoorOpen,
    Jammed,
    Offline,
    ServiceRequested,
    InputTrayMissing,
    OutputTrayMissing,
    MarkerSupplyMissing,
    OutputNearFull,
    OutputFull,
    InputTrayEmpty,
    OverduePreventMaint,
}

impl ErrorBit {
    pub const ALL: [ErrorBit; 15] = [
        Self::LowPaper,
        Self::NoPaper,
        Self::LowToner,
        Self::NoToner,
        Self::DoorOpen,
        Self::Jammed,
        Self::Offline,
        Self::ServiceRequested,
        Self::InputTrayMissing,
        Self::OutputTrayMissing,
        Self::MarkerSupplyMissing,
        Self::OutputNearFull,
        Self::OutputFull,
        Self::InputTrayEmpty,
        Self::OverduePreventMaint,
    ];

    fn mask(self) -> u16 {
        0x8000 >> (self as u16)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::LowPaper => "lowPaper",
            Self::NoPaper => "noPaper",
            Self::LowToner => "lowToner",
            Self::NoToner => "noToner",
            Self::DoorOpen => "doorOpen",
            Self::Jammed => "jammed",
            Self::Offline => "offline",
            Self::ServiceRequested => "serviceRequested",
            Self::InputTrayMissing => "inputTrayMissing",
            Self::OutputTrayMissing => "outputTrayMissing",
            Self::MarkerSupplyMissing => "markerSupplyMissing",
            Self::OutputNearFull => "outputNearFull",
            Self::OutputFull => "outputFull",
            Self::InputTrayEmpty => "inputTrayEmpty",
            Self::OverduePreventMaint => "overduePreventMaint",
        }
    }

    /// CUPS `printer-state-reasons` keyword for this bit.
    pub fn reason(self) -> &'static str {
        match self {
            Self::LowPaper => "media-low-report",
            Self::NoPaper | Self::InputTrayEmpty => "media-empty-error",
            Self::LowToner => "marker-supply-low-warning",
            Self::NoToner => "marker-supply-empty-error",
            Self::DoorOpen => "cover-open",
            Self::Jammed => "media-jam-error",
            Self::Offline => "offline-report",
            Self::ServiceRequested => "other-error",
            Self::InputTrayMissing => "input-tray-missing-error",
            Self::OutputTrayMissing => "output-tray-missing-error",
            Self::MarkerSupplyMissing => "marker-supply-missing-error",
            Self::OutputNearFull => "output-area-almost-full-warning",
            Self::OutputFull => "output-area-full-error",
            Self::OverduePreventMaint => "other-warning",
        }
    }
}

/// Decoded hrPrinterDetectedErrorState.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DetectedErrors(u16);

impl DetectedErrors {
    /// From the OCTET STRING value. Octets beyond the second (none defined) are ignored.
    pub fn from_octets(b: &[u8]) -> Self {
        let hi = b.first().copied().unwrap_or(0);
        let lo = b.get(1).copied().unwrap_or(0);
        let all = ErrorBit::ALL.iter().fold(0u16, |m, b| m | b.mask());
        DetectedErrors(u16::from_be_bytes([hi, lo]) & all)
    }

    pub fn contains(self, bit: ErrorBit) -> bool {
        self.0 & bit.mask() != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn bits(self) -> u16 {
        self.0
    }

    pub fn iter(self) -> impl Iterator<Item = ErrorBit> {
        ErrorBit::ALL.into_iter().filter(move |b| self.contains(*b))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InkColor {
    Black,
    Cyan,
    Magenta,
    Yellow,
}

impl InkColor {
    pub fn name(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::Cyan => "cyan",
            Self::Magenta => "magenta",
            Self::Yellow => "yellow",
        }
    }

    pub fn hex(self) -> &'static str {
        match self {
            Self::Black => "#000000",
            Self::Cyan => "#00FFFF",
            Self::Magenta => "#FF00FF",
            Self::Yellow => "#FFFF00",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplyKind {
    Ink(InkColor),
    MaintenanceBox,
    Other,
}

impl SupplyKind {
    /// Identify a supply by its prtMarkerSuppliesDescription (SJIC22P K/C/M/Y, SJMB3500).
    pub fn classify(description: &str) -> Self {
        let d = description.to_ascii_lowercase();
        let has = |s: &str| d.contains(s);
        if has("maint") || has("waste") || has("sjmb") {
            Self::MaintenanceBox
        } else if has("black") || has("(k)") || has("(bk)") {
            Self::Ink(InkColor::Black)
        } else if has("cyan") || has("(c)") {
            Self::Ink(InkColor::Cyan)
        } else if has("magenta") || has("(m)") {
            Self::Ink(InkColor::Magenta)
        } else if has("yellow") || has("(y)") {
            Self::Ink(InkColor::Yellow)
        } else {
            Self::Other
        }
    }
}

/// prtMarkerSuppliesClass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplyClass {
    Other,
    /// supplyThatIsConsumed (3): level = amount remaining.
    Consumed,
    /// receptacleThatIsFilled (4): level = amount already filled.
    Filled,
}

/// Low/empty thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    /// Ink is "low" at or below this percentage remaining.
    pub ink_low_percent: u8,
    /// The maintenance box is "near full" at or above this percentage used.
    pub waste_near_full_percent: u8,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            ink_low_percent: 10,
            waste_near_full_percent: 90,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelState {
    Ok,
    /// Ink low / maintenance box near full.
    Low,
    /// Ink empty / maintenance box full.
    Empty,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Supply {
    pub index: u32,
    pub description: String,
    pub kind: SupplyKind,
    pub class: Option<SupplyClass>,
    pub max_capacity: Option<i64>,
    pub level: Option<i64>,
}

impl Supply {
    /// Percentage of capacity remaining (for the maintenance box: free space), 0..=100.
    /// `None` when the printer reports "other" (-1), "unknown" (-2) or "some remaining" (-3).
    pub fn percent_remaining(&self) -> Option<u8> {
        let max = self.max_capacity.filter(|&m| m > 0)?;
        let lvl = self.level.filter(|&l| l >= 0)?;
        let pct = (i128::from(lvl.min(max)) * 100 / i128::from(max)) as u8;
        Some(if self.class == Some(SupplyClass::Filled) {
            100 - pct
        } else {
            pct
        })
    }

    pub fn level_state(&self, thr: &Thresholds) -> LevelState {
        let Some(p) = self.percent_remaining() else {
            return if self.level == Some(-3) {
                LevelState::Ok
            } else {
                LevelState::Unknown
            };
        };
        let low = match self.kind {
            SupplyKind::MaintenanceBox => 100u8.saturating_sub(thr.waste_near_full_percent),
            _ => thr.ink_low_percent,
        };
        if p == 0 {
            LevelState::Empty
        } else if p <= low {
            LevelState::Low
        } else {
            LevelState::Ok
        }
    }
}

/// Alert event kinds (spec section 5). `name()` is the event name passed to hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventKind {
    StateChange,
    InkLow,
    InkEmpty,
    MaintenanceBoxNearFull,
    MaintenanceBoxFull,
    MediaOut,
    MediaJam,
    CoverOpen,
    CutterError,
    Offline,
    PrinterError,
    JobHeld,
    JobFailed,
    Unreachable,
    Recovered,
}

impl EventKind {
    pub const ALL: &'static [EventKind] = &[
        Self::StateChange,
        Self::InkLow,
        Self::InkEmpty,
        Self::MaintenanceBoxNearFull,
        Self::MaintenanceBoxFull,
        Self::MediaOut,
        Self::MediaJam,
        Self::CoverOpen,
        Self::CutterError,
        Self::Offline,
        Self::PrinterError,
        Self::JobHeld,
        Self::JobFailed,
        Self::Unreachable,
        Self::Recovered,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::StateChange => "state-change",
            Self::InkLow => "ink-low",
            Self::InkEmpty => "ink-empty",
            Self::MaintenanceBoxNearFull => "maintenance-box-near-full",
            Self::MaintenanceBoxFull => "maintenance-box-full",
            Self::MediaOut => "media-out",
            Self::MediaJam => "media-jam",
            Self::CoverOpen => "cover-open",
            Self::CutterError => "cutter-error",
            Self::Offline => "offline",
            Self::PrinterError => "printer-error",
            Self::JobHeld => "job-held",
            Self::JobFailed => "job-failed",
            Self::Unreachable => "unreachable",
            Self::Recovered => "recovered",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|e| e.name() == s)
    }

    pub fn severity(self) -> Severity {
        match self {
            Self::StateChange | Self::Recovered => Severity::Info,
            Self::InkLow | Self::MaintenanceBoxNearFull | Self::Offline => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn name(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// An alertable condition currently present on the printer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    pub event: EventKind,
    /// Repeat-suppression key, e.g. `ink-low:cyan`.
    pub key: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrinterStatus {
    pub sys_descr: Option<String>,
    pub device: Option<DeviceStatus>,
    pub printer: Option<PrinterState>,
    pub errors: DetectedErrors,
    /// True if hrPrinterDetectedErrorState was present in the reply.
    pub errors_known: bool,
    pub supplies: Vec<Supply>,
    pub life_count: Option<u64>,
    pub power_on_count: Option<u64>,
    pub uptime_ticks: Option<u64>,
    pub epson: Option<EpsonStatus>,
}

fn printable(b: &[u8], max: usize) -> String {
    String::from_utf8_lossy(b)
        .chars()
        .filter(|c| *c != '\0')
        .take(max)
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Integer(i) => Some(*i),
        Value::Gauge32(u) | Value::Counter32(u) => Some(i64::from(*u)),
        _ => None,
    }
}

fn uint(v: &Value) -> Option<u64> {
    match v {
        Value::Counter64(u) => Some(*u),
        _ => v.as_i64().and_then(|i| u64::try_from(i).ok()),
    }
}

impl PrinterStatus {
    /// Build from the varbinds of a GetResponse. Unknown OIDs and mistyped values are ignored.
    pub fn from_varbinds(vbs: &[VarBind]) -> Self {
        let mut st = PrinterStatus::default();
        let supplies_root = oid(oids::PRT_MARKER_SUPPLIES_ENTRY);
        let mut rows: std::collections::BTreeMap<u32, Supply> = Default::default();
        for vb in vbs {
            let o = vb.oid.to_string();
            match o.as_str() {
                oids::SYS_DESCR => st.sys_descr = vb.value.as_bytes().map(|b| printable(b, 256)),
                oids::SYS_UPTIME => st.uptime_ticks = uint(&vb.value),
                oids::HR_DEVICE_STATUS => st.device = int(&vb.value).map(DeviceStatus::from_i64),
                oids::HR_PRINTER_STATUS => st.printer = int(&vb.value).map(PrinterState::from_i64),
                oids::HR_PRINTER_DETECTED_ERROR_STATE => {
                    if let Some(b) = vb.value.as_bytes() {
                        st.errors = DetectedErrors::from_octets(b);
                        st.errors_known = true;
                    }
                }
                oids::PRT_MARKER_LIFE_COUNT => st.life_count = uint(&vb.value),
                oids::PRT_MARKER_POWER_ON_COUNT => st.power_on_count = uint(&vb.value),
                _ => {
                    let s = vb.oid.as_slice();
                    let root = supplies_root.as_slice();
                    if vb.oid.starts_with(&supplies_root) && s.len() == root.len() + 3 {
                        let (col, dev, idx) = (s[root.len()], s[root.len() + 1], s[root.len() + 2]);
                        if dev == 1 && (1..=MAX_SUPPLIES).contains(&idx) {
                            Self::supply_value(&mut rows, col, idx, &vb.value);
                        }
                    }
                }
            }
        }
        st.supplies = rows
            .into_values()
            .filter(|s| !s.description.is_empty() || s.level.is_some())
            .collect();
        st
    }

    fn supply_value(
        rows: &mut std::collections::BTreeMap<u32, Supply>,
        col: u32,
        idx: u32,
        v: &Value,
    ) {
        let present = !matches!(
            v,
            Value::NoSuchObject | Value::NoSuchInstance | Value::EndOfMibView | Value::Null
        );
        if !present {
            return;
        }
        let row = rows.entry(idx).or_insert_with(|| Supply {
            index: idx,
            description: String::new(),
            kind: SupplyKind::Other,
            class: None,
            max_capacity: None,
            level: None,
        });
        match col {
            SUPPLY_DESCRIPTION => {
                if let Some(b) = v.as_bytes() {
                    row.description = printable(b, MAX_DESCRIPTION);
                    row.kind = SupplyKind::classify(&row.description);
                }
            }
            SUPPLY_CLASS => {
                row.class = int(v).map(|c| match c {
                    3 => SupplyClass::Consumed,
                    4 => SupplyClass::Filled,
                    _ => SupplyClass::Other,
                })
            }
            SUPPLY_MAX_CAPACITY => row.max_capacity = int(v),
            SUPPLY_LEVEL => row.level = int(v),
            _ => {}
        }
    }

    /// Add the details from an Epson status reply (see [`super::epson`]).
    pub fn merge_epson(&mut self, e: EpsonStatus) {
        self.epson = Some(e);
    }

    /// True if the reply carried the core status values (hrDeviceStatus, hrPrinterStatus and
    /// hrPrinterDetectedErrorState). Without them the printer state is unknown, not "OK".
    pub fn has_core_status(&self) -> bool {
        self.device.is_some() && self.printer.is_some() && self.errors_known
    }

    pub fn device_status_or_unknown(&self) -> DeviceStatus {
        self.device.unwrap_or(DeviceStatus::Unknown)
    }

    /// CUPS `printer-state-reasons` keywords (sorted, without `none`).
    pub fn reasons(&self, thr: &Thresholds) -> BTreeSet<&'static str> {
        let mut r: BTreeSet<&'static str> = self.errors.iter().map(ErrorBit::reason).collect();
        for s in &self.supplies {
            let kw = match (s.kind, s.level_state(thr)) {
                (SupplyKind::Ink(_), LevelState::Low) => "marker-supply-low-warning",
                (SupplyKind::Ink(_), LevelState::Empty) => "marker-supply-empty-error",
                (SupplyKind::MaintenanceBox, LevelState::Low) => "marker-waste-almost-full-warning",
                (SupplyKind::MaintenanceBox, LevelState::Empty) => "marker-waste-full-error",
                _ => continue,
            };
            r.insert(kw);
        }
        if let Some(e) = &self.epson {
            r.extend(e.reasons.iter().copied());
            if e.cutter_error {
                r.insert("other-error");
            }
        }
        if self.device == Some(DeviceStatus::Down) && !r.iter().any(|k| is_blocking_reason(k)) {
            r.insert("other-error");
        }
        r
    }

    /// True if the printer cannot print now (an `-error` reason, cover open or offline).
    pub fn is_blocking(&self, thr: &Thresholds) -> bool {
        self.reasons(thr).iter().any(|k| is_blocking_reason(k))
    }

    /// Alertable conditions currently present, in a stable order.
    pub fn conditions(&self, thr: &Thresholds) -> Vec<Condition> {
        let mut c = Vec::new();
        let mut push = |event: EventKind, key: String, message: String| {
            if !c.iter().any(|x: &Condition| x.key == key) {
                c.push(Condition {
                    event,
                    key,
                    message,
                });
            }
        };
        if self.epson.as_ref().is_some_and(|e| e.cutter_error) {
            push(
                EventKind::CutterError,
                "cutter-error".into(),
                "Auto-cutter error".into(),
            );
        }
        let e = self.errors;
        if e.contains(ErrorBit::NoPaper) || e.contains(ErrorBit::InputTrayEmpty) {
            push(
                EventKind::MediaOut,
                "media-out".into(),
                "Out of media".into(),
            );
        }
        if e.contains(ErrorBit::Jammed) {
            push(EventKind::MediaJam, "media-jam".into(), "Media jam".into());
        }
        if e.contains(ErrorBit::DoorOpen) {
            push(
                EventKind::CoverOpen,
                "cover-open".into(),
                "Cover open".into(),
            );
        }
        if e.contains(ErrorBit::Offline) {
            push(
                EventKind::Offline,
                "offline".into(),
                "Printer offline".into(),
            );
        }
        let (mut ink_low, mut ink_empty) = (false, false);
        for s in &self.supplies {
            let pct = s
                .percent_remaining()
                .map_or(String::new(), |p| format!(" ({p}% left)"));
            match (s.kind, s.level_state(thr)) {
                (SupplyKind::Ink(col), LevelState::Low) => {
                    ink_low = true;
                    push(
                        EventKind::InkLow,
                        format!("ink-low:{}", col.name()),
                        format!("{} ink low{pct}", col.name()),
                    )
                }
                (SupplyKind::Ink(col), LevelState::Empty) => {
                    ink_empty = true;
                    push(
                        EventKind::InkEmpty,
                        format!("ink-empty:{}", col.name()),
                        format!("{} ink empty", col.name()),
                    )
                }
                (SupplyKind::MaintenanceBox, LevelState::Low) => push(
                    EventKind::MaintenanceBoxNearFull,
                    "maintenance-box-near-full".into(),
                    format!("Maintenance box near full{pct}"),
                ),
                (SupplyKind::MaintenanceBox, LevelState::Empty) => push(
                    EventKind::MaintenanceBoxFull,
                    "maintenance-box-full".into(),
                    "Maintenance box full".into(),
                ),
                _ => {}
            }
        }
        if e.contains(ErrorBit::LowToner) && !ink_low {
            push(
                EventKind::InkLow,
                "ink-low:unknown".into(),
                "Ink low".into(),
            );
        }
        if e.contains(ErrorBit::NoToner) && !ink_empty {
            push(
                EventKind::InkEmpty,
                "ink-empty:unknown".into(),
                "Ink empty".into(),
            );
        }
        let handled = [
            "media-empty-error",
            "media-jam-error",
            "cover-open",
            "offline-report",
            "marker-supply-low-warning",
            "marker-supply-empty-error",
            "marker-waste-almost-full-warning",
            "marker-waste-full-error",
            "media-low-report",
        ];
        let cutter = self.epson.as_ref().is_some_and(|e| e.cutter_error);
        for r in self.reasons(thr) {
            if handled.contains(&r) || (cutter && r == "other-error") {
                continue;
            }
            if r.ends_with("-error") || r.ends_with("-warning") {
                push(
                    EventKind::PrinterError,
                    format!("printer-error:{r}"),
                    format!("Printer reports {r}"),
                );
            }
        }
        c
    }

    /// Values for CUPS `ATTR:` lines (`marker-colors`, `marker-levels`, `marker-names`,
    /// `marker-types`), one string per attribute, without the `ATTR: ` prefix.
    /// Empty when no supplies are known.
    pub fn marker_attrs(&self) -> Vec<String> {
        if self.supplies.is_empty() {
            return Vec::new();
        }
        let join = |f: &dyn Fn(&Supply) -> String| {
            self.supplies.iter().map(f).collect::<Vec<_>>().join(",")
        };
        let colors = join(&|s| match s.kind {
            SupplyKind::Ink(c) => c.hex().to_string(),
            _ => "none".to_string(),
        });
        let levels = join(&|s| {
            s.percent_remaining()
                .map_or("-1".to_string(), |p| p.to_string())
        });
        let names = join(&|s| {
            let mut q = String::from("\"");
            let name = if s.description.is_empty() {
                format!("Supply {}", s.index)
            } else {
                s.description.clone()
            };
            for ch in name.chars() {
                if ch == '"' || ch == '\\' {
                    q.push('\\');
                }
                q.push(ch);
            }
            q.push('"');
            q
        });
        let types = join(&|s| {
            match s.kind {
                SupplyKind::Ink(_) => "ink-cartridge",
                SupplyKind::MaintenanceBox => "waste-ink",
                SupplyKind::Other => "other",
            }
            .to_string()
        });
        vec![
            format!("marker-colors={colors}"),
            format!("marker-levels={levels}"),
            format!("marker-names={names}"),
            format!("marker-types={types}"),
        ]
    }
}

/// Human message for a `printer-state-reasons` keyword.
pub fn reason_message(reason: &str) -> String {
    match reason {
        "media-empty-error" => "The printer is out of media".into(),
        "media-low-report" => "Media low".into(),
        "media-jam-error" => "Media jam".into(),
        "cover-open" => "The printer cover is open".into(),
        "offline-report" => "The printer is offline".into(),
        "marker-supply-low-warning" => "Ink low".into(),
        "marker-supply-empty-error" => "Ink cartridge empty".into(),
        "marker-supply-missing-error" => "Ink cartridge missing".into(),
        "marker-waste-almost-full-warning" => "Maintenance box almost full".into(),
        "marker-waste-full-error" => "Maintenance box full".into(),
        "connecting-to-device" => "Connecting to the printer".into(),
        "other-error" => "The printer reports an error".into(),
        r => format!("Printer reports {r}"),
    }
}

/// Reasons that stop printing.
pub fn is_blocking_reason(k: &str) -> bool {
    k.ends_with("-error") || k == "cover-open" || k == "offline-report"
}

#[cfg(test)]
mod tests;
