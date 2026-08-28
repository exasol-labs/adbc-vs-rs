//! The Exasol objects a virtual schema over MySQL needs: the two `CONNECTION`
//! objects it resolves at run time, and the virtual schema itself.
//!
//! Property names are the adapter's own (`DRIVER`, `CONNECTION_NAME`,
//! `BUCKETFS_CONNECTION`) and are spelled once, here.

use super::exasol::{Exasol, BUCKETFS_WRITE_USER};
use super::mysql;
use super::udf::ADAPTER_SCRIPT;

/// The `CONNECTION` object holding BucketFS write credentials, which driver
/// self-install needs.
pub const BUCKETFS_CONNECTION: &str = "ADBC_VS_BUCKETFS";

/// The one driver value this release supports.
const DRIVER: &str = "mysql";

/// The name, address, and credentials a `CONNECTION` object is created from.
pub struct ConnectionSpec<'a> {
    pub name: &'a str,
    pub address: &'a str,
    pub user: &'a str,
    pub password: &'a str,
}

/// The schema and the two `CONNECTION` objects a `CREATE VIRTUAL SCHEMA`
/// statement is built from.
pub struct VirtualSchemaSpec<'a> {
    pub schema: &'a str,
    pub connection: &'a str,
    pub bucketfs_connection: &'a str,
}

/// Creates the `CONNECTION` object the adapter writes the driver through.
pub fn create_bucketfs_connection(exasol: &Exasol) {
    exasol.execute(&connection_statement(&ConnectionSpec {
        name: BUCKETFS_CONNECTION,
        address: &exasol.bucketfs_write_url(),
        user: BUCKETFS_WRITE_USER,
        password: exasol.bucketfs_write_password(),
    }));
}

/// Creates a `CONNECTION` object addressing `database` in the stack's MySQL.
pub fn create_mysql_connection(exasol: &Exasol, name: &str, database: &str) {
    create_connection_to(exasol, name, &mysql::driver_uri(database));
}

/// Creates a `CONNECTION` object addressing an arbitrary MySQL URI, including
/// one nothing answers.
pub fn create_connection_to(exasol: &Exasol, name: &str, address: &str) {
    exasol.execute(&connection_statement(&ConnectionSpec {
        name,
        address,
        user: mysql::USER,
        password: mysql::PASSWORD,
    }));
}

/// Creates a virtual schema over the MySQL database `connection` addresses,
/// replacing any schema of the same name left by an earlier run.
pub fn create_virtual_schema(exasol: &Exasol, schema: &str, connection: &str) {
    create_virtual_schema_through(
        exasol,
        VirtualSchemaSpec {
            schema,
            connection,
            bucketfs_connection: BUCKETFS_CONNECTION,
        },
    );
}

/// Creates a virtual schema that provisions its driver through
/// `spec.bucketfs_connection`, which a test that means to observe whether
/// provisioning ran at all points somewhere unreachable.
pub fn create_virtual_schema_through(exasol: &Exasol, spec: VirtualSchemaSpec<'_>) {
    drop_virtual_schema(exasol, spec.schema);
    exasol.execute(&create_statement(
        spec.schema,
        spec.connection,
        spec.bucketfs_connection,
    ));
}

/// Creates a virtual schema that is expected to fail, and returns the error
/// Exasol surfaced to the session.
pub fn create_virtual_schema_expecting_failure(
    exasol: &Exasol,
    schema: &str,
    connection: &str,
) -> String {
    drop_virtual_schema(exasol, schema);
    exasol.execute_expecting_failure(&create_statement(schema, connection, BUCKETFS_CONNECTION))
}

/// Drops a virtual schema and everything Exasol derived from it, if it exists.
pub fn drop_virtual_schema(exasol: &Exasol, schema: &str) {
    exasol.execute(&format!("DROP VIRTUAL SCHEMA IF EXISTS {schema} CASCADE"));
}

/// Drops a `CONNECTION` object, if it exists.
///
/// Every test that creates one drops it again before its own assertions, so a
/// failing assertion leaves the next run the stack it started from rather
/// than one carrying objects it never created. Drop the virtual schema that
/// resolves it first.
pub fn drop_connection(exasol: &Exasol, name: &str) {
    exasol.execute(&format!("DROP CONNECTION IF EXISTS {name}"));
}

fn connection_statement(spec: &ConnectionSpec<'_>) -> String {
    format!(
        "CREATE OR REPLACE CONNECTION {} TO {} USER {} IDENTIFIED BY {}",
        spec.name,
        quote_literal(spec.address),
        quote_literal(spec.user),
        quote_literal(spec.password)
    )
}

fn create_statement(schema: &str, connection: &str, bucketfs_connection: &str) -> String {
    format!(
        "CREATE VIRTUAL SCHEMA {schema} USING {ADAPTER_SCRIPT} WITH \
         DRIVER = {} CONNECTION_NAME = {} BUCKETFS_CONNECTION = {}",
        quote_literal(DRIVER),
        quote_literal(connection),
        quote_literal(bucketfs_connection)
    )
}

/// Quotes an Exasol string literal, doubling any embedded single quote — a
/// generated BucketFS write password is arbitrary text.
fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_statement_carries_its_address_and_credentials_as_literals() {
        assert_eq!(
            connection_statement(&ConnectionSpec {
                name: "C",
                address: "mysql://172.28.5.10:3306/adbc_vs",
                user: "u",
                password: "p",
            }),
            "CREATE OR REPLACE CONNECTION C TO 'mysql://172.28.5.10:3306/adbc_vs' \
             USER 'u' IDENTIFIED BY 'p'"
        );
    }

    #[test]
    fn a_quote_in_a_generated_password_cannot_end_the_literal() {
        assert_eq!(
            connection_statement(&ConnectionSpec {
                name: "C",
                address: "https://host/default",
                user: "w",
                password: "a'b",
            }),
            "CREATE OR REPLACE CONNECTION C TO 'https://host/default' \
             USER 'w' IDENTIFIED BY 'a''b'"
        );
    }

    #[test]
    fn the_create_statement_names_the_adapter_script_and_all_three_properties() {
        assert_eq!(
            create_statement("MYSQLVS", "ADBC_VS_MYSQL", BUCKETFS_CONNECTION),
            "CREATE VIRTUAL SCHEMA MYSQLVS USING ADBC_VS.ADBC_ADAPTER WITH \
             DRIVER = 'mysql' CONNECTION_NAME = 'ADBC_VS_MYSQL' \
             BUCKETFS_CONNECTION = 'ADBC_VS_BUCKETFS'"
        );
    }

    #[test]
    fn the_create_statement_provisions_through_the_bucketfs_connection_it_is_given() {
        assert!(
            create_statement("S", "C", "ELSEWHERE").ends_with("BUCKETFS_CONNECTION = 'ELSEWHERE'")
        );
    }

    #[test]
    fn the_connection_property_is_not_spelled_connection() {
        let statement = create_statement("S", "C", BUCKETFS_CONNECTION);
        assert!(
            !statement.contains(" CONNECTION = "),
            "Exasol reserves `CONNECTION` as a property name and rejects the statement: \
             {statement}"
        );
    }
}
