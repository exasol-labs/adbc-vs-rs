//! The one decision both E2E binaries need to agree on: whether a BucketFS
//! path segment is a driver version at all.
//!
//! `e2e_mysql_test.rs` and `e2e_postgres_test.rs` share one BucketFS, and each
//! binary's setup can leave a non-version object under a driver's prefix — a
//! pre-placed library copied there to prove self-install does not
//! short-circuit off it. A bare `Version::parse` in one binary's filter and a
//! different rule in the other's would let that object surface as "the"
//! installed driver in whichever binary runs second, so this is the one place
//! either binary may decide it.

use semver::Version;

/// Parses a driver's version segment (e.g. `v0.6.0` or `1.12.0`, the two
/// spellings the registry index mixes) as a comparable semantic version, or
/// `None` when the segment is not a version at all — such as an
/// operator-supplied path segment used to test that self-install does not
/// short-circuit off a pre-placed library.
pub fn parse_driver_semver(version: &str) -> Option<Version> {
    Version::parse(version.strip_prefix('v').unwrap_or(version)).ok()
}

/// Reads a bucket entry as an installed driver's version, or `None` when it
/// is not one.
///
/// This owns both halves of the decision either binary would otherwise make
/// on its own: which path segment holds the version (the first one
/// remaining after stripping `prefix`) and whether that segment parses as one
/// ([`parse_driver_semver`]). Splitting the two across the binaries is what
/// let a `supplied` object surface as "the" installed driver in whichever
/// binary ran second; keeping both here is what rules that out.
pub fn installed_driver_version(entry: &str, prefix: &str) -> Option<Version> {
    entry
        .strip_prefix(prefix)
        .and_then(|rest| rest.split('/').next())
        .and_then(parse_driver_semver)
}
