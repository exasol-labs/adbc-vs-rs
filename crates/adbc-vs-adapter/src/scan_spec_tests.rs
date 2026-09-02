use adbc_vs_scan::driver::Driver;

use super::*;
use crate::exa_type::ExasolType;

fn sample() -> ScanSpec {
    ScanSpec {
        driver: Driver::MySql,
        driver_path: "/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_mysql.so".to_string(),
        connection: "MY_MYSQL_CONNECTION".to_string(),
        sql: "SELECT `id` FROM `shop`.`orders`".to_string(),
        columns: vec![ScanColumn {
            name: "id".to_string(),
            exasol_type: ExasolType::Decimal {
                precision: 19,
                scale: 0,
            },
        }],
    }
}

#[test]
fn round_trips_through_json() {
    let spec = sample();

    let text = spec.to_json_string();
    let parsed = parse(&text).expect("a spec this module wrote must parse");

    assert_eq!(parsed.driver, spec.driver);
    assert_eq!(parsed.driver_path, spec.driver_path);
    assert_eq!(parsed.connection, spec.connection);
    assert_eq!(parsed.sql, spec.sql);
    assert_eq!(parsed.columns.len(), 1);
    assert_eq!(parsed.columns[0].name, "id");
    assert_eq!(parsed.columns[0].exasol_type, spec.columns[0].exasol_type);
}

/// `vs-adapter/driver-selection`: the specification records which technology
/// the scan loads, beside the path it loads from, so the scan reads its
/// entry-point symbol off the identity rather than guessing it from the
/// library's file name.
#[test]
fn scan_spec_carries_driver_identity() {
    let spec = sample();

    let text = spec.to_json_string();
    assert!(
        text.contains("\"driver\":\"mysql\""),
        "the specification must record the driver identity: {text}"
    );

    let parsed = parse(&text).expect("a spec this module wrote must parse");
    assert_eq!(parsed.driver, Driver::MySql);
    assert_eq!(
        parsed.driver.package().entry_point,
        Driver::MySql.package().entry_point,
        "the identity read back must resolve the same entry point the scan loads through"
    );
}

#[test]
fn json_carries_no_credential_field() {
    let text = sample().to_json_string();
    assert!(!text.to_lowercase().contains("password"));
    assert!(!text.to_lowercase().contains("username"));
}

#[test]
fn malformed_json_names_the_parse_failure() {
    let error = parse("not json").unwrap_err();
    assert!(error.contains("scan spec"));
}

/// The scan loads its library through the entry-point symbol the identity
/// names, so a specification recording a technology this release cannot load
/// must fail where it is read rather than at `dlopen`.
#[test]
fn a_specification_naming_an_unsupported_driver_names_it() {
    let text = sample().to_json_string().replace("\"mysql\"", "\"sybase\"");

    let error = parse(&text).expect_err("a driver this release does not ship must not parse back");

    assert!(
        error.contains("sybase"),
        "the error must name the driver the specification records: {error}"
    );
}
