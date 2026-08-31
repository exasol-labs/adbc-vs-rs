//! Provisioning Exasol's official JDBC-based `mysql-virtual-schema` (v6.0.3)
//! — the benchmark's comparison target for the ADBC-fronted adapter this repo
//! ships.
//!
//! [`install`] is the whole interface: it puts both jars into BucketFS,
//! registers the JDBC driver for ExaLoader, and creates the adapter script,
//! the connection, and the virtual schema, so a caller learns only that the
//! schema is queryable. Everything below it builds statement text and nothing
//! else, which is what keeps the exact SQL this virtual schema needs under
//! unit test without a container.
//!
//! The three identifiers every statement here agrees on — the script schema,
//! the adapter script's qualified name, and the connection name — are spelled
//! once, as constants, mirroring how [`super::udf`] spells `SCRIPT_SCHEMA` and
//! `ADAPTER_SCRIPT` once for the ADBC side.
//!
//! **Lowercase identifiers.** Unlike the ADBC-vs virtual schema, this JDBC
//! virtual schema exposes every MySQL table and column name in lowercase. Any
//! query against it — a benchmark's `SELECT`/CTAS and its verification
//! queries alike — must double-quote every such identifier
//! (`MYSQLVS_JDBC."orders"`, `SUM("id")`); an unquoted reference resolves to
//! Exasol's own upper-cased default and fails with "object not found". This
//! module only builds provisioning DDL, so it never needs to do this itself,
//! but a caller building query SQL against the schema this module creates
//! does.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use sha2::{Digest, Sha256};

use super::exasol::{self, Exasol};
use super::mysql;
use super::run;

/// The Exasol schema the adapter script lives in.
pub const SCRIPT_SCHEMA: &str = "SCHEMA_FOR_VS_SCRIPT";

/// The adapter script a `CREATE VIRTUAL SCHEMA` statement names.
pub const ADAPTER_SCRIPT: &str = "SCHEMA_FOR_VS_SCRIPT.ADAPTER_SCRIPT_MYSQL";

/// The `CONNECTION` object a `CREATE VIRTUAL SCHEMA` statement resolves.
pub const CONNECTION_NAME: &str = "MYSQL_JDBC_CONNECTION";

/// A jar this virtual schema needs in BucketFS, pinned by content.
///
/// The digest is checked against every copy that is used, freshly downloaded
/// or found in the cache, so a re-tagged upstream release never gets
/// provisioned and benchmarked as if it were the version measured here.
struct Jar {
    filename: &'static str,
    url: &'static str,
    sha256: &'static str,
}

/// The MySQL virtual-schema adapter, from its GitHub release. The two versions
/// in its name are the shared virtual-schema framework's and the MySQL
/// dialect's, in that order.
const VS_JAR: Jar = Jar {
    filename: "virtual-schema-dist-14.0.5-mysql-6.0.3.jar",
    url: "https://github.com/exasol/mysql-virtual-schema/releases/download/6.0.3/\
          virtual-schema-dist-14.0.5-mysql-6.0.3.jar",
    sha256: "a75db2793422eb3a8d73bb3af3a84b5048af349f57e564eab51073436cee3888",
};

/// MySQL Connector/J, from Maven Central.
///
/// Maven Central publishes only `.sha1` and `.md5` beside this artifact, so
/// the digest pinned here is the SHA-256 of the very bytes that published
/// SHA-1, `0a76b3cf1facc2faca4b6e34c1a63ef5e7840131`, authenticates — one
/// algorithm for both jars rather than a second one carried for a single file.
const CONNECTOR_JAR: Jar = Jar {
    filename: "mysql-connector-j-9.6.0.jar",
    url: "https://repo1.maven.org/maven2/com/mysql/mysql-connector-j/9.6.0/\
          mysql-connector-j-9.6.0.jar",
    sha256: "66df1d453789dc8cb759a7dc17f58646893bf28483f262328650f170472a6ead",
};

/// The bucket directory ExaLoader reads its JDBC driver registration from.
///
/// Fixed by the database rather than by this benchmark: the loader looks for a
/// registration and the jars it names here, and nowhere else.
const DRIVER_DIRECTORY: &str = "drivers/jdbc";

/// The registration file inside [`DRIVER_DIRECTORY`].
const SETTINGS_FILE: &str = "settings.cfg";

/// Provisions the virtual schema `name` over the stack's MySQL fixture
/// database, and returns once it can be queried.
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
    upload_jars(exasol);
    register_driver(exasol);
    create_objects(exasol, name);
}

/// Puts both jars where the adapter script's `%jar` directives find them.
fn upload_jars(exasol: &Exasol) {
    for jar in [&VS_JAR, &CONNECTOR_JAR] {
        exasol.put_library(&cached_jar(jar), jar.filename);
    }
}

