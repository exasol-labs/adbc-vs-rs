//! Driver loading and ADBC database/connection setup for the scan path.
//!
//! This module never learns whether the driver `.so` was just fetched from
//! the registry, was already resident in BucketFS, or came from somewhere
//! else — by the time it runs, the caller has already resolved a filesystem
//! path. It owns exactly two decisions: how a resolved Exasol `CONNECTION`
//! object reaches the driver, and what a failure to reach it is allowed to
//! say.
//!
//! Neither decision is one answer. How the credential travels is per
//! technology, because the drivers disagree about it: MySQL's takes a user and
//! a password as ADBC options of their own, while PostgreSQL's rejects both as
//! unknown and accepts only `uri`, so its credential has to travel inside that
//! one option's value. What is *not* per technology is the invariant around
//! it — the credential is resolved for one open, reaches no structure that
//! outlives the attempt, and appears in no message this module produces.
//!
//! That invariant is why a failure names a [`DatabaseTarget`]'s own subject
//! rather than the value handed to the driver. For MySQL the two coincide;
//! for PostgreSQL the value is a libpq connection string carrying the password
//! verbatim, so the subject is the host and database parsed out of the address
//! instead. Redaction is then applied to the assembled message rather than to
//! the driver's half of it, so no caller can leak a secret by formatting
//! before stripping.
//!
//! Which symbol the library is loaded through is a third per-technology fact,
//! read off the [`Driver`] the caller resolved, so a driver added to the
//! identity brings its own entry point rather than needing an edit here.

use std::fmt;

use adbc_core::options::{AdbcVersion, OptionDatabase, OptionValue};
use adbc_core::{Database, Driver as _};
use adbc_driver_manager::{ManagedConnection, ManagedDriver};

use crate::driver::Driver;

/// A source connection target resolved from an Exasol `CONNECTION` object.
///
/// `uri` is the object's address, which MUST already be a full driver URI
/// (`<driver>://host:port/database`) rather than a bare `host:port`, and which
/// carries no credential of its own — `username` and `password` are the
/// credential, resolved beside it.
///
/// How that credential then reaches the driver is the driver's own answer, not
/// this type's: MySQL takes it as separate ADBC options, and PostgreSQL takes
/// it inside a libpq connection string this module builds at connect time.
/// Either way the value is resolved fresh for each open and never stored, and
/// it renders without its password under every formatting trait it implements,
/// so no accidental diagnostic can put the secret into a message or a log
/// line.
#[derive(Clone)]
pub struct SourceConnection {
    pub uri: String,
    pub username: String,
    pub password: String,
}

impl fmt::Debug for SourceConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SourceConnection {{ uri: {:?}, username: {:?}, password: <redacted> }}",
            self.uri, self.username
        )
    }
}

/// Failure opening an ADBC connection to the source: which step failed, the
/// subject that step names, and the driver's own diagnostic message — the
/// whole of it already stripped of `connection`'s password.
#[derive(Debug)]
pub struct ConnectError(String);

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConnectError {}

/// What one driver is configured with, and what a failure to configure it is
/// allowed to name.
///
/// The two are separate fields because for one driver they differ: PostgreSQL's
/// options carry a connection string holding the password verbatim, and naming
/// that as the subject of an error would disclose it. Keeping the printable
/// subject beside the options — rather than deriving it from them at the point
/// of failure — is what leaves no path that prints the value.
pub(crate) struct DatabaseTarget {
    pub(crate) options: Vec<(OptionDatabase, OptionValue)>,
    pub(crate) subject: String,
}

/// The context every failure of one open names, and the secret every one of
/// them is stripped against.
///
/// Assembling and stripping in one call is the point: a `format!` that ran
/// before the strip would have already produced the disclosing text, and
/// nothing in a signature could stop a caller doing it in that order.
pub(crate) struct OpenDiagnostic<'a> {
    subject: &'a str,
    password: &'a str,
}

