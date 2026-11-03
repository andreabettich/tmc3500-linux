//! Tracks the `printer-state-reasons` this backend has reported and prints `STATE:` / `ATTR:`
//! lines only for changes.
use crate::log::Log;
use std::collections::BTreeSet;
use tmc3500_core::status::printer::{is_blocking_reason, reason_message};

#[derive(Debug, Default)]
pub struct StateReporter {
    printer: BTreeSet<String>,
    own: BTreeSet<String>,
    reported: BTreeSet<String>,
    attrs: Vec<String>,
}

/// Human message for a newly reported reason.
pub fn message(reason: &str) -> String {
    reason_message(reason)
}

impl StateReporter {
    /// Reasons currently reported to CUPS.
    pub fn current(&self) -> Vec<String> {
        self.reported.iter().cloned().collect()
    }

    pub fn set_printer_reasons(&mut self, log: &Log, reasons: &BTreeSet<&'static str>) {
        self.printer = reasons.iter().map(|s| s.to_string()).collect();
        self.flush(log);
    }

    /// Reasons owned by the backend itself (e.g. `connecting-to-device`).
    pub fn set_own(&mut self, log: &Log, reason: &str, on: bool) {
        if on {
            self.own.insert(reason.to_string());
        } else {
            self.own.remove(reason);
        }
        self.flush(log);
    }

    /// Remove error reasons that can no longer be confirmed (status unavailable).
    pub fn clear_blocking(&mut self, log: &Log) {
        self.printer.retain(|r| !is_blocking_reason(r));
        self.flush(log);
    }

    fn flush(&mut self, log: &Log) {
        let want: BTreeSet<String> = self.printer.union(&self.own).cloned().collect();
        let removed: Vec<_> = self.reported.difference(&want).cloned().collect();
        let added: Vec<_> = want.difference(&self.reported).cloned().collect();
        if !removed.is_empty() {
            log.state(&format!("-{}", removed.join(",")));
        }
        if !added.is_empty() {
            log.state(&format!("+{}", added.join(",")));
            for r in &added {
                if is_blocking_reason(r) {
                    log.warning(&message(r));
                } else {
                    log.info(&message(r));
                }
            }
        }
        self.reported = want;
    }

    /// Print `ATTR:` lines when the marker attributes changed.
    pub fn attrs(&mut self, log: &Log, attrs: Vec<String>) {
        if attrs != self.attrs {
            for a in &attrs {
                log.attr(a);
            }
            self.attrs = attrs;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn lines(b: &Buf) -> Vec<String> {
        String::from_utf8(std::mem::take(&mut *b.0.lock().unwrap()))
            .unwrap()
            .lines()
            .map(String::from)
            .collect()
    }

    #[test]
    fn reports_only_changes() {
        let b = Buf::default();
        let log = Log::to_writer(Box::new(b.clone()));
        let mut r = StateReporter::default();
        r.set_printer_reasons(&log, &["media-empty-error", "cover-open"].into());
        assert_eq!(
            lines(&b),
            vec![
                "STATE: +cover-open,media-empty-error",
                "WARNING: The printer cover is open",
                "WARNING: The printer is out of media",
            ]
        );
        r.set_printer_reasons(&log, &["media-empty-error", "cover-open"].into());
        assert!(lines(&b).is_empty());
        r.set_own(&log, "connecting-to-device", true);
        r.set_printer_reasons(&log, &["marker-supply-low-warning"].into());
        assert_eq!(
            lines(&b),
            vec![
                "STATE: +connecting-to-device",
                "INFO: Connecting to the printer",
                "STATE: -cover-open,media-empty-error",
                "STATE: +marker-supply-low-warning",
                "INFO: Ink low",
            ]
        );
        r.clear_blocking(&log);
        assert!(lines(&b).is_empty());
        assert_eq!(
            r.current(),
            vec!["connecting-to-device", "marker-supply-low-warning"]
        );
    }

    #[test]
    fn attrs_only_on_change() {
        let b = Buf::default();
        let log = Log::to_writer(Box::new(b.clone()));
        let mut r = StateReporter::default();
        r.attrs(&log, vec!["marker-levels=1".into()]);
        r.attrs(&log, vec!["marker-levels=1".into()]);
        assert_eq!(lines(&b), vec!["ATTR: marker-levels=1"]);
    }

    #[test]
    fn log_lines_are_single_lines() {
        let b = Buf::default();
        let log = Log::to_writer(Box::new(b.clone()));
        log.info("a\nERROR: b");
        assert_eq!(lines(&b), vec!["INFO: a ERROR: b"]);
    }
}
