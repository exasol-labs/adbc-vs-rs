//! The one ADBC driver binary release 0.1 loads.
//!
//! `adbc-vs-adapter`'s provisioning fetches exactly this package, verifies it
//! against exactly this digest, and installs exactly this library name; the
//! MySQL integration tier fetches the same package so it exercises the
//! identical binary. Both sides need the same six facts and the dependency
//! runs adapter -> scan, so they are declared here, where each side can read
//! them, rather than restated on both sides where they could drift.
//!
//! Changing any of them is a reviewed code change, never an operator-supplied
//! property: the digest is the only integrity check the registry offers, and a
//! version bumped on one side alone would leave a tier certifying a driver the
//! product no longer installs.

/// The driver release 0.1 installs.
pub const DRIVER_VERSION: &str = "v0.6.0";

/// The shared library the pinned package installs, as its `MANIFEST` names it.
pub const DRIVER_LIBRARY: &str = "libadbc_driver_mysql.so";

/// SHA-256 of the pinned `linux_amd64` package. The registry publishes no
/// checksum of its own, so this digest is the only integrity check there is.
pub const PACKAGE_SHA256: &str = "d4e2c17ce1f61be4e2a395b1f39d71e73bd136ff63d88bfa29ef238498a5a6b1";

/// The host the driver registry serves its index and its packages from.
pub const REGISTRY_BASE_URL: &str = "https://dbc-cdn.columnar.tech";

/// The path the registry index publishes this driver's versions under.
pub const REGISTRY_DRIVER_PATH: &str = "mysql";

/// The only platform the Exasol script language container runs on, and so the
/// only package variant either side ever asks the registry for.
pub const PLATFORM: &str = "linux_amd64";