impl<'a> OpenDiagnostic<'a> {
    pub(crate) fn new(subject: &'a str, password: &'a str) -> Self {
        OpenDiagnostic { subject, password }
    }

    pub(crate) fn failure(&self, step: &str, source: &dyn fmt::Display) -> ConnectError {
        ConnectError(redact(
            &format!("{step} {}: {source}", self.subject),
            self.password,
        ))
    }
}

/// The database options `driver` accepts for `connection`, and the subject a
/// failure to open it may name.
///
/// Matched exhaustively rather than defaulted: the two drivers disagree about
/// how a credential travels, and a technology added to the identity has to
/// state its own answer instead of inheriting one that would either drop its
/// credential or disclose it.
pub(crate) fn database_target(
    driver: Driver,
    connection: &SourceConnection,
) -> Result<DatabaseTarget, ConnectError> {
    match driver {
        Driver::MySql => Ok(mysql_target(connection)),
        Driver::Postgres => postgres_target(connection),
    }
}

/// MySQL's three options: the credential-free address, and the username and
/// password as options of their own, never concatenated into the address.
///
/// The address is also the failure subject, because it carries no credential.
fn mysql_target(connection: &SourceConnection) -> DatabaseTarget {
    DatabaseTarget {
        options: vec![
            (OptionDatabase::Uri, connection.uri.as_str().into()),
            (
                OptionDatabase::Username,
                connection.username.as_str().into(),
            ),
            (
                OptionDatabase::Password,
                connection.password.as_str().into(),
            ),
        ],
        subject: connection.uri.clone(),
    }
}

/// PostgreSQL's one option: a libpq keyword/value connection string carrying
/// the address's target, its parameters, and the resolved credential.
///
/// The driver rejects `username` and `password` as unknown options and hands
/// `uri`'s value verbatim to libpq's connect call, so the credential has no
/// other route. Because that value therefore holds the password, the failure
/// subject is built from the parsed host and database instead.
fn postgres_target(connection: &SourceConnection) -> Result<DatabaseTarget, ConnectError> {
    let address = PostgresAddress::parse(&connection.uri)?;
    let conninfo = address.conninfo(&connection.username, &connection.password);

    Ok(DatabaseTarget {
        options: vec![(OptionDatabase::Uri, conninfo.as_str().into())],
        subject: address.describe(),
    })
}

/// The parts of a PostgreSQL `CONNECTION` address a libpq connection string is
/// built from.
///
/// Userinfo is dropped rather than kept: a user and password written into the
/// address are not the credential — Exasol resolves that beside the address —
/// and treating them as part of the authority would both address the wrong
/// host and put a secret into the subject an error names.
struct PostgresAddress {
    host: String,
    port: Option<String>,
    database: String,
    parameters: Vec<(String, String)>,
}

impl PostgresAddress {
    fn parse(address: &str) -> Result<Self, ConnectError> {
        let after_scheme = address.split_once("://").map_or(address, |(_, rest)| rest);
        let (authority, path) = after_scheme.split_once('/').ok_or_else(missing_database)?;
        let authority = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let (host, port) = split_host_port(authority);

        let (database, query) = split_query(path);
        if database.is_empty() {
            return Err(missing_database());
        }

        Ok(PostgresAddress {
            host,
            port,
            database: database.to_string(),
            parameters: query_parameters(query)?,
        })
    }

    /// The connection string libpq is handed, with the resolved credential
    /// last.
    ///
    /// Order is load-bearing at the end: libpq reads the last occurrence of a
    /// repeated keyword, so emitting the credential after the address's own
    /// parameters is what keeps a repeated `sslmode` or `connect_timeout`
    /// resolving the way the address wrote it while the credential still wins.
    /// The address carries no credential keyword of its own to displace —
    /// [`query_parameters`] discards those.
    fn conninfo(&self, username: &str, password: &str) -> String {
        let mut keywords: Vec<(&str, &str)> = vec![("host", &self.host)];
        if let Some(port) = &self.port {
            keywords.push(("port", port));
        }
        keywords.push(("dbname", &self.database));
        for (name, value) in &self.parameters {
            keywords.push((name, value));
        }
        keywords.push(("user", username));
        keywords.push(("password", password));

        keywords
            .iter()
            .map(|(name, value)| format!("{name}={}", quote(value)))
            .collect::<Vec<String>>()
            .join(" ")
    }

