//! Driver loading and ADBC database/connection setup for the MySQL scan
//! path.
//!
//! This module never learns whether the driver `.so` was just fetched from
//! the registry, was already resident in BucketFS, or came from somewhere
//! else — by the time it runs, the caller has already resolved a filesystem
//! path. It owns exactly two decisions: which symbol is the driver's ADBC
//! entry point, and which three ADBC options carry a resolved Exasol
//! `CONNECTION` object into the driver, so no credential ever needs to pass
//! through a connection URI.

use std::fmt;

use adbc_core::options::{AdbcVersion, OptionDatabase, OptionValue};
use adbc_core::{Database, Driver};
use adbc_driver_manager::{ManagedConnection, ManagedDriver};

/// The MySQL ADBC driver `.so` exports both `AdbcDriverMysqlInit` and
/// `AdbcDriverMySQLInit`. Naming this one explicitly avoids depending on
/// `adbc_driver_manager`'s filename-derivation heuristic, which happens to
/// pick the same symbol today but is pinned by nothing in the driver's own
/// manifest.
const DRIVER_ENTRY_POINT: &[u8] = b"AdbcDriverMysqlInit";

/// A MySQL connection target resolved from an Exasol `CONNECTION` object.
///
/// `uri` MUST already be a full driver URI (`mysql://host:port/database`),
/// never a bare `host:port`, and MUST carry no credential — `username` and
/// `password` travel as separate ADBC options instead, so no password ever
/// reaches SQL text, `adapterNotes`, or an audit table.
#[derive(Debug, Clone)]
pub struct MysqlConnection {
    pub uri: String,
    pub username: String,
    pub password: String,
}

/// Failure opening an ADBC connection to MySQL: which step failed, the path
/// or URI involved, and the driver's own diagnostic message with
/// `connection`'s password redacted.
#[derive(Debug)]
pub struct ConnectError(String);

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConnectError {}

/// Builds the three ADBC database options for `connection`: the
/// credential-free URI, and the username and password as separate options,
/// never concatenated into the URI.
pub(crate) fn database_options(connection: &MysqlConnection) -> [(OptionDatabase, OptionValue); 3] {
    [
        (OptionDatabase::Uri, connection.uri.as_str().into()),
        (
            OptionDatabase::Username,
            connection.username.as_str().into(),
        ),
        (
            OptionDatabase::Password,
            connection.password.as_str().into(),
        ),
    ]
}

/// Loads the driver `.so` at `driver_path` through the explicitly named
/// ADBC entry point, opens a database configured from `connection`, and
/// opens a connection to it. Every failure names the step, the path or URI
/// involved, and the driver's own diagnostic message with `connection`'s
/// password redacted.
pub fn open_mysql_connection(
    driver_path: &str,
    connection: &MysqlConnection,
) -> Result<ManagedConnection, ConnectError> {
    let mut driver = ManagedDriver::load_dynamic_from_filename(
        driver_path,
        Some(DRIVER_ENTRY_POINT),
        AdbcVersion::V110,
    )
    .map_err(|error| {
        connect_error(
            "loading the MySQL ADBC driver from",
            driver_path,
            redact(&error.to_string(), &connection.password),
        )
    })?;

    let database = driver
        .new_database_with_opts(database_options(connection))
        .map_err(|error| {
            connect_error(
                "opening an ADBC database for",
                &connection.uri,
                redact(&error.to_string(), &connection.password),
            )
        })?;

    database.new_connection().map_err(|error| {
        connect_error(
            "opening an ADBC connection to",
            &connection.uri,
            redact(&error.to_string(), &connection.password),
        )
    })
}

pub(crate) fn connect_error(step: &str, subject: &str, redacted_source: String) -> ConnectError {
    ConnectError(format!("{step} {subject}: {redacted_source}"))
}

/// Strips every occurrence of `password` from `message`, as defense in
/// depth — the driver's own errors are not expected to echo a password,
/// since it never reaches the URI, but nothing enforces that upstream.
pub(crate) fn redact(message: &str, password: &str) -> String {
    if password.is_empty() {
        return message.to_string();
    }
    message.replace(password, "<redacted>")
}
