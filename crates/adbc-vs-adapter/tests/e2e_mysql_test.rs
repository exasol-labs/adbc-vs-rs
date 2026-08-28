//! Drives adbc-vs end to end against the Docker Compose stack: an Exasol
//! virtual schema over the MySQL container, with the ADBC driver fetched from
//! the public registry into BucketFS on the way.
//!
//! There is no skip path. A stack that is down, a driver that will not
//! install, or a query that returns the wrong rows fails the run, because a
//! tier that quietly skips itself proves nothing.
//!
//! Every test shares one stack, one virtual schema, and the driver installed
//! into one BucketFS, which is why `make test-e2e` runs them with
//! `--test-threads=1`.
#![cfg(feature = "exasol-e2e")]

mod common;

use adbc_vs_adapter::exa_type::{self, ExasolType};
use common::exasol::cell;
use common::stack::{self, MYSQL_CONNECTION, VIRTUAL_SCHEMA};
use common::{items, mirror, mysql, probe, schema};

/// Where the pinned driver is installed, in the two forms the test asserts
/// against: the bucket-relative path BucketFS lists it at, and the file path
/// the adapter records for a UDF to open.
const DRIVER_BUCKET_PATH: &str = "adbc-drivers/libadbc_driver_mysql.so";
const DRIVER_UDF_PATH: &str = "/buckets/bfsdefault/default/adbc-drivers/libadbc_driver_mysql.so";

/// A lower bound distinguishing an installed driver package from a stray
/// placeholder file: the pinned `libadbc_driver_mysql.so` measured 46,408,368
/// bytes (about 44.3 MiB) on the registry this release pins, so any file
/// large enough to be the real library clears this by two orders of
/// magnitude.
const MIN_PLAUSIBLE_DRIVER_BYTES: u64 = 1_000_000;

/// How many times the ordering tier repeats a query before trusting its
/// stability. Decision-log evidence measured a query lacking Exasol's own
/// `ORDER BY` reordering 48.8% of rows when joined to a local table, so an
/// unstable order shuffles roughly every other row rather than rarely — a
/// second run already makes a coincidental match implausible, and this third
/// repetition guards against that one coincidence.
const STABILITY_RUN_COUNT: u32 = 3;

/// This test deletes the driver every other test in this file reads, so it
/// puts the stack back as it found it before it asserts anything: its own
/// `CREATE VIRTUAL SCHEMA` is the reinstall, and an assertion panicking ahead
/// of the drop would leave a schema behind for the next run to inherit.
/// Nothing here may depend on the order libtest happens to run these in.
///
/// `stack.exasol.bucketfs_write_url()` always names an `https://` endpoint,
/// so a successful install is also the proof that the PUT succeeded against
/// the container's self-signed certificate: a client rejecting it would fail
/// here rather than silently falling back to plain HTTP.
#[test]
fn first_use_installs_driver_from_registry() {
    let stack = stack::shared();
    stack.exasol.delete_file(DRIVER_BUCKET_PATH);
    let before_install = stack.exasol.bucket_file_size(DRIVER_BUCKET_PATH);

    schema::create_virtual_schema(&stack.exasol, "FIRSTUSE", MYSQL_CONNECTION);

    let installed = stack.exasol.bucket_file_size(DRIVER_BUCKET_PATH);
    schema::drop_virtual_schema(&stack.exasol, "FIRSTUSE");

    assert_eq!(
        before_install, None,
        "the driver must be gone before the install under test"
    );
    let installed = installed.expect(
        "the first CREATE VIRTUAL SCHEMA must install the driver, which requires the PUT to \
         BucketFS to succeed against the container's self-signed certificate",
    );
    assert!(
        installed > MIN_PLAUSIBLE_DRIVER_BYTES,
        "the installed driver is {installed} bytes, which is not the fetched package's library"
    );
}

#[test]
fn driver_lands_in_bucketfs() {
    let stack = stack::shared();

    let entries = stack.exasol.bucket_entries();

    assert!(
        entries.iter().any(|entry| entry == DRIVER_BUCKET_PATH),
        "BucketFS lists {entries:?}, which does not include the installed driver"
    );
    assert_eq!(
        adapter_notes(stack)["driverPath"],
        DRIVER_UDF_PATH,
        "the recorded driver path must be the path a UDF opens"
    );
}

/// Provisioning reads its `BUCKETFS_CONNECTION` only after the presence check
/// misses, so a second schema created through a BucketFS endpoint nothing
/// answers proves the driver was reused rather than fetched and written again:
/// had provisioning re-run, the upload would have failed the statement.
#[test]
fn second_schema_reuses_driver() {
    let stack = stack::shared();
    stack.exasol.execute(
        "CREATE OR REPLACE CONNECTION ADBC_VS_BUCKETFS_CLOSED TO 'https://127.0.0.1:1/default' \
         USER 'w' IDENTIFIED BY 'irrelevant'",
    );

    schema::create_virtual_schema_through(
        &stack.exasol,
        schema::VirtualSchemaSpec {
            schema: "SECONDVS",
            connection: MYSQL_CONNECTION,
            bucketfs_connection: "ADBC_VS_BUCKETFS_CLOSED",
        },
    );

    let rows = stack
        .exasol
        .query_rows("SELECT COUNT(*) AS N FROM SECONDVS.\"orders\"");
    schema::drop_virtual_schema(&stack.exasol, "SECONDVS");
    schema::drop_connection(&stack.exasol, "ADBC_VS_BUCKETFS_CLOSED");

    assert_eq!(cell(&rows[0], "N"), Some("3".to_string()));
}

#[test]
fn select_all_returns_seeded_values() {
    let stack = stack::shared();

    let rows = stack.exasol.query_rows(&format!(
        "SELECT * FROM {VIRTUAL_SCHEMA}.\"orders\" ORDER BY \"id\""
    ));

    assert_eq!(rows.len(), 3, "the fixture seeds three rows");
    assert_eq!(cell(&rows[0], "id"), Some("1".to_string()));
    assert_eq!(cell(&rows[0], "big_id"), Some("9000000001".to_string()));
    assert_eq!(cell(&rows[0], "amount"), Some("1234.56".to_string()));
    assert_eq!(cell(&rows[0], "name"), Some("Widget".to_string()));
    assert_eq!(cell(&rows[0], "code"), Some("CODE0001".to_string()));
    assert_eq!(cell(&rows[0], "order_date"), Some("2026-01-15".to_string()));
    assert_eq!(
        cell(&rows[0], "created_at"),
        Some("2026-01-15T08:30:00".to_string())
    );
    assert_eq!(cell(&rows[0], "price"), Some("19.99".to_string()));

    assert_eq!(
        cell(&rows[1], "amount"),
        None,
        "the seeded NULL DECIMAL must survive coercion as a NULL"
    );
    assert_eq!(
        cell(&rows[2], "amount"),
        Some("99999999.9999".to_string()),
        "the seeded maximum-scale DECIMAL must survive coercion exactly"
    );
}

