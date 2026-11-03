//! # EXTENSION POINT: Epson-specific status reply (phase 5)
//!
//! The TM-C3500 is expected to send an Epson status reply on the job connection (back-channel) and
//! possibly answer an Epson status request. Neither the request bytes nor the reply format are known
//! yet: they will be recorded in the real-printer session (phase 5, Wireshark) and turned into parser
//! fixtures here.
//!
//! Until then:
//! - [`status_request`] returns `None` (nothing is ever sent to the printer);
//! - [`parse_reply`] only checks the size limit and reports the reply as unrecognised, so callers can
//!   log it (hex, capped) for the phase-5 analysis;
//! - [`EpsonStatus`] is the shape the parser will fill in; `PrinterStatus::merge_epson` already folds
//!   it into the CUPS reasons and alert conditions, so wiring stays unchanged when the parser lands.

/// Largest Epson reply accepted.
pub const MAX_REPLY: usize = 16 * 1024;

/// Details only the Epson reply provides (SNMP has no standard object for them).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpsonStatus {
    /// Auto-cutter error.
    pub cutter_error: bool,
    /// Additional CUPS `printer-state-reasons` keywords derived from the reply.
    pub reasons: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpsonReply {
    /// Reply parsed.
    Parsed(EpsonStatus),
    /// Format not known yet (phase 5); `len` bytes received.
    Unrecognised { len: usize },
    /// Larger than [`MAX_REPLY`]; ignored.
    TooLarge { len: usize },
}

/// Bytes to request an Epson status reply. Unknown until phase 5.
pub fn status_request() -> Option<&'static [u8]> {
    None
}

/// Parse an Epson status reply. Never panics.
pub fn parse_reply(b: &[u8]) -> EpsonReply {
    if b.len() > MAX_REPLY {
        return EpsonReply::TooLarge { len: b.len() };
    }
    // PHASE 5: decode the recorded reply format here.
    EpsonReply::Unrecognised { len: b.len() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn nothing_known_yet() {
        assert_eq!(status_request(), None);
        assert_eq!(
            parse_reply(b"@BDC ST2"),
            EpsonReply::Unrecognised { len: 8 }
        );
        assert_eq!(
            parse_reply(&vec![0; MAX_REPLY + 1]),
            EpsonReply::TooLarge { len: MAX_REPLY + 1 }
        );
    }

    proptest! {
        #[test]
        fn never_panics(b in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let _ = parse_reply(&b);
        }
    }
}
