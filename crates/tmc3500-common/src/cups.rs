//! CUPS backend exit codes (`cups/backend.h`).

pub const CUPS_BACKEND_OK: i32 = 0;
pub const CUPS_BACKEND_FAILED: i32 = 1;
pub const CUPS_BACKEND_AUTH_REQUIRED: i32 = 2;
pub const CUPS_BACKEND_HOLD: i32 = 3;
pub const CUPS_BACKEND_STOP: i32 = 4;
pub const CUPS_BACKEND_CANCEL: i32 = 5;
pub const CUPS_BACKEND_RETRY: i32 = 6;
pub const CUPS_BACKEND_RETRY_CURRENT: i32 = 7;