#[test]
fn projection_reaches_mysql() {
    let stack = stack::shared();

    let rows = stack.exasol.query_rows(&format!(
        "SELECT \"id\", \"name\" FROM {VIRTUAL_SCHEMA}.\"orders\" ORDER BY \"id\""
    ));

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].len(), 2, "only the two named columns come back");
    assert_eq!(cell(&rows[0], "name"), Some("Widget".to_string()));

    let pushed_down = pushdown_statement(&format!(
        "SELECT \"id\", \"name\" FROM {VIRTUAL_SCHEMA}.\"orders\""
    ));
    assert!(
        pushed_down.contains("SELECT `id`, `name` FROM `adbc_vs`.`orders`"),
        "the statement sent to MySQL must project only the named columns: {pushed_down}"
    );
}

/// The shipped `MySqlDialect` deliberately withholds `FN_AGG_SUM`,
/// `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT`
/// (decision-log `[implement-gate] Delegated AVG returns a different value
/// from Exasol's own AVG`), so no query a shipped virtual schema delegates
/// can make Exasol declare a `SUM` or `AVG` `selectListDataTypes` entry. Both
/// tests below reach Exasol's compiler through [`common::probe`] instead, a
/// scratch adapter that advertises those four capabilities anyway, so the
/// declared types this task pins are measured against the deployed stack
/// rather than assumed.
#[test]
fn exasol_supplies_the_measured_aggregate_types() {
    let stack = stack::shared();
    probe::create(&stack.exasol);

    let declared = probe::measured_select_list_data_types(
        &stack.exasol,
        &format!(
            "SELECT SUM(\"d\"), SUM(\"e\"), SUM(\"f\"), AVG(\"d\"), SUM(DISTINCT \"d\"), \
             COUNT(*), COUNT(\"d\"), COUNT(DISTINCT \"d\"), MIN(\"d\"), MAX(\"d\") \
             FROM {}.\"t\" GROUP BY \"g\"",
            probe::VIRTUAL_SCHEMA
        ),
    );
    probe::drop_virtual_schema(&stack.exasol);
    let types: Vec<ExasolType> = declared
        .iter()
        .map(|value| exa_type::from_json(value).expect("every entry is a declarable type"))
        .collect();

    assert_eq!(
        types[0],
        ExasolType::Decimal {
            precision: 23,
            scale: 4
        },
        "SUM over DECIMAL(12,4) must carry DECIMAL(min(p+11,36),s): {declared:?}"
    );
    assert_eq!(
        types[1],
        ExasolType::Decimal {
            precision: 36,
            scale: 2
        },
        "SUM over DECIMAL(30,2) must carry DECIMAL(min(p+11,36),s) capped at 36: {declared:?}"
    );
    assert_eq!(
        types[2],
        ExasolType::Double,
        "SUM over DOUBLE must carry DOUBLE: {declared:?}"
    );
    assert_eq!(
        types[3],
        ExasolType::Double,
        "AVG must carry DOUBLE for every numeric operand: {declared:?}"
    );
    assert_eq!(
        types[4], types[0],
        "SUM(DISTINCT) must carry the same type as the non-distinct form: {declared:?}"
    );
    let count_type = ExasolType::Decimal {
        precision: 18,
        scale: 0,
    };
    assert_eq!(
        types[5], count_type,
        "COUNT(*) must carry DECIMAL(18,0): {declared:?}"
    );
    assert_eq!(
        types[6], count_type,
        "COUNT(column) must carry DECIMAL(18,0): {declared:?}"
    );
    assert_eq!(
        types[7], count_type,
        "COUNT(DISTINCT) must carry DECIMAL(18,0): {declared:?}"
    );
    let operand_type = ExasolType::Decimal {
        precision: 12,
        scale: 4,
    };
    assert_eq!(
        types[8], operand_type,
        "MIN must carry the operand's own type: {declared:?}"
    );
    assert_eq!(
        types[9], operand_type,
        "MAX must carry the operand's own type: {declared:?}"
    );
}

#[test]
fn exasol_supplies_the_measured_type_shapes() {
    let stack = stack::shared();
    probe::create(&stack.exasol);

    let declared = probe::measured_select_list_data_types(
        &stack.exasol,
        &format!(
            "SELECT \"h\", MIN(\"g\"), COUNT(*), \"h\" < TIMESTAMP'2030-01-01 00:00:00' \
             FROM {}.\"t\" GROUP BY \"h\"",
            probe::VIRTUAL_SCHEMA
        ),
    );
    probe::drop_virtual_schema(&stack.exasol);

    let timestamp_entry = &declared[0];
    assert_eq!(
        exa_type::from_json(timestamp_entry),
        Ok(ExasolType::Timestamp),
        "a TIMESTAMP entry must still parse as the declarable Timestamp type: {declared:?}"
    );
    assert_eq!(
        timestamp_entry.get("fractionalSecondsPrecision"),
        Some(&serde_json::json!(3)),
        "a TIMESTAMP entry must carry fractionalSecondsPrecision 3: {declared:?}"
    );

    let varchar_entry = &declared[1];
    assert_eq!(
        exa_type::from_json(varchar_entry),
        Ok(ExasolType::Varchar { size: 100 }),
        "a VARCHAR entry must still carry its size: {declared:?}"
    );
    assert!(
        varchar_entry.get("characterSet").is_none(),
        "unlike involvedTables[].columns[].dataType, a selectListDataTypes VARCHAR entry must \
         carry no characterSet field: {declared:?}"
    );

    let predicate_entry = &declared[3];
    assert_eq!(
        exa_type::from_json(predicate_entry),
        Ok(ExasolType::Boolean),
        "a select-list predicate's entry must carry BOOLEAN: {declared:?}"
    );
}

#[test]
fn all_request_types_return_matching_type() {
    let stack = stack::shared();
    schema::create_mysql_connection(
        &stack.exasol,
        "ADBC_VS_DISPATCH_EMPTY",
        mysql::EMPTY_DATABASE,
    );

    schema::create_virtual_schema(&stack.exasol, "DISPATCHVS", MYSQL_CONNECTION);
    stack
        .exasol
        .execute("ALTER VIRTUAL SCHEMA DISPATCHVS REFRESH");
    let rows = stack
        .exasol
        .query_rows("SELECT COUNT(*) AS N FROM DISPATCHVS.\"orders\"");
    stack
        .exasol
        .execute("ALTER VIRTUAL SCHEMA DISPATCHVS SET CONNECTION_NAME = 'ADBC_VS_DISPATCH_EMPTY'");
    let tables_after_set = table_names("DISPATCHVS");
    stack
        .exasol
        .execute("DROP VIRTUAL SCHEMA DISPATCHVS CASCADE");
    schema::drop_connection(&stack.exasol, "ADBC_VS_DISPATCH_EMPTY");

    assert_eq!(
        cell(&rows[0], "N"),
        Some("3".to_string()),
        "getCapabilities and pushdown both ran to produce this count"
    );
    assert!(
        tables_after_set.is_empty(),
        "setProperties must re-discover against the CONNECTION_NAME the statement changed, \
         which addresses a database holding no table: {tables_after_set:?}"
    );
}

