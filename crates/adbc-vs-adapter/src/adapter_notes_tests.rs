use super::*;

fn sample() -> AdapterNotes {
    AdapterNotes {
        driver: "mysql".to_string(),
        driver_version: Some("v0.6.0".to_string()),
        driver_path: "/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_mysql.so".to_string(),
        connection: "MY_MYSQL_CONNECTION".to_string(),
        database: "shop".to_string(),
        omitted: vec![
            Omission {
                table: "orders".to_string(),
                column: Some("payload".to_string()),
                source_type: Some("json".to_string()),
                reason: "MySQL type JSON is not one of the eight types this release maps"
                    .to_string(),
            },
            Omission {
                table: "logs".to_string(),
                column: None,
                source_type: None,
                reason: "table has no mappable column".to_string(),
            },
        ],
    }
}

#[test]
fn round_trips_through_json() {
    let notes = sample();

    let parsed = AdapterNotes::parse(&notes.to_json_string()).expect("must parse what it wrote");

    assert_eq!(parsed.driver, notes.driver);
    assert_eq!(parsed.driver_version, notes.driver_version);
    assert_eq!(parsed.driver_path, notes.driver_path);
    assert_eq!(parsed.connection, notes.connection);
    assert_eq!(parsed.database, notes.database);
    assert_eq!(parsed.omitted.len(), 2);
    assert_eq!(parsed.omitted[0].column.as_deref(), Some("payload"));
    assert_eq!(parsed.omitted[1].column, None);
}

#[test]
fn an_omission_is_written_under_its_source_neutral_name() {
    let text = sample().to_json_string();

    assert!(
        text.contains("\"sourceType\":\"json\""),
        "an omission records the source's own type name: {text}"
    );
    assert!(
        !text.contains("mysqlType"),
        "the technology-specific spelling is read but never written: {text}"
    );
}

/// Exasol persists `adapterNotes` and hands them back on every later request,
/// so notes an earlier release wrote outlive the rename. A `refresh` or a
/// `setProperties` over such a schema reads them before it writes the new
/// spelling, and must not fail on the old one.
#[test]
fn omission_source_type_accepts_legacy_alias() {
    let recorded_by_release_0_2 = r#"{
        "driver": "mysql",
        "driverVersion": "v0.6.0",
        "driverPath": "/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_mysql.so",
        "connection": "MY_MYSQL_CONNECTION",
        "database": "shop",
        "omitted": [{"table": "orders", "column": "payload", "mysqlType": "json", "reason": "unmapped"}]
    }"#;

    let parsed = AdapterNotes::parse(recorded_by_release_0_2)
        .expect("notes written before the rename must still parse");

    assert_eq!(parsed.omitted[0].source_type.as_deref(), Some("json"));
}

/// A virtual schema whose operator supplied the driver library has no version
/// to record, and every later request reads its notes back — so the field is
/// left out on the way in and absent rather than a failure on the way out.
#[test]
fn notes_for_a_supplied_driver_carry_no_version_and_read_back() {
    let supplied = AdapterNotes {
        driver_version: None,
        driver_path: "/buckets/bfsdefault/default/mine/libadbc_driver_mysql.so".to_string(),
        ..sample()
    };

    let text = supplied.to_json_string();

    assert!(
        !text.contains("driverVersion"),
        "a version adbc-vs never resolved must not be written at all, not even as null: \
         {text}"
    );
    let parsed = AdapterNotes::parse(&text).expect("notes recording no driver version must parse");
    assert_eq!(parsed.driver_version, None);
    assert_eq!(parsed.driver_path, supplied.driver_path);
}

#[test]
fn json_never_carries_a_credential_field() {
    let text = sample().to_json_string().to_lowercase();
    assert!(!text.contains("password"));
    assert!(!text.contains("\"user\""));
}

#[test]
fn malformed_json_names_the_parse_failure() {
    let error = AdapterNotes::parse("not json").unwrap_err();
    assert!(error.contains("adapter notes"));
}
