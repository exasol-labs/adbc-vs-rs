//! Which driver technologies adbc-vs can load, and the package identity of
//! each.
//!
//! A virtual schema names its technology once, in its `DRIVER` property, and
//! every later decision follows from that one value: which package to fetch,
//! which library file the BucketFS path ends in, which symbol the driver
//! manager loads the library through, and — in the adapter — which SQL dialect
//! renders its pushdowns. [`Driver`] is that value made a type, so those
//! decisions are matched exhaustively rather than defaulted, and adding a
//! technology surfaces as a compile error at each of them rather than as a
//! silent fallback to the first one.
//!
//! The facts themselves live beside the code, one `drivers/<name>.toml` per
//! technology, and `build.rs` turns them into the `&'static` tables this module
//! includes — the package row, the capability set its dialect advertises, and
//! how many name segments its source qualifier carries.
//! They are read at build time and never at run time: a technology
//! adbc-vs did not ship Rust for cannot be reached by dropping a file onto a
//! cluster, and a malformed declaration fails the build instead of the first
//! `CREATE VIRTUAL SCHEMA` that reads it.
//!
//! Both crates need these facts and the dependency runs adapter -> scan, so
//! they are declared here, where each side reads them, rather than restated on
//! both sides where they could drift.

use std::fmt;

/// One technology's driver package: everything provisioning needs to fetch and
/// install it, and everything the scan needs to load it.
///
/// Every field is read without a fallback, which is why the build script
/// refuses a declaration that leaves one empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverPackage {
    /// The path the registry index publishes this driver's versions under.
    pub registry_path: &'static str,
    /// The shared library the package installs, as its `MANIFEST` names it.
    /// The BucketFS install path ends in it.
    pub library: &'static str,
    /// The ADBC entry-point symbol the driver manager loads the library
    /// through, named rather than derived from the file name.
    pub entry_point: &'static str,
    /// The published versions this release accepts, as a semver range. Each
    /// driver's range is its own: nothing derives one from another.
    pub version_range: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/driver_packages.rs"));

/// The driver technologies this release can load.
///
/// Closed on purpose. Exasol hands the adapter a `DRIVER` string, and every
/// resolution that follows — dialect, package, metadata source — matches on
/// this type, so a technology with no implementation cannot be reached by
/// spelling its name in a `CREATE VIRTUAL SCHEMA` statement.
///
/// One variant per `drivers/<name>.toml`. The two are held together by
/// [`DECLARED_DRIVERS`] and asserted by this module's tests, because the build
/// script that reads the files cannot see this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Driver {
    /// The `DRIVER` property value `mysql`.
    MySql,
    /// The `DRIVER` property value `postgresql`.
    Postgres,
    /// The `DRIVER` property value `trino`.
    Trino,
}

impl Driver {
    /// Every technology this release can load, in a fixed order so anything
    /// enumerated from it — a supported-value list, a per-driver test arm — is
    /// the same between calls.
    pub const ALL: &'static [Driver] = &[Driver::MySql, Driver::Postgres, Driver::Trino];

    /// The canonical, lower-case name of this technology: the value `DRIVER`
    /// takes, the value `adapterNotes.driver` records, and the name of its
    /// declaration file.
    pub fn as_str(self) -> &'static str {
        match self {
            Driver::MySql => "mysql",
            Driver::Postgres => "postgresql",
            Driver::Trino => "trino",
        }
    }

    /// The compiled-in package facts for this technology.
    pub fn package(self) -> &'static DriverPackage {
        match self {
            Driver::MySql => &MYSQL_PACKAGE,
            Driver::Postgres => &POSTGRESQL_PACKAGE,
            Driver::Trino => &TRINO_PACKAGE,
        }
    }

    /// The capability set this technology's dialect advertises, as its
    /// declaration states it.
    ///
    /// The values are data and the claim is not: `SqlDialect::capabilities` is
    /// still what answers Exasol, and it answers with this list only because
    /// that dialect's renderer covers it. Reading them through the identity is
    /// what keeps how the generated constants are named a fact of this module,
    /// so a dialect names its technology rather than a code generator's
    /// spelling.
    pub fn capabilities(self) -> &'static [&'static str] {
        match self {
            Driver::MySql => MYSQL_CAPABILITIES,
            Driver::Postgres => POSTGRESQL_CAPABILITIES,
            Driver::Trino => TRINO_CAPABILITIES,
        }
    }

    /// How many name segments this technology's source qualifier carries, as
    /// its declaration states it.
    ///
    /// One owner, two readers, and they answer at moments that cannot check
    /// each other: `create_schema` reads it to decide whether a `CONNECTION`
    /// address's path spells the qualifier, before any connection exists, and
    /// `metadata` reads it to decide whether the source produced one, only
    /// after a connection does. Two modules deciding how deeply a technology
    /// nests its tables could disagree, and a statement built on the
    /// disagreement would name a different object rather than fail.
    pub fn qualifier_segment_count(self) -> usize {
        match self {
            Driver::MySql => MYSQL_QUALIFIER_SEGMENTS,
            Driver::Postgres => POSTGRESQL_QUALIFIER_SEGMENTS,
            Driver::Trino => TRINO_QUALIFIER_SEGMENTS,
        }
    }

    /// Resolves a `DRIVER` property value, ignoring case, or reports every
    /// value it would have accepted.
    ///
    /// Case-insensitive because the value is operator-typed SQL text; the
    /// canonical spelling is what gets recorded, so what is read back later is
    /// spelled one way however it was typed.
    pub fn parse(value: &str) -> Result<Self, UnsupportedDriver> {
        Self::ALL
            .iter()
            .copied()
            .find(|driver| value.eq_ignore_ascii_case(driver.as_str()))
            .ok_or_else(|| UnsupportedDriver(value.to_owned()))
    }
}

/// A `DRIVER` property value naming no technology this release ships.
///
/// Renders the value it rejected and every value it would have accepted: it
/// reaches an operator as the whole diagnosis of a failed `CREATE VIRTUAL
/// SCHEMA`, and the accepted set is read off [`Driver::ALL`] so it cannot fall
/// behind the drivers actually shipped.
#[derive(Debug)]
pub struct UnsupportedDriver(String);

impl fmt::Display for UnsupportedDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let supported: Vec<String> = Driver::ALL
            .iter()
            .map(|driver| format!("`{}`", driver.as_str()))
            .collect();
        write!(
            f,
            "DRIVER `{}` names no driver adbc-vs supports; the supported drivers are {}",
            self.0,
            supported.join(", ")
        )
    }
}

impl std::error::Error for UnsupportedDriver {}

/// The host the driver registry serves its index and its packages from.
pub const REGISTRY_BASE_URL: &str = "https://dbc-cdn.columnar.tech";

/// The only platform the Exasol script language container runs on, and so the
/// only package variant either side ever asks the registry for.
pub const PLATFORM: &str = "linux_amd64";
