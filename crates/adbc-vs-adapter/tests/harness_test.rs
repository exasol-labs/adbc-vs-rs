//! Builds the shared stack harness and runs its unit tests.
//!
//! Cargo turns each file directly under `tests/` into a test binary but leaves
//! a subdirectory alone, so `tests/common/` compiles only through a target that
//! declares it. This target is that declaration, and it holds no test of its
//! own: the harness's assertions live beside the code they cover.

mod common;
