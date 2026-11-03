//! Errors for option parsing.
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptError {
    /// The option string itself is malformed (quotes, length).
    Syntax(String),
    /// A `Tm*` option this driver does not know.
    UnknownOption(String),
    /// A value that cannot be parsed for this option.
    BadValue { key: String, value: String },
    /// A parsable value outside the allowed range.
    OutOfRange {
        key: String,
        value: String,
        allowed: String,
    },
    /// Two options that cannot be combined (e.g. Plain coating with Quality mode).
    Conflict(String),
    /// A valid printer setting whose encoding has not been captured yet.
    NotYetSupported(String),
}

impl fmt::Display for OptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(s) => write!(f, "malformed option string: {s}"),
            Self::UnknownOption(k) => write!(f, "unknown option {k}"),
            Self::BadValue { key, value } => write!(f, "invalid value {value:?} for {key}"),
            Self::OutOfRange {
                key,
                value,
                allowed,
            } => {
                write!(
                    f,
                    "value {value:?} for {key} out of range (allowed: {allowed})"
                )
            }
            Self::Conflict(s) => write!(f, "conflicting options: {s}"),
            Self::NotYetSupported(s) => write!(f, "not supported yet: {s}"),
        }
    }
}

impl std::error::Error for OptError {}
