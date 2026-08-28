use crate::mysql_dialect::MySqlDialect;

use super::*;

/// The `MainCapability` constants this release advertises, as
/// `vs-adapter/pushdown-dialect`'s capability scenario names them. Every other
/// advertised entry is a function or literal capability, told apart by its
/// prefix rather than by a second list.
const ADVERTISED_MAIN_CAPABILITIES: &[&str] = &[
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

/// The `getCapabilities` response, read the way Exasol reads it.
///
/// Every test here goes through the response rather than through the
/// dialect's own accessor, because the response is what reaches Exasol: a
/// correct set inside a malformed envelope advertises nothing, and the
/// envelope is this module's only behaviour of its own.
fn advertised() -> Vec<String> {
    let response = capabilities_response(&MySqlDialect);

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

/// Asserts that the advertised capabilities carrying `prefix` are exactly
/// `expected`, compared as sets.
///
/// Exasol reads the array as a set, and `mysql_dialect.rs` groups
/// `CAPABILITIES` for a reader rather than in any meaningful order, so a
/// reordering that changes no behaviour must not fail a test here.
fn assert_prefixed_capabilities_are(capabilities: &[String], prefix: &str, expected: &[&str]) {
    let mut advertised: Vec<&str> = capabilities
        .iter()
        .map(String::as_str)
        .filter(|capability| capability.starts_with(prefix))
        .collect();
    advertised.sort_unstable();
    let mut expected: Vec<&str> = expected.to_vec();
    expected.sort_unstable();

    assert_eq!(
        advertised, expected,
        "the advertised `{prefix}` capabilities must be exactly the fragments this dialect \
         renders: {capabilities:?}"
    );
}

fn assert_advertises(capabilities: &[String], required: &str) {
    assert!(
        capabilities.iter().any(|entry| entry == required),
        "capability `{required}` has a rendering path and must be advertised: {capabilities:?}"
    );
}

fn assert_withholds(capabilities: &[String], withheld: &str) {
    assert!(
        !capabilities.iter().any(|entry| entry == withheld),
        "capability `{withheld}` has no rendering path and must not be advertised: \
         {capabilities:?}"
    );
}

/// `vs-adapter/pushdown-dialect`'s capability scenario: the response names the
/// request it answers, carries every main capability this release renders, and
/// carries nothing outside the naming convention that scenario fixes — an
/// entry matching neither the `MainCapability` list nor a function prefix is a
/// capability string Exasol would not recognize at all.
#[test]
fn advertised_set_matches_the_rendered_fragments() {
    let response = capabilities_response(&MySqlDialect);
    assert_eq!(
        response["type"], "getCapabilities",
        "the response must name the request it answers, or Exasol reads no capability out of \
         it at all: {response}"
    );

    let capabilities = advertised();
    for required in ADVERTISED_MAIN_CAPABILITIES {
        assert_advertises(&capabilities, required);
    }
    for capability in &capabilities {
        assert!(
            ADVERTISED_MAIN_CAPABILITIES.contains(&capability.as_str())
                || FUNCTION_CAPABILITY_PREFIXES
                    .iter()
                    .any(|prefix| capability.starts_with(prefix)),
            "advertised capability `{capability}` is neither a `MainCapability` constant this \
             release renders nor a function or literal capability: {capabilities:?}"
        );
    }
}

/// `vs-adapter/result-ordering`, decision `[1]`: a script-emitted derived
/// table carries no ordering guarantee, so Exasol keeps both clauses. Nothing
/// else in the crate asserts the withholding, and advertising any of the four
/// would return rows in an order Exasol never applied — the silent
/// wrong-rows failure mode, not an error.
#[test]
fn ordering_and_limiting_are_withheld() {
    let capabilities = advertised();

    for withheld in [
        "ORDER_BY_COLUMN",
        "ORDER_BY_EXPRESSION",
        "LIMIT",
        "LIMIT_WITH_OFFSET",
    ] {
        assert_withholds(&capabilities, withheld);
    }
}

/// `vs-adapter/pushdown-join`: MySQL implements no `FULL OUTER JOIN` and this
/// release renders no non-equi condition, so advertising either capability
/// would make Exasol delegate a join `join_keyword` and `render_predicate`
/// can only fail on. The four advertised join capabilities are asserted
/// alongside them, because withholding everything would pass a
/// withheld-only test.
#[test]
fn join_capabilities_exclude_full_outer_and_non_equi() {
    let capabilities = advertised();

    for withheld in ["JOIN_TYPE_FULL_OUTER", "JOIN_CONDITION_ALL"] {
        assert_withholds(&capabilities, withheld);
    }
    for required in [
        "JOIN",
        "JOIN_TYPE_INNER",
        "JOIN_TYPE_LEFT_OUTER",
        "JOIN_TYPE_RIGHT_OUTER",
        "JOIN_CONDITION_EQUI",
    ] {
        assert_advertises(&capabilities, required);
    }
}

/// `vs-adapter/pushdown-filter`'s capability scenario: the advertised set
/// covers exactly the predicate and literal forms the dialect renders, per
/// `mysql_dialect.rs`'s `render_predicate` and `render_literal`.
#[test]
fn filter_capabilities_match_rendered_predicates() {
    let capabilities = advertised();

    assert_advertises(&capabilities, "FILTER_EXPRESSIONS");
    assert_prefixed_capabilities_are(
        &capabilities,
        "FN_PRED_",
        &[
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
        ],
    );
    assert_prefixed_capabilities_are(
        &capabilities,
        "LITERAL_",
        &[
            "LITERAL_NULL",
            "LITERAL_EXACTNUMERIC",
            "LITERAL_DOUBLE",
            "LITERAL_STRING",
            "LITERAL_DATE",
            "LITERAL_TIMESTAMP",
        ],
    );

    for withheld in [
        "FN_PRED_REGEXP_LIKE",
        "FN_PRED_IS_JSON",
        "FN_PRED_IS_NOT_JSON",
        "LITERAL_BOOL",
        "LITERAL_INTERVAL",
        "LITERAL_TIMESTAMP_UTC",
    ] {
        assert_withholds(&capabilities, withheld);
    }
}

/// `vs-adapter/pushdown-aggregation`'s capability scenario:
/// `SELECTLIST_EXPRESSIONS` and the aggregation capabilities are advertised
/// together, because gate 3.0 measured that no aggregate reaches the adapter
/// without `SELECTLIST_EXPRESSIONS`, and the grouping cap decision `[18]`
/// recorded is withheld by omission rather than by a separate flag.
#[test]
fn aggregation_capabilities_match_rendered_aggregates() {
    let capabilities = advertised();

    for required in [
        "SELECTLIST_EXPRESSIONS",
        "AGGREGATE_SINGLE_GROUP",
        "AGGREGATE_GROUP_BY_COLUMN",
    ] {
        assert_advertises(&capabilities, required);
    }
    assert_prefixed_capabilities_are(
        &capabilities,
        "FN_AGG_",
        &[
            "FN_AGG_COUNT",
            "FN_AGG_COUNT_STAR",
            "FN_AGG_COUNT_DISTINCT",
            "FN_AGG_MIN",
            "FN_AGG_MAX",
        ],
    );

    for withheld in [
        "AGGREGATE_GROUP_BY_EXPRESSION",
        "AGGREGATE_GROUP_BY_TUPLE",
        "AGGREGATE_HAVING",
        "FN_AGG_AVG",
        "FN_AGG_AVG_DISTINCT",
        "FN_AGG_SUM",
        "FN_AGG_SUM_DISTINCT",
    ] {
        assert_withholds(&capabilities, withheld);
    }
}