/// Registers the JDBC driver for ExaLoader, which moves the rows a query over
/// this virtual schema actually reads.
///
/// The connector jar is uploaded a second time here, and that is not
/// redundant: ExaLoader runs outside the UDF sandbox, so it sees neither the
/// adapter script's `%jar` directives nor any jar outside its own driver
/// directory. Without this the adapter still plans a query and then the
/// transfer fails with `ETL-1014: No default DRIVER registered`.
fn register_driver(exasol: &Exasol) {
    exasol.put_library(
        &cached_jar(&CONNECTOR_JAR),
        &driver_path(CONNECTOR_JAR.filename),
    );
    exasol.put_library(&written_settings(), &driver_path(SETTINGS_FILE));
}

/// Creates every database object the virtual schema resolves.
///
/// The order is the one a re-run survives. Dropping a virtual schema calls the
/// adapter script it was created with and resolves that script's connection,
/// so both are replaced first and an earlier run's broken script can never
/// block the drop that would have repaired it.
fn create_objects(exasol: &Exasol, name: &str) {
    exasol.execute(&create_schema_statement());
    exasol.execute(&adapter_script_statement(
        &exasol::udf_object_path(VS_JAR.filename),
        &exasol::udf_object_path(CONNECTOR_JAR.filename),
    ));
    exasol.execute(&connection_statement(
        mysql::ADDRESS,
        mysql::PORT,
        mysql::USER,
        mysql::PASSWORD,
    ));
    exasol.execute(&drop_virtual_schema_statement(name));
    exasol.execute(&create_virtual_schema_statement(name, mysql::DATABASE));
}

