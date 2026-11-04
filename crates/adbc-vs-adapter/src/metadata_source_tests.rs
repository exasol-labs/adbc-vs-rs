//! The rules that bind every [`MetadataSource`] rather than one of them.
//!
//! Each source's own catalog, queries, and type table are asserted in that
//! technology's own test module. What is asserted here is what the trait
//! promises across all of them, so a source added later inherits the rules
//! instead of restating them.

use super::*;
use crate::exa_type::{self, ExasolType};
use crate::mysql_source;
use crate::mysql_type_mapping;
use crate::postgres_source;
use crate::postgres_type_mapping;
use crate::trino_source;
use crate::trino_type_mapping;

/// One table assembled out of catalog rows in the shape the named source's own
/// catalog reports them: a column that maps, and a column of that source's own
/// that does not.
///
/// Matched exhaustively over the identities, so a technology added to
/// [`Driver`] cannot reach these rules without stating what its catalog looks
/// like — and each arm asks that source's own module for its fixture, because
/// no row struct is shared between two sources, which is the very thing
/// `each_source_maps_its_own_catalog_row` is about.
fn discovered(driver: Driver) -> DiscoveredTable {
    let mut tables = match driver {
        Driver::MySql => mysql_source::tests::one_mapped_and_one_unmapped_column(),
        Driver::Postgres => postgres_source::tests::one_mapped_and_one_unmapped_column(),
        Driver::Trino => trino_source::tests::one_mapped_and_one_unmapped_column(),
    };
    tables.pop().expect("the named table is always reported")
}

/// Every source maps its own catalog row, and hands back a column already
/// carrying the Exasol type its virtual column declares.
///
/// This is what keeps a shared source-column struct out of the interface: no
/// caller ever sees a column in the source's own vocabulary, so nothing
/// outside the source needs a field for `COLUMN_TYPE`, for a character set
/// name, or for whatever the next technology's catalog reports instead.
#[test]
fn each_source_maps_its_own_catalog_row() {
    for driver in Driver::ALL {
        let table = discovered(*driver);

        assert_eq!(
            table.columns.len(),
            1,
            "`{}` must hand back exactly the columns of its own catalog that map",
            driver.as_str()
        );
        assert!(
            matches!(
                table.columns[0].mapped.exasol_type,
                ExasolType::Decimal { .. }
            ),
            "`{}` must map the column itself rather than hand its catalog row on",
            driver.as_str()
        );

        assert_eq!(
            table.omissions.len(),
            1,
            "`{}` must record the column of its own catalog that does not map",
            driver.as_str()
        );
        let omission = &table.omissions[0];
        assert_eq!(omission.table, "orders");
        assert!(
            omission.source_type.is_some(),
            "`{}` must record the source's own name for the type it could not map: {omission:?}",
            driver.as_str()
        );
        assert!(
            !omission.reason.is_empty(),
            "`{}` must record why the column was left out: {omission:?}",
            driver.as_str()
        );
    }
}

/// What one source's precondition check reads: the statements it runs, and the
/// catalog subject naming those statements' whole business.
///
/// A subject is the load-bearing half. It is what makes "PostgreSQL's
/// encoding check does not run for another source" assertable at this tier:
/// no source can be constructed without a live ADBC connection, so what each
/// check reads is the strongest available evidence that none runs another's.
struct Precondition {
    queries: Vec<&'static str>,
    subjects: Vec<&'static str>,
}

/// Matched exhaustively, so a technology added to [`Driver`] has to state its
/// own precondition rather than inherit one.
///
/// MySQL's arm is empty: its check refuses an unsafe database name without a
/// round trip and queries neither a server collation nor a server version,
/// because the MySQL dialect renders no collation.
fn precondition(driver: Driver) -> Precondition {
    match driver {
        Driver::MySql => Precondition {
            queries: vec![],
            subjects: vec![],
        },
        Driver::Postgres => Precondition {
            queries: vec![postgres_source::server_encoding_query()],
            subjects: vec!["server_encoding"],
        },
        Driver::Trino => Precondition {
            queries: vec![],
            subjects: vec![],
        },
    }
}

/// Each source checks its own precondition, and reads no other source's
/// catalog to do it.
///
/// The preconditions genuinely differ — PostgreSQL's needs a UTF-8 database
/// encoding, while MySQL's and Trino's query nothing at the server — and each
/// is stated for the source it guards. A source that ran another's check
/// would refuse a server for a reason its own rendering does not have.
#[test]
fn each_sources_precondition_reads_only_its_own_catalog() {
    for driver in Driver::ALL {
        let own = precondition(*driver);

        assert_eq!(
            own.subjects.is_empty(),
            own.queries.is_empty(),
            "`{}` must either name the subjects it reads or run no query at all; MySQL and \
             Trino deliberately run no precondition query, and a source that gained a query \
             without naming its subject would otherwise pass both loops below vacuously: {:?}",
            driver.as_str(),
            own.queries
        );

        for subject in &own.subjects {
            assert!(
                own.queries.iter().any(|sql| sql.contains(subject)),
                "`{}`'s precondition must read `{subject}`: {:?}",
                driver.as_str(),
                own.queries
            );
        }

        for other in Driver::ALL.iter().filter(|other| *other != driver) {
            for subject in precondition(*other).subjects {
                assert!(
                    !own.queries.iter().any(|sql| sql.contains(subject)),
                    "`{}`'s precondition must not read `{subject}`, which is `{}`'s: {:?}",
                    driver.as_str(),
                    other.as_str(),
                    own.queries
                );
            }
        }
    }
}

