//! A throwaway Lua Virtual Schema adapter that observes what Exasol's own
//! query compiler sends in a `pushdown` request, independent of this
//! project's shipped adapter.
//!
//! This probe pins the wire-protocol facts `vs-adapter/pushdown-aggregation`
//! measured — the Exasol type an aggregate's `selectListDataTypes` entry
//! carries, that a `VARCHAR` entry there carries no `characterSet`, that a
//! `TIMESTAMP` entry carries `fractionalSecondsPrecision`, and that a
//! select-list predicate is declared `BOOLEAN` — against the deployed stack.
//! The shipped `MySqlDialect` withholds `FN_AGG_SUM`, `FN_AGG_SUM_DISTINCT`,
//! `FN_AGG_AVG`, and `FN_AGG_AVG_DISTINCT` (decision-log
//! `[implement-gate] Delegated AVG returns a different value from Exasol's
//! own AVG`), so no query it delegates can make Exasol declare a `SUM` or
//! `AVG` type. Measuring those declared types anyway needs an adapter that
//! advertises the four withheld capabilities: a scratch adapter, not the
//! adapter under test.
//!
//! This probe answers `createVirtualSchema` with one fixed table, `t`,
//! carrying no MySQL connection at all — a schema over Exasol types made up
//! for exactly this measurement. Its `pushdown` handler never renders
//! anything; it fails every request with the raw `pushdownRequest` JSON
//! wrapped in two markers, which Exasol relays verbatim into the SQL error
//! text a session sees. That is the whole mechanism: every measured query
//! here is expected to fail, and the failure is what carries the answer.

use serde_json::Value;

use super::exasol::Exasol;

/// The schema the probe's adapter script lives in.
const SCHEMA: &str = "PROBE";

/// The adapter script every probe virtual schema is created over.
const ADAPTER_SCRIPT: &str = "PROBE.PROBE_ADAPTER";

/// The virtual schema a measured query runs against. Its one table, `t`,
/// carries columns `d DECIMAL(12,4)`, `e DECIMAL(30,2)`, `f DOUBLE`,
/// `g VARCHAR(100)`, and `h TIMESTAMP`.
pub const VIRTUAL_SCHEMA: &str = "PROBEVS";

const BEGIN_MARKER: &str = "PUSHDOWN_JSON_BEGIN";
const END_MARKER: &str = "PUSHDOWN_JSON_END";

/// The measurement's own minimum capability set — exactly what the two
/// queries [`measured_select_list_data_types`] runs need to reach the
/// probe's `pushdown` handler at all, not a mirror of any shipped set:
/// `SELECTLIST_PROJECTION` for a bare column, `SELECTLIST_EXPRESSIONS`
/// because decision-log `[implement-gate] Gate 3.0 falsified the assumption
/// that an aggregate reaches the adapter without SELECTLIST_EXPRESSIONS`
/// measured that Exasol delegates no aggregate without it,
/// `AGGREGATE_GROUP_BY_COLUMN` since both queries group by one column,
/// `FN_PRED_LESS` and `LITERAL_TIMESTAMP` for the one predicate the second
/// query carries, and the seven `FN_AGG_*` capabilities both queries'
/// aggregate calls need. It carries no `AGGREGATE_GROUP_BY_TUPLE`: measured
/// live against this stack, a `GROUP BY` naming more than one column is
/// withdrawn entirely without it.
const CAPABILITIES: &str = r#""SELECTLIST_PROJECTION","SELECTLIST_EXPRESSIONS","AGGREGATE_GROUP_BY_COLUMN","FN_AGG_SUM","FN_AGG_SUM_DISTINCT","FN_AGG_COUNT","FN_AGG_COUNT_STAR","FN_AGG_COUNT_DISTINCT","FN_AGG_AVG","FN_AGG_MIN","FN_AGG_MAX","FN_PRED_LESS","LITERAL_TIMESTAMP""#;

