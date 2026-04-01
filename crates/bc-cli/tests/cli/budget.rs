//! Integration tests for the `budget` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use pretty_assertions::assert_eq;

use crate::cmd_snapshot;
use crate::common::TestContext;

#[test]
fn list_envelopes_empty() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["budget", "envelopes", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn create_envelope_with_colour_persists_colour() {
    let ctx = TestContext::new();
    let out = ctx
        .command()
        .args([
            "--json",
            "budget",
            "envelopes",
            "create",
            "--name",
            "Groceries",
            "--commodity",
            "AUD",
            "--colour",
            "#FF5733",
        ])
        .output()
        .expect("command executed");

    assert!(
        out.status.success(),
        "envelope create should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(
        json.get("colour").and_then(serde_json::Value::as_str),
        Some("#FF5733"),
        "colour should be persisted"
    );
}

#[test]
fn move_envelope_to_group() {
    let ctx = TestContext::new();

    // Create a group.
    let group_out = ctx
        .command()
        .args([
            "--json",
            "budget",
            "groups",
            "create",
            "--name",
            "Transport",
        ])
        .output()
        .expect("command executed");
    assert!(
        group_out.status.success(),
        "group create should succeed: {}",
        String::from_utf8_lossy(&group_out.stderr)
    );
    let group_json: serde_json::Value =
        serde_json::from_slice(&group_out.stdout).expect("valid JSON");
    let group_id = group_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("group id");

    // Create an envelope with no group.
    let env_out = ctx
        .command()
        .args([
            "--json",
            "budget",
            "envelopes",
            "create",
            "--name",
            "Fuel",
            "--commodity",
            "AUD",
        ])
        .output()
        .expect("command executed");
    assert!(
        env_out.status.success(),
        "envelope create should succeed: {}",
        String::from_utf8_lossy(&env_out.stderr)
    );
    let env_json: serde_json::Value = serde_json::from_slice(&env_out.stdout).expect("valid JSON");
    let env_id = env_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("envelope id");

    // Move the envelope to the group.
    let move_out = ctx
        .command()
        .args([
            "--json",
            "budget",
            "envelopes",
            "move",
            env_id,
            "--group",
            group_id,
        ])
        .output()
        .expect("command executed");
    assert!(
        move_out.status.success(),
        "envelope move should succeed: {}",
        String::from_utf8_lossy(&move_out.stderr)
    );
    let moved_json: serde_json::Value =
        serde_json::from_slice(&move_out.stdout).expect("valid JSON");
    assert_eq!(
        moved_json
            .get("parent_id")
            .and_then(serde_json::Value::as_str),
        Some(group_id),
        "parent_id should match the target group"
    );
}

#[test]
fn move_envelope_to_root_clears_group() {
    let ctx = TestContext::new();

    // Create a group.
    let group_out = ctx
        .command()
        .args(["--json", "budget", "groups", "create", "--name", "Food"])
        .output()
        .expect("command executed");
    assert!(group_out.status.success(), "group create should succeed");
    let group_json: serde_json::Value =
        serde_json::from_slice(&group_out.stdout).expect("valid JSON");
    let group_id = group_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("group id");

    // Create an envelope in the group.
    let env_out = ctx
        .command()
        .args([
            "--json",
            "budget",
            "envelopes",
            "create",
            "--name",
            "Groceries",
            "--commodity",
            "AUD",
            "--group",
            group_id,
        ])
        .output()
        .expect("command executed");
    assert!(env_out.status.success(), "envelope create should succeed");
    let env_json: serde_json::Value = serde_json::from_slice(&env_out.stdout).expect("valid JSON");
    let env_id = env_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("envelope id");

    // Move to root by omitting --group.
    let move_out = ctx
        .command()
        .args(["--json", "budget", "envelopes", "move", env_id])
        .output()
        .expect("command executed");
    assert!(
        move_out.status.success(),
        "envelope move to root should succeed: {}",
        String::from_utf8_lossy(&move_out.stderr)
    );
    let moved_json: serde_json::Value =
        serde_json::from_slice(&move_out.stdout).expect("valid JSON");
    assert!(
        moved_json.get("parent_id").is_none()
            || moved_json
                .get("parent_id")
                .is_some_and(serde_json::Value::is_null),
        "parent_id should be absent or null after moving to root, got: {moved_json:?}"
    );
}