/// A source's namespace is checked against the segment count its own
/// declaration states, and that declaration is the one owner of the count: the
/// other reader is the check that a `CONNECTION` address's path spells it. The
/// two run at different moments — the address check before a connection exists,
/// this one only once a source has answered — so a technology whose depth the
/// two disagreed about would have no single point at which the disagreement
/// showed.
///
/// Both directions are driven per identity. The too-shallow one is reachable
/// only for an identity declaring more than one segment — Trino is the first —
/// because a qualifier of no segment cannot be built at all, so for a
/// one-segment identity the shallow case stays the address check's
/// `carried == 0` case.
#[test]
fn a_namespace_disagreeing_with_its_drivers_declared_depth_is_refused_naming_both_counts() {
    for driver in Driver::ALL {
        let declared = driver.qualifier_segment_count();
        let segments = |count: usize| {
            SourceQualifier::of((0..count).map(|index| format!("n{index}")).collect())
                .expect("a qualifier of at least one segment is well formed")
        };
        let refusal_names_both_counts = |produced: usize| {
            let error = check_namespace_arity(*driver, &segments(produced))
                .expect_err("a namespace disagreeing with the declaration must be refused");
            let message = error.to_string();
            assert!(
                matches!(error, MetadataError::NamespaceArity { .. }),
                "the refusal must be the namespace-arity one: {message}"
            );
            assert!(
                message.contains(driver.as_str())
                    && message.contains(&declared.to_string())
                    && message.contains(&produced.to_string()),
                "the error must name the driver, the depth declared, and the depth produced: \
                 {message}"
            );
        };

        check_namespace_arity(*driver, &segments(declared)).unwrap_or_else(|error| {
            panic!(
                "`{}` must accept the depth it declares: {error}",
                driver.as_str()
            )
        });

        refusal_names_both_counts(declared + 1);
        if declared > 1 {
            refusal_names_both_counts(declared - 1);
        }
    }
}

/// The `dataType` spelling of each of the seven types a mapped column may
/// declare.
///
/// Matched with no wildcard, so a variant added to [`ExasolType`] fails to
/// compile here rather than silently widening the set the pushdown closure
/// argument rests on.
fn closed_set_name(declared: ExasolType) -> &'static str {
    match declared {
        ExasolType::Decimal { .. } => "DECIMAL",
        ExasolType::Varchar { .. } => "VARCHAR",
        ExasolType::Char { .. } => "CHAR",
        ExasolType::Date => "DATE",
        ExasolType::Timestamp { .. } => "TIMESTAMP",
        ExasolType::Double => "DOUBLE",
        ExasolType::Boolean => "BOOLEAN",
    }
}

/// Every column the named source's own mapping tests cover, as the catalog
/// spelled its type, mapped through that source's mapping.
fn every_covered_mapping(driver: Driver) -> Vec<(String, MappedColumn)> {
    fn mapped<C>(
        columns: Vec<C>,
        spelling: impl Fn(&C) -> &str,
        map: impl Fn(&C) -> Result<MappedColumn, String>,
    ) -> Vec<(String, MappedColumn)> {
        columns
            .iter()
            .map(|column| {
                let mapped = map(column).unwrap_or_else(|reason| {
                    panic!("covered column `{}` must map: {reason}", spelling(column))
                });
                (spelling(column).to_string(), mapped)
            })
            .collect()
    }
    match driver {
        Driver::MySql => mapped(
            mysql_type_mapping::tests::one_column_of_every_mapped_type(),
            |column| &column.data_type,
            mysql_type_mapping::map_column,
        ),
        Driver::Postgres => mapped(
            postgres_type_mapping::tests::one_column_of_every_mapped_type(),
            |column| &column.data_type,
            postgres_type_mapping::map_column,
        ),
        Driver::Trino => mapped(
            trino_type_mapping::tests::one_column_of_every_mapped_type(),
            |column| &column.data_type,
            trino_type_mapping::map_column,
        ),
    }
}

/// `vs-adapter/type-mapping`, scenario "A mapped column records its source
/// type in its adapterNotes": the recorded value is the catalog's own
/// spelling, which each technology's read expression is keyed on.
#[test]
fn every_mapped_column_records_the_catalog_spelling_of_its_type() {
    for driver in Driver::ALL {
        for (spelling, mapped) in every_covered_mapping(*driver) {
            assert_eq!(
                mapped.source_type,
                spelling,
                "`{}` must record the type as its catalog spelled it",
                driver.as_str()
            );
        }
    }
}

/// `vs-adapter/type-mapping`, scenario "A mapped column declares only an
/// Exasol type this release produces": no source maps a column to an Exasol
/// type outside the seven an `EMITS` column may declare.
///
/// The round trip is the membership test, because `from_json` reads exactly
/// that set and refuses `TIMESTAMP WITH LOCAL TIME ZONE`, `INTERVAL`,
/// `GEOMETRY`, and `HASHTYPE`.
#[test]
fn every_mapped_column_declares_a_type_of_the_closed_set() {
    for driver in Driver::ALL {
        let covered = every_covered_mapping(*driver);
        assert!(
            !covered.is_empty(),
            "`{}` must cover at least one mapped type",
            driver.as_str()
        );

        for exasol_type in covered.into_iter().map(|(_, mapped)| mapped.exasol_type) {
            let json = exa_type::to_json(exasol_type);
            assert_eq!(
                json["type"],
                closed_set_name(exasol_type),
                "`{}` declares {exasol_type:?} as {json}",
                driver.as_str()
            );
            assert_eq!(
                exa_type::from_json(&json),
                Ok(exasol_type),
                "`{}` declares {exasol_type:?}, which must read back from {json} as one of the \
                 seven declarable types",
                driver.as_str()
            );
        }
    }
}
