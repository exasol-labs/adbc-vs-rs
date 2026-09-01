use adbc_core::error::{Error as AdbcError, Status};
use adbc_core::options::{OptionDatabase, OptionValue};
use std::os::raw::c_char;

use super::connection::*;
use super::driver::Driver;

/// A password distinctive enough that finding it in any rendering is
/// unambiguous rather than a substring coincidence.
const PASSWORD: &str = "s3cret";

fn value_as_str(value: &OptionValue) -> &str {
    match value {
        OptionValue::String(text) => text,
        other => panic!("expected OptionValue::String, got {other:?}"),
    }
}

/// A password holding each of the four characters
/// `scan-udf/postgresql-scan` names: a single quote, a backslash, a space, and
/// an ampersand. Every one of them changes the parse of some connection-value
/// form, which is why the escaping has to be total rather than per character.
const AWKWARD_PASSWORD: &str = "pa'b\\c d&e";

fn connection() -> SourceConnection {
    SourceConnection {
        uri: "mysql://10.0.0.5:3306/adbc_vs".to_string(),
        username: "root".to_string(),
        password: PASSWORD.to_string(),
    }
}

fn postgres_connection(address: &str, password: &str) -> SourceConnection {
    SourceConnection {
        uri: address.to_string(),
        username: "appuser".to_string(),
        password: password.to_string(),
    }
}

fn conninfo(address: &str, password: &str) -> String {
    let connection = postgres_connection(address, password);
    let target = database_target(Driver::Postgres, &connection)
        .unwrap_or_else(|error| panic!("`{address}` must build a connection value: {error}"));

    assert_eq!(
        target.options.len(),
        1,
        "the PostgreSQL driver accepts exactly one database option"
    );
    assert_eq!(target.options[0].0, OptionDatabase::Uri);
    value_as_str(&target.options[0].1).to_string()
}

fn fake_source_error(message: &str) -> AdbcError {
    AdbcError {
        message: message.to_string(),
        status: Status::IO,
        vendor_code: 0,
        sqlstate: [0 as c_char; 5],
        details: None,
    }
}

// ---------------------------------------------------------------------------
// The framework invariant `scan-udf/arrow-scan` owns, over every identity
// ---------------------------------------------------------------------------

/// The credential reaches the driver only through the value the caller
/// resolved for this one open, and no structure that outlives the call, or
/// that a diagnostic can print, carries it.
///
/// Run once per identity rather than once for MySQL: it is a property of the
/// open path itself, so a technology added later inherits the assertion
/// instead of needing its own copy of it.
#[test]
fn credentials_are_resolved_per_open_and_never_persisted() {
    for driver in Driver::ALL {
        let connection = connection();

        let Err(error) = open(*driver, "/nonexistent/path/driver.so", &connection) else {
            panic!("a driver path that does not exist must fail to load");
        };

        assert!(
            !error.to_string().contains(PASSWORD),
            "the failure of `{}`'s open leaked the password: {error}",
            driver.as_str()
        );
        assert!(
            !format!("{error:?}").contains(PASSWORD),
            "the debug rendering of `{}`'s open failure leaked the password: {error:?}",
            driver.as_str()
        );
        assert!(
            !format!("{connection:?}").contains(PASSWORD),
            "the resolved connection value prints its password: {connection:?}"
        );
        assert_eq!(
            connection.password, PASSWORD,
            "open must read the credential the caller resolved rather than take it over"
        );
    }
}

#[test]
fn missing_driver_path_named_in_error() {
    let result = open(
        Driver::MySql,
        "/nonexistent/path/libadbc_driver_mysql.so",
        &connection(),
    );

    let error = match result {
        Ok(_) => panic!("a nonexistent driver path must fail to load"),
        Err(error) => error,
    };

    let message = error.to_string();
    assert!(
        message.contains("/nonexistent/path/libadbc_driver_mysql.so"),
        "error message {message:?} does not name the missing path"
    );
    assert!(
        message.contains(Driver::MySql.as_str()),
        "error message {message:?} does not name the driver it tried to load"
    );
}

#[test]
fn redact_replaces_every_password_occurrence() {
    let redacted = redact(
        "Access denied for user 'root'@'host' (using password: s3cret)",
        PASSWORD,
    );

    assert_eq!(
        redacted,
        "Access denied for user 'root'@'host' (using password: <redacted>)"
    );
}

#[test]
fn redact_leaves_message_untouched_when_password_is_empty() {
    let redacted = redact("Access denied for user 'root'@'host'", "");

    assert_eq!(redacted, "Access denied for user 'root'@'host'");
}