/// The refresh under test runs against the virtual schema every other test in
/// this file reads, so this one takes the column back out of both sides
/// before it asserts: dropping the MySQL column without a second refresh
/// would leave the shared schema permanently declaring a column the source no
/// longer has, which is a defect for whichever test reads it next rather than
/// for this one.
#[test]
fn refresh_picks_up_new_column() {
    let stack = stack::shared();
    let before = column_names("refresh_target");

    stack
        .mysql
        .execute("ALTER TABLE refresh_target ADD COLUMN extra VARCHAR(20) NULL");
    stack
        .exasol
        .execute(&format!("ALTER VIRTUAL SCHEMA {VIRTUAL_SCHEMA} REFRESH"));
    let after_add = column_names("refresh_target");

    stack
        .mysql
        .execute("ALTER TABLE refresh_target DROP COLUMN extra");
    stack
        .exasol
        .execute(&format!("ALTER VIRTUAL SCHEMA {VIRTUAL_SCHEMA} REFRESH"));
    let after_drop = column_names("refresh_target");

    assert!(
        !before.contains(&"extra".to_string()),
        "the fixture must not already carry the column the refresh is to discover: {before:?}"
    );
    assert!(
        after_add.contains(&"extra".to_string()),
        "a refresh must re-run discovery and expose the new source column: {after_add:?}"
    );
    assert_eq!(
        after_drop, before,
        "the second refresh must leave the shared virtual schema declaring exactly the columns \
         the source still has"
    );
}

#[test]
fn create_exposes_source_tables() {
    let tables = table_names(VIRTUAL_SCHEMA);

    assert!(
        tables.contains(&"orders".to_string()) && tables.contains(&"customers".to_string()),
        "both core-type tables must be exposed: {tables:?}"
    );
    assert_eq!(
        column_names("customers"),
        vec!["id", "name", "score"],
        "columns must keep the source's ordinal position order"
    );
    let types = column_types("orders");
    assert_eq!(types["id"], "DECIMAL(10,0)");
    assert_eq!(types["amount"], "DECIMAL(12,4)");
    assert_eq!(types["name"], "VARCHAR(100) UTF8");
    assert_eq!(types["code"], "VARCHAR(8) UTF8");
    assert_eq!(types["order_date"], "DATE");
    assert!(types["created_at"].starts_with("TIMESTAMP"));
    assert_eq!(types["price"], "DOUBLE");
}

#[test]
fn identifiers_keep_original_case() {
    let tables = table_names(VIRTUAL_SCHEMA);

    assert!(
        tables.contains(&"orders".to_string()),
        "a lower-case MySQL table must stay lower case: {tables:?}"
    );
    assert!(
        !tables.contains(&"ORDERS".to_string()),
        "no identifier may be folded to upper case: {tables:?}"
    );
    assert_eq!(column_names("orders")[0], "id");
}

#[test]
fn views_are_exposed() {
    let stack = stack::shared();

    let tables = table_names(VIRTUAL_SCHEMA);

    assert!(
        tables.contains(&"order_names".to_string()),
        "a MySQL view must be exposed alongside base tables: {tables:?}"
    );
    let rows = stack.exasol.query_rows(&format!(
        "SELECT COUNT(*) AS N FROM {VIRTUAL_SCHEMA}.\"order_names\""
    ));
    assert_eq!(cell(&rows[0], "N"), Some("3".to_string()));
}

#[test]
fn adapter_notes_hold_state_not_credentials() {
    let stack = stack::shared();

    let notes = adapter_notes(stack);

    assert_eq!(notes["driver"], "mysql");
    assert_eq!(notes["driverVersion"], "v0.6.0");
    assert_eq!(notes["driverPath"], DRIVER_UDF_PATH);
    assert_eq!(notes["connection"], MYSQL_CONNECTION);
    assert_eq!(notes["database"], mysql::DATABASE);

    let rendered = notes.to_string();
    assert!(
        !rendered.contains(mysql::PASSWORD)
            && !rendered.contains(stack.exasol.bucketfs_write_password()),
        "adapter notes must carry no credential"
    );
}

#[test]
fn unmappable_column_omitted_and_recorded() {
    let stack = stack::shared();

    let columns = column_names("mixed_types");

    assert_eq!(
        columns,
        vec!["id", "label"],
        "the JSON column must be left out of the virtual table"
    );
    let omitted = adapter_notes(stack)["omitted"].clone();
    let entry = omitted
        .as_array()
        .expect("omitted is an array")
        .iter()
        .find(|entry| entry["column"] == "payload")
        .expect("the omitted JSON column must be recorded");
    assert_eq!(entry["table"], "mixed_types");
    assert_eq!(entry["mysqlType"], "json");
    assert!(
        entry["reason"].as_str().is_some_and(|r| !r.is_empty()),
        "the omission must carry a reason: {entry}"
    );
}

