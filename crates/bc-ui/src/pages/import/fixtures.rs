//! DTOs built from JSON for the import page's native tests.
//!
//! The `bc_ipc` import DTOs are `#[non_exhaustive]`, so a crate outside
//! `bc-ipc` builds them through serde, the path the wire takes.

use bc_ipc::CommitResult;
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

/// A row dated 2026-01-05 with the given fate JSON and no legs.
pub(crate) fn row(fate: Value) -> bc_ipc::PreviewRow {
    let base = json!({
        "location": "statement.csv data row 1",
        "date": "2026-01-05",
        "description": "Woolworths Metro",
        "fate": null,
        "legs": [],
        "diagnostics": []
    });
    let patch: serde_json::Map<String, Value> =
        core::iter::once(("fate".to_owned(), fate)).collect();
    serde_json::from_value(merge(base, Value::Object(patch))).expect("fixture matches PreviewRow")
}

/// A leg fate from its JSON.
pub(crate) fn leg_fate(value: Value) -> bc_ipc::LegFateInfo {
    serde_json::from_value(value).expect("fixture matches LegFateInfo")
}

/// An import result for batch `batch-0001` with `patch`'s fields replaced.
fn import_result_json(patch: Value) -> Value {
    merge(
        json!({
            "batch_id": "batch-0001",
            "new_transactions": 0,
            "attached_postings": 0,
            "skipped_postings": 0,
            "skips_by_cause": [],
            "unresolved_accounts": [],
            "unresolved_commodities": [],
            "created_tags": [],
            "created_accounts": [],
            "warnings": [],
            "snapshot": null
        }),
        patch,
    )
}

/// A `CommitResult` of the given kind: `imported` takes import-result fields,
/// `changed` preview fields, `failed` failure fields.
pub(crate) fn commit(kind: &str, patch: Value) -> CommitResult {
    let body = match kind {
        "imported" => import_result_json(patch),
        "changed" => merge(preview_json(), patch),
        _ => patch,
    };
    serde_json::from_value(merge(json!({ "kind": kind }), body))
        .expect("fixture matches CommitResult")
}

/// An [`bc_ipc::ImportResult`] with `patch`'s fields replaced.
pub(crate) fn import_result(patch: Value) -> bc_ipc::ImportResult {
    serde_json::from_value(import_result_json(patch)).expect("fixture matches ImportResult")
}

/// A complete batch `batch-0001` of profile `everyday`, with `patch`'s fields replaced.
pub(crate) fn batch(patch: Value) -> bc_ipc::ImportBatchInfo {
    serde_json::from_value(merge(
        json!({
            "id": "batch-0001",
            "profile": "everyday",
            "importer": "csv",
            "started_at": "2026-10-10T01:00:00Z",
            "finished_at": "2026-10-10T01:00:05Z",
            "state": { "kind": "complete" },
            "counts": { "new_transactions": 42, "attached_postings": 3, "skipped_postings": 7 },
            "discard": null
        }),
        patch,
    ))
    .expect("fixture matches ImportBatchInfo")
}

/// Every discard count zero, with `patch`'s fields replaced.
fn counts_json(patch: Value) -> Value {
    merge(
        json!({
            "removed_postings": 0, "removed_transactions": 0, "detached_adopted": 0,
            "freed_tombstones": 0, "other_batch_references_removed": 0,
            "other_batch_references_tombstoned": 0, "edited_postings": 0,
            "reconciled_postings": 0, "flagged_postings": 0, "removed_tags": 0,
            "kept_tags": 0, "removed_accounts": 0, "kept_accounts": 0, "reverted_fields": 0
        }),
        patch,
    )
}

/// [`bc_ipc::DiscardCounts`] with `patch`'s fields replaced.
pub(crate) fn counts(patch: Value) -> bc_ipc::DiscardCounts {
    serde_json::from_value(counts_json(patch)).expect("fixture matches DiscardCounts")
}

/// `DiscardPreview::Ready` with those counts.
pub(crate) fn discard_ready(patch: Value, snapshot_planned: bool) -> bc_ipc::DiscardPreview {
    serde_json::from_value(json!({
        "kind": "ready",
        "counts": counts_json(patch),
        "snapshot_planned": snapshot_planned
    }))
    .expect("fixture matches DiscardPreview::Ready")
}

/// `DiscardPreview::Blocked` with one dependant, batch `batch-0002`.
pub(crate) fn discard_blocked() -> bc_ipc::DiscardPreview {
    serde_json::from_value(json!({
        "kind": "blocked",
        "dependants": [{
            "batch_id": "batch-0002", "importer": "csv",
            "started_at": "2026-10-11T09:30:00Z", "postings": 3, "transactions": 2
        }]
    }))
    .expect("fixture matches DiscardPreview::Blocked")
}
