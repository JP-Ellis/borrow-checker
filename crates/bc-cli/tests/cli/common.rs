//! Shared test helpers for CLI integration tests.

use std::path::PathBuf;

use assert_cmd::Command;
use assert_fs::TempDir;
use regex::Regex;

/// Isolated test environment: fresh SQLite database in a temp directory.
#[expect(
    clippy::partial_pub_fields,
    reason = "filters is an internal implementation detail"
)]
pub struct TestContext {
    /// Temporary home directory (cleaned up on drop).
    pub home_dir: TempDir,
    /// Path to the isolated SQLite database.
    pub db_path: PathBuf,
    /// Output filters applied before snapshot comparison.
    filters: Vec<(Regex, String)>,
}

impl TestContext {
    /// Creates a new isolated test context with a fresh SQLite database.
    #[expect(clippy::expect_used, reason = "test helper panics on setup failure")]
    pub fn new() -> Self {
        let home_dir = TempDir::new().expect("create temp dir");
        let db_path = home_dir.path().join("test.db");

        let home_path_escaped =
            regex::escape(home_dir.path().to_str().expect("temp dir path is UTF-8"));
        let filters = vec![
            (
                Regex::new("borrow-checker\\.exe").expect("valid regex"),
                "borrow-checker".to_owned(),
            ),
            (
                // SQLx emits WARN lines for slow statements on Windows (where SQLite
                // migration DDL can exceed the 1 s threshold). Strip them so snapshots
                // remain platform-independent.
                Regex::new(r"WARN slow statement:[^\n]*\n?").expect("valid regex"),
                String::new(),
            ),
            (
                Regex::new("account_[0-9a-z]{26}").expect("valid regex"),
                "[ACCOUNT_ID]".to_owned(),
            ),
            (
                Regex::new("transaction_[0-9a-z]{26}").expect("valid regex"),
                "[TRANSACTION_ID]".to_owned(),
            ),
            (
                Regex::new("posting_[0-9a-z]{26}").expect("valid regex"),
                "[POSTING_ID]".to_owned(),
            ),
            (
                Regex::new("profile_[0-9a-z]{26}").expect("valid regex"),
                "[PROFILE_ID]".to_owned(),
            ),
            (
                Regex::new("valuation_[0-9a-z]{26}").expect("valid regex"),
                "[VALUATION_ID]".to_owned(),
            ),
            (
                Regex::new("loan_[0-9a-z]{26}").expect("valid regex"),
                "[LOAN_ID]".to_owned(),
            ),
            (
                Regex::new("depreciation_[0-9a-z]{26}").expect("valid regex"),
                "[DEPRECIATION_ID]".to_owned(),
            ),
            (
                Regex::new(&home_path_escaped).expect("valid temp-dir regex"),
                "[TEMP_DIR]".to_owned(),
            ),
            (
                Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z").expect("valid regex"),
                "[TIMESTAMP]".to_owned(),
            ),
        ];

        Self {
            home_dir,
            db_path,
            filters,
        }
    }

    /// Returns a configured `Command` pointing at the `borrow-checker` binary.
    ///
    /// Passes `--db-path` as an explicit CLI flag so the test's isolated
    /// database is used on every platform. The `BC_DB_PATH` environment
    /// variable is intentionally not used here: the config layer maps it via
    /// the underscore separator which treats `DB_PATH` as the nested key
    /// `db.path` rather than the flat `db_path` field, silently ignoring it.
    ///
    /// On Windows, `SystemRoot` is preserved after the `env_clear()` call so
    /// that the spawned process can load system DLLs from the standard path.
    #[expect(clippy::expect_used, reason = "test helper panics on setup failure")]
    pub fn command(&self) -> Command {
        let mut cmd = Command::cargo_bin("borrow-checker").expect("borrow-checker binary");
        cmd.env_clear()
            .env("LANG", "C")
            .env("TZ", "UTC")
            .env("HOME", self.home_dir.path())
            .arg("--db-path")
            .arg(&self.db_path);
        // On Windows, preserve SystemRoot so the spawned binary can locate
        // system DLLs (env_clear() removes it, but it is required at runtime).
        #[cfg(windows)]
        if let Some(v) = std::env::var_os("SystemRoot") {
            cmd.env("SystemRoot", v);
        }
        cmd
    }

    /// Executes `cmd`, formats stdout/stderr/exit code, applies filters, returns string.
    #[expect(clippy::expect_used, reason = "test helper panics on setup failure")]
    pub fn run(&self, cmd: &mut Command) -> String {
        let output = cmd.output().expect("command executed");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        let mut result = format!(
            "success: {}\nexit_code: {}\n----- stdout -----\n{}----- stderr -----\n{}",
            output.status.success(),
            output.status.code().unwrap_or(-1_i32),
            stdout,
            stderr,
        );

        for (pattern, replacement) in &self.filters {
            result = pattern
                .replace_all(&result, replacement.as_str())
                .into_owned();
        }
        result
    }
}
