use adbc_vs_scan::driver::Driver;

use crate::pushdown_query::Query;
use crate::sql_dialect::dialect_for;

use super::*;

/// A pushdown request that exercises `capability`, and the fragment `driver`'s
/// dialect must put in the statement for it.
///
/// Matched exhaustively over the identities, and each arm reads that dialect's
/// own fixture table: the request shapes are the protocol's, but what a
/// correctly rendered statement looks like is the dialect's own answer, so
/// neither can be written once for both.
fn query_exercising(driver: Driver, capability: &str) -> (&'static str, Query) {
    match driver {
        Driver::MySql => crate::mysql_dialect::tests::query_exercising(capability),
        Driver::Postgres => crate::postgres_dialect::tests::query_exercising(capability),
        Driver::Trino => crate::trino_dialect::tests::query_exercising(capability),
    }
}

/// The `getCapabilities` response for `driver`, read the way Exasol reads it.
fn advertised_by(driver: Driver) -> Vec<String> {
    let response = capabilities_response(dialect_for(driver));

    response["capabilities"]
        .as_array()
        .unwrap_or_else(|| panic!("the response must carry a `capabilities` array: {response}"))
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .unwrap_or_else(|| panic!("every advertised capability is a string: {response}"))
                .to_string()
        })
        .collect()
}

/// `vs-adapter/pushdown-dialect`, decision `[2]`: the advertised set and the
/// renderer are one decision with one owner, and nothing in the type system
/// enforces their agreement. Rendering must not only succeed — it must put the
/// capability's own fragment in the statement, or the capability reached a
/// path that dropped it and Exasol returns rows for a fragment it believes was
/// applied.
///
/// Run once per identity rather than for one named dialect: the invariant is
/// what makes a per-dialect capability set safe at all, so every dialect owes
/// it, and a dialect added later inherits the arm instead of needing its own.
#[test]
fn every_advertised_capability_renders_per_dialect() {
    for driver in Driver::ALL {
        let dialect = dialect_for(*driver);
        assert!(
            !dialect.capabilities().is_empty(),
            "`{}` advertises nothing, so this invariant would pass vacuously",
            driver.as_str()
        );

        for capability in dialect.capabilities() {
            let (fragment, query) = query_exercising(*driver, capability);

            let sql = dialect.render(&query).unwrap_or_else(|error| {
                panic!(
                    "`{}` advertises `{capability}` but has no rendering path for it: {error}",
                    driver.as_str()
                )
            });

            assert!(
                sql.contains(fragment),
                "`{}` rendered `{capability}` as a statement carrying no `{fragment}`, so its \
                 fragment reached a path that dropped it: {sql}",
                driver.as_str()
            );
        }
    }
}

/// Every advertised capability's own fixture is re-rendered, rather than one
/// named shape, because which fixture carries the mutable state that could
/// falsify determinism is per dialect: alias assignment walks the `from` tree
/// with a running position, so a dialect advertising `JOIN` is exercised
/// through its join fixture here, while a dialect advertising no join has no
/// such state to falsify and its own fixtures are the whole of what it can be
/// asked. Naming one capability instead would ask a dialect for a fixture it
/// never advertised.
#[test]
fn same_request_renders_identically_per_dialect() {
    for driver in Driver::ALL {
        let dialect = dialect_for(*driver);

        for capability in dialect.capabilities() {
            let (_, query) = query_exercising(*driver, capability);

            let first = dialect.render(&query).expect("render must succeed");
            let second = dialect.render(&query).expect("render must succeed");

            assert_eq!(
                first,
                second,
                "`{}` must render the `{capability}` request byte-for-byte identically on every \
                 call",
                driver.as_str()
            );
        }
    }
}

/// Every `MainCapability` constant this release advertises for any dialect, as
/// `vs-adapter/pushdown-dialect`'s capability scenario names them. Every other
/// advertised entry is a function or literal capability, told apart by its
/// prefix rather than by a second list. Read only to recognize an entry: an
/// advertised name in neither this list nor a prefix is a capability string
/// Exasol would not recognize at all.
const MAIN_CAPABILITY_CONSTANTS: &[&str] = &[
    "SELECTLIST_PROJECTION",
    "SELECTLIST_EXPRESSIONS",
    "FILTER_EXPRESSIONS",
    "AGGREGATE_SINGLE_GROUP",
    "AGGREGATE_GROUP_BY_COLUMN",
    "AGGREGATE_GROUP_BY_TUPLE",
    "JOIN",
    "JOIN_TYPE_INNER",
    "JOIN_TYPE_LEFT_OUTER",
    "JOIN_TYPE_RIGHT_OUTER",
    "JOIN_TYPE_FULL_OUTER",
    "JOIN_CONDITION_EQUI",
];

