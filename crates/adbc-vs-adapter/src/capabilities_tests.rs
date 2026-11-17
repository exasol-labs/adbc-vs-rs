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

/// A request that exercises `capability` over a `BOOLEAN` operand, and the
/// fragment `driver`'s dialect must put in the statement for it, for a dialect
/// that renders the capability differently for that one declared type.
///
/// Matched exhaustively like [`query_exercising`], so a dialect added later
/// states whether it has such an arm.
fn boolean_operand_exercising(driver: Driver, capability: &str) -> Option<(&'static str, Query)> {
    match driver {
        Driver::MySql | Driver::Trino => None,
        Driver::Postgres => crate::postgres_dialect::tests::boolean_operand_exercising(capability),
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
///
/// A capability a dialect renders differently for a `BOOLEAN` operand has that
/// operand exercised as its own arm.
#[test]
fn every_advertised_capability_renders_per_dialect() {
    let mut boolean_arms = 0;
    for driver in Driver::ALL {
        let dialect = dialect_for(*driver);
        assert!(
            !dialect.capabilities().is_empty(),
            "`{}` advertises nothing, so this invariant would pass vacuously",
            driver.as_str()
        );

        for capability in dialect.capabilities() {
            let mut exercised = vec![query_exercising(*driver, capability)];
            if let Some(boolean_arm) = boolean_operand_exercising(*driver, capability) {
                boolean_arms += 1;
                exercised.push(boolean_arm);
            }

            for (fragment, query) in exercised {
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
    assert!(
        boolean_arms > 0,
        "PostgreSQL renders `MIN` and `MAX` differently over a `BOOLEAN` operand, so the BOOLEAN \
         arm must run at least once"
    );
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
    "AGGREGATE_GROUP_BY_EXPRESSION",
    "AGGREGATE_GROUP_BY_TUPLE",
    "AGGREGATE_HAVING",
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
    "AGGREGATE_GROUP_BY_EXPRESSION",
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

/// The scalar-function capabilities `vs-adapter/pushdown-scalar-function`
/// names, the only `FN_` entries without a further prefix a dialect may carry.
const SCALAR_FUNCTION_CAPABILITIES: &[&str] = &[
    "FN_YEAR",
    "FN_MONTH",
    "FN_DAY",
    "FN_UPPER",
    "FN_LOWER",
    "FN_TRIM",
    "FN_LENGTH",
    "FN_SUBSTR",
    "FN_ADD",
    "FN_SUB",
    "FN_MULT",
];

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
                    || SCALAR_FUNCTION_CAPABILITIES.contains(&capability.as_str())
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

/// Every capability the MySQL dialect advertises.
///
/// The set is a released interface: Exasol delegates a fragment for each entry
/// and re-applies none of them, so an entry gained or lost changes which
/// queries reach MySQL and which rows come back. It is written out in full
/// rather than derived, because a derivation would be the second capability
/// list `vs-adapter/pushdown-dialect` forbids, and because the point of
/// asserting it is to have one statement of the set that did not move when the
/// values did.
const MYSQL_CAPABILITIES: &[&str] = &[
    "SELECTLIST_PROJECTION",
    "SELECTLIST_EXPRESSIONS",
    "FILTER_EXPRESSIONS",
    "AGGREGATE_SINGLE_GROUP",
    "AGGREGATE_GROUP_BY_COLUMN",
    "AGGREGATE_GROUP_BY_EXPRESSION",
    "AGGREGATE_HAVING",
    "FN_AGG_COUNT",
    "FN_AGG_COUNT_STAR",
    "FN_AGG_COUNT_DISTINCT",
    "FN_AGG_MIN",
    "FN_AGG_MAX",
    // Advertised although a floating-point sum follows the source's accumulation order:
    // refusing a DOUBLE operand at render time would fail a query Exasol answers.
    "FN_AGG_SUM",
    "FN_AGG_SUM_DISTINCT",
    "FN_AGG_AVG",
    "FN_AGG_AVG_DISTINCT",
    // AGGREGATE_GROUP_BY_TUPLE is withheld: MySQL groups by one key.
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
    "FN_YEAR",
    "FN_MONTH",
    "FN_DAY",
    "FN_UPPER",
    "FN_LOWER",
    // FN_TRIM is withheld: MySQL's TRIM removes a substring rather than a set of characters.
    "FN_LENGTH",
    "FN_SUBSTR",
    "FN_ADD",
    "FN_SUB",
    "FN_MULT",
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

/// `vs-adapter/mysql-pushdown`: MySQL advertises exactly the 50 entries of
/// [`MYSQL_CAPABILITIES`], element for element.
///
/// The values themselves are declared data now — they live in
/// `crates/adbc-vs-scan/drivers/mysql.toml` and reach the dialect through the
/// build — and a set that is generated is a set that can be generated wrong: a
/// dropped entry silently stops a delegation Exasol used to hand over, and an
/// added one delegates a fragment the renderer may not express. Compared as
/// sets rather than in order, because Exasol reads the array as a set and the
/// declaration groups it for a reader; a duplicate still fails, since it makes
/// the advertised sequence one element longer than the stated one.
#[test]
fn mysql_capability_set_is_exact() {
    assert_set_is(Driver::MySql, MYSQL_CAPABILITIES);
}

/// Every capability the PostgreSQL dialect advertises, written out in full for
/// the reason [`MYSQL_CAPABILITIES`] is: a set that is generated is
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
    "AGGREGATE_GROUP_BY_EXPRESSION",
    "AGGREGATE_HAVING",
    "FN_AGG_COUNT",
    "FN_AGG_COUNT_STAR",
    "FN_AGG_COUNT_DISTINCT",
    "FN_AGG_MIN",
    "FN_AGG_MAX",
    // Advertised although a floating-point sum follows the source's accumulation order:
    // refusing a DOUBLE operand at render time would fail a query Exasol answers.
    "FN_AGG_SUM",
    "FN_AGG_SUM_DISTINCT",
    "FN_AGG_AVG",
    "FN_AGG_AVG_DISTINCT",
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
    "FN_YEAR",
    "FN_MONTH",
    "FN_DAY",
    "FN_UPPER",
    "FN_LOWER",
    "FN_TRIM",
    "FN_LENGTH",
    "FN_SUBSTR",
    "FN_ADD",
    "FN_SUB",
    "FN_MULT",
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
    // `JOIN_TYPE_FULL_OUTER` and `FN_TRIM` are the whole of the difference between this set
    // and `MYSQL_CAPABILITIES`: PostgreSQL implements the keyword directly,
    // where MySQL implements none and emulating one needs the union rewrite mission.md
    // places out of scope. It withholds `JOIN_CONDITION_ALL` for the same reason MySQL
    // does, which is what keeps the full join safe as well: PostgreSQL refuses a full
    // join whose condition is neither merge-joinable nor hash-joinable, and the
    // equality `JOIN_CONDITION_EQUI` admits is both.
    "JOIN_TYPE_FULL_OUTER",
    "JOIN_CONDITION_EQUI",
];

/// `vs-adapter/postgresql-pushdown`: PostgreSQL advertises exactly the 52
/// entries of [`POSTGRES_CAPABILITIES`], element for element.
#[test]
fn postgres_capability_set_is_exact() {
    assert_set_is(Driver::Postgres, POSTGRES_CAPABILITIES);
}

/// Every capability the Trino dialect advertises, written out independently
/// of [`POSTGRES_CAPABILITIES`] for the reason [`MYSQL_CAPABILITIES`]
/// is: a set that is generated is a set that can be generated wrong, and
/// deriving this one from PostgreSQL's would be the second capability list
/// `vs-adapter/pushdown-dialect` forbids.
///
/// `drivers/trino.toml` states the set as PostgreSQL's 52 entries, minus
/// `FN_PRED_LIKE_ESCAPE`, plus `AGGREGATE_GROUP_BY_TUPLE`, and that is exactly
/// what this list spells out. `FN_PRED_LIKE_ESCAPE` is withheld because Trino
/// defines no default escape character and rejects an escape character
/// followed by anything other than `%`, `_`, or itself. `AGGREGATE_GROUP_BY_TUPLE`
/// is the one entry gate 2.3(i) measured Trino alone able to advertise: a
/// multi-column `GROUP BY` executes on Trino and needs no rendering change
/// `render_grouping` does not already have.
const TRINO_CAPABILITIES: &[&str] = &[
    "SELECTLIST_PROJECTION",
    "SELECTLIST_EXPRESSIONS",
    "FILTER_EXPRESSIONS",
    "AGGREGATE_SINGLE_GROUP",
    "AGGREGATE_GROUP_BY_COLUMN",
    "AGGREGATE_GROUP_BY_EXPRESSION",
    "AGGREGATE_GROUP_BY_TUPLE",
    "AGGREGATE_HAVING",
    "FN_AGG_COUNT",
    "FN_AGG_COUNT_STAR",
    "FN_AGG_COUNT_DISTINCT",
    "FN_AGG_MIN",
    "FN_AGG_MAX",
    // Advertised although a floating-point sum follows the source's accumulation order:
    // refusing a DOUBLE operand at render time would fail a query Exasol answers.
    "FN_AGG_SUM",
    "FN_AGG_SUM_DISTINCT",
    "FN_AGG_AVG",
    "FN_AGG_AVG_DISTINCT",
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
    "FN_PRED_BETWEEN",
    "FN_PRED_IN_CONSTLIST",
    "FN_YEAR",
    "FN_MONTH",
    "FN_DAY",
    "FN_UPPER",
    "FN_LOWER",
    "FN_TRIM",
    "FN_LENGTH",
    "FN_SUBSTR",
    "FN_ADD",
    "FN_SUB",
    "FN_MULT",
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

/// `vs-adapter/trino-pushdown`: Trino advertises exactly the 52 entries of
/// [`TRINO_CAPABILITIES`], element for element.
#[test]
fn trino_capability_set_is_exact() {
    assert_set_is(Driver::Trino, TRINO_CAPABILITIES);
}

/// `vs-adapter/trino-pushdown`: the Trino and PostgreSQL sets differ by exactly
/// two named entries, one in each direction. Each set is stated by its own
/// declaration, so the 51 entries they share are an outcome of two decisions,
/// and a change to either set fails here rather than being absorbed.
#[test]
fn trino_and_postgresql_sets_differ_by_two_named_entries() {
    let trino = advertised_by(Driver::Trino);
    let postgresql = advertised_by(Driver::Postgres);

    let only_trino: Vec<&String> = trino
        .iter()
        .filter(|capability| !postgresql.contains(capability))
        .collect();
    let only_postgresql: Vec<&String> = postgresql
        .iter()
        .filter(|capability| !trino.contains(capability))
        .collect();

    assert_eq!(only_trino, ["AGGREGATE_GROUP_BY_TUPLE"]);
    assert_eq!(only_postgresql, ["FN_PRED_LIKE_ESCAPE"]);
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

/// `vs-adapter/pushdown-scalar-function`: every dialect advertises the scalar
/// functions it renders, `FN_TRIM` everywhere but MySQL, and no other.
#[test]
fn each_dialect_advertises_the_scalar_functions_it_renders() {
    for driver in Driver::ALL {
        let advertised: Vec<String> = advertised_by(*driver)
            .into_iter()
            .filter(|capability| {
                capability.starts_with("FN_")
                    && !FUNCTION_CAPABILITY_PREFIXES
                        .iter()
                        .any(|prefix| capability.starts_with(prefix))
            })
            .collect();
        let expected: Vec<&str> = SCALAR_FUNCTION_CAPABILITIES
            .iter()
            .copied()
            .filter(|capability| *driver != Driver::MySql || *capability != "FN_TRIM")
            .collect();

        assert_eq!(advertised, expected, "{}", driver.as_str());
        assert!(!advertised.iter().any(|capability| capability == "FN_CAST"));
    }
}

/// `vs-adapter/pushdown-dialect`: each advertised scalar function renders
/// over one operand of every class Exasol admits for it, because a
/// capability is negotiated once and admits every operand type.
#[test]
fn every_scalar_function_renders_over_each_admitted_operand_class() {
    use crate::dialect_test_fixtures::{column, numeric_literal, scalar, scalar_request};
    use serde_json::json;

    let interval = || scalar("SUB", vec![column("ts"), column("ts")]);
    let texts = ["v", "i", "d", "f", "dt", "ts", "b"];
    let one = |name: &str| -> Vec<serde_json::Value> {
        texts
            .iter()
            .map(|operand| scalar(name, vec![column(operand)]))
            .collect()
    };
    let arms =
        |capability: &str| -> Vec<serde_json::Value> {
            match capability {
                "FN_YEAR" | "FN_MONTH" | "FN_DAY" => ["dt", "ts", "v"]
                    .iter()
                    .map(|operand| scalar(&capability[3..], vec![column(operand)]))
                    .collect(),
                "FN_UPPER" | "FN_LOWER" | "FN_LENGTH" => one(&capability[3..]),
                "FN_TRIM" => {
                    let mut arms = one("TRIM");
                    arms.push(scalar("TRIM", vec![column("v"), column("d")]));
                    arms
                }
                "FN_SUBSTR" => texts
                    .iter()
                    .map(|operand| scalar("SUBSTR", vec![column(operand), column("d")]))
                    .chain(["i", "f", "v"].iter().map(|start| {
                        scalar("SUBSTR", vec![column("v"), column(start), column(start)])
                    }))
                    .collect(),
                "FN_ADD" | "FN_SUB" | "FN_MULT" => {
                    let name = &capability[3..];
                    let mut pairs = vec![
                        (column("i"), column("d")),
                        (column("f"), column("d")),
                        (column("v"), numeric_literal("1")),
                        (column("b"), numeric_literal("1")),
                    ];
                    match name {
                        "ADD" => pairs.extend([
                            (column("dt"), column("f")),
                            (numeric_literal("1"), column("ts")),
                            (column("ts"), interval()),
                            (interval(), column("dt")),
                            (interval(), interval()),
                        ]),
                        "SUB" => pairs.extend([
                            (column("dt"), column("d")),
                            (column("dt"), column("dt")),
                            (column("ts"), column("dt")),
                            (column("dt"), column("ts")),
                            (column("ts"), interval()),
                            (interval(), interval()),
                        ]),
                        _ => pairs.extend([(interval(), column("d")), (column("f"), interval())]),
                    }
                    pairs
                        .into_iter()
                        .map(|(left, right)| scalar(name, vec![left, right]))
                        .collect()
                }
                other => panic!("no operand arms for `{other}`"),
            }
        };

    for driver in Driver::ALL {
        let dialect = dialect_for(*driver);
        let namespace = crate::adapter_notes::SourceQualifier::of(
            (0..driver.qualifier_segment_count())
                .map(|segment| format!("s{segment}"))
                .collect(),
        )
        .expect("a qualifier");
        for capability in dialect
            .capabilities()
            .iter()
            .filter(|capability| SCALAR_FUNCTION_CAPABILITIES.contains(capability))
        {
            for entry in arms(capability) {
                let request = scalar_request(entry.clone(), json!({"type": "DOUBLE"}));
                let query = crate::pushdown_query::parse(&request, &namespace)
                    .unwrap_or_else(|error| panic!("{entry} must parse: {error}"));
                dialect.render(&query).unwrap_or_else(|error| {
                    panic!(
                        "`{}` advertises `{capability}` but cannot render {entry}: {error}",
                        driver.as_str()
                    )
                });
            }
        }
    }
}
