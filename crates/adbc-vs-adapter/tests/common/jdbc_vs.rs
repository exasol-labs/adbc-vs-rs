//! Provisioning Exasol's official JDBC-based virtual schemas — the
//! benchmark's comparison targets for the ADBC-fronted adapter this repo
//! ships: `mysql-virtual-schema` (v6.0.3) over MySQL Connector/J and
//! `postgresql-virtual-schema` (v4.0.2) over pgJDBC.
//!
//! [`install`] (MySQL) and [`install_postgres`] are the whole interface: each
//! puts both jars into BucketFS, registers the JDBC driver for ExaLoader, and
//! creates the adapter script, the connection, and the virtual schema, so a
//! caller learns only that the schema is queryable. Everything below them
//! builds statement text and nothing else, which is what keeps the exact SQL
//! these virtual schemas need under unit test without a container.
//!
//! Everything that differs between the two sources — jars, script and
//! connection names, ExaLoader registration, JDBC URL, and virtual-schema
//! properties — is one [`Dialect`] value, [`MYSQL`] or [`POSTGRES`]; the
//! provisioning steps are shared.
//!
//! **Lowercase identifiers.** Unlike the ADBC-vs virtual schema, these JDBC
//! virtual schemas expose every source table and column name in lowercase
//! (MySQL's by default, PostgreSQL's because [`POSTGRES`] sets
//! `POSTGRESQL_IDENTIFIER_MAPPING = 'PRESERVE_ORIGINAL_CASE'`; the dialect's
//! own default, `CONVERT_TO_UPPER`, would upper-case them). Any query against
//! them — a benchmark's `SELECT`/CTAS and its verification queries alike —
//! must double-quote every such identifier (`MYSQLVS_JDBC."orders"`,
//! `SUM("id")`); an unquoted reference resolves to Exasol's own upper-cased
//! default and fails with "object not found". This module only builds
//! provisioning DDL, so it never needs to do this itself, but a caller
//! building query SQL against a schema this module creates does.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use sha2::{Digest, Sha256};

use super::exasol::{self, Exasol};
use super::mysql;
use super::postgres;
use super::run;

/// The Exasol schema every adapter script lives in.
pub const SCRIPT_SCHEMA: &str = "SCHEMA_FOR_VS_SCRIPT";

/// A jar a virtual schema needs in BucketFS, pinned by content.
///
/// The digest is checked against every copy that is used, freshly downloaded
/// or found in the cache, so a re-tagged upstream release never gets
/// provisioned and benchmarked as if it were the version measured here.
struct Jar {
    filename: &'static str,
    url: &'static str,
    sha256: &'static str,
}

/// Everything one source's JDBC virtual schema differs in.
///
/// The three identifiers every statement agrees on — the script schema
/// ([`SCRIPT_SCHEMA`], shared), the adapter script's qualified name, and the
/// connection name — are spelled once per dialect, mirroring how
/// [`super::udf`] spells `SCRIPT_SCHEMA` and `ADAPTER_SCRIPT` once for the
/// ADBC side.
pub struct Dialect {
    vs_jar: Jar,
    driver_jar: Jar,
    /// The adapter script a `CREATE VIRTUAL SCHEMA` statement names.
    adapter_script: &'static str,
    /// The `CONNECTION` object a `CREATE VIRTUAL SCHEMA` statement resolves.
    connection_name: &'static str,
    /// The bucket directory this driver's ExaLoader registration sits in.
    driver_directory: &'static str,
    /// The local cache filename the generated `settings.cfg` is written to,
    /// distinct per dialect so two dialects' registrations never overwrite
    /// each other on disk. It is always uploaded as [`SETTINGS_FILE`].
    settings_cache_file: &'static str,
    /// ExaLoader's `DRIVERNAME`.
    loader_driver_name: &'static str,
    /// The JDBC driver class ExaLoader loads.
    driver_class: &'static str,
    /// The JDBC URL scheme, e.g. `mysql` for `jdbc:mysql:`.
    url_scheme: &'static str,
    host: &'static str,
    port: u16,
    /// The path segment after `host:port/` in the JDBC URL; empty when the
    /// database is selected through a virtual-schema property instead.
    url_database: &'static str,
    user: &'static str,
    password: &'static str,
    /// Every `CREATE VIRTUAL SCHEMA` property besides `CONNECTION_NAME`, in
    /// the order they are rendered.
    properties: &'static [(&'static str, &'static str)],
}

