//! Alert delivery off the I/O path: hooks run on a worker thread fed by a bounded queue, so a
//! slow hook never delays the job connection or cancellation. `finish` waits for queued alerts
//! (each hook is bounded by its own timeout) before the backend exits.
use crate::log::Log;
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::thread::JoinHandle;
use tmc3500_common::alert::{Alerter, ConditionTracker, Delivery, Event, HookOutcome};
use tmc3500_core::status::printer::Condition;

/// Queued alert messages before new ones are dropped (with a warning).
pub const QUEUE: usize = 64;

pub enum Msg {
    Event(Event),
    Conditions {
        conditions: Vec<Condition>,
        reasons: Vec<String>,
    },
    Clear(String),
}

pub struct AlertWorker {
    tx: Option<SyncSender<Msg>>,
    handle: Option<JoinHandle<()>>,
    log: Log,
}

pub fn log_delivery(log: &Log, ev: &Event, d: &Delivery) {
    for w in &d.warnings {
        log.warning(&format!("alert: {w}"));
    }
    if d.suppressed {
        log.debug(&format!("alert {} suppressed (repeat)", ev.key));
        return;
    }
    log.debug(&format!(
        "alert {} sent to {} hook(s)",
        ev.key,
        d.hooks.len()
    ));
    for h in &d.hooks {
        if h.outcome != HookOutcome::Ok {
            log.warning(&format!(
                "alert hook {} for {}: {:?}",
                h.hook.display(),
                ev.event,
                h.outcome
            ));
        }
    }
}

impl AlertWorker {
    pub fn start(
        alerter: Alerter,
        log: Log,
        printer: String,
        queue: Option<String>,
        job_id: String,
    ) -> AlertWorker {
        let (tx, rx) = sync_channel::<Msg>(QUEUE);
        let wlog = log.clone();
        let handle = std::thread::Builder::new()
            .name("alerts".into())
            .spawn(move || {
                let mut tracker = ConditionTracker::default();
                for m in rx {
                    match m {
                        Msg::Event(ev) => {
                            let d = alerter.emit(&ev);
                            log_delivery(&wlog, &ev, &d);
                        }
                        Msg::Conditions {
                            conditions,
                            reasons,
                        } => {
                            for (ev, d) in
                                tracker.observe(&alerter, &conditions, &printer, "backend", |e| {
                                    e.job_id = Some(job_id.clone());
                                    e.queue = queue.clone();
                                    e.reasons = reasons.clone();
                                })
                            {
                                log_delivery(&wlog, &ev, &d);
                            }
                        }
                        Msg::Clear(k) => {
                            for w in alerter.clear(&printer, &k) {
                                wlog.warning(&format!("alert: {w}"));
                            }
                        }
                    }
                }
            })
            .ok();
        if handle.is_none() {
            log.warning("cannot start the alert thread; alerts disabled");
        }
        AlertWorker {
            tx: handle.as_ref().map(|_| tx),
            handle,
            log,
        }
    }

    /// Queue a message; never blocks.
    pub fn send(&self, m: Msg) {
        let Some(tx) = &self.tx else { return };
        match tx.try_send(m) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => self
                .log
                .warning("alert queue full (hooks too slow); alert dropped"),
            Err(TrySendError::Disconnected(_)) => {
                self.log.warning("alert thread stopped; alert dropped")
            }
        }
    }

    /// Wait until every queued alert has been delivered.
    pub fn finish(&mut self) {
        self.tx = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for AlertWorker {
    fn drop(&mut self) {
        self.finish();
    }
}
