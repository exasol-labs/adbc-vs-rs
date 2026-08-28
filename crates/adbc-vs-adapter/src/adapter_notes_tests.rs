use super::*;

fn sample() -> AdapterNotes {
    AdapterNotes {
        driver: "mysql".to_string(),
        driver_version: "v0.6.0".to_string(),
        driver_path: "/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_mysql.so".to_string(),
        connection: "MY_MYSQL_CONNECTION".to_string(),
        database: "shop".to_string(),
        omitted: vec![
            Omission {
                table: "orders".to_string(),
                column: Some("payload".to_string()),
                mysql_type: Some("json".to_string()),
                reason: "MySQL type JSON is not one of the eight types adbc-vs 0.1 maps"
                    .to_string(),
            },
            Omission {
                table: "logs".to_string(),
                column: None,
                mysql_type: None,
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