/// The fixed bucket directory ExaLoader reads JDBC driver registrations from.
///
/// Fixed by the database rather than by this benchmark: the loader looks for
/// registrations and the jars they name here (and in its subdirectories), and
/// nowhere else.
const DRIVER_DIRECTORY: &str = "drivers/jdbc";

/// The registration file inside a dialect's driver directory.
const SETTINGS_FILE: &str = "settings.cfg";

/// Exasol's `mysql-virtual-schema` over MySQL Connector/J.
pub const MYSQL: Dialect = Dialect {
    // From its GitHub release. The two versions in its name are the shared
    // virtual-schema framework's and the MySQL dialect's, in that order.
    vs_jar: Jar {
        filename: "virtual-schema-dist-14.0.5-mysql-6.0.3.jar",
        url: "https://github.com/exasol/mysql-virtual-schema/releases/download/6.0.3/\
              virtual-schema-dist-14.0.5-mysql-6.0.3.jar",
        sha256: "a75db2793422eb3a8d73bb3af3a84b5048af349f57e564eab51073436cee3888",
    },
    // From Maven Central. Maven Central publishes only `.sha1` and `.md5`
    // beside this artifact, so the digest pinned here is the SHA-256 of the
    // very bytes that published SHA-1,
    // `0a76b3cf1facc2faca4b6e34c1a63ef5e7840131`, authenticates — one
    // algorithm for every jar rather than a second one carried for a single
    // file.
    driver_jar: Jar {
        filename: "mysql-connector-j-9.6.0.jar",
        url: "https://repo1.maven.org/maven2/com/mysql/mysql-connector-j/9.6.0/\
              mysql-connector-j-9.6.0.jar",
        sha256: "66df1d453789dc8cb759a7dc17f58646893bf28483f262328650f170472a6ead",
    },
    adapter_script: "SCHEMA_FOR_VS_SCRIPT.ADAPTER_SCRIPT_MYSQL",
    connection_name: "MYSQL_JDBC_CONNECTION",
    // The bucket's top-level driver directory: the verified working layout the
    // MySQL benchmark has always used.
    driver_directory: DRIVER_DIRECTORY,
    settings_cache_file: SETTINGS_FILE,
    loader_driver_name: "MYSQL",
    // Connector/J 9.6.0 still ships the pre-8.0 `com.mysql.jdbc.Driver` name
    // as a deprecated shim, and it still works — but this is the canonical
    // name, and nothing here should assume the shim persists.
    driver_class: "com.mysql.cj.jdbc.Driver",
    url_scheme: "mysql",
    host: mysql::ADDRESS,
    port: mysql::PORT,
    // No database segment: `CATALOG_NAME` selects the database instead, so one
    // connection can back a virtual schema over any database on the same
    // MySQL instance.
    url_database: "",
    user: mysql::USER,
    password: mysql::PASSWORD,
    properties: &[("CATALOG_NAME", mysql::DATABASE)],
};