/// `scan-udf/postgresql-scan` requires the password absent from a surfaced
/// message "in any encoding or escaping". A connection string holds it with
/// its quotes and backslashes escaped, so a strip that matched only the
/// verbatim spelling would leave that rendering — the one a message actually
/// carries — intact.
#[test]
fn redact_strips_the_password_in_its_escaped_rendering_too() {
    let escaped = "pa\\'b\\\\c d&e";
    assert_ne!(
        escaped, AWKWARD_PASSWORD,
        "the fixture must exercise a password the escaping changes"
    );

    let redacted = redact(&format!("password='{escaped}'"), AWKWARD_PASSWORD);

    assert!(
        !redacted.contains(escaped) && !redacted.contains(AWKWARD_PASSWORD),
        "the escaped rendering survived the strip: {redacted}"
    );
}

/// No driver's failure subject may be the value the driver was configured
/// with, because for one of them that value carries the credential.
///
/// Run once per identity rather than once for PostgreSQL: the rule is a
/// property of the open path, so a technology added later inherits it instead
/// of needing its own copy — which is what would have let PostgreSQL's
/// conninfo become a subject in the first place.
#[test]
fn no_drivers_failure_subject_carries_the_credential() {
    for driver in Driver::ALL {
        let connection = SourceConnection {
            uri: format!("{}://10.0.0.5:5432/adbc_vs", driver.as_str()),
            username: "appuser".to_string(),
            password: AWKWARD_PASSWORD.to_string(),
        };
        let target = database_target(*driver, &connection)
            .unwrap_or_else(|error| panic!("`{}`'s target is built: {error}", driver.as_str()));

        assert!(
            !target.subject.contains(AWKWARD_PASSWORD),
            "`{}`'s failure subject carries the password: {}",
            driver.as_str(),
            target.subject
        );
        assert!(
            target.subject.contains("10.0.0.5") && target.subject.contains("adbc_vs"),
            "`{}`'s failure subject must still name the host and the database: {}",
            driver.as_str(),
            target.subject
        );
    }
}

/// Redaction is not a step a caller can forget: the failure is assembled and
/// stripped in one place, so a subject or a driver message that carried the
/// secret cannot reach a message by being formatted before the strip.
#[test]
fn a_failure_message_never_contains_the_password() {
    let source = fake_source_error("Access denied for user 'root'@'host' (using password: s3cret)");
    let diagnostic = OpenDiagnostic::new("mysql://10.0.0.5:3306/adbc_vs", PASSWORD);

    let message = diagnostic
        .failure("opening an ADBC connection to", &source)
        .to_string();

    assert!(
        !message.contains(PASSWORD),
        "error message leaked the password: {message}"
    );
    assert!(message.contains("mysql://10.0.0.5:3306/adbc_vs"));
    assert!(message.contains("opening an ADBC connection to"));
}

// ---------------------------------------------------------------------------
// The MySQL mechanism `scan-udf/mysql-scan` owns
// ---------------------------------------------------------------------------

/// MySQL takes the user and password as ADBC options of their own, so neither
/// has to be concatenated into a connection string that SQL text, a log line,
/// or an audit table could then carry.
#[test]
fn mysql_passes_credentials_as_separate_options() {
    let target = database_target(Driver::MySql, &connection()).expect("MySQL's target is built");
    let options = target.options;

    assert_eq!(options[0].0, OptionDatabase::Uri);
    assert_eq!(value_as_str(&options[0].1), "mysql://10.0.0.5:3306/adbc_vs");
    assert_eq!(options[1].0, OptionDatabase::Username);
    assert_eq!(value_as_str(&options[1].1), "root");
    assert_eq!(options[2].0, OptionDatabase::Password);
    assert_eq!(value_as_str(&options[2].1), PASSWORD);
    assert_eq!(options.len(), 3);
}

#[test]
fn mysql_connection_string_carries_no_credential() {
    let target = database_target(Driver::MySql, &connection()).expect("MySQL's target is built");

    let uri = value_as_str(&target.options[0].1);
    assert!(!uri.contains(PASSWORD));
    assert!(!uri.contains('@'));
}

/// MySQL's connection-failure subject stays the address itself, which carries
/// no credential, so a wrong-password failure still names the whole target.
#[test]
fn mysql_names_the_address_as_the_failure_subject() {
    let target = database_target(Driver::MySql, &connection()).expect("MySQL's target is built");

    assert_eq!(target.subject, "mysql://10.0.0.5:3306/adbc_vs");
}

// ---------------------------------------------------------------------------
// The PostgreSQL mechanism `scan-udf/postgresql-scan` owns
// ---------------------------------------------------------------------------

