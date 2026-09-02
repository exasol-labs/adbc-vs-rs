//! Builds the shared integration-tier harness and keeps it linted.
//!
//! Cargo turns each file directly under `tests/` into a test binary but
//! leaves a subdirectory alone, so `tests/common/` compiles only through a
//! target that declares it. This target is that declaration, and it holds no
//! test of its own.
//!
//! `common::driver_fetch`'s helpers are exercised by `mysql_it_test.rs` and
//! `postgres_it_test.rs`, not by this target — its own job is only to prove
//! the shared module compiles even when neither integration feature is
//! enabled, so nothing here calls them.
#![allow(dead_code)]

mod common;