/// Exasol's `postgresql-virtual-schema` over pgJDBC.
pub const POSTGRES: Dialect = Dialect {
    // From its GitHub release, which publishes a `.sha256` beside the jar;
    // the digest pinned here is that published one, re-computed from the
    // downloaded bytes. Same framework version (14.0.5) as [`MYSQL`]'s jar.
    vs_jar: Jar {
        filename: "virtual-schema-dist-14.0.5-postgresql-4.0.2.jar",
        url: "https://github.com/exasol/postgresql-virtual-schema/releases/download/4.0.2/\
              virtual-schema-dist-14.0.5-postgresql-4.0.2.jar",
        sha256: "03cae993e07b38a40e526ac0f4e48c6e9bb9ef1bc5288f6e46ac027b11cbfd4f",
    },
    // From Maven Central, which publishes only `.sha1` and `.md5` here; the
    // digest pinned is the SHA-256 of the bytes the published SHA-1,
    // `a6e1bd21b412d6ffb3df23cd13d507bc2cc9e37d`, authenticates — the same
    // treatment as [`MYSQL`]'s Connector/J.
    driver_jar: Jar {
        filename: "postgresql-42.7.13.jar",
        url: "https://repo1.maven.org/maven2/org/postgresql/postgresql/42.7.13/\
              postgresql-42.7.13.jar",
        sha256: "6e0e4cc2d8cae902084f8a2b18728b073a6fd9d1f87c9d8bff8f298c18185b93",
    },
    adapter_script: "SCHEMA_FOR_VS_SCRIPT.ADAPTER_SCRIPT_POSTGRESQL",
    connection_name: "POSTGRESQL_JDBC_CONNECTION",
    // A subdirectory of its own, the layout Exasol's docker-db documents for
    // more than one driver: the MySQL registration owns the top-level
    // `settings.cfg`, and one `settings.cfg` registers exactly one driver.
    driver_directory: "drivers/jdbc/postgresql",
    settings_cache_file: "postgresql-settings.cfg",
    loader_driver_name: "POSTGRESQL",
    driver_class: "org.postgresql.Driver",
    url_scheme: "postgresql",
    host: postgres::ADDRESS,
    port: postgres::PORT,
    // pgJDBC connects to exactly one database, so it must be in the URL;
    // `CATALOG_NAME` then names that same database for metadata reads.
    url_database: postgres::DATABASE,
    user: postgres::USER,
    password: postgres::PASSWORD,
    properties: &[
        ("CATALOG_NAME", postgres::DATABASE),
        // adbc-vs exposes PostgreSQL's `public` schema only.
        ("SCHEMA_NAME", "public"),
        // Mirror PostgreSQL's (lowercase) identifiers as they are, like
        // the ADBC side does, rather than the dialect's `CONVERT_TO_UPPER`
        // default — the column gate and every benchmark query compare
        // double-quoted lowercase names.
        ("POSTGRESQL_IDENTIFIER_MAPPING", "PRESERVE_ORIGINAL_CASE"),
    ],
};

/// Provisions the MySQL JDBC virtual schema `name` over the stack's MySQL
/// fixture database, and returns once it can be queried.
///
/// Repeat calls are safe, so a benchmark re-run against a stack an earlier run
/// left standing starts from the state one against a freshly raised stack
/// does: every database object is replaced, and a cached jar whose digest
/// still matches is not fetched again.
///
/// A failure anywhere — an unreachable download, a digest that does not match,
/// a rejected statement — fails the run. Nothing here reports a half-installed
/// virtual schema as anything but a failure, because a benchmark that measured
/// one would publish a number rather than an error.
pub fn install(exasol: &Exasol, name: &str) {
    install_dialect(exasol, &MYSQL, name);
}

/// Provisions the PostgreSQL JDBC virtual schema `name` over the stack's
/// PostgreSQL fixture database's `public` schema; otherwise exactly
/// [`install`].
pub fn install_postgres(exasol: &Exasol, name: &str) {
    install_dialect(exasol, &POSTGRES, name);
}

fn install_dialect(exasol: &Exasol, dialect: &Dialect, name: &str) {
    upload_jars(exasol, dialect);
    register_driver(exasol, dialect);
    create_objects(exasol, dialect, name);
}

/// Puts both jars where the adapter script's `%jar` directives find them.
fn upload_jars(exasol: &Exasol, dialect: &Dialect) {
    for jar in [&dialect.vs_jar, &dialect.driver_jar] {
        exasol.put_library(&cached_jar(jar), jar.filename);
    }
}

/// Registers the JDBC driver for ExaLoader, which moves the rows a query over
/// this virtual schema actually reads.
///
/// The driver jar is uploaded a second time here, and that is not redundant:
/// ExaLoader runs outside the UDF sandbox, so it sees neither the adapter
/// script's `%jar` directives nor any jar outside its own driver directory.
/// Without this the adapter still plans a query and then the transfer fails
/// with `ETL-1014: No default DRIVER registered`.
fn register_driver(exasol: &Exasol, dialect: &Dialect) {
    exasol.put_library(
        &cached_jar(&dialect.driver_jar),
        &driver_path(dialect, dialect.driver_jar.filename),
    );
    exasol.put_library(
        &written_settings(dialect),
        &driver_path(dialect, SETTINGS_FILE),
    );
}