/// The Lua adapter's entry point. `createVirtualSchema`, `refresh`, and
/// `setProperties` all answer with the same fixed table; `pushdown` never
/// renders anything and instead fails with the request it received, wrapped
/// in [`BEGIN_MARKER`] and [`END_MARKER`] so a caller can pull the JSON back
/// out of the SQL error Exasol surfaces.
fn lua_source() -> String {
    format!(
        r#"
function adapter_call(request_json)
    local cjson = require("cjson")
    local req = cjson.decode(request_json)
    local t = req.type

    if t == "createVirtualSchema" or t == "refresh" or t == "setProperties" then
        return cjson.encode({{
            type = t,
            schemaMetadata = {{
                tables = {{{{
                    name = "t",
                    columns = {{
                        {{name = "d", dataType = {{type="DECIMAL", precision=12, scale=4}}, isNullable=true}},
                        {{name = "e", dataType = {{type="DECIMAL", precision=30, scale=2}}, isNullable=true}},
                        {{name = "f", dataType = {{type="DOUBLE"}}, isNullable=true}},
                        {{name = "g", dataType = {{type="VARCHAR", size=100, characterSet="UTF8"}}, isNullable=true}},
                        {{name = "h", dataType = {{type="TIMESTAMP"}}, isNullable=true}},
                    }}
                }}}},
                adapterNotes = "{{}}"
            }}
        }})
    elseif t == "dropVirtualSchema" then
        return cjson.encode({{type="dropVirtualSchema"}})
    elseif t == "getCapabilities" then
        return cjson.encode({{type="getCapabilities", capabilities={{{CAPABILITIES}}}}})
    elseif t == "pushdown" then
        error("{BEGIN_MARKER}" .. request_json .. "{END_MARKER}")
    else
        error("unhandled request type: " .. tostring(t))
    end
end
"#
    )
}

/// Deploys the probe's adapter script and creates [`VIRTUAL_SCHEMA`] over it,
/// replacing anything an earlier run left behind.
pub fn create(exasol: &Exasol) {
    exasol.execute(&format!("CREATE SCHEMA IF NOT EXISTS {SCHEMA}"));
    exasol.execute(&format!(
        "CREATE OR REPLACE LUA ADAPTER SCRIPT {ADAPTER_SCRIPT} AS{}",
        lua_source()
    ));
    drop_virtual_schema(exasol);
    exasol.execute(&format!(
        "CREATE VIRTUAL SCHEMA {VIRTUAL_SCHEMA} USING {ADAPTER_SCRIPT}"
    ));
}

/// Drops [`VIRTUAL_SCHEMA`], if it exists.
pub fn drop_virtual_schema(exasol: &Exasol) {
    exasol.execute(&format!(
        "DROP VIRTUAL SCHEMA IF EXISTS {VIRTUAL_SCHEMA} CASCADE"
    ));
}

/// Runs `query` against [`VIRTUAL_SCHEMA`] — which the probe's `pushdown`
/// handler always fails — and returns the `pushdownRequest.selectListDataTypes`
/// array Exasol sent, exactly as received.
pub fn measured_select_list_data_types(exasol: &Exasol, query: &str) -> Vec<Value> {
    let error = exasol.execute_expecting_failure(query);
    let request = extract_pushdown_request(&error);
    request
        .pointer("/pushdownRequest/selectListDataTypes")
        .unwrap_or_else(|| {
            panic!("the captured pushdown request carries no selectListDataTypes: {request}")
        })
        .as_array()
        .unwrap_or_else(|| panic!("selectListDataTypes is not an array: {request}"))
        .clone()
}

/// Pulls the `pushdownRequest` JSON out of the SQL error the probe's
/// `pushdown` handler raised, reading between [`BEGIN_MARKER`] and
/// [`END_MARKER`].
fn extract_pushdown_request(error: &str) -> Value {
    let start = error
        .find(BEGIN_MARKER)
        .unwrap_or_else(|| panic!("the probe's error carries no {BEGIN_MARKER} marker: {error}"))
        + BEGIN_MARKER.len();
    let end = error[start..]
        .find(END_MARKER)
        .unwrap_or_else(|| panic!("the probe's error carries no {END_MARKER} marker: {error}"))
        + start;
    serde_json::from_str(error[start..end].trim()).unwrap_or_else(|parse_error| {
        panic!(
            "parsing the captured pushdown request: {parse_error}\n{}",
            &error[start..end]
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_pushdown_request_reads_the_json_between_the_markers() {
        let error = format!(
            "Lua Error \"{BEGIN_MARKER}{{\"type\":\"pushdown\"}}{END_MARKER}\" caught in script"
        );

        assert_eq!(
            extract_pushdown_request(&error),
            serde_json::json!({"type": "pushdown"})
        );
    }

    #[test]
    #[should_panic(expected = "carries no PUSHDOWN_JSON_BEGIN marker")]
    fn extract_pushdown_request_panics_when_the_begin_marker_is_missing() {
        extract_pushdown_request("no markers in this error at all");
    }

    #[test]
    #[should_panic(expected = "carries no PUSHDOWN_JSON_END marker")]
    fn extract_pushdown_request_panics_when_the_end_marker_is_missing() {
        extract_pushdown_request(&format!("{BEGIN_MARKER}{{}}"));
    }

    #[test]
    fn lua_source_carries_the_markers_and_the_aggregate_capabilities() {
        let source = lua_source();

        assert!(source.contains(BEGIN_MARKER));
        assert!(source.contains(END_MARKER));
        assert!(source.contains("SELECTLIST_EXPRESSIONS"));
        assert!(source.contains("FN_AGG_SUM_DISTINCT"));
    }
}
