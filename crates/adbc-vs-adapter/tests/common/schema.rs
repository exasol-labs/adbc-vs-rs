//! The Exasol objects a virtual schema needs, whatever technology it is over:
//! the two `CONNECTION` objects it resolves at run time, and the virtual schema
//! itself.
//!
//! Property names are the adapter's own (`DRIVER`, `CONNECTION_NAME`,
//! `DRIVER_PATH`) and are spelled once, here. `BUCKETFS_CONNECTION` is
//! optional on every statement this harness builds, since the adapter itself
//! now falls back to a reserved connection when it is absent. The `DRIVER`
//! value is never spelled here at all: it comes from [`Driver`], the same
//! identity the adapter resolves the statement's property back into, so a
//! test enumerating the supported technologies and the statement it generates
//! cannot disagree about how one is spelled.
//!
//! How a driver becomes loadable differs per Exasol backend, and
//! [`provide_drivers`] is the one place that decides it, together with the
//! `BUCKETFS_CONNECTION` the shared statements name.

#[cfg(feature = "exasol-personal")]
use std::path::Path;

#[cfg(feature = "exasol-personal")]
use adbc_vs_adapter::provisioning::default_driver_path;
use adbc_vs_scan::driver::Driver;

#[cfg(feature = "exasol-personal")]
use super::driver_fetch;
use super::exasol::Exasol;
#[cfg(not(feature = "exasol-personal"))]
use super::exasol::BUCKETFS_WRITE_USER;
use super::udf::ADAPTER_SCRIPT;
use super::{mysql, postgres};

/// The `CONNECTION` object holding BucketFS write credentials, which driver
/// self-install needs. Re-exports the adapter's own reserved-connection
/// constant, so the harness cannot create a connection under a name the
/// product no longer reserves.
pub use adbc_vs_adapter::create_schema::DEFAULT_BUCKETFS_CONNECTION as BUCKETFS_CONNECTION;

/// The `BUCKETFS_CONNECTION` the shared statements name: the reserved
/// connection self-install writes through on Docker Exasol, and none on
/// Exasol Personal, where the adapter then loads the `supplied` library.
#[cfg(not(feature = "exasol-personal"))]
const DRIVER_ROUTE_CONNECTION: Option<&str> = Some(BUCKETFS_CONNECTION);
#[cfg(feature = "exasol-personal")]
const DRIVER_ROUTE_CONNECTION: Option<&str> = None;

/// Makes each of `drivers` loadable by the virtual schemas a bring-up then
/// creates over it.
///
/// On Docker Exasol, self-install fetches each driver when the first schema
/// over it is created, so the route needs only the reserved connection it
/// writes through, and `drivers` names nothing this backend has to act on.
#[cfg(not(feature = "exasol-personal"))]
pub fn provide_drivers(exasol: &Exasol, _drivers: &[Driver]) {
    create_bucketfs_connection(exasol);
}

/// Makes each of `drivers` loadable by the virtual schemas a bring-up then
/// creates over it.
///
/// Exasol Personal has no BucketFS endpoint self-install could write through,
/// so each driver is fetched from the registry here and placed at the
/// `supplied` default location, which the adapter falls back to when the
/// reserved connection does not resolve. No reserved connection is created.
#[cfg(feature = "exasol-personal")]
pub fn provide_drivers(exasol: &Exasol, drivers: &[Driver]) {
    for driver in drivers {
        let library = driver_fetch::fetch_driver_library(*driver);
        exasol.put_library(&library, &supplied_bucket_path(*driver));
    }
}

/// The bucket-relative path of `driver`'s `supplied` default location, laid
/// out by the adapter's own provisioning code over an empty mount.
#[cfg(feature = "exasol-personal")]
fn supplied_bucket_path(driver: Driver) -> String {
    let path = default_driver_path(Path::new(""), driver);
    path.to_str()
        .unwrap_or_else(|| panic!("the supplied driver path {path:?} is not UTF-8"))
        .to_owned()
}

/// The name, address, and credentials a `CONNECTION` object is created from.
pub struct ConnectionSpec<'a> {
    pub name: &'a str,
    pub address: &'a str,
    pub user: &'a str,
    pub password: &'a str,
}

/// The schema, the technology, the `CONNECTION` objects, and the optional
/// driver library a `CREATE VIRTUAL SCHEMA` statement is built from.
pub struct VirtualSchemaSpec<'a> {
    pub schema: &'a str,
    /// The technology the statement's `DRIVER` property names.
    pub driver: Driver,
    pub connection: &'a str,
    /// The `CONNECTION` self-install writes the driver through, or nothing for
    /// a statement that supplies its own library and so needs no write
    /// credential.
    pub bucketfs_connection: Option<&'a str>,
    /// A driver shared library the operator placed themselves, which skips
    /// self-install entirely.
    pub driver_path: Option<&'a str>,
}