/// The `MainCapability` floor every dialect this release ships holds, which
/// is [`MAIN_CAPABILITY_CONSTANTS`] less the entries at least one dialect
/// withholds. Two entries are per dialect rather than shared, each stated
/// here by which side of it holds which: `JOIN_TYPE_FULL_OUTER` — PostgreSQL
/// and Trino advertise it, MySQL withholds it, because MySQL implements no
/// `FULL OUTER JOIN` and emulating one needs the union rewrite mission.md
/// places out of scope; and `AGGREGATE_GROUP_BY_TUPLE` — Trino alone
/// advertises it, because gate 2.3(i) measured a multi-column `GROUP BY`
/// executing on Trino with no rendering change any dialect's `render_grouping`
/// needs, and neither MySQL nor PostgreSQL was measured or granted the same
/// widening. Each dialect's own exact-set test states its full advertised
/// set, this list only the part every one of them shares.
const SHARED_MAIN_CAPABILITIES: &[&str] = &[
    "SELECTLIST_PROJECTION",
    "SELECTLIST_EXPRESSIONS",
    "FILTER_EXPRESSIONS",
    "AGGREGATE_SINGLE_GROUP",
    "AGGREGATE_GROUP_BY_COLUMN",
    "JOIN",
    "JOIN_TYPE_INNER",
    "JOIN_TYPE_LEFT_OUTER",
    "JOIN_TYPE_RIGHT_OUTER",
    "JOIN_CONDITION_EQUI",
];

/// The prefixes a capability that is not a `MainCapability` constant carries,
/// one per question Exasol asks the set: which predicates, which aggregate
/// functions, and which literal kinds a delegated fragment may contain.
const FUNCTION_CAPABILITY_PREFIXES: &[&str] = &["FN_PRED_", "FN_AGG_", "LITERAL_"];

fn assert_advertises(capabilities: &[String], required: &str) {
    assert!(
        capabilities.iter().any(|entry| entry == required),
        "capability `{required}` has a rendering path and must be advertised: {capabilities:?}"
    );
}

/// `vs-adapter/pushdown-dialect`'s capability scenario: the response names the
/// request it answers, carries every main capability this release renders for
/// every dialect, and carries nothing outside the naming convention that
/// scenario fixes — an entry matching neither the `MainCapability` list nor a
/// function prefix is a capability string Exasol would not recognize at all.
#[test]
fn advertised_set_matches_the_rendered_fragments() {
    for driver in Driver::ALL {
        let response = capabilities_response(dialect_for(*driver));
        assert_eq!(
            response["type"], "getCapabilities",
            "the response must name the request it answers, or Exasol reads no capability out of \
             it at all: {response}"
        );

        let capabilities = advertised_by(*driver);
        for required in SHARED_MAIN_CAPABILITIES {
            assert_advertises(&capabilities, required);
        }
        for capability in &capabilities {
            assert!(
                MAIN_CAPABILITY_CONSTANTS.contains(&capability.as_str())
                    || FUNCTION_CAPABILITY_PREFIXES
                        .iter()
                        .any(|prefix| capability.starts_with(prefix)),
                "`{}` advertises `{capability}`, which is neither a `MainCapability` constant \
                 this release renders nor a function or literal capability: {capabilities:?}",
                driver.as_str()
            );
        }
    }
}

