//! `tmc3500-monitor`: polls the printer over SNMP, emits alerts on state changes (including
//! unreachable / recovered) and writes a Prometheus textfile.
#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fmt::Write as _;
use tmc3500_common::alert::{Alerter, ConditionTracker, Delivery, Event};
use tmc3500_core::status::printer::{EventKind, PrinterStatus, SupplyKind, Thresholds};

/// One poll result: the status, or why it could not be read.
pub type Snapshot = Result<PrinterStatus, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Prev {
    Unknown,
    Up(BTreeSet<String>),
    Down,
}

pub struct Monitor {
    printer: String,
    alerter: Alerter,
    thr: Thresholds,
    tracker: ConditionTracker,
    prev: Prev,
    /// Reasons when the printer was last reachable.
    last_reasons: Option<BTreeSet<String>>,
    /// Problems from the last step (alert warnings, hook failures) for the caller to log.
    pub warnings: Vec<String>,
}

impl Monitor {
    pub fn new(printer: String, alerter: Alerter, thr: Thresholds) -> Monitor {
        Monitor {
            printer,
            alerter,
            thr,
            tracker: ConditionTracker::default(),
            prev: Prev::Unknown,
            last_reasons: None,
            warnings: Vec::new(),
        }
    }

    fn note(&mut self, d: &Delivery, ev: &Event) {
        self.warnings.extend(d.warnings.iter().cloned());
        for h in &d.hooks {
            if h.outcome != tmc3500_common::alert::HookOutcome::Ok {
                self.warnings.push(format!(
                    "alert hook {} for {}: {:?}",
                    h.hook.display(),
                    ev.event,
                    h.outcome
                ));
            }
        }
    }

    /// Emit `kind` without repeat suppression (transitions are always reported).
    fn transition(&mut self, kind: EventKind, key: &str, msg: &str, reasons: &[String]) -> Event {
        self.warnings.extend(self.alerter.clear(key));
        let mut ev = Event::new(kind, key, &self.printer, msg, "monitor");
        ev.reasons = reasons.to_vec();
        let d = self.alerter.emit(&ev);
        self.note(&d, &ev);
        ev
    }

    /// Process one poll; returns the events that were sent (not suppressed).
    pub fn step(&mut self, snap: &Snapshot) -> Vec<Event> {
        self.warnings.clear();
        let mut out = Vec::new();
        match snap {
            Ok(st) => {
                let reasons: BTreeSet<String> = st
                    .reasons(&self.thr)
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                let rv: Vec<String> = reasons.iter().cloned().collect();
                if self.prev == Prev::Down {
                    out.push(self.transition(
                        EventKind::Recovered,
                        "recovered",
                        "Printer reachable again",
                        &rv,
                    ));
                    self.warnings.extend(self.alerter.clear("unreachable"));
                }
                if let Some(last) = &self.last_reasons {
                    if *last != reasons {
                        let msg = if rv.is_empty() {
                            "Printer state: ready".to_string()
                        } else {
                            format!("Printer state: {}", rv.join(", "))
                        };
                        let key = format!(
                            "state-change:{}",
                            if rv.is_empty() {
                                "none".into()
                            } else {
                                rv.join(",")
                            }
                        );
                        out.push(self.transition(EventKind::StateChange, &key, &msg, &rv));
                    }
                }
                let conditions = st.conditions(&self.thr);
                let printer = self.printer.clone();
                let sent =
                    self.tracker
                        .observe(&self.alerter, &conditions, &printer, "monitor", |e| {
                            e.reasons = rv.clone()
                        });
                for (ev, d) in sent {
                    self.note(&d, &ev);
                    out.push(ev);
                }
                self.last_reasons = Some(reasons.clone());
                self.prev = Prev::Up(reasons);
            }
            Err(e) => {
                let mut ev = Event::new(
                    EventKind::Unreachable,
                    "unreachable",
                    &self.printer,
                    &format!("Printer status unavailable: {e}"),
                    "monitor",
                );
                ev.reasons = self
                    .last_reasons
                    .as_ref()
                    .map(|r| r.iter().cloned().collect())
                    .unwrap_or_default();
                let d = self.alerter.emit(&ev);
                self.note(&d, &ev);
                if !d.suppressed {
                    out.push(ev);
                }
                self.prev = Prev::Down;
            }
        }
        out
    }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            c if c.is_control() => o.push(' '),
            c => o.push(c),
        }
    }
    o
}

/// Prometheus text exposition for one poll.
pub fn render_metrics(printer: &str, snap: &Snapshot, thr: &Thresholds, now: u64) -> String {
    let p = format!("printer=\"{}\"", esc(printer));
    let mut t = String::new();
    let mut metric = |name: &str, help: &str, kind: &str, rows: &[(String, String)]| {
        let _ = writeln!(t, "# HELP {name} {help}");
        let _ = writeln!(t, "# TYPE {name} {kind}");
        for (labels, value) in rows {
            let _ = writeln!(t, "{name}{{{labels}}} {value}");
        }
    };
    let up = snap.is_ok();
    metric(
        "tmc3500_up",
        "Whether the printer answered the last SNMP poll.",
        "gauge",
        &[(p.clone(), u8::from(up).to_string())],
    );
    metric(
        "tmc3500_last_poll_timestamp_seconds",
        "Unix time of the last poll.",
        "gauge",
        &[(p.clone(), now.to_string())],
    );
    let Ok(st) = snap else { return t };
    if let Some(d) = st.device {
        metric(
            "tmc3500_device_status",
            "hrDeviceStatus (1 unknown, 2 running, 3 warning, 4 testing, 5 down).",
            "gauge",
            &[(p.clone(), d.value().to_string())],
        );
    }
    if let Some(s) = st.printer {
        metric(
            "tmc3500_printer_status",
            "hrPrinterStatus (1 other, 2 unknown, 3 idle, 4 printing, 5 warmup).",
            "gauge",
            &[(p.clone(), s.value().to_string())],
        );
    }
    let reasons = st.reasons(thr);
    metric(
        "tmc3500_blocking",
        "1 if the printer reports an error that stops printing.",
        "gauge",
        &[(p.clone(), u8::from(st.is_blocking(thr)).to_string())],
    );
    let rows: Vec<_> = reasons
        .iter()
        .map(|r| (format!("{p},reason=\"{}\"", esc(r)), "1".to_string()))
        .collect();
    metric(
        "tmc3500_state_reason",
        "Current CUPS printer-state-reasons keywords.",
        "gauge",
        &rows,
    );
    let rows: Vec<_> = st
        .supplies
        .iter()
        .filter_map(|s| {
            let marker = match s.kind {
                SupplyKind::Ink(c) => c.name(),
                SupplyKind::MaintenanceBox => "maintenance-box",
                SupplyKind::Other => "other",
            };
            s.percent_remaining().map(|v| {
                (
                    format!(
                        "{p},index=\"{}\",marker=\"{marker}\",description=\"{}\"",
                        s.index,
                        esc(&s.description)
                    ),
                    v.to_string(),
                )
            })
        })
        .collect();
    metric(
        "tmc3500_marker_level_percent",
        "Supply remaining in percent (maintenance box: free space).",
        "gauge",
        &rows,
    );
    if let Some(c) = st.life_count {
        metric(
            "tmc3500_marker_life_count",
            "prtMarkerLifeCount.",
            "counter",
            &[(p.clone(), c.to_string())],
        );
    }
    t
}