/// Creates the `CONNECTION` object the adapter writes the driver through.
#[cfg(not(feature = "exasol-personal"))]
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

/// Creates a `CONNECTION` object addressing `database` in the stack's
/// PostgreSQL.
pub fn create_postgres_connection(exasol: &Exasol, name: &str, database: &str) {
    exasol.execute(&connection_statement(&ConnectionSpec {
        name,
        address: &postgres::driver_uri(database),
        user: postgres::USER,
        password: postgres::PASSWORD,
    }));
}

/// Creates a virtual schema over the source database `connection` addresses,
/// replacing any schema of the same name left by an earlier run.
pub fn create_virtual_schema(exasol: &Exasol, driver: Driver, schema: &str, connection: &str) {
    create_virtual_schema_through(exasol, provided_driver_spec(schema, driver, connection));
}

/// A statement that loads its driver through the route [`provide_drivers`]
/// prepared.
fn provided_driver_spec<'a>(
    schema: &'a str,
    driver: Driver,
    connection: &'a str,
) -> VirtualSchemaSpec<'a> {
    VirtualSchemaSpec {
        schema,
        driver,
        connection,
        bucketfs_connection: DRIVER_ROUTE_CONNECTION,
        driver_path: None,
    }
}

/// Creates a virtual schema that provisions its driver through
/// `spec.bucketfs_connection`, which a test that means to observe whether
/// provisioning ran at all points somewhere unreachable.
pub fn create_virtual_schema_through(exasol: &Exasol, spec: VirtualSchemaSpec<'_>) {
    drop_virtual_schema(exasol, spec.schema);
    exasol.execute(&create_statement(&spec));
}

/// Creates a virtual schema that is expected to fail, and returns the error
/// Exasol surfaced to the session.
pub fn create_virtual_schema_expecting_failure(
    exasol: &Exasol,
    driver: Driver,
    schema: &str,
    connection: &str,
) -> String {
    drop_virtual_schema(exasol, schema);
    exasol.execute_expecting_failure(&create_statement(&provided_driver_spec(
        schema, driver, connection,
    )))
}

