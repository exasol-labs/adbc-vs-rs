//! Shared harness for the driver-technology integration tiers.
//!
//! Cargo builds a `tests/` subdirectory only through a test target that
//! declares it, so `tests/harness_test.rs` exists to keep this module
//! compiling and linted even when no integration feature is enabled.

pub mod driver_fetch;