#[test]
fn latin1_column_omitted_and_recorded() {
    let stack = stack::shared();

    let columns = column_names("items");

    assert!(
        !columns.contains(&"region".to_string()),
        "the latin1 column must be left out of the virtual table: {columns:?}"
    );
    let omitted = adapter_notes(stack)["omitted"].clone();
    let entry = omitted
        .as_array()
        .expect("omitted is an array")
        .iter()
        .find(|entry| entry["table"] == "items" && entry["column"] == "region")
        .expect("the omitted latin1 column must be recorded");
    assert_eq!(entry["mysqlType"], "varchar");
    assert!(
        entry["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("latin1")),
        "the reason must name the column's character set: {entry}"
    );
}

#[test]
fn fully_unmappable_table_omitted() {
    let stack = stack::shared();

    let tables = table_names(VIRTUAL_SCHEMA);

    assert!(
        !tables.contains(&"unmapped_only".to_string()),
        "a table with no mappable column must not be exposed: {tables:?}"
    );
    let omitted = adapter_notes(stack)["omitted"].clone();
    assert!(
        omitted
            .as_array()
            .expect("omitted is an array")
            .iter()
            .any(|entry| entry["table"] == "unmapped_only" && entry["column"].is_null()),
        "the omitted table must be recorded: {omitted}"
    );
}

#[test]
fn empty_database_yields_empty_tables() {
    let stack = stack::shared();
    schema::create_mysql_connection(&stack.exasol, "ADBC_VS_EMPTY", mysql::EMPTY_DATABASE);

    schema::create_virtual_schema(&stack.exasol, "EMPTYVS", "ADBC_VS_EMPTY");

    let tables = table_names("EMPTYVS");
    schema::drop_virtual_schema(&stack.exasol, "EMPTYVS");
    schema::drop_connection(&stack.exasol, "ADBC_VS_EMPTY");

    assert!(
        tables.is_empty(),
        "a source database with no table yields no virtual table: {tables:?}"
    );
}

#[test]
fn scan_resolves_connection_object() {
    let stack = stack::shared();
    schema::create_mysql_connection(&stack.exasol, "ADBC_VS_ROTATING", mysql::DATABASE);
    schema::create_virtual_schema(&stack.exasol, "ROTATINGVS", "ADBC_VS_ROTATING");
    let query = "SELECT COUNT(*) AS N FROM ROTATINGVS.\"orders\"";
    let before_rotation = cell(&stack.exasol.query_rows(query)[0], "N");

    stack.exasol.execute(&format!(
        "CREATE OR REPLACE CONNECTION ADBC_VS_ROTATING TO '{}' USER '{}' IDENTIFIED BY 'wrong'",
        mysql::driver_uri(mysql::DATABASE),
        mysql::USER
    ));
    let error = stack.exasol.execute_expecting_failure(query);

    schema::create_mysql_connection(&stack.exasol, "ADBC_VS_ROTATING", mysql::DATABASE);
    let after_restore = cell(&stack.exasol.query_rows(query)[0], "N");
    schema::drop_virtual_schema(&stack.exasol, "ROTATINGVS");
    schema::drop_connection(&stack.exasol, "ADBC_VS_ROTATING");

    assert_eq!(before_rotation, Some("3".to_string()));
    assert!(
        !error.contains("wrong") && !error.contains(mysql::PASSWORD),
        "the scan's failure must not echo a credential: {error}"
    );
    assert_eq!(
        after_restore,
        Some("3".to_string()),
        "restoring the CONNECTION object must restore the scan, which proves the scan \
         resolves it per statement rather than at creation time"
    );
}

#[test]
fn unreachable_source_fails_create() {
    let stack = stack::shared();
    schema::create_connection_to(&stack.exasol, "ADBC_VS_CLOSED", &mysql::unreachable_uri());

    let error = schema::create_virtual_schema_expecting_failure(
        &stack.exasol,
        "CLOSEDVS",
        "ADBC_VS_CLOSED",
    );

    let left_behind = virtual_schema_exists("CLOSEDVS");
    schema::drop_connection(&stack.exasol, "ADBC_VS_CLOSED");

    assert!(
        !left_behind,
        "a failed CREATE VIRTUAL SCHEMA must leave no schema behind"
    );
    assert!(
        !error.contains(mysql::PASSWORD),
        "the failure must not echo the MySQL password: {error}"
    );
    assert!(
        error.contains(&mysql::unreachable_uri()),
        "the error surfaced to the session must name the address it could not reach: {error}"
    );
}

/// The mirror is the oracle every differential test compares a delegated query
/// against, so these four tests establish that it is a valid one before any
/// such comparison rests on it. They assert, in order: that every fixture value
/// survived the seed into Exasol byte for byte, that the join partner's
/// unmatched rows on both sides survived with it, that the mirror declares the
/// same Exasol types under the same column names as the virtual table it
/// mirrors, and that Exasol's own answers over the mirror are the ones the
/// delegated path has to reproduce rather than MySQL's defaults. Without the
/// last of those, a differential test comparing two engines that both fold
/// `Widget` and `widget` together would agree and prove nothing.
#[test]
fn mirror_holds_every_fixture_item_row_as_authored() {
    let stack = stack::shared();

    let rows = stack.exasol.query_rows(&format!(
        "SELECT \"id\", \"category\", \"code\", \"batch\", \"quantity\", \"rank_key\", \
         \"tie_break\" FROM {}.\"items\" ORDER BY \"id\"",
        mirror::SCHEMA
    ));

    assert_eq!(
        rows.len(),
        items::ROWS.len(),
        "the mirror must hold every seeded row"
    );
    for (row, expected) in rows.iter().zip(items::ROWS.iter()) {
        let id = expected.id;
        assert_eq!(cell(row, "id"), Some(id.to_string()));
        assert_eq!(
            cell(row, "category"),
            Some(expected.category.to_string()),
            "row {id}'s category must reach Exasol byte for byte: a case or accent difference \
             from the source turns every grouping comparison into a false alarm"
        );
        assert_eq!(
            cell(row, "batch"),
            Some(expected.batch.to_string()),
            "row {id}'s batch must keep its trailing space, which decision [6] measured MIN \
             diverging over; a trimmed value would make that test pass whatever collation is \
             rendered"
        );
        assert_eq!(cell(row, "code"), Some(expected.code.to_string()));
        assert_eq!(
            cell(row, "quantity"),
            expected.quantity.map(|quantity| quantity.to_string()),
            "row {id}'s quantity must stay null where the source is null, so a projected \
             predicate over it has a null to produce"
        );
        assert_eq!(cell(row, "rank_key"), Some(expected.rank_key.to_string()));
        assert_eq!(cell(row, "tie_break"), Some(expected.tie_break.to_string()));
    }
}

#[test]
fn mirror_holds_every_join_partner_row_including_the_unmatched_ones() {
    let stack = stack::shared();

    let rows = stack.exasol.query_rows(&format!(
        "SELECT \"item_id\", \"note\" FROM {}.\"item_notes\" ORDER BY \"item_id\"",
        mirror::SCHEMA
    ));

    assert_eq!(
        rows.len(),
        items::NOTES.len(),
        "the mirror must hold every seeded join-partner row"
    );
    for (row, expected) in rows.iter().zip(items::NOTES.iter()) {
        assert_eq!(cell(row, "item_id"), Some(expected.item_id.to_string()));
        assert_eq!(cell(row, "note"), Some(expected.note.to_string()));
    }

    let unmatched = stack.exasol.query_scalar(&format!(
        "SELECT COUNT(*) FROM {schema}.\"items\" i LEFT JOIN {schema}.\"item_notes\" n \
         ON i.\"id\" = n.\"item_id\" WHERE n.\"item_id\" IS NULL",
        schema = mirror::SCHEMA
    ));
    assert_eq!(
        unmatched,
        Some("1".to_string()),
        "exactly one mirrored item must carry no note, so an outer join has a null side to \
         return"
    );
    let orphaned = stack.exasol.query_scalar(&format!(
        "SELECT COUNT(*) FROM {schema}.\"item_notes\" n LEFT JOIN {schema}.\"items\" i \
         ON n.\"item_id\" = i.\"id\" WHERE i.\"id\" IS NULL",
        schema = mirror::SCHEMA
    ));
    assert_eq!(
        orphaned,
        Some("1".to_string()),
        "exactly one mirrored note must match no item, so the join is unmatched on both sides"
    );
}

#[test]
fn mirror_declares_the_same_columns_and_types_as_the_virtual_table() {
    for table in ["items", "item_notes"] {
        let mirrored = declared_columns(mirror::SCHEMA, table);
        let virtual_table = declared_columns(VIRTUAL_SCHEMA, table);

        assert!(
            !mirrored.is_empty(),
            "the mirror must declare table {table}"
        );
        assert_eq!(
            mirrored, virtual_table,
            "the mirror of {table} must declare the same column names in the same order with \
             the same Exasol types as the virtual table, or a differential comparison compares \
             two different things and an aggregate's declared type differs between the two \
             sides for a reason that is not a pushdown defect"
        );
    }
}

#[test]
fn mirror_answers_under_exasols_own_comparison_and_ordering_rules() {
    let stack = stack::shared();

    let rows = stack.exasol.query_rows(&format!(
        "SELECT COUNT(DISTINCT \"category\") AS CATEGORIES, MIN(\"batch\") AS MIN_BATCH, \
         MAX(\"batch\") AS MAX_BATCH, MIN(\"category\") AS MIN_CATEGORY, \
         MAX(\"category\") AS MAX_CATEGORY, COUNT(\"quantity\") AS QUANTITIES \
         FROM {}.\"items\"",
        mirror::SCHEMA
    ));
    let answers = &rows[0];

    assert_eq!(
        cell(answers, "CATEGORIES"),
        Some("4".to_string()),
        "Exasol compares character values by exact value, so Widget, widget, Gizmo, and Gîzmo \
         are four distinct values. MySQL's own utf8mb4_0900_ai_ci returns 2, which is the \
         wrong answer the rendered comparison collation exists to prevent"
    );
    assert_eq!(
        cell(answers, "MIN_BATCH"),
        Some("ab".to_string()),
        "Exasol pads nothing, so MIN over 'ab ', 'ab', 'ab!' is 'ab'. A PAD SPACE collation at \
         the source returns 'ab ' — decision [6]'s measured divergence"
    );
    assert_eq!(cell(answers, "MAX_BATCH"), Some("ab!".to_string()));
    assert_eq!(
        cell(answers, "MIN_CATEGORY"),
        Some("Gizmo".to_string()),
        "Exasol orders character values by code point, so 'Gizmo' precedes 'Gîzmo' and every \
         upper-case initial precedes 'widget'. A collation ordering by weight returns another \
         value here"
    );
    assert_eq!(
        cell(answers, "MAX_CATEGORY"),
        Some("widget".to_string()),
        "code-point order puts 'widget' last, where a case-insensitive collation would not"
    );
    assert_eq!(
        cell(answers, "QUANTITIES"),
        Some("6".to_string()),
        "two of the eight rows carry a null quantity, so COUNT over the column must differ \
         from COUNT(*)"
    );
}

/// Runs `query(schema)` against the virtual schema and against its local
/// mirror and asserts the two answers are identical.
///
/// A correctly delegated fragment returns exactly what Exasol's own
/// evaluation over the mirror returns. The two sides are independent
/// computations — MySQL evaluates one, Exasol the other — so their agreement
/// is the differential proof plan.md's "Differential verification" pattern
/// relies on: a semantic gap between the two engines shows up as a
/// difference rather than having to be predicted case by case.
fn assert_matches_mirror(
    query: impl Fn(&str) -> String,
) -> Vec<serde_json::Map<String, serde_json::Value>> {
    let stack = stack::shared();
    let virtual_rows = stack.exasol.query_rows(&query(VIRTUAL_SCHEMA));
    let mirror_rows = stack.exasol.query_rows(&query(mirror::SCHEMA));
    assert_eq!(
        virtual_rows, mirror_rows,
        "the delegated query must return exactly the rows the local Exasol mirror returns: \
         virtual {virtual_rows:?}, mirror {mirror_rows:?}"
    );
    virtual_rows
}

#[test]
fn filter_matches_local_exasol_result() {
    let query = |schema: &str| {
        format!(
            "SELECT \"id\" FROM {schema}.\"items\" WHERE (\"category\" = 'Widget' OR \
             \"quantity\" IS NULL) AND \"batch\" LIKE 'ab%' AND \"rank_key\" BETWEEN 1 AND 4 \
             AND \"tie_break\" > 0 ORDER BY \"id\""
        )
    };

    let statement = pushdown_statement(&query(VIRTUAL_SCHEMA));
    for fragment in [
        "WHERE",
        "LIKE",
        "BETWEEN",
        "IS NULL",
        "AND",
        "OR",
        "COLLATE utf8mb4_0900_as_cs",
    ] {
        assert!(
            statement.contains(fragment),
            "the generated statement must carry {fragment}: {statement}"
        );
    }

    let rows = assert_matches_mirror(query);
    assert_eq!(
        rows.iter()
            .filter_map(|row| cell(row, "id"))
            .collect::<Vec<_>>(),
        vec![
            "1".to_string(),
            "2".to_string(),
            "3".to_string(),
            "6".to_string()
        ],
        "the combined filter must match exactly the rows worked out by hand from the fixture: \
         {rows:?}"
    );

    for (label, value) in [("case", "GIZMO"), ("accent", "Gïzmo")] {
        let variant_query = |schema: &str| {
            format!("SELECT \"id\" FROM {schema}.\"items\" WHERE \"category\" = '{value}'")
        };
        let variant_rows = assert_matches_mirror(variant_query);
        assert!(
            variant_rows.is_empty(),
            "a value differing from a stored category only by {label} must match no row in \
             either query: {variant_rows:?}"
        );
    }

    let char_query =
        |schema: &str| format!("SELECT \"id\" FROM {schema}.\"items\" WHERE \"code\" = 'ITEM0002'");
    let char_rows = assert_matches_mirror(char_query);
    assert_eq!(
        char_rows
            .iter()
            .filter_map(|row| cell(row, "id"))
            .collect::<Vec<_>>(),
        vec!["2".to_string()],
        "the CHAR-mapped-to-VARCHAR column must compare identically on both sides: {char_rows:?}"
    );
    assert_eq!(
        column_types("items")["code"],
        "VARCHAR(8) UTF8",
        "MySQL CHAR must surface as Exasol VARCHAR rather than the blank-padded CHAR"
    );
}

#[test]
fn aggregation_matches_local_exasol_result() {
    let single_group_query = |schema: &str| {
        format!(
            "SELECT COUNT(*) AS N, COUNT(\"quantity\") AS QN, MIN(\"category\") AS MINCAT, \
             MAX(\"category\") AS MAXCAT FROM {schema}.\"items\""
        )
    };

    let statement = pushdown_statement(&single_group_query(VIRTUAL_SCHEMA));
    for fragment in ["COUNT", "MIN(", "MAX("] {
        assert!(
            statement.contains(fragment),
            "the generated statement must carry {fragment}: {statement}"
        );
    }
    assert!(
        !statement.contains("GROUP BY"),
        "a single-group aggregation must render no GROUP BY clause: {statement}"
    );

    let stack = stack::shared();
    let virtual_row = stack
        .exasol
        .query_rows(&single_group_query(VIRTUAL_SCHEMA))
        .into_iter()
        .next()
        .expect("single-group aggregation returns exactly one row");
    let mirror_row = stack
        .exasol
        .query_rows(&single_group_query(mirror::SCHEMA))
        .into_iter()
        .next()
        .expect("single-group aggregation returns exactly one row");
    for column in ["N", "QN", "MINCAT", "MAXCAT"] {
        assert_eq!(
            cell(&virtual_row, column),
            cell(&mirror_row, column),
            "column {column} must match the local mirror's answer: virtual {virtual_row:?}, \
             mirror {mirror_row:?}"
        );
    }
    assert_eq!(cell(&virtual_row, "N"), Some("8".to_string()));
    assert_eq!(cell(&virtual_row, "QN"), Some("6".to_string()));
    assert_eq!(
        cell(&virtual_row, "MINCAT"),
        Some("Gizmo".to_string()),
        "Exasol's code-point order, which the delegated MIN must reproduce: {virtual_row:?}"
    );
    assert_eq!(
        cell(&virtual_row, "MAXCAT"),
        Some("widget".to_string()),
        "{virtual_row:?}"
    );

    let grouped_query = |schema: &str| {
        format!(
            "SELECT \"category\" AS CATEGORY, COUNT(*) AS N FROM {schema}.\"items\" GROUP BY \
             \"category\" ORDER BY \"category\""
        )
    };

    let grouped_statement = pushdown_statement(&grouped_query(VIRTUAL_SCHEMA));
    assert!(
        grouped_statement.contains("GROUP BY") && grouped_statement.contains("COUNT("),
        "a grouped aggregation must render both COUNT and GROUP BY: {grouped_statement}"
    );

    let grouped_rows = assert_matches_mirror(grouped_query);
    assert_eq!(
        grouped_rows.len(),
        4,
        "four distinct categories, each seeded onto two rows: {grouped_rows:?}"
    );
    assert!(
        grouped_rows
            .iter()
            .all(|row| cell(row, "N") == Some("2".to_string())),
        "each category was seeded onto exactly two rows: {grouped_rows:?}"
    );
}

/// `mysql_dialect.rs` withholds `FN_AGG_AVG`, `FN_AGG_AVG_DISTINCT`,
/// `FN_AGG_SUM`, and `FN_AGG_SUM_DISTINCT` whole, so Exasol never delegates
/// either function and computes both itself over whatever the pushed-down
/// statement projects — the evidence that withholding costs no answer.
#[test]
fn avg_and_sum_are_withdrawn_and_match_local_exasol_result() {
    for function in ["AVG", "SUM"] {
        let query = |schema: &str| {
            format!("SELECT {function}(\"quantity\") AS VAL FROM {schema}.\"items\"")
        };

        let statement = pushdown_statement(&query(VIRTUAL_SCHEMA));
        assert!(
            !statement.contains(function),
            "a query {} \"quantity\" must render no delegated {function}, since this release \
             withholds {function} and Exasol computes it itself: {statement}",
            if function == "AVG" {
                "averaging"
            } else {
                "summing"
            }
        );

        let rows = assert_matches_mirror(query);
        assert_eq!(
            rows.len(),
            1,
            "a single-group aggregation returns exactly one row: {rows:?}"
        );
    }

    assert_sum_over_a_double_column_answers();
}

/// `pushdown-aggregation` requires a `SUM` over a `DOUBLE` column to return
/// Exasol's own value rather than an adapter error, and that clause is the
/// whole difference between the shipped design and the one it replaced: a
/// render-time operand-type guard would fail this query while passing every
/// other test in this tier. The capabilities are withheld instead, so no
/// operand type reaches the renderer at all.
///
/// `orders."price"` is the fixture's `DOUBLE NOT NULL` column. The `items`
/// mirror carries no `DOUBLE` column and gains none: the gate-6.3a correction
/// settled that `SUM` is delegated over no operand type, so there is no
/// delegated value to compare differentially and the fixture stays as it is.
/// What is left to prove is that the statement carries no `SUM` and the query
/// still answers with the sum of the seeded values.
fn assert_sum_over_a_double_column_answers() {
    let query = format!("SELECT SUM(\"price\") AS TOTAL FROM {VIRTUAL_SCHEMA}.\"orders\"");
    let seeded_total = 19.99 + 42.5 + 0.01;

    let statement = pushdown_statement(&query);
    assert!(
        !statement.to_uppercase().contains("SUM"),
        "a query summing the DOUBLE column must render no delegated SUM: {statement}"
    );

    let rows = stack::shared().exasol.query_rows(&query);
    let total: f64 = cell(&rows[0], "TOTAL")
        .expect("summing a NOT NULL column over three rows returns a value")
        .parse()
        .expect("Exasol returns the sum as a number");
    assert!(
        (total - seeded_total).abs() < 1e-9,
        "the query must answer with Exasol's own sum of the seeded prices rather than failing: \
         {total} against {seeded_total}"
    );
}

#[test]
fn join_matches_local_exasol_result() {
    let inner_query = |schema: &str| {
        format!(
            "SELECT i.\"id\" AS ID, n.\"note\" AS NOTE FROM {schema}.\"items\" i INNER JOIN \
             {schema}.\"item_notes\" n ON i.\"id\" = n.\"item_id\" ORDER BY i.\"id\""
        )
    };
    let left_query = |schema: &str| {
        format!(
            "SELECT i.\"id\" AS ID, n.\"note\" AS NOTE FROM {schema}.\"items\" i LEFT JOIN \
             {schema}.\"item_notes\" n ON i.\"id\" = n.\"item_id\" ORDER BY i.\"id\""
        )
    };
    let right_query = |schema: &str| {
        format!(
            "SELECT i.\"id\" AS ID, n.\"note\" AS NOTE FROM {schema}.\"items\" i RIGHT JOIN \
             {schema}.\"item_notes\" n ON i.\"id\" = n.\"item_id\" ORDER BY n.\"item_id\""
        )
    };

    let inner_statement = pushdown_statement(&inner_query(VIRTUAL_SCHEMA));
    assert!(inner_statement.contains("INNER JOIN"), "{inner_statement}");
    let inner_rows = assert_matches_mirror(inner_query);
    assert_eq!(
        inner_rows.len(),
        7,
        "the one items row with no matching note must be excluded: {inner_rows:?}"
    );

    let left_statement = pushdown_statement(&left_query(VIRTUAL_SCHEMA));
    assert!(
        left_statement.contains("LEFT OUTER JOIN"),
        "{left_statement}"
    );
    let left_rows = assert_matches_mirror(left_query);
    assert_eq!(left_rows.len(), 8);
    let unmatched_left = left_rows
        .iter()
        .find(|row| cell(row, "ID") == Some("1".to_string()))
        .expect("item 1 must be present in a left outer join over items");
    assert_eq!(
        cell(unmatched_left, "NOTE"),
        None,
        "the unmatched left row must return a null note, even though MySQL declares that \
         column NOT NULL: {unmatched_left:?}"
    );

    let right_statement = pushdown_statement(&right_query(VIRTUAL_SCHEMA));
    assert!(
        right_statement.contains("RIGHT OUTER JOIN"),
        "{right_statement}"
    );
    let right_rows = assert_matches_mirror(right_query);
    assert_eq!(right_rows.len(), 8);
    let unmatched_right = right_rows
        .iter()
        .find(|row| cell(row, "NOTE") == Some("orphan".to_string()))
        .expect("the orphaned note must be present in a right outer join over item_notes");
    assert_eq!(
        cell(unmatched_right, "ID"),
        None,
        "the unmatched right row must return a null id: {unmatched_right:?}"
    );
}

#[test]
fn projected_predicate_matches_local_exasol_result() {
    let nullable_query = |schema: &str| {
        format!(
            "SELECT \"id\" AS ID, \"quantity\" > 5 AS OVER5 FROM {schema}.\"items\" ORDER BY \
             \"id\""
        )
    };

    let statement = pushdown_statement(&nullable_query(VIRTUAL_SCHEMA));
    assert!(
        statement.contains("(5 < `quantity`)"),
        "the generated statement must project the comparison, with Exasol's normalized operand \
         order: {statement}"
    );
    assert!(
        statement.contains("\"c2\" BOOLEAN"),
        "the EMITS clause must declare the projected predicate's column BOOLEAN: {statement}"
    );

    let rows = assert_matches_mirror(nullable_query);
    let over5 = |id: &str| {
        rows.iter()
            .find(|row| cell(row, "ID") == Some(id.to_string()))
            .and_then(|row| cell(row, "OVER5"))
    };
    assert_eq!(
        over5("1"),
        Some("true".to_string()),
        "quantity 10 > 5: {rows:?}"
    );
    assert_eq!(
        over5("2"),
        Some("false".to_string()),
        "quantity 2 > 5: {rows:?}"
    );
    assert_eq!(
        over5("3"),
        None,
        "a null quantity must project a null rather than a false: {rows:?}"
    );

    let aggregate_query =
        |schema: &str| format!("SELECT COUNT(*) > 0 AS HASROWS FROM {schema}.\"items\"");
    let aggregate_rows = assert_matches_mirror(aggregate_query);
    assert_eq!(
        cell(&aggregate_rows[0], "HASROWS"),
        Some("true".to_string())
    );
}

#[test]
fn grouping_over_character_variants_matches_local_exasol_result() {
    let group_by_query = |schema: &str| {
        format!(
            "SELECT \"category\" AS CATEGORY, COUNT(*) AS N FROM {schema}.\"items\" GROUP BY \
             \"category\" ORDER BY \"category\""
        )
    };

    let statement = pushdown_statement(&group_by_query(VIRTUAL_SCHEMA));
    let select_list = statement
        .split_once("SELECT")
        .and_then(|(_, rest)| rest.split_once("FROM"))
        .map(|(select_list, _)| select_list)
        .unwrap_or_else(|| panic!("the generated statement must carry a SELECT list: {statement}"));
    let group_by_clause = statement
        .split_once("GROUP BY")
        .map(|(_, clause)| clause)
        .unwrap_or_else(|| {
            panic!("the generated statement must carry a GROUP BY clause: {statement}")
        });
    assert!(
        select_list.contains("COLLATE utf8mb4_0900_as_cs"),
        "the bare select-list occurrence of the grouped column must carry the comparison \
         collation, or MySQL would reject the statement with error 1055: {statement}"
    );
    assert!(
        group_by_clause.contains("COLLATE utf8mb4_0900_as_cs"),
        "the grouping key itself must carry the comparison collation, or MySQL would reject the \
         statement with error 1055: {statement}"
    );

    let rows = assert_matches_mirror(group_by_query);
    assert_eq!(
        rows.len(),
        4,
        "MySQL's default collation folds Widget/widget and Gizmo/Gîzmo into 2 groups; the \
         rendered collation must keep all 4 apart, matching Exasol: {rows:?}"
    );

    let distinct_query =
        |schema: &str| format!("SELECT COUNT(DISTINCT \"category\") AS N FROM {schema}.\"items\"");
    let distinct_rows = assert_matches_mirror(distinct_query);
    assert_eq!(cell(&distinct_rows[0], "N"), Some("4".to_string()));
}

#[test]
fn two_column_group_by_is_not_delegated() {
    let query = |schema: &str| {
        format!(
            "SELECT \"category\" AS CATEGORY, \"code\" AS CODE, COUNT(*) AS N FROM \
             {schema}.\"items\" GROUP BY \"category\", \"code\" ORDER BY \"category\", \"code\""
        )
    };

    let statement = pushdown_statement(&query(VIRTUAL_SCHEMA));
    assert!(
        !statement.contains("GROUP BY") && !statement.contains("COUNT"),
        "a two-column GROUP BY must be withdrawn whole, because this release advertises no \
         AGGREGATE_GROUP_BY_TUPLE: {statement}"
    );

    let rows = assert_matches_mirror(query);
    assert_eq!(
        rows.len(),
        8,
        "code is unique per row, so grouping by both columns yields one group per row: {rows:?}"
    );
}

#[test]
fn select_list_predicate_over_grouped_character_column_fails() {
    let stack = stack::shared();
    let failing_query = format!(
        "SELECT \"category\", \"category\" > 'M' FROM {VIRTUAL_SCHEMA}.\"items\" GROUP BY \
         \"category\""
    );

    let error = stack.exasol.execute_expecting_failure(&failing_query);

    assert!(
        error.contains("category"),
        "the adapter error must name the grouped column the select-list predicate reads: {error}"
    );
    assert!(
        error.contains("the select list carries a predicate reading"),
        "the failure must be the adapter's own diagnosis, which is the whole point of failing \
         before a statement is generated: MySQL's error 1055 names the column too, so naming it \
         is not on its own evidence that `reject_collated_grouping_key` ran: {error}"
    );
    assert!(
        !error.contains("nonaggregated column"),
        "no statement may reach MySQL for this shape; that phrase is MySQL's own error 1055, so \
         reading it here means the adapter rendered the statement and MySQL rejected it: {error}"
    );

    let succeeding_query = |schema: &str| {
        format!(
            "SELECT \"category\" AS CATEGORY, COUNT(*) > 0 AS HASROWS FROM {schema}.\"items\" \
             GROUP BY \"category\" ORDER BY \"category\""
        )
    };
    let rows = assert_matches_mirror(succeeding_query);
    assert!(
        rows.iter()
            .all(|row| cell(row, "HASROWS") == Some("true".to_string())),
        "a predicate over COUNT(*) instead of the grouped column must delegate and succeed \
         rather than fail the way the bare-column predicate does: {rows:?}"
    );
}

#[test]
fn ordered_query_is_stable_across_runs() {
    let query = format!(
        "SELECT \"id\", \"rank_key\" FROM {VIRTUAL_SCHEMA}.\"items\" ORDER BY \"rank_key\""
    );

    let runs: Vec<Vec<serde_json::Map<String, serde_json::Value>>> = (0..STABILITY_RUN_COUNT)
        .map(|_| stack::shared().exasol.query_rows(&query))
        .collect();

    for rows in &runs {
        let rank_keys: Vec<i64> = rows
            .iter()
            .filter_map(|row| cell(row, "rank_key"))
            .map(|value| value.parse().expect("rank_key is numeric"))
            .collect();
        let mut sorted = rank_keys.clone();
        sorted.sort_unstable();
        assert_eq!(
            rank_keys, sorted,
            "every run must return rows non-decreasing in the repeating sort key rank_key: \
             {rows:?}"
        );
    }

    assert_eq!(
        runs[0], runs[1],
        "the first and second run over the same repeating sort key must return byte-identical \
         row sequences: {runs:?}"
    );
    assert_eq!(
        runs[1], runs[2],
        "the second and third run over the same repeating sort key must return byte-identical \
         row sequences: {runs:?}"
    );
}

#[test]
fn ordered_query_with_ties_is_stable() {
    let query = format!(
        "SELECT \"id\" FROM {VIRTUAL_SCHEMA}.\"items\" ORDER BY \"rank_key\", \"tie_break\""
    );

    let expected = ["4", "3", "8", "7", "2", "1", "6", "5"]
        .map(str::to_string)
        .to_vec();

    let runs: Vec<Vec<String>> = (0..STABILITY_RUN_COUNT)
        .map(|_| {
            stack::shared()
                .exasol
                .query_rows(&query)
                .iter()
                .filter_map(|row| cell(row, "id"))
                .collect()
        })
        .collect();

    assert_eq!(
        runs[0], expected,
        "rank_key and tie_break jointly define one order, worked out by hand from the fixture: \
         {runs:?}"
    );
    for (run, ids) in runs.iter().enumerate().skip(1) {
        assert_eq!(
            ids, &runs[0],
            "run {run} must return the same order as run 0 among tied keys: {runs:?}"
        );
    }
}

#[test]
fn ordering_survives_a_local_join() {
    let query = format!(
        "SELECT v.\"id\" AS ID FROM {VIRTUAL_SCHEMA}.\"items\" v INNER JOIN \
         {}.\"item_notes\" m ON v.\"id\" = m.\"item_id\" ORDER BY v.\"rank_key\", \
         v.\"tie_break\"",
        mirror::SCHEMA
    );

    let rows = stack::shared().exasol.query_rows(&query);
    let ids: Vec<String> = rows.iter().filter_map(|row| cell(row, "ID")).collect();

    assert_eq!(
        ids,
        vec!["4", "3", "8", "7", "2", "6", "5"],
        "joining the virtual table against a genuinely local Exasol table puts an operator \
         above the scan, which decision-log evidence measured reordering 48.8% of rows when \
         ordering relies on emission order instead of the top-level ORDER BY Exasol applies: \
         {rows:?}"
    );
}

#[test]
fn generated_statement_carries_no_order_or_limit() {
    for suffix in ["ORDER BY \"id\"", "LIMIT 2", "ORDER BY \"id\" LIMIT 2"] {
        let query = move |schema: &str| format!("SELECT \"id\" FROM {schema}.\"items\" {suffix}");

        let statement = pushdown_statement(&query(VIRTUAL_SCHEMA));
        let upper = statement.to_uppercase();
        assert!(
            !upper.contains("ORDER BY"),
            "Exasol applies ORDER BY itself; the generated statement for `{suffix}` must not \
             also sort: {statement}"
        );
        assert!(
            !upper.contains("LIMIT"),
            "a delegated LIMIT would return an arbitrary N rows for Exasol to sort, which is \
             unsound once ORDER BY is withheld; the generated statement for `{suffix}`: \
             {statement}"
        );
        assert!(
            !upper.contains("OFFSET"),
            "OFFSET is withheld for the same reason LIMIT is; the generated statement for \
             `{suffix}`: {statement}"
        );

        assert_matches_mirror(query);
    }
}

/// The `adapterNotes` Exasol persisted for the shared virtual schema, parsed.
fn adapter_notes(stack: &stack::Stack) -> serde_json::Value {
    let notes = stack
        .exasol
        .query_scalar(&format!(
            "SELECT ADAPTER_NOTES FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS \
             WHERE SCHEMA_NAME = '{VIRTUAL_SCHEMA}'"
        ))
        .expect("the virtual schema records adapter notes");
    serde_json::from_str(&notes).expect("adapter notes are a JSON object")
}

/// The statement the adapter sent to MySQL for `query`, as
/// `EXPLAIN VIRTUAL` reports it.
fn pushdown_statement(query: &str) -> String {
    stack::shared()
        .exasol
        .query_scalar(&format!(
            "SELECT PUSHDOWN_SQL FROM (EXPLAIN VIRTUAL {query})"
        ))
        .expect("EXPLAIN VIRTUAL reports the pushed-down statement")
}

fn table_names(schema: &str) -> Vec<String> {
    stack::shared()
        .exasol
        .query_rows(&format!(
            "SELECT TABLE_NAME FROM SYS.EXA_ALL_TABLES WHERE TABLE_SCHEMA = '{schema}' \
             ORDER BY TABLE_NAME"
        ))
        .iter()
        .filter_map(|row| cell(row, "TABLE_NAME"))
        .collect()
}

fn virtual_schema_exists(schema: &str) -> bool {
    !stack::shared()
        .exasol
        .query_rows(&format!(
            "SELECT SCHEMA_NAME FROM SYS.EXA_ALL_VIRTUAL_SCHEMAS WHERE SCHEMA_NAME = '{schema}'"
        ))
        .is_empty()
}

fn column_names(table: &str) -> Vec<String> {
    columns(VIRTUAL_SCHEMA, table)
        .iter()
        .filter_map(|row| cell(row, "COLUMN_NAME"))
        .collect()
}

fn column_types(table: &str) -> std::collections::BTreeMap<String, String> {
    declared_columns(VIRTUAL_SCHEMA, table)
        .into_iter()
        .collect()
}

/// The name and declared Exasol type of every column of `schema.table`, in
/// ordinal-position order — the form that compares a mirror table against the
/// virtual table it mirrors.
fn declared_columns(schema: &str, table: &str) -> Vec<(String, String)> {
    columns(schema, table)
        .iter()
        .filter_map(|row| Some((cell(row, "COLUMN_NAME")?, cell(row, "COLUMN_TYPE")?)))
        .collect()
}

fn columns(schema: &str, table: &str) -> Vec<serde_json::Map<String, serde_json::Value>> {
    stack::shared().exasol.query_rows(&format!(
        "SELECT COLUMN_NAME, COLUMN_TYPE FROM SYS.EXA_ALL_COLUMNS \
         WHERE COLUMN_SCHEMA = '{schema}' AND COLUMN_TABLE = '{table}' \
         ORDER BY COLUMN_ORDINAL_POSITION"
    ))
}

/// Keeps the scan script's registered name asserted from the tier that would
/// otherwise only fail with a parse error deep inside a pushed-down statement.
/// Both halves of the pair come from the constant the pushdown response calls,
/// so a rename cannot leave this green.
#[test]
fn the_scan_script_is_registered_under_the_name_pushdown_calls() {
    let stack = stack::shared();
    let (schema, script) = adbc_vs_adapter::pushdown::SCAN_SCRIPT_NAME
        .split_once('.')
        .expect("the scan script name every pushdown calls is schema-qualified");

    let registered = stack.exasol.query_rows(&format!(
        "SELECT SCRIPT_NAME FROM SYS.EXA_ALL_SCRIPTS WHERE SCRIPT_SCHEMA = '{schema}'"
    ));

    let names: Vec<String> = registered
        .iter()
        .filter_map(|row| cell(row, "SCRIPT_NAME"))
        .collect();
    assert!(
        names.contains(&script.to_string()),
        "schema {schema} holds scripts {names:?}, which does not include {script}, the script \
         every pushed-down statement calls"
    );
}
