use super::*;
use crate::exa_type::ExasolType;

fn sample() -> ScanSpec {
    ScanSpec {
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

    assert_eq!(parsed.driver_path, spec.driver_path);
    assert_eq!(parsed.connection, spec.connection);
    assert_eq!(parsed.sql, spec.sql);
    assert_eq!(parsed.columns.len(), 1);
    assert_eq!(parsed.columns[0].name, "id");
    assert_eq!(parsed.columns[0].exasol_type, spec.columns[0].exasol_type);
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
