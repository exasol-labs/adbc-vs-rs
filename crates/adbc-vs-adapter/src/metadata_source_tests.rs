//! The rules that bind every [`MetadataSource`] rather than one of them.
//!
//! Each source's own catalog, queries, and type table are asserted in that
//! technology's own test module. What is asserted here is what the trait
//! promises across all of them, so a source added later inherits the rules
//! instead of restating them.

use super::*;
use crate::exa_type::{self, ExasolType};
use crate::mysql_source::{self, COLLATION_NAMES_QUERY, SERVER_VERSION_QUERY};
use crate::postgres_source;
use crate::trino_source;
use crate::type_mapping::MappedType;

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
/// outside the source needs a field for `COLUMN_TYPE`, for a collation name,
/// or for whatever the next technology's catalog reports instead.
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
                MappedType::Decimal { .. }
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
/// A subject is the load-bearing half. It is what makes "MySQL's check does
/// not run for a PostgreSQL source" assertable at this tier: no source can be
/// constructed without a live ADBC connection, so what the two checks read is
/// the strongest available evidence that neither runs the other's.
struct Precondition {
    queries: Vec<&'static str>,
    subjects: Vec<&'static str>,
}

/// Matched exhaustively, so a technology added to [`Driver`] has to state the
/// precondition its own dialect needs rather than inherit one.
fn precondition(driver: Driver) -> Precondition {
    match driver {
        Driver::MySql => Precondition {
            queries: vec![COLLATION_NAMES_QUERY, SERVER_VERSION_QUERY],
            subjects: vec!["information_schema.COLLATIONS", "VERSION()"],
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

/// Each source checks the precondition its own dialect states, and reads no
/// other source's catalog to do it.
///
/// The preconditions genuinely differ — MySQL's rendering needs two named
/// collations at the server, PostgreSQL's needs a UTF-8 database encoding —
/// and each is stated by the dialect that renders for that source. A source
/// that ran another's check would refuse a server for a reason its own
/// rendering does not have, or accept one its rendering cannot be used
/// against.
#[test]
fn each_sources_precondition_reads_only_its_own_catalog() {
    for driver in Driver::ALL {
        let own = precondition(*driver);

        assert_eq!(
            own.subjects.is_empty(),
            own.queries.is_empty(),
            "`{}` must either name the subjects it reads or run no query at all; Trino \
             deliberately runs no precondition query, and a source that gained a query without \
             naming its subject would otherwise pass both loops below vacuously: {:?}",
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

/// Every type in the closed set, one representative each.
const EVERY_MAPPED_TYPE: &[MappedType] = &[
    MappedType::Decimal {
        precision: 36,
        scale: 6,
    },
    MappedType::Varchar { size: 2_000_000 },
    MappedType::Date,
    MappedType::Timestamp,
    MappedType::Double,
];

/// Where `mapped` sits in [`EVERY_MAPPED_TYPE`].
///
/// Matched with no wildcard, so a type added to [`MappedType`] fails to
/// compile here rather than being quietly left untested.
fn position_in_the_closed_set(mapped: MappedType) -> usize {
    match mapped {
        MappedType::Decimal { .. } => 0,
        MappedType::Varchar { .. } => 1,
        MappedType::Date => 2,
        MappedType::Timestamp => 3,
        MappedType::Double => 4,
    }
}

/// No source maps a column to an Exasol type outside the set a virtual column
/// can declare.
///
/// `vs-adapter/pushdown-dialect` argues about a closed set of declared types:
/// every rendering path, every `EMITS` clause, and every scan coercion is
/// total over it. A source that could produce a type outside it would leave
/// that argument resting on nothing.
#[test]
fn every_mapped_type_is_in_the_declarable_set() {
    let covered: Vec<usize> = EVERY_MAPPED_TYPE
        .iter()
        .map(|mapped| position_in_the_closed_set(*mapped))
        .collect();
    for position in 0..EVERY_MAPPED_TYPE.len() {
        assert!(
            covered.contains(&position),
            "every type in the closed set needs a representative here: {covered:?}"
        );
    }

    for mapped in EVERY_MAPPED_TYPE {
        let declarable = ExasolType::from(*mapped);

        assert!(
            !exa_type::to_ddl(declarable).is_empty(),
            "{mapped:?} must render as an EMITS declaration"
        );
        assert_eq!(
            exa_type::from_json(&exa_type::to_json(declarable))
                .unwrap_or_else(|error| panic!("{mapped:?} must round-trip: {error}")),
            declarable,
            "{mapped:?} must survive the encoding schemaMetadata and the scan specification share"
        );
    }
}
