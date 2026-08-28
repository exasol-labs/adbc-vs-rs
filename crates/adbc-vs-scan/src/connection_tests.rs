use adbc_core::error::{Error as AdbcError, Status};
use adbc_core::options::{OptionDatabase, OptionValue};
use std::os::raw::c_char;

use super::connection::*;

fn value_as_str(value: &OptionValue) -> &str {
    match value {
        OptionValue::String(text) => text,
        other => panic!("expected OptionValue::String, got {other:?}"),
    }
}

fn connection() -> MysqlConnection {
    MysqlConnection {
        uri: "mysql://10.0.0.5:3306/adbc_vs".to_string(),
        username: "root".to_string(),
        password: "s3cret".to_string(),
    }
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

#[test]
fn database_options_carries_uri_username_password_as_separate_keys() {
    let options = database_options(&connection());

    assert_eq!(options[0].0, OptionDatabase::Uri);
    assert_eq!(value_as_str(&options[0].1), "mysql://10.0.0.5:3306/adbc_vs");
    assert_eq!(options[1].0, OptionDatabase::Username);
    assert_eq!(value_as_str(&options[1].1), "root");
    assert_eq!(options[2].0, OptionDatabase::Password);
    assert_eq!(value_as_str(&options[2].1), "s3cret");
    assert_eq!(options.len(), 3);
}

#[test]
fn database_options_never_embeds_the_credential_in_the_uri() {
    let options = database_options(&connection());

    let uri = value_as_str(&options[0].1);
    assert!(!uri.contains("s3cret"));
    assert!(!uri.contains('@'));
}

#[test]
fn missing_driver_path_named_in_error() {
    let result = open_mysql_connection("/nonexistent/path/libadbc_driver_mysql.so", &connection());

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
        !message.contains("s3cret"),
        "error message leaked the password"
    );
}

#[test]
fn redact_replaces_every_password_occurrence() {
    let redacted = redact(
        "Access denied for user 'root'@'host' (using password: s3cret)",
        "s3cret",
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

#[test]
fn connect_error_message_never_contains_the_password() {
    let source = fake_source_error("Access denied for user 'root'@'host' (using password: s3cret)");

    let error = connect_error(
        "opening an ADBC connection to",
        "mysql://10.0.0.5:3306/adbc_vs",
        redact(&source.to_string(), "s3cret"),
    );

    let message = error.to_string();
    assert!(
        !message.contains("s3cret"),
        "error message leaked the password"
    );
    assert!(message.contains("mysql://10.0.0.5:3306/adbc_vs"));
    assert!(message.contains("opening an ADBC connection to"));
}