/// `scan-udf/postgresql-scan`: the driver accepts `uri` and rejects `username`
/// and `password`, so the credential travels inside that one option's value as
/// a libpq keyword/value connection string built from the address.
#[test]
fn postgres_passes_one_conninfo_option_and_no_credential_option() {
    let connection = postgres_connection("postgresql://host:5432/appdb", PASSWORD);
    let target = database_target(Driver::Postgres, &connection).expect("the target is built");

    assert_eq!(target.options.len(), 1);
    assert_eq!(target.options[0].0, OptionDatabase::Uri);
    for (option, _) in &target.options {
        assert_ne!(*option, OptionDatabase::Username);
        assert_ne!(*option, OptionDatabase::Password);
    }
}

/// The host, port, and database come from the address, every parameter its
/// query string carries crosses over as a keyword of the same name so a
/// `sslmode` an operator set is not dropped, and the credential comes from the
/// `CONNECTION` object rather than from the address.
#[test]
fn postgres_builds_a_conninfo_from_the_address_and_the_credential() {
    assert_eq!(
        conninfo("postgresql://host:5432/appdb?sslmode=require", PASSWORD),
        "host='host' port='5432' dbname='appdb' sslmode='require' user='appuser' \
         password='s3cret'"
    );
}

/// `scan-udf/postgresql-scan`: single-quoting a value and backslash-escaping
/// an embedded quote or backslash escapes any byte a password can hold, which
/// is why conninfo form was chosen over a URI whose percent-encoding would let
/// an `&` change the parse rather than the value.
#[test]
fn postgres_escapes_every_credential_byte() {
    let built = conninfo("postgresql://host:5432/appdb", AWKWARD_PASSWORD);

    assert_eq!(
        built,
        "host='host' port='5432' dbname='appdb' user='appuser' password='pa\\'b\\\\c d&e'"
    );
}

/// Every parameter crosses over, in the address's own order, so a
/// `connect_timeout` beside an `sslmode` reaches libpq as both.
#[test]
fn postgres_carries_every_address_parameter_across() {
    assert_eq!(
        conninfo(
            "postgresql://host:5432/appdb?sslmode=verify-full&connect_timeout=10",
            PASSWORD
        ),
        "host='host' port='5432' dbname='appdb' sslmode='verify-full' connect_timeout='10' \
         user='appuser' password='s3cret'"
    );
}

/// The `CONNECTION` object is the only source of a credential, so a credential
/// keyword written into the address is dropped rather than carried across and
/// overridden.
///
/// Carrying it bought nothing — libpq reads the last occurrence of a repeated
/// keyword, and the resolved credential is emitted last — while costing the
/// one disclosure this module exists to close: a driver echoing the connection
/// string back would name a secret [`redact`] cannot strip, because redaction
/// knows only the credential Exasol resolved.
#[test]
fn postgres_discards_a_credential_the_address_carries() {
    let built = conninfo(
        "postgresql://host:5432/appdb?user=someone&password=hunter2&sslpassword=keyphrase",
        PASSWORD,
    );

    assert_eq!(
        built, "host='host' port='5432' dbname='appdb' user='appuser' password='s3cret'",
        "no credential keyword of the address may reach libpq, and the resolved credential is \
         emitted last"
    );
}

/// Only the credential keywords are dropped. Every other parameter still
/// crosses over, in the address's own order, because an `sslmode` or a
/// `connect_timeout` is the operator's configuration rather than a secret.
#[test]
fn postgres_discards_only_the_credential_keywords() {
    assert_eq!(
        conninfo(
            "postgresql://host:5432/appdb?sslmode=verify-full&password=hunter2&connect_timeout=10",
            PASSWORD
        ),
        "host='host' port='5432' dbname='appdb' sslmode='verify-full' connect_timeout='10' \
         user='appuser' password='s3cret'"
    );
}

/// A keyword is matched however the address spelled its case, because libpq
/// itself reads conninfo keywords case-insensitively and a `PASSWORD=` that
/// slipped through would disclose exactly as a lower-case one does.
#[test]
fn postgres_discards_a_credential_keyword_whatever_its_case() {
    let built = conninfo(
        "postgresql://host:5432/appdb?PassWord=hunter2&SSLPassword=keyphrase",
        PASSWORD,
    );

    assert!(
        !built.contains("hunter2") && !built.contains("keyphrase"),
        "no spelling of a credential keyword may carry its value into the connection string: \
         {built}"
    );
}

/// A user and password written into the address are not the connection's
/// credential and are not part of its host, so the authority's userinfo is
/// dropped rather than parsed as a host name.
#[test]
fn postgres_reads_the_host_without_the_addresss_userinfo() {
    assert_eq!(
        conninfo("postgresql://someone:wrong@host:5432/appdb", PASSWORD),
        "host='host' port='5432' dbname='appdb' user='appuser' password='s3cret'"
    );
}