    /// What a failure to reach this address is allowed to say: the host and
    /// the database, and neither the credential nor the parameters, so a wrong
    /// password stays diagnosable without the message carrying one.
    fn describe(&self) -> String {
        let host = match &self.port {
            Some(port) => format!("{}:{port}", self.host),
            None => self.host.clone(),
        };
        format!(
            "the postgresql source at `{host}`, database `{}`",
            self.database
        )
    }
}

/// Encloses a conninfo value in single quotes, escaping the two characters
/// that would otherwise end it or escape the next one.
///
/// Total by construction: every other byte a password can hold — a space, an
/// ampersand, a `#`, a `/`, a multi-byte character — is already literal inside
/// single quotes. That totality is the reason conninfo form was chosen over a
/// URI, whose percent-encoding lets an unescaped byte change the parse rather
/// than the value.
fn quote(value: &str) -> String {
    format!("'{}'", escape_conninfo_value(value))
}

/// A conninfo value's body, without the quotes that enclose it.
///
/// Separate from [`quote`] because [`redact`] needs the same rendering: this
/// is the second form a password takes inside this crate, and a message
/// carrying it discloses the secret exactly as the verbatim form does.
fn escape_conninfo_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Splits an authority into its host and its optional port, reading a
/// bracketed IPv6 literal as one host rather than splitting on a colon inside
/// it. libpq takes such an address without its brackets.
fn split_host_port(authority: &str) -> (String, Option<String>) {
    if let Some((host, after)) = authority
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
    {
        let port = after.strip_prefix(':').filter(|port| !port.is_empty());
        return (host.to_string(), port.map(str::to_string));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() => (host.to_string(), Some(port.to_string())),
        _ => (authority.to_string(), None),
    }
}

/// Splits an address's path into the database name and the query string, after
/// dropping a fragment, which a URI always carries last and which names
/// nothing a connection needs.
fn split_query(path: &str) -> (&str, &str) {
    let path = path.split('#').next().unwrap_or_default();
    path.split_once('?').unwrap_or((path, ""))
}

/// The conninfo keywords that carry a credential, and which an address is
/// therefore never read for.
///
/// libpq matches a keyword case-insensitively, so the comparison is too: a
/// `PASSWORD=` that slipped through would disclose exactly as a lower-case one
/// does.
const CREDENTIAL_KEYWORDS: [&str; 3] = ["password", "user", "sslpassword"];

/// Reads a query string as the conninfo keywords it becomes, keeping the
/// address's own order so a repeated parameter resolves the way libpq resolves
/// one.
///
/// A parameter carrying no `=` becomes a keyword with an empty value rather
/// than being dropped, because dropping it silently is the outcome carrying
/// every parameter across exists to prevent.
///
/// The exception is a credential. The resolved `CONNECTION` object is the only
/// source of one, so a [`CREDENTIAL_KEYWORDS`] parameter written into the
/// address is discarded rather than carried across and overridden: carrying it
/// changes no connection — [`PostgresAddress::conninfo`] emits the resolved
/// credential last, and libpq reads the last occurrence — while putting a
/// secret [`redact`] has never seen into the value a driver may echo back.
fn query_parameters(query: &str) -> Result<Vec<(String, String)>, ConnectError> {
    query
        .split('&')
        .filter(|parameter| !parameter.is_empty())
        .map(|parameter| parameter.split_once('=').unwrap_or((parameter, "")))
        .filter(|(name, _)| !is_credential_keyword(name))
        .map(|(name, value)| {
            require_keyword(name)?;
            Ok((name.to_string(), value.to_string()))
        })
        .collect()
}