/// Creates every database object the virtual schema resolves.
///
/// The order is the one a re-run survives. Dropping a virtual schema calls the
/// adapter script it was created with and resolves that script's connection,
/// so both are replaced first and an earlier run's broken script can never
/// block the drop that would have repaired it.
fn create_objects(exasol: &Exasol, dialect: &Dialect, name: &str) {
    exasol.execute(&create_schema_statement());
    exasol.execute(&adapter_script_statement(
        dialect,
        &exasol::udf_object_path(dialect.vs_jar.filename),
        &exasol::udf_object_path(dialect.driver_jar.filename),
    ));
    exasol.execute(&connection_statement(dialect, dialect.password));
    exasol.execute(&drop_virtual_schema_statement(name));
    exasol.execute(&create_virtual_schema_statement(dialect, name));
}

/// Where `filename` sits in the bucket for ExaLoader to find it.
fn driver_path(dialect: &Dialect, filename: &str) -> String {
    format!("{}/{filename}", dialect.driver_directory)
}

/// The jar on disk, downloaded on first use, its digest checked every time.
///
/// A download lands on a private path and is renamed into place only once its
/// digest matches, so a file under the cached name is one an earlier run
/// already verified — which is why a cached file that no longer matches is a
/// failure rather than a reason to fetch it again.
fn cached_jar(jar: &Jar) -> PathBuf {
    let directory = cache_directory();
    let cached = directory.join(jar.filename);
    if cached.is_file() {
        verify(&cached, jar);
        return cached;
    }

    let partial = directory.join(format!("{}.{}", jar.filename, process::id()));
    let destination = partial.to_string_lossy().into_owned();
    run(
        &format!("downloading {} from {}", jar.filename, jar.url),
        "curl",
        &[
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--max-time",
            "600",
            "--output",
            &destination,
            jar.url,
        ],
    );
    verify(&partial, jar);

    fs::rename(&partial, &cached).unwrap_or_else(|error| {
        panic!("moving the downloaded jar to {}: {error}", cached.display())
    });
    cached
}

/// Fails the run unless `path` holds exactly the bytes `jar` pins.
fn verify(path: &Path, jar: &Jar) {
    let bytes =
        fs::read(path).unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
    assert_eq!(
        sha256(&bytes),
        jar.sha256,
        "{} does not hold the pinned {}; delete it and let the next run fetch it again",
        path.display(),
        jar.filename
    );
}

/// The SHA-256 of `bytes`, in the lower-case hexadecimal a published digest is
/// written in.
fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Writes `dialect`'s ExaLoader driver registration into the cache and
/// returns it.
///
/// It is rewritten on every call rather than cached, because it is generated
/// from this module's own constants and an earlier run's copy would otherwise
/// outlive a change to them.
fn written_settings(dialect: &Dialect) -> PathBuf {
    let settings = cache_directory().join(dialect.settings_cache_file);
    fs::write(&settings, settings_cfg(dialect))
        .unwrap_or_else(|error| panic!("writing {}: {error}", settings.display()));
    settings
}

/// Where downloaded jars and the generated driver registrations are kept
/// between runs, created on first use.
fn cache_directory() -> PathBuf {
    let directory = exasol::workspace_root().join("target").join("bench");
    fs::create_dir_all(&directory)
        .unwrap_or_else(|error| panic!("creating {}: {error}", directory.display()));
    directory
}

/// ExaLoader's `settings.cfg` content, registering `dialect`'s JDBC driver.
///
/// `JAR` names the driver jar exactly as it sits in BucketFS beside the file
/// (a bare filename, not a path). The load tuning — `FETCHSIZE=100000`,
/// `INSERTSIZE=-1`, `NOSECURITY=YES` — is identical for every dialect, so no
/// source's baseline is tuned differently from another's.
pub fn settings_cfg(dialect: &Dialect) -> String {
    format!(
        "DRIVERNAME={}\n\
         JAR={}\n\
         DRIVERMAIN={}\n\
         PREFIX=jdbc:{}:\n\
         FETCHSIZE=100000\n\
         INSERTSIZE=-1\n\
         NOSECURITY=YES\n",
        dialect.loader_driver_name,
        dialect.driver_jar.filename,
        dialect.driver_class,
        dialect.url_scheme
    )
}