/// Every capability release 0.2 advertised for MySQL, in that release's own
/// order.
///
/// The set is a released interface: Exasol delegates a fragment for each entry
/// and re-applies none of them, so an entry gained or lost changes which
/// queries reach MySQL and which rows come back. It is written out in full
/// rather than derived, because a derivation would be the second capability
/// list `vs-adapter/pushdown-dialect` forbids, and because the point of
/// asserting it is to have one statement of the set that did not move when the
/// values did.
const RELEASE_0_2_MYSQL_CAPABILITIES: &[&str] = &[
    "SELECTLIST_PROJECTION",
    "SELECTLIST_EXPRESSIONS",
    "FILTER_EXPRESSIONS",
    "AGGREGATE_SINGLE_GROUP",
    "AGGREGATE_GROUP_BY_COLUMN",
    // `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`, `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT` are
    // withheld for every dialect, for one reason that is not about any target's SQL:
    // a floating-point sum is order-dependent because addition is not associative, and
    // the accumulation order is not stable between two runs under a parallel plan.
    // Advertising either would return a value that can differ from Exasol's own — the
    // silent wrong-value failure, not an error. PostgreSQL's exact-numeric `sum` being
    // exact and order-independent is not a reason to advertise it there either.
    "FN_AGG_COUNT",
    "FN_AGG_COUNT_STAR",
    "FN_AGG_COUNT_DISTINCT",
    "FN_AGG_MIN",
    "FN_AGG_MAX",
    // `AGGREGATE_GROUP_BY_EXPRESSION`, `AGGREGATE_GROUP_BY_TUPLE`, and
    // `AGGREGATE_HAVING` are withheld: gate 3.0 measured that no aggregate reaches the
    // adapter without `SELECTLIST_EXPRESSIONS`, and the grouping cap is withheld by
    // omission rather than by a separate flag.
    "FN_PRED_AND",
    "FN_PRED_OR",
    "FN_PRED_NOT",
    "FN_PRED_EQUAL",
    "FN_PRED_NOTEQUAL",
    "FN_PRED_LESS",
    "FN_PRED_LESSEQUAL",
    "FN_PRED_IS_NULL",
    "FN_PRED_IS_NOT_NULL",
    "FN_PRED_LIKE",
    "FN_PRED_LIKE_ESCAPE",
    "FN_PRED_BETWEEN",
    "FN_PRED_IN_CONSTLIST",
    // `FN_PRED_REGEXP_LIKE`, `FN_PRED_IS_JSON`, `FN_PRED_IS_NOT_JSON`, `LITERAL_BOOL`,
    // `LITERAL_INTERVAL`, and `LITERAL_TIMESTAMP_UTC` are withheld: no dialect's
    // `render_predicate` or `render_literal` has a rendering path for them.
    "LITERAL_NULL",
    "LITERAL_EXACTNUMERIC",
    "LITERAL_DOUBLE",
    "LITERAL_STRING",
    "LITERAL_DATE",
    "LITERAL_TIMESTAMP",
    // MySQL implements no `FULL OUTER JOIN` and this release renders no non-equi
    // condition, so `JOIN_TYPE_FULL_OUTER` and `JOIN_CONDITION_ALL` are withheld — the
    // fragment `render_predicate` can only fail on. `ORDER_BY_COLUMN`,
    // `ORDER_BY_EXPRESSION`, `LIMIT`, and `LIMIT_WITH_OFFSET` are withheld for every
    // dialect: a script-emitted derived table carries no ordering guarantee, so Exasol
    // keeps both clauses regardless of what is advertised.
    "JOIN",
    "JOIN_TYPE_INNER",
    "JOIN_TYPE_LEFT_OUTER",
    "JOIN_TYPE_RIGHT_OUTER",
    "JOIN_CONDITION_EQUI",
];

/// `vs-adapter/mysql-pushdown`: MySQL advertises exactly what release 0.2
/// advertised, element for element.
///
/// The values themselves are declared data now — they live in
/// `crates/adbc-vs-scan/drivers/mysql.toml` and reach the dialect through the
/// build — and a set that is generated is a set that can be generated wrong: a
/// dropped entry silently stops a delegation Exasol used to hand over, and an
/// added one delegates a fragment the renderer may not express. Compared as
/// sets rather than in order, because Exasol reads the array as a set and the
/// declaration groups it for a reader; a duplicate still fails, since it makes
/// the advertised sequence one element longer than the released one.
#[test]
fn mysql_capability_set_is_exact() {
    assert_set_is(Driver::MySql, RELEASE_0_2_MYSQL_CAPABILITIES);
}

/// Every capability the PostgreSQL dialect advertises, written out in full for
/// the reason [`RELEASE_0_2_MYSQL_CAPABILITIES`] is: a set that is generated is
/// a set that can be generated wrong, and deriving this one from MySQL's would
/// be the second capability list `vs-adapter/pushdown-dialect` forbids — and
/// would state that the two sets are one decision, which
/// `vs-adapter/postgresql-pushdown` says they are not.
const POSTGRES_CAPABILITIES: &[&str] = &[
    "SELECTLIST_PROJECTION",
    "SELECTLIST_EXPRESSIONS",
    "FILTER_EXPRESSIONS",
    "AGGREGATE_SINGLE_GROUP",
    "AGGREGATE_GROUP_BY_COLUMN",
    "FN_AGG_COUNT",
    "FN_AGG_COUNT_STAR",
    "FN_AGG_COUNT_DISTINCT",
    "FN_AGG_MIN",
    "FN_AGG_MAX",
    "FN_PRED_AND",
    "FN_PRED_OR",
    "FN_PRED_NOT",
    "FN_PRED_EQUAL",
    "FN_PRED_NOTEQUAL",
    "FN_PRED_LESS",
    "FN_PRED_LESSEQUAL",
    "FN_PRED_IS_NULL",
    "FN_PRED_IS_NOT_NULL",
    "FN_PRED_LIKE",
    "FN_PRED_LIKE_ESCAPE",
    "FN_PRED_BETWEEN",
    "FN_PRED_IN_CONSTLIST",
    "LITERAL_NULL",
    "LITERAL_EXACTNUMERIC",
    "LITERAL_DOUBLE",
    "LITERAL_STRING",
    "LITERAL_DATE",
    "LITERAL_TIMESTAMP",
    "JOIN",
    "JOIN_TYPE_INNER",
    "JOIN_TYPE_LEFT_OUTER",
    "JOIN_TYPE_RIGHT_OUTER",
    // `JOIN_TYPE_FULL_OUTER` is the whole of the difference between this set and
    // `RELEASE_0_2_MYSQL_CAPABILITIES`: PostgreSQL implements the keyword directly,
    // where MySQL implements none and emulating one needs the union rewrite mission.md
    // places out of scope. It withholds `JOIN_CONDITION_ALL` for the same reason MySQL
    // does, which is what keeps the full join safe as well: PostgreSQL refuses a full
    // join whose condition is neither merge-joinable nor hash-joinable, and the
    // equality `JOIN_CONDITION_EQUI` admits is both.
    "JOIN_TYPE_FULL_OUTER",
    "JOIN_CONDITION_EQUI",
];

