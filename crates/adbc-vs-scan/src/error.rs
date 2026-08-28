//! The failure the scan path reports to whoever drives it.
//!
//! Executing a statement, streaming a batch, and coercing a batch fail for
//! very different reasons, but a caller can do nothing about any of them
//! beyond reporting: the scan runs inside one UDF invocation with no
//! fallback to choose. One opaque error therefore keeps the ADBC status
//! taxonomy and Arrow's error enum out of every caller, while the message
//! carries everything a person diagnosing the failure needs.

use std::fmt;

/// A failed scan: what was attempted, the statement or column it failed on,
/// and — for a driver failure — the ADBC status and the driver's own
/// message, with the MySQL password stripped out.
#[derive(Debug)]
pub struct ScanError(String);

impl ScanError {
    pub(crate) fn new(message: String) -> Self {
        ScanError(message)
    }
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ScanError {}