/// Creates a virtual schema that is expected to fail, from a caller-supplied
/// spec, and returns the error Exasol surfaced to the session.
///
/// Unlike [`create_virtual_schema_expecting_failure`], which names the
/// connection of the route [`provide_drivers`] prepared, this takes the full
/// [`VirtualSchemaSpec`] — needed by a test asserting on a statement whose
/// `BUCKETFS_CONNECTION` is something other than the reserved connection.
pub fn create_virtual_schema_through_expecting_failure(
    exasol: &Exasol,
    spec: VirtualSchemaSpec<'_>,
) -> String {
    drop_virtual_schema(exasol, spec.schema);
    exasol.execute_expecting_failure(&create_statement(&spec))
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

/// The `CREATE VIRTUAL SCHEMA` statement `spec` describes.
///
/// Only the properties the spec carries are written: `BUCKETFS_CONNECTION` is
/// required only while the adapter may need to write, so a statement supplying
/// its own driver library through `DRIVER_PATH` leaves it out entirely rather
/// than naming a connection nothing resolves.
fn create_statement(spec: &VirtualSchemaSpec<'_>) -> String {
    let mut statement = format!(
        "CREATE VIRTUAL SCHEMA {} USING {ADAPTER_SCRIPT} WITH DRIVER = {} CONNECTION_NAME = {}",
        spec.schema,
        quote_literal(spec.driver.as_str()),
        quote_literal(spec.connection)
    );
    if let Some(bucketfs_connection) = spec.bucketfs_connection {
        statement.push_str(&format!(
            " BUCKETFS_CONNECTION = {}",
            quote_literal(bucketfs_connection)
        ));
    }
    if let Some(driver_path) = spec.driver_path {
        statement.push_str(&format!(" DRIVER_PATH = {}", quote_literal(driver_path)));
    }
    statement
}

/// Quotes an Exasol string literal, doubling any embedded single quote — a
/// generated BucketFS write password is arbitrary text.
pub fn quote_literal(value: &str) -> String {
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

    /// A statement over the given connections, provisioning its driver.
    fn spec<'a>(schema: &'a str, connection: &'a str, bucketfs: &'a str) -> VirtualSchemaSpec<'a> {
        VirtualSchemaSpec {
            schema,
            driver: Driver::MySql,
            connection,
            bucketfs_connection: Some(bucketfs),
            driver_path: None,
        }
    }

    #[test]
    fn the_create_statement_names_the_adapter_script_and_all_three_properties() {
        assert_eq!(
            create_statement(&spec("MYSQLVS", "ADBC_VS_MYSQL", BUCKETFS_CONNECTION)),
            "CREATE VIRTUAL SCHEMA MYSQLVS USING ADBC_VS.ADBC_ADAPTER WITH \
             DRIVER = 'mysql' CONNECTION_NAME = 'ADBC_VS_MYSQL' \
             BUCKETFS_CONNECTION = 'ADBC_VS_BUCKETFS'"
        );
    }

    /// The `DRIVER` property carries the technology the spec names, and every
    /// technology this release supports is spelled by [`Driver`] itself rather
    /// than by this module — a test creating one virtual schema per supported
    /// driver generates the same value the adapter parses back.
    #[test]
    fn the_create_statement_names_the_driver_the_spec_carries() {
        for driver in Driver::ALL {
            let statement = create_statement(&VirtualSchemaSpec {
                schema: "S",
                driver: *driver,
                connection: "C",
                bucketfs_connection: Some(BUCKETFS_CONNECTION),
                driver_path: None,
            });

            assert!(
                statement.contains(&format!("DRIVER = '{}'", driver.as_str())),
                "the statement must name the technology the spec carries: {statement}"
            );
        }
    }

    #[test]
    fn the_create_statement_provisions_through_the_bucketfs_connection_it_is_given() {
        assert!(create_statement(&spec("S", "C", "ELSEWHERE"))
            .ends_with("BUCKETFS_CONNECTION = 'ELSEWHERE'"));
    }

    /// A statement supplying its own driver library carries no
    /// `BUCKETFS_CONNECTION` at all: the property's only purpose is the write
    /// credential self-install needs, and this statement writes nothing.
    #[test]
    fn a_statement_supplying_a_driver_path_names_no_bucketfs_connection() {
        let statement = create_statement(&VirtualSchemaSpec {
            schema: "PGVS",
            driver: Driver::Postgres,
            connection: "ADBC_VS_PG",
            bucketfs_connection: None,
            driver_path: Some("/buckets/bfsdefault/default/mine/libadbc_driver_postgresql.so"),
        });

        assert_eq!(
            statement,
            "CREATE VIRTUAL SCHEMA PGVS USING ADBC_VS.ADBC_ADAPTER WITH \
             DRIVER = 'postgresql' CONNECTION_NAME = 'ADBC_VS_PG' \
             DRIVER_PATH = '/buckets/bfsdefault/default/mine/libadbc_driver_postgresql.so'"
        );
    }

    /// Docker Exasol's shared statements self-install through the reserved
    /// connection [`provide_drivers`] created.
    #[cfg(not(feature = "exasol-personal"))]
    #[test]
    fn a_provided_driver_statement_self_installs_through_the_reserved_connection() {
        let statement = create_statement(&provided_driver_spec("S", Driver::MySql, "C"));

        assert!(
            statement.ends_with("BUCKETFS_CONNECTION = 'ADBC_VS_BUCKETFS'"),
            "{statement}"
        );
    }

    /// Exasol Personal's shared statements name no BucketFS connection and no
    /// library, so the adapter falls back to the `supplied` default location
    /// [`provide_drivers`] placed each driver at.
    #[cfg(feature = "exasol-personal")]
    #[test]
    fn a_provided_driver_statement_leaves_the_route_to_the_supplied_default() {
        let statement = create_statement(&provided_driver_spec("S", Driver::MySql, "C"));

        assert!(
            !statement.contains("BUCKETFS_CONNECTION") && !statement.contains("DRIVER_PATH"),
            "{statement}"
        );
    }

    /// Where the harness writes each driver is where the adapter looks for a
    /// supplied one, once the bucket is mounted where a UDF opens it.
    #[cfg(feature = "exasol-personal")]
    #[test]
    fn each_driver_is_supplied_where_the_adapter_loads_its_default_library() {
        assert_eq!(
            supplied_bucket_path(Driver::MySql),
            "adbc-drivers/mysql/supplied/libadbc_driver_mysql.so"
        );
        for driver in Driver::ALL {
            assert_eq!(
                Path::new(&super::super::exasol::udf_object_path(
                    &supplied_bucket_path(*driver)
                )),
                default_driver_path(Path::new("/buckets/bfsdefault/default"), *driver),
                "{}",
                driver.as_str()
            );
        }
    }

    #[test]
    fn the_connection_property_is_not_spelled_connection() {
        let statement = create_statement(&spec("S", "C", BUCKETFS_CONNECTION));
        assert!(
            !statement.contains(" CONNECTION = "),
            "Exasol reserves `CONNECTION` as a property name and rejects the statement: \
             {statement}"
        );
    }
}