/// `vs-adapter/postgresql-pushdown`: PostgreSQL advertises exactly the set
/// `drivers/postgresql.toml` declares, element for element.
#[test]
fn postgres_capability_set_is_exact() {
    assert_set_is(Driver::Postgres, POSTGRES_CAPABILITIES);
}

/// Every capability the Trino dialect advertises, written out independently
/// of [`POSTGRES_CAPABILITIES`] for the reason [`RELEASE_0_2_MYSQL_CAPABILITIES`]
/// is: a set that is generated is a set that can be generated wrong, and
/// deriving this one from PostgreSQL's would be the second capability list
/// `vs-adapter/pushdown-dialect` forbids.
///
/// `drivers/trino.toml` states the set as PostgreSQL's 35 entries plus
/// `AGGREGATE_GROUP_BY_TUPLE`, and that is exactly what this list spells out:
/// every entry [`POSTGRES_CAPABILITIES`] carries, plus the one entry gate
/// 2.3(i) measured Trino alone able to advertise — a multi-column `GROUP BY`
/// executes on Trino and needs no rendering change `render_grouping` does not
/// already have.
const TRINO_CAPABILITIES: &[&str] = &[
    "SELECTLIST_PROJECTION",
    "SELECTLIST_EXPRESSIONS",
    "FILTER_EXPRESSIONS",
    "AGGREGATE_SINGLE_GROUP",
    "AGGREGATE_GROUP_BY_COLUMN",
    "AGGREGATE_GROUP_BY_TUPLE",
    "FN_AGG_COUNT",
    "FN_AGG_COUNT_STAR",
    "FN_AGG_COUNT_DISTINCT",
    "FN_AGG_MIN",
    "FN_AGG_MAX",
    "FN_PRED_AND",
    "FN_PRED_OR",
    "FN_PRED_NOT",
    "FN_PRED_EQUAL",
    "FN_PRED_NOTEQUAL",
    "FN_PRED_LESS",
    "FN_PRED_LESSEQUAL",
    "FN_PRED_IS_NULL",
    "FN_PRED_IS_NOT_NULL",
    "FN_PRED_LIKE",
    "FN_PRED_LIKE_ESCAPE",
    "FN_PRED_BETWEEN",
    "FN_PRED_IN_CONSTLIST",
    "LITERAL_NULL",
    "LITERAL_EXACTNUMERIC",
    "LITERAL_DOUBLE",
    "LITERAL_STRING",
    "LITERAL_DATE",
    "LITERAL_TIMESTAMP",
    "JOIN",
    "JOIN_TYPE_INNER",
    "JOIN_TYPE_LEFT_OUTER",
    "JOIN_TYPE_RIGHT_OUTER",
    "JOIN_TYPE_FULL_OUTER",
    "JOIN_CONDITION_EQUI",
];

/// `vs-adapter/trino-pushdown`: Trino advertises exactly the set
/// `drivers/trino.toml` declares, element for element.
#[test]
fn trino_capability_set_is_exact() {
    assert_set_is(Driver::Trino, TRINO_CAPABILITIES);
}

/// Asserts that `driver` advertises exactly `expected`, compared as sets.
///
/// Exasol reads the array as a set, and a declaration groups its list for a
/// reader rather than in any meaningful order, so a reordering that changes no
/// behaviour must not fail. A duplicate still fails, since it makes the
/// advertised sequence one element longer than the stated one.
fn assert_set_is(driver: Driver, expected: &[&str]) {
    let mut advertised = advertised_by(driver);
    advertised.sort();
    let mut expected: Vec<String> = expected
        .iter()
        .map(|capability| (*capability).to_string())
        .collect();
    expected.sort();

    assert_eq!(
        advertised,
        expected,
        "the advertised `{}` set must be the stated one element for element; a difference here \
         changes which fragments Exasol delegates to an existing virtual schema",
        driver.as_str()
    );
}