/// An address naming no port omits the keyword rather than inventing one, so
/// libpq applies its own default.
#[test]
fn postgres_omits_the_port_when_the_address_names_none() {
    assert_eq!(
        conninfo("postgresql://host/appdb", PASSWORD),
        "host='host' dbname='appdb' user='appuser' password='s3cret'"
    );
}

/// A bracketed IPv6 literal is a host whose colons are not a port separator.
/// libpq takes the address without its brackets.
#[test]
fn postgres_reads_a_bracketed_host_without_its_brackets() {
    assert_eq!(
        conninfo("postgresql://[2001:db8::1]:5432/appdb", PASSWORD),
        "host='2001:db8::1' port='5432' dbname='appdb' user='appuser' password='s3cret'"
    );
}

/// An address with no database path component cannot produce a `dbname`, and
/// libpq would silently connect to a database named after the user instead, so
/// it is refused. The refusal names the form it expected and never echoes the
/// address, which may carry a credential of its own.
#[test]
fn postgres_refuses_an_address_naming_no_database() {
    for address in ["postgresql://host:5432", "postgresql://host:5432/"] {
        let connection = postgres_connection(address, AWKWARD_PASSWORD);
        let Err(error) = database_target(Driver::Postgres, &connection) else {
            panic!("`{address}` names no database and must be refused");
        };

        let message = error.to_string();
        assert!(
            message.contains("database"),
            "the refusal must name the missing component: {message}"
        );
        assert!(
            !message.contains(AWKWARD_PASSWORD) && !message.contains(address),
            "the refusal must not echo the address it was given: {message}"
        );
    }
}

/// A conninfo keyword is a bare identifier rather than a quoted value, so an
/// address parameter whose name is not one would be handed to libpq as two
/// keywords. It is refused rather than carried across malformed or silently
/// dropped, since dropping it is what carrying every parameter exists to
/// prevent.
#[test]
fn postgres_refuses_an_address_parameter_that_is_not_a_keyword() {
    let connection = postgres_connection(
        "postgresql://host:5432/appdb?ssl mode=require",
        AWKWARD_PASSWORD,
    );

    let Err(error) = database_target(Driver::Postgres, &connection) else {
        panic!("a parameter name that is not a bare identifier must be refused");
    };

    let message = error.to_string();
    assert!(
        message.contains("ssl mode"),
        "the refusal must name the parameter it rejected: {message}"
    );
    assert!(
        !message.contains(AWKWARD_PASSWORD),
        "the refusal must not carry the credential: {message}"
    );
}

/// `scan-udf/postgresql-scan`: the connection value MUST NOT be the subject an
/// error names, because it carries the password verbatim. The host and the
/// database are named instead, so a wrong credential stays diagnosable.
#[test]
fn postgres_names_the_host_and_database_as_the_failure_subject() {
    let connection = postgres_connection(
        "postgresql://10.0.0.7:5432/appdb?sslmode=require",
        AWKWARD_PASSWORD,
    );
    let target = database_target(Driver::Postgres, &connection).expect("the target is built");

    assert!(
        target.subject.contains("10.0.0.7") && target.subject.contains("appdb"),
        "the subject must name the host and the database: {}",
        target.subject
    );
    assert!(
        !target.subject.contains(AWKWARD_PASSWORD) && !target.subject.contains("password="),
        "the subject must not be the connection value: {}",
        target.subject
    );
}

/// The redaction holds against a driver that echoes back the whole connection
/// string it was configured with — the case the adapter cannot rule out,
/// because libpq's own error text quotes connection parameters.
#[test]
fn postgres_redaction_survives_a_driver_error_carrying_the_connection_string() {
    let connection = postgres_connection(
        "postgresql://10.0.0.7:5432/appdb?sslmode=require",
        AWKWARD_PASSWORD,
    );
    let target = database_target(Driver::Postgres, &connection).expect("the target is built");
    let built = value_as_str(&target.options[0].1).to_string();
    let echoed = fake_source_error(&format!("[libpq] Failed to connect using `{built}`"));

    let message = OpenDiagnostic::new(&target.subject, &connection.password)
        .failure("opening an ADBC connection to", &echoed)
        .to_string();

    assert!(
        !message.contains(AWKWARD_PASSWORD),
        "the driver's echo leaked the password: {message}"
    );
    assert!(
        !message.contains(&built),
        "the connection value the adapter built survived into the message: {message}"
    );
    assert!(
        message.contains("10.0.0.7") && message.contains("appdb"),
        "the failure must still name the host and the database: {message}"
    );
}
