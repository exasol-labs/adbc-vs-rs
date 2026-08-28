//! Resolves the Exasol type an `EMITS` column must declare for each entry
//! of a `pushdown` request's select list.
//!
//! Exasol compares a pushdown result's column types against the virtual
//! table's stored metadata for exact equality, so a declared type is never
//! inferred from an expression tree: it is read from Exasol's own compiler
//! output, `pushdownRequest.selectListDataTypes`, positionally parallel to
//! `selectList`. That field is version-gated — Exasol 8.6.0, or 7.1.14 on
//! the 7.x line — so an older Exasol omits it. A select list holding only
//! `column` entries can still be resolved without it, from
//! `involvedTables[].columns[].dataType`; any other entry with no supplied
//! type fails, because guessing would reintroduce exactly the divergence
//! the field removes.

use serde_json::Value;

use crate::exa_type::{self, ExasolType};

/// The Exasol version `pushdownRequest.selectListDataTypes` requires, named
/// in every error that traces back to its absence.
pub const MIN_EXASOL_VERSION: &str = "Exasol 8.6.0, or 7.1.14 on the 7.x line";

/// Reads `pushdownRequest.selectListDataTypes`, positionally parallel to a
/// select list of `select_list_len` entries.
///
/// `Ok(None)` means the field is absent: the request predates the Exasol
/// version that supplies it, and the caller must resolve every entry some
/// other way or fail. `Err` means the field is present but does not
/// describe the select list — a length mismatch, or an entry naming a type
/// outside the declarable set.
pub fn read_select_list_data_types(
    pushdown_request: &Value,
    select_list_len: usize,
) -> Result<Option<Vec<ExasolType>>, String> {
    let Some(types) = pushdown_request.get("selectListDataTypes") else {
        return Ok(None);
    };
    let types = types.as_array().ok_or_else(|| {
        "pushdownRequest.selectListDataTypes is present but is not an array".to_string()
    })?;
    if types.len() != select_list_len {
        return Err(format!(
            "pushdownRequest.selectListDataTypes has {} entries but selectList has {} entries; \
             the two arrays must be the same length",
            types.len(),
            select_list_len
        ));
    }

    types
        .iter()
        .map(exa_type::from_json)
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// The error a non-`column` select-list entry fails with when
/// `selectListDataTypes` supplies no type for it.
pub fn missing_declared_type(entry_type: &str) -> String {
    format!(
        "selectList entry of type `{entry_type}` carries no supplied type, and pushdownRequest \
         carries no selectListDataTypes; delegating a non-column select-list entry requires \
         {MIN_EXASOL_VERSION}"
    )
}

#[cfg(test)]
#[path = "emit_type_tests.rs"]
mod tests;