/// The `CREATE OR REPLACE JAVA ADAPTER SCRIPT` DDL registering `dialect`'s
/// virtual-schema adapter over both jars it needs in BucketFS.
///
/// `vs_jar_path` and `driver_jar_path` are the UDF-visible, bucket-absolute
/// paths (`/buckets/...`) — the same form [`super::exasol::udf_object_path`]
/// returns for the ADBC side's shared object. Every directive ends with a
/// semicolon: Exasol accepts nothing else here, and a dropped one produces a
/// script that creates jar-less and then fails at connection time with a
/// cryptic error rather than at creation time with a clear one. The trailing
/// `/` terminates the script body as one statement for `exapump sql`
/// (`exapump` 0.10.1+; a bug in earlier versions split it on every unquoted
/// `;` and broke this statement — see `install()`).
pub fn adapter_script_statement(
    dialect: &Dialect,
    vs_jar_path: &str,
    driver_jar_path: &str,
) -> String {
    format!(
        "CREATE OR REPLACE JAVA ADAPTER SCRIPT {} AS\n\
         %scriptclass com.exasol.adapter.RequestDispatcher;\n\
         %jar {vs_jar_path};\n\
         %jar {driver_jar_path};\n\
         /",
        dialect.adapter_script
    )
}

/// Creates the schema the adapter scripts live in, replacing nothing: a
/// second run finds it already there and does nothing.
pub fn create_schema_statement() -> String {
    format!("CREATE SCHEMA IF NOT EXISTS {SCRIPT_SCHEMA}")
}

/// The JDBC URL `dialect`'s connection object carries.
fn jdbc_url(dialect: &Dialect) -> String {
    format!(
        "jdbc:{}://{}:{}/{}",
        dialect.url_scheme, dialect.host, dialect.port, dialect.url_database
    )
}

/// The `CREATE OR REPLACE CONNECTION` statement ExaLoader resolves at CTAS
/// time to reach the source over JDBC.
///
/// `password` is a parameter rather than read from `dialect` only so a test
/// can show an embedded quote cannot end the literal.
pub fn connection_statement(dialect: &Dialect, password: &str) -> String {
    format!(
        "CREATE OR REPLACE CONNECTION {} TO {} USER {} IDENTIFIED BY {}",
        dialect.connection_name,
        quote_literal(&jdbc_url(dialect)),
        quote_literal(dialect.user),
        quote_literal(password)
    )
}

/// Creates the virtual schema `name` with `dialect`'s fixed adapter script,
/// connection, and properties.
pub fn create_virtual_schema_statement(dialect: &Dialect, name: &str) -> String {
    let properties: String = dialect
        .properties
        .iter()
        .map(|(key, value)| format!(" {key} = {}", quote_literal(value)))
        .collect();
    format!(
        "CREATE VIRTUAL SCHEMA {name} USING {} WITH CONNECTION_NAME = {}{properties}",
        dialect.adapter_script,
        quote_literal(dialect.connection_name)
    )
}

/// Drops a virtual schema and everything Exasol derived from it, if it
/// exists — the JDBC-side counterpart of [`super::schema::drop_virtual_schema`].
pub fn drop_virtual_schema_statement(name: &str) -> String {
    format!("DROP VIRTUAL SCHEMA IF EXISTS {name} CASCADE")
}