/// Where `filename` sits in the bucket for ExaLoader to find it.
fn driver_path(filename: &str) -> String {
    format!("{DRIVER_DIRECTORY}/{filename}")
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

/// Writes ExaLoader's driver registration into the cache and returns it.
///
/// It is rewritten on every call rather than cached, because it is generated
/// from this module's own constants and an earlier run's copy would otherwise
/// outlive a change to them.
fn written_settings() -> PathBuf {
    let settings = cache_directory().join(SETTINGS_FILE);
    fs::write(&settings, settings_cfg(CONNECTOR_JAR.filename))
        .unwrap_or_else(|error| panic!("writing {}: {error}", settings.display()));
    settings
}

/// Where downloaded jars and the generated driver registration are kept
/// between runs, created on first use.
fn cache_directory() -> PathBuf {
    let directory = exasol::workspace_root().join("target").join("bench");
    fs::create_dir_all(&directory)
        .unwrap_or_else(|error| panic!("creating {}: {error}", directory.display()));
    directory
}

/// The fixed driver class name ExaLoader loads for every MySQL connection.
///
/// Connector/J 9.6.0 still ships the pre-8.0 `com.mysql.jdbc.Driver` name as a
/// deprecated shim, and it still works — but this is the canonical name, and
/// nothing here should assume the shim persists.
const DRIVER_CLASS: &str = "com.mysql.cj.jdbc.Driver";

/// ExaLoader's `settings.cfg` content, registering the MySQL JDBC driver.
///
/// `driver_jar_filename` names the Connector/J jar exactly as it sits in
/// BucketFS (a bare filename, not a path) — `install()` decides which jar
/// that is. The driver class, JDBC URL prefix, and load tuning are fixed by
/// this benchmark's verified working configuration, not by the jar version.
pub fn settings_cfg(driver_jar_filename: &str) -> String {
    format!(
        "DRIVERNAME=MYSQL\n\
         JAR={driver_jar_filename}\n\
         DRIVERMAIN={DRIVER_CLASS}\n\
         PREFIX=jdbc:mysql:\n\
         FETCHSIZE=100000\n\
         INSERTSIZE=-1\n\
         NOSECURITY=YES\n"
    )
}

/// The `CREATE OR REPLACE JAVA ADAPTER SCRIPT` DDL registering the MySQL
/// virtual-schema adapter over both jars it needs in BucketFS.
///
/// `vs_jar_path` and `connector_jar_path` are the UDF-visible, bucket-absolute
/// paths (`/buckets/...`) — the same form [`super::exasol::udf_object_path`]
/// returns for the ADBC side's shared object. Every directive ends with a
/// semicolon: Exasol accepts nothing else here, and a dropped one produces a
/// script that creates jar-less and then fails at connection time with a
/// cryptic error rather than at creation time with a clear one. The trailing
/// `/` terminates the script body as one statement for `exapump sql`
/// (`exapump` 0.10.1+; a bug in earlier versions split it on every unquoted
/// `;` and broke this statement — see `install()`).
pub fn adapter_script_statement(vs_jar_path: &str, connector_jar_path: &str) -> String {
    format!(
        "CREATE OR REPLACE JAVA ADAPTER SCRIPT {ADAPTER_SCRIPT} AS\n\
         %scriptclass com.exasol.adapter.RequestDispatcher;\n\
         %jar {vs_jar_path};\n\
         %jar {connector_jar_path};\n\
         /"
    )
}

/// Creates the schema the adapter script lives in, replacing nothing: a
/// second run finds it already there and does nothing.
pub fn create_schema_statement() -> String {
    format!("CREATE SCHEMA IF NOT EXISTS {SCRIPT_SCHEMA}")
}

/// The `CREATE OR REPLACE CONNECTION` statement ExaLoader resolves at CTAS
/// time to reach MySQL over JDBC.
///
/// The JDBC URL carries no database segment — `create_virtual_schema_statement`
/// selects the database through `CATALOG_NAME` instead, so one connection can
/// back a virtual schema over any database on the same MySQL instance.
pub fn connection_statement(host: &str, port: u16, user: &str, password: &str) -> String {
    format!(
        "CREATE OR REPLACE CONNECTION {CONNECTION_NAME} TO {} USER {} IDENTIFIED BY {}",
        quote_literal(&format!("jdbc:mysql://{host}:{port}/")),
        quote_literal(user),
        quote_literal(password)
    )
}

/// Creates a virtual schema over `catalog`, using the fixed adapter script
/// and connection this module provisions.
pub fn create_virtual_schema_statement(name: &str, catalog: &str) -> String {
    format!(
        "CREATE VIRTUAL SCHEMA {name} USING {ADAPTER_SCRIPT} WITH \
         CONNECTION_NAME = {} CATALOG_NAME = {}",
        quote_literal(CONNECTION_NAME),
        quote_literal(catalog)
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

    #[test]
    fn settings_cfg_registers_the_named_jar_with_fixed_driver_tuning() {
        assert_eq!(
            settings_cfg("mysql-connector-j-9.6.0.jar"),
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
    fn settings_cfg_ends_with_a_trailing_newline() {
        assert!(settings_cfg("driver.jar").ends_with("NOSECURITY=YES\n"));
    }

    #[test]
    fn adapter_script_statement_names_both_jars_and_the_fixed_dispatcher_class() {
        assert_eq!(
            adapter_script_statement(
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
    fn every_adapter_script_directive_ends_with_a_semicolon() {
        let statement = adapter_script_statement("/buckets/a.jar", "/buckets/b.jar");
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

    #[test]
    fn adapter_script_statement_terminates_the_script_body() {
        let statement = adapter_script_statement("/buckets/a.jar", "/buckets/b.jar");
        assert!(
            statement.ends_with('/'),
            "a Java adapter script body must end with the `/` terminator: {statement}"
        );
    }

    #[test]
    fn create_schema_statement_is_idempotent_and_names_the_fixed_schema() {
        assert_eq!(
            create_schema_statement(),
            "CREATE SCHEMA IF NOT EXISTS SCHEMA_FOR_VS_SCRIPT"
        );
    }

    #[test]
    fn connection_statement_carries_a_jdbc_url_with_no_database_segment() {
        assert_eq!(
            connection_statement("172.28.5.10", 3306, "adbc_vs", "adbc_vs_password"),
            "CREATE OR REPLACE CONNECTION MYSQL_JDBC_CONNECTION TO 'jdbc:mysql://172.28.5.10:3306/' \
             USER 'adbc_vs' IDENTIFIED BY 'adbc_vs_password'"
        );
    }

    #[test]
    fn a_quote_in_the_password_cannot_end_the_literal() {
        let statement = connection_statement("host", 1, "u", "a'b");
        assert!(
            statement.ends_with("IDENTIFIED BY 'a''b'"),
            "an embedded quote must be doubled, not left to close the literal early: {statement}"
        );
    }

    #[test]
    fn create_virtual_schema_statement_names_the_fixed_adapter_script_and_connection() {
        assert_eq!(
            create_virtual_schema_statement("MYSQLVS_JDBC", "adbc_vs"),
            "CREATE VIRTUAL SCHEMA MYSQLVS_JDBC USING SCHEMA_FOR_VS_SCRIPT.ADAPTER_SCRIPT_MYSQL \
             WITH CONNECTION_NAME = 'MYSQL_JDBC_CONNECTION' CATALOG_NAME = 'adbc_vs'"
        );
    }

    #[test]
    fn create_virtual_schema_statement_selects_the_database_through_catalog_name() {
        let statement = create_virtual_schema_statement("MYSQLVS_JDBC", "adbc_vs_empty");
        assert!(statement.ends_with("CATALOG_NAME = 'adbc_vs_empty'"));
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

    #[test]
    fn every_pinned_digest_is_a_sha256_in_lower_case_hexadecimal() {
        for jar in [&VS_JAR, &CONNECTOR_JAR] {
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
        for jar in [&VS_JAR, &CONNECTOR_JAR] {
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
    fn the_driver_registration_names_the_connector_jar_that_is_uploaded_beside_it() {
        assert!(
            settings_cfg(CONNECTOR_JAR.filename)
                .contains(&format!("JAR={}\n", CONNECTOR_JAR.filename)),
            "the registered driver jar and the uploaded one must be the same file"
        );
    }

    #[test]
    fn the_driver_registration_and_its_jar_share_one_directory() {
        assert_eq!(driver_path(SETTINGS_FILE), "drivers/jdbc/settings.cfg");
        assert_eq!(
            driver_path(CONNECTOR_JAR.filename),
            "drivers/jdbc/mysql-connector-j-9.6.0.jar"
        );
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
        let path = driver_path(SETTINGS_FILE);
        assert!(
            !path.starts_with('/') && !path.contains("bfs:") && !path.contains("default/"),
            "a BucketFS destination is bucket-relative; anything else is rejected: {path}"
        );
    }
}
