//! DTOs built from JSON for the import page's native tests.
//!
//! The `bc_ipc` import DTOs are `#[non_exhaustive]`, so a crate outside
//! `bc-ipc` builds them through serde, the path the wire takes.

use bc_ipc::ImportPreview;
use bc_ipc::ImportProfiles;
use bc_ipc::PreviewResult;
use serde_json::Value;
use serde_json::json;

/// Replaces `base`'s top-level keys with `patch`'s.
fn merge(mut base: Value, patch: Value) -> Value {
    if let (Some(target), Value::Object(fields)) = (base.as_object_mut(), patch) {
        target.extend(fields);
    }
    base
}

/// A preview of profile `everyday` with every count zero and every list empty.
fn preview_json() -> Value {
    json!({
        "profile": "everyday",
        "fingerprint": "00000000000000aa",
        "new_transactions": 0,
        "attached_postings": 0,
        "already_imported": 0,
        "skipped_postings": 0,
        "unresolved_accounts": [],
        "unresolved_commodities": [],
        "skips_by_cause": [],
        "warnings": [],
        "would_create_accounts": [],
        "would_create_tags": [],
        "account_totals": [],
        "rows": [],
        "other_diagnostics": []
    })
}

/// Builds an [`ImportProfiles`] from its JSON.
pub(crate) fn profiles(value: Value) -> ImportProfiles {
    serde_json::from_value(value).expect("fixture matches ImportProfiles")
}

/// The empty preview with `patch`'s fields replaced.
pub(crate) fn preview(patch: Value) -> ImportPreview {
    serde_json::from_value(merge(preview_json(), patch)).expect("fixture matches ImportPreview")
}

/// `PreviewResult::Ready` around [`preview`]`(patch)`.
pub(crate) fn ready(patch: Value) -> PreviewResult {
    let body = merge(preview_json(), patch);
    serde_json::from_value(merge(json!({ "kind": "ready" }), body))
        .expect("fixture matches PreviewResult::Ready")
}

/// `PreviewResult::Failed` with the given failure fields.
pub(crate) fn failed(failure: Value) -> PreviewResult {
    serde_json::from_value(merge(json!({ "kind": "failed" }), failure))
        .expect("fixture matches PreviewResult::Failed")
}