fn is_credential_keyword(name: &str) -> bool {
    CREDENTIAL_KEYWORDS
        .iter()
        .any(|keyword| name.eq_ignore_ascii_case(keyword))
}

/// Refuses a parameter name a conninfo keyword cannot be.
///
/// A value is quoted and so may hold anything, but a keyword is bare: a name
/// carrying a space or an equals sign would reach libpq as two keywords, which
/// is a silent misparse rather than the parameter the operator wrote.
fn require_keyword(name: &str) -> Result<(), ConnectError> {
    let is_keyword = !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
    if is_keyword {
        return Ok(());
    }
    Err(ConnectError(format!(
        "the postgresql CONNECTION address carries the parameter `{name}`, whose name is not a \
         libpq connection keyword; a keyword is letters, digits, and underscores"
    )))
}

/// States the address form expected without echoing the address given, which
/// may carry a credential of its own in its userinfo or its query string.
fn missing_database() -> ConnectError {
    ConnectError(
        "the postgresql CONNECTION address names no database path component; expected \
         postgresql://host:port/database"
            .to_string(),
    )
}

/// Loads the driver `.so` at `driver_path` through the ADBC entry point
/// `driver` names, opens a database configured from `connection`, and opens a
/// connection to it.
///
/// The entry point is read off the identity rather than derived from the
/// library's file name: the MySQL driver exports both `AdbcDriverMysqlInit`
/// and `AdbcDriverMySQLInit`, and the driver manager's filename heuristic
/// happens to pick the same symbol today but is pinned by nothing in the
/// driver's own manifest.
///
/// The driver's options are resolved before the library is loaded, so an
/// address the driver could not have been configured from fails without
/// touching the filesystem. Every failure names the step, the subject that
/// step addresses, and the driver's own diagnostic message, the whole message
/// stripped of `connection`'s password.
pub fn open(
    driver: Driver,
    driver_path: &str,
    connection: &SourceConnection,
) -> Result<ManagedConnection, ConnectError> {
    let target = database_target(driver, connection)?;
    let entry_point = driver.package().entry_point.as_bytes();

    let loading = OpenDiagnostic::new(driver_path, &connection.password);
    let mut loaded = ManagedDriver::load_dynamic_from_filename(
        driver_path,
        Some(entry_point),
        AdbcVersion::V110,
    )
    .map_err(|error| {
        loading.failure(
            &format!("loading the {} ADBC driver from", driver.as_str()),
            &error,
        )
    })?;

    let opening = OpenDiagnostic::new(&target.subject, &connection.password);
    let database = loaded
        .new_database_with_opts(target.options)
        .map_err(|error| opening.failure("opening an ADBC database for", &error))?;

    database
        .new_connection()
        .map_err(|error| opening.failure("opening an ADBC connection to", &error))
}

/// Strips every occurrence of `password` from `message`, in each of the two
/// renderings this crate can produce of it.
///
/// The verbatim form is the obvious one. The escaped form is the one a
/// PostgreSQL failure actually carries: the connection string holds the
/// password with its quotes and backslashes escaped, so a driver echoing that
/// string back discloses the secret in a spelling the verbatim strip does not
/// match. `scan-udf/postgresql-scan` requires the password absent "in any
/// encoding or escaping", and these are the encodings that exist here.
///
/// The escaped form is stripped first, because it contains the verbatim form's
/// characters and stripping the shorter one first would leave its remainder
/// behind.
///
/// Applied to a whole assembled failure rather than to the driver's half of
/// one, because both halves can carry the secret.
pub(crate) fn redact(message: &str, password: &str) -> String {
    if password.is_empty() {
        return message.to_string();
    }
    let escaped = escape_conninfo_value(password);
    let message = if escaped == password {
        message.to_string()
    } else {
        message.replace(&escaped, REDACTED)
    };
    message.replace(password, REDACTED)
}

/// What every stripped secret is replaced by, so a reader sees that something
/// was removed rather than that a field was empty.
const REDACTED: &str = "<redacted>";
