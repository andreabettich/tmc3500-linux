//! CUPS backend logging: one line per message with a CUPS prefix on stderr.
use std::io::Write;
use std::sync::{Arc, Mutex};

/// Where log lines go (stderr in production, a buffer in unit tests).
#[derive(Clone)]
pub struct Log {
    sink: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl Log {
    pub fn stderr() -> Log {
        Log {
            sink: Arc::new(Mutex::new(Box::new(std::io::stderr()))),
        }
    }

    pub fn to_writer(w: Box<dyn Write + Send>) -> Log {
        Log {
            sink: Arc::new(Mutex::new(w)),
        }
    }

    /// Write `PREFIX: message`; control characters are replaced so one message is one line.
    pub fn line(&self, prefix: &str, msg: &str) {
        let msg: String = msg
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let mut s = self.sink.lock().unwrap_or_else(|p| p.into_inner());
        let _ = writeln!(s, "{prefix}: {msg}");
        let _ = s.flush();
    }

    pub fn error(&self, m: &str) {
        self.line("ERROR", m)
    }
    pub fn warning(&self, m: &str) {
        self.line("WARNING", m)
    }
    pub fn info(&self, m: &str) {
        self.line("INFO", m)
    }
    pub fn debug(&self, m: &str) {
        self.line("DEBUG", m)
    }
    pub fn state(&self, m: &str) {
        self.line("STATE", m)
    }
    pub fn attr(&self, m: &str) {
        self.line("ATTR", m)
    }
}