/// Quotes an Exasol string literal, doubling any embedded single quote.
fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use std::panic;

    use tempfile::NamedTempFile;

    use super::*;

    const DIALECTS: [&Dialect; 2] = [&MYSQL, &POSTGRES];

    #[test]
    fn settings_cfg_registers_the_mysql_jar_with_fixed_driver_tuning() {
        assert_eq!(
            settings_cfg(&MYSQL),
            "DRIVERNAME=MYSQL\n\
             JAR=mysql-connector-j-9.6.0.jar\n\
             DRIVERMAIN=com.mysql.cj.jdbc.Driver\n\
             PREFIX=jdbc:mysql:\n\
             FETCHSIZE=100000\n\
             INSERTSIZE=-1\n\
             NOSECURITY=YES\n"
        );
    }

    #[test]
    fn settings_cfg_registers_pgjdbc_with_the_same_driver_tuning_as_mysql() {
        assert_eq!(
            settings_cfg(&POSTGRES),
            "DRIVERNAME=POSTGRESQL\n\
             JAR=postgresql-42.7.13.jar\n\
             DRIVERMAIN=org.postgresql.Driver\n\
             PREFIX=jdbc:postgresql:\n\
             FETCHSIZE=100000\n\
             INSERTSIZE=-1\n\
             NOSECURITY=YES\n"
        );
    }

    #[test]
    fn settings_cfg_ends_with_a_trailing_newline() {
        for dialect in DIALECTS {
            assert!(settings_cfg(dialect).ends_with("NOSECURITY=YES\n"));
        }
    }

    #[test]
    fn adapter_script_statement_names_both_jars_and_the_fixed_dispatcher_class() {
        assert_eq!(
            adapter_script_statement(
                &MYSQL,
                "/buckets/bfsdefault/default/virtual-schema-dist-14.0.5-mysql-6.0.3.jar",
                "/buckets/bfsdefault/default/mysql-connector-j-9.6.0.jar"
            ),
            "CREATE OR REPLACE JAVA ADAPTER SCRIPT SCHEMA_FOR_VS_SCRIPT.ADAPTER_SCRIPT_MYSQL AS\n\
             %scriptclass com.exasol.adapter.RequestDispatcher;\n\
             %jar /buckets/bfsdefault/default/virtual-schema-dist-14.0.5-mysql-6.0.3.jar;\n\
             %jar /buckets/bfsdefault/default/mysql-connector-j-9.6.0.jar;\n\
             /"
        );
    }

    #[test]
    fn each_dialect_has_its_own_adapter_script_and_connection() {
        assert_ne!(MYSQL.adapter_script, POSTGRES.adapter_script);
        assert_ne!(MYSQL.connection_name, POSTGRES.connection_name);
        assert!(
            adapter_script_statement(&POSTGRES, "/buckets/a.jar", "/buckets/b.jar").starts_with(
                "CREATE OR REPLACE JAVA ADAPTER SCRIPT \
                 SCHEMA_FOR_VS_SCRIPT.ADAPTER_SCRIPT_POSTGRESQL AS\n"
            )
        );
    }

    #[test]
    fn every_adapter_script_directive_ends_with_a_semicolon() {
        for dialect in DIALECTS {
            let statement = adapter_script_statement(dialect, "/buckets/a.jar", "/buckets/b.jar");
            for directive in [
                "%scriptclass com.exasol.adapter.RequestDispatcher",
                "%jar /buckets/a.jar",
                "%jar /buckets/b.jar",
            ] {
                assert!(
                    statement.contains(&format!("{directive};")),
                    "directive `{directive}` must end with a semicolon or Exasol rejects the \
                     script: {statement}"
                );
            }
        }
    }

    #[test]
    fn adapter_script_statement_terminates_the_script_body() {
        for dialect in DIALECTS {
            let statement = adapter_script_statement(dialect, "/buckets/a.jar", "/buckets/b.jar");
            assert!(
                statement.ends_with('/'),
                "a Java adapter script body must end with the `/` terminator: {statement}"
            );
        }
    }

    #[test]
    fn create_schema_statement_is_idempotent_and_names_the_fixed_schema() {
        assert_eq!(
            create_schema_statement(),
            "CREATE SCHEMA IF NOT EXISTS SCHEMA_FOR_VS_SCRIPT"
        );
    }

    #[test]
    fn mysql_connection_statement_carries_a_jdbc_url_with_no_database_segment() {
        assert_eq!(
            connection_statement(&MYSQL, "adbc_vs_password"),
            "CREATE OR REPLACE CONNECTION MYSQL_JDBC_CONNECTION TO 'jdbc:mysql://172.28.5.10:3306/' \
             USER 'adbc_vs' IDENTIFIED BY 'adbc_vs_password'"
        );
    }

    #[test]
    fn postgres_connection_statement_names_the_database_in_the_jdbc_url() {
        assert_eq!(
            connection_statement(&POSTGRES, "adbc_vs_password"),
            "CREATE OR REPLACE CONNECTION POSTGRESQL_JDBC_CONNECTION \
             TO 'jdbc:postgresql://172.28.5.11:5432/adbc_vs' \
             USER 'adbc_vs' IDENTIFIED BY 'adbc_vs_password'"
        );
    }

    #[test]
    fn a_quote_in_the_password_cannot_end_the_literal() {
        for dialect in DIALECTS {
            let statement = connection_statement(dialect, "a'b");
            assert!(
                statement.ends_with("IDENTIFIED BY 'a''b'"),
                "an embedded quote must be doubled, not left to close the literal early: \
                 {statement}"
            );
        }
    }

    #[test]
    fn mysql_create_virtual_schema_statement_selects_the_database_through_catalog_name() {
        assert_eq!(
            create_virtual_schema_statement(&MYSQL, "MYSQLVS_JDBC"),
            "CREATE VIRTUAL SCHEMA MYSQLVS_JDBC USING SCHEMA_FOR_VS_SCRIPT.ADAPTER_SCRIPT_MYSQL \
             WITH CONNECTION_NAME = 'MYSQL_JDBC_CONNECTION' CATALOG_NAME = 'adbc_vs'"
        );
    }

    #[test]
    fn postgres_create_virtual_schema_statement_preserves_identifier_case_over_public() {
        assert_eq!(
            create_virtual_schema_statement(&POSTGRES, "PGVS_JDBC"),
            "CREATE VIRTUAL SCHEMA PGVS_JDBC USING SCHEMA_FOR_VS_SCRIPT.ADAPTER_SCRIPT_POSTGRESQL \
             WITH CONNECTION_NAME = 'POSTGRESQL_JDBC_CONNECTION' CATALOG_NAME = 'adbc_vs' \
             SCHEMA_NAME = 'public' POSTGRESQL_IDENTIFIER_MAPPING = 'PRESERVE_ORIGINAL_CASE'"
        );
    }

    #[test]
    fn drop_virtual_schema_statement_is_idempotent() {
        assert_eq!(
            drop_virtual_schema_statement("MYSQLVS_JDBC"),
            "DROP VIRTUAL SCHEMA IF EXISTS MYSQLVS_JDBC CASCADE"
        );
    }

    #[test]
    fn sha256_renders_a_digest_the_way_a_published_one_is_written() {
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    fn every_jar() -> impl Iterator<Item = &'static Jar> {
        DIALECTS
            .into_iter()
            .flat_map(|dialect| [&dialect.vs_jar, &dialect.driver_jar])
    }

    #[test]
    fn every_pinned_digest_is_a_sha256_in_lower_case_hexadecimal() {
        for jar in every_jar() {
            assert_eq!(
                jar.sha256.len(),
                64,
                "{} pins something that is not a SHA-256: {}",
                jar.filename,
                jar.sha256
            );
            assert!(
                jar.sha256
                    .chars()
                    .all(|character| character.is_ascii_digit()
                        || ('a'..='f').contains(&character)),
                "{} pins a digest no computed one can ever equal: {}",
                jar.filename,
                jar.sha256
            );
        }
    }

    #[test]
    fn every_jar_is_fetched_over_https_from_a_url_ending_in_its_own_filename() {
        for jar in every_jar() {
            assert!(
                jar.url.starts_with("https://"),
                "{} would be fetched unauthenticated: {}",
                jar.filename,
                jar.url
            );
            assert!(
                jar.url.ends_with(jar.filename),
                "{} would be cached under a name the URL does not serve: {}",
                jar.filename,
                jar.url
            );
        }
    }

    #[test]
    fn every_jar_is_cached_under_a_distinct_filename() {
        let filenames: std::collections::BTreeSet<&str> =
            every_jar().map(|jar| jar.filename).collect();
        assert_eq!(filenames.len(), every_jar().count());
    }

    #[test]
    fn the_driver_registration_names_the_driver_jar_that_is_uploaded_beside_it() {
        for dialect in DIALECTS {
            assert!(
                settings_cfg(dialect).contains(&format!("JAR={}\n", dialect.driver_jar.filename)),
                "the registered driver jar and the uploaded one must be the same file"
            );
        }
    }

    #[test]
    fn the_driver_registration_and_its_jar_share_one_directory() {
        assert_eq!(
            driver_path(&MYSQL, SETTINGS_FILE),
            "drivers/jdbc/settings.cfg"
        );
        assert_eq!(
            driver_path(&MYSQL, MYSQL.driver_jar.filename),
            "drivers/jdbc/mysql-connector-j-9.6.0.jar"
        );
        assert_eq!(
            driver_path(&POSTGRES, SETTINGS_FILE),
            "drivers/jdbc/postgresql/settings.cfg"
        );
        assert_eq!(
            driver_path(&POSTGRES, POSTGRES.driver_jar.filename),
            "drivers/jdbc/postgresql/postgresql-42.7.13.jar"
        );
    }

    #[test]
    fn two_dialects_never_share_a_driver_registration() {
        assert_ne!(
            driver_path(&MYSQL, SETTINGS_FILE),
            driver_path(&POSTGRES, SETTINGS_FILE),
            "one settings.cfg registers one driver, so a second dialect writing the same \
             path would unregister the first"
        );
        assert_ne!(MYSQL.settings_cache_file, POSTGRES.settings_cache_file);
        for dialect in DIALECTS {
            assert!(
                dialect.driver_directory.starts_with(DRIVER_DIRECTORY),
                "ExaLoader reads registrations only under {DRIVER_DIRECTORY}: {}",
                dialect.driver_directory
            );
        }
    }

    /// A jar pinned to the digest of no bytes at all, so a test can produce a
    /// file that matches it without carrying a fixture.
    const EMPTY_JAR: Jar = Jar {
        filename: "empty.jar",
        url: "https://example.invalid/empty.jar",
        sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    };

    fn file_holding(bytes: &[u8]) -> NamedTempFile {
        let file = NamedTempFile::new().expect("a temporary file");
        fs::write(file.path(), bytes).expect("writing the temporary file");
        file
    }

    #[test]
    fn a_file_holding_exactly_the_pinned_bytes_passes_verification() {
        verify(file_holding(b"").path(), &EMPTY_JAR);
    }

    #[test]
    fn a_file_that_is_not_the_pinned_jar_fails_the_run() {
        let file = file_holding(b"not the jar");
        let path = file.path().to_owned();

        let payload = panic::catch_unwind(|| verify(&path, &EMPTY_JAR))
            .expect_err("a file that is not the pinned jar must fail the run");

        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .expect("the failure carries a message");
        assert!(
            message.contains(&path.display().to_string()) && message.contains(EMPTY_JAR.filename),
            "the failure must name the file that is wrong and the jar it should have been: \
             {message}"
        );
        assert!(
            message.contains(EMPTY_JAR.sha256),
            "the failure must name the digest that was expected: {message}"
        );
    }

    #[test]
    fn a_missing_file_fails_the_run_rather_than_verifying_as_absent() {
        let payload =
            panic::catch_unwind(|| verify(Path::new("/nonexistent/empty.jar"), &EMPTY_JAR))
                .expect_err("a jar that is not there must fail the run");

        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .expect("the failure carries a message");
        assert!(
            message.contains("/nonexistent/empty.jar"),
            "the failure must name the file it could not read: {message}"
        );
    }

    #[test]
    fn a_driver_path_carries_no_bucket_or_scheme_segment() {
        for dialect in DIALECTS {
            let path = driver_path(dialect, SETTINGS_FILE);
            assert!(
                !path.starts_with('/') && !path.contains("bfs:") && !path.contains("default/"),
                "a BucketFS destination is bucket-relative; anything else is rejected: {path}"
            );
        }
    }
}
