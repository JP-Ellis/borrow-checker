//! Shared test helpers for CLI integration tests.

use std::path::PathBuf;

use assert_cmd::Command;
use assert_fs::TempDir;
use regex::Regex;

/// Isolated test environment: fresh SQLite database in a temp directory.
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
    pub fn new() -> Self {
        let home_dir = TempDir::new().expect("create temp dir");
        let db_path = home_dir.path().join("test.db");

        let mut filters: Vec<(Regex, String)> = Vec::new();
        filters.push((
            Regex::new(r"account_[0-9a-z]{26}").expect("valid regex"),
            "[ACCOUNT_ID]".into(),
        ));
        filters.push((
            Regex::new(r"transaction_[0-9a-z]{26}").expect("valid regex"),
            "[TRANSACTION_ID]".into(),
        ));
        filters.push((
            Regex::new(r"profile_[0-9a-z]{26}").expect("valid regex"),
            "[PROFILE_ID]".into(),
        ));

        Self {
            home_dir,
            db_path,
            filters,
        }
    }

    /// Returns a configured `Command` pointing at the `borrow-checker` binary.
    pub fn command(&self) -> Command {
        let mut cmd = Command::cargo_bin("borrow-checker").expect("borrow-checker binary");
        cmd.env_clear()
            .env("LANG", "C")
            .env("TZ", "UTC")
            .env("HOME", self.home_dir.path())
            .env("BC_DB_PATH", &self.db_path);
        cmd
    }

    /// Executes `cmd`, formats stdout/stderr/exit code, applies filters, returns string.
    pub fn run(&self, cmd: &mut Command) -> String {
        let output = cmd.output().expect("command executed");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        let mut result = format!(
            "success: {}\nexit_code: {}\n----- stdout -----\n{}----- stderr -----\n{}",
            output.status.success(),
            output.status.code().unwrap_or(-1),
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
