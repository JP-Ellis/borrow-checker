//! Application configuration for BorrowChecker.
//!
//! Settings are loaded from a hierarchy: built-in defaults → user config
//! file(s) → local project file → environment variables (`BC_` prefix, `__`
//! between table and key). Keys are kebab-case; `snake_case` is accepted.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

use std::path::PathBuf;

use bc_models::CommodityCode;
use config::Source as _;
use jiff::civil::Date;

#[cfg(feature = "ipc")]
mod ipc;
mod source;

/// Returns the user-local plugin directory.
///
/// This is the XDG data home equivalent: `~/.local/share/borrow-checker/plugins/`
/// on Linux/macOS (or the platform equivalent via the `directories` crate).
///
/// All frontends (CLI, TUI, GUI) should use this function when installing
/// or removing plugins so the install location is consistent.
///
/// # Returns
///
/// `Some(path)` if the home directory can be determined, `None` otherwise.
#[inline]
#[must_use]
pub fn user_plugin_dir() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|b| b.data_dir().join("borrow-checker").join("plugins"))
}

/// Returns the platform-appropriate default database path.
///
/// Priority:
/// 1. Platform data directory: `$XDG_DATA_HOME/borrow-checker/db.sqlite`
///    (Linux), `~/Library/Application Support/borrow-checker/db.sqlite`
///    (macOS).
/// 2. Fallback: `./borrow-checker.db` in the current working directory.
///
/// All frontends (CLI, TUI, GUI) should use this function rather than
/// implementing their own path resolution.
#[must_use]
#[inline]
pub fn default_db_path() -> std::path::PathBuf {
    directories::ProjectDirs::from("", "", "borrow-checker").map_or_else(
        || std::path::PathBuf::from("borrow-checker.db"),
        |dirs| dirs.data_dir().join("db.sqlite"),
    )
}

/// Returns the platform-appropriate default backup directory.
///
/// This is a `backups/` subdirectory of the platform data directory, a sibling
/// of the default database file (see [`default_db_path`]). Falls back to
/// `./borrow-checker-backups` when no platform directory can be determined.
#[must_use]
#[inline]
pub fn default_backup_dir() -> std::path::PathBuf {
    directories::ProjectDirs::from("", "", "borrow-checker").map_or_else(
        || std::path::PathBuf::from("borrow-checker-backups"),
        |dirs| dirs.data_dir().join("backups"),
    )
}

/// Error returned when loading or validating configuration.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// A configuration source could not be read or parsed.
    #[error("configuration error: {0}")]
    Load(#[from] config::ConfigError),
    /// A field value is out of range.
    #[error("invalid configuration: {0}")]
    Validation(String),
}

/// Raw deserialized `[cli]` settings.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawCliSection {
    /// Emit machine-readable JSON by default.
    json: bool,
    /// Log filter in `RUST_LOG` format, e.g. `"bc_cli=debug,bc_core=info"`.
    log: Option<String>,
}

/// CLI-specific settings from the `[cli]` section of the config file.
///
/// These settings allow users to persist command-line flag defaults so they
/// do not need to pass `--json` or set `RUST_LOG` on every invocation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct CliSection {
    /// Emit machine-readable JSON by default (equivalent to `--json`).
    json: bool,
    /// Default log filter in `RUST_LOG` format.
    ///
    /// When set, this acts as the default log filter unless overridden by
    /// the `RUST_LOG` environment variable or the `-v`/`-q` CLI flags.
    log: Option<String>,
}

impl CliSection {
    /// Returns `true` if JSON output is the configured default.
    #[inline]
    #[must_use]
    pub fn json(&self) -> bool {
        self.json
    }

    /// Returns the configured log filter string, if any.
    #[inline]
    #[must_use]
    pub fn log(&self) -> Option<&str> {
        self.log.as_deref()
    }
}

impl Default for CliSection {
    #[inline]
    fn default() -> Self {
        Self {
            json: false,
            log: None,
        }
    }
}

/// Raw deserialized `[backup]` settings.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawBackupSection {
    /// Optional override for the backup directory.
    dir: Option<String>,
    /// Keep the N newest backups (union with `retain-days`).
    retain_count: Option<u32>,
    /// Keep backups newer than N days (union with `retain-count`).
    retain_days: Option<u32>,
    /// Take an automatic snapshot before applying schema migrations.
    auto_pre_migration: bool,
    /// Take an automatic snapshot before each import run.
    auto_pre_import: bool,
    /// Take an automatic snapshot before discarding an import batch.
    auto_pre_discard: bool,
}

/// Returns the default raw `[backup]` section used by `RawSettings`'s serde default.
fn default_raw_backup() -> RawBackupSection {
    RawBackupSection {
        dir: None,
        retain_count: Some(5),
        retain_days: None,
        auto_pre_migration: true,
        auto_pre_import: true,
        auto_pre_discard: true,
    }
}

/// Backup and rotation settings from the `[backup]` section.
///
/// Retention applies to each automatic kind independently, as a conservative
/// union: a backup is kept if it is among the `retain_count` newest of its kind
/// **or** newer than `retain_days`; it is deleted only if it satisfies neither.
/// Manual backups are never deleted by retention. When both are `None`,
/// retention is disabled.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct BackupSection {
    /// Directory backups are written to; `None` ⇒ [`default_backup_dir`].
    dir: Option<std::path::PathBuf>,
    /// Keep the N newest backups.
    retain_count: Option<u32>,
    /// Keep backups newer than N days.
    retain_days: Option<u32>,
    /// Take an automatic snapshot before applying schema migrations.
    auto_pre_migration: bool,
    /// Take an automatic snapshot before each import run.
    auto_pre_import: bool,
    /// Take an automatic snapshot before discarding an import batch.
    auto_pre_discard: bool,
}

impl BackupSection {
    /// Returns the resolved backup directory (configured value or default).
    #[inline]
    #[must_use]
    pub fn resolved_dir(&self) -> std::path::PathBuf {
        self.dir.clone().unwrap_or_else(default_backup_dir)
    }

    /// Returns the configured backup directory override, if any.
    #[inline]
    #[must_use]
    pub fn dir(&self) -> Option<&std::path::Path> {
        self.dir.as_deref()
    }

    /// Returns the "keep N newest" retention limit, if set.
    #[inline]
    #[must_use]
    pub fn retain_count(&self) -> Option<u32> {
        self.retain_count
    }

    /// Returns the "keep newer than N days" retention limit, if set.
    #[inline]
    #[must_use]
    pub fn retain_days(&self) -> Option<u32> {
        self.retain_days
    }

    /// Returns whether automatic pre-migration snapshots are enabled.
    #[inline]
    #[must_use]
    pub fn auto_pre_migration(&self) -> bool {
        self.auto_pre_migration
    }

    /// Returns whether an automatic snapshot is taken before each import.
    #[inline]
    #[must_use]
    pub fn auto_pre_import(&self) -> bool {
        self.auto_pre_import
    }

    /// Returns whether an automatic snapshot is taken before a batch discard.
    ///
    /// # Returns
    ///
    /// `true` when discard snapshots the database first.
    #[inline]
    #[must_use]
    pub fn auto_pre_discard(&self) -> bool {
        self.auto_pre_discard
    }
}

impl Default for BackupSection {
    #[inline]
    fn default() -> Self {
        Self {
            dir: None,
            retain_count: Some(5),
            retain_days: None,
            auto_pre_migration: true,
            auto_pre_import: true,
            auto_pre_discard: true,
        }
    }
}

/// Raw deserialized `[db]` settings.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawDbSection {
    /// Database file path; relative values were anchored by their source.
    path: Option<String>,
}

/// Raw deserialized `[financial-year]` settings.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawFinancialYearSection {
    /// Start month (1-based, 1–12).
    start_month: u8,
    /// Start day (1-based, 1–28), capped so the day exists in every month.
    start_day: u8,
}

/// Raw deserialized `[periods]` settings.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawPeriodsSection {
    /// Fortnightly anchor date string, if set.
    fortnightly_anchor: Option<String>,
}

/// Raw deserialized `[import]` settings.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawImportSection {
    /// Root directory for importer source documents.
    documents_root: Option<String>,
}

/// Raw deserialized `[plugins]` settings.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawPluginsSection {
    /// Additional plugin directories, searched in order.
    #[serde(default)]
    dirs: Vec<String>,
}

/// Raw deserialized `[server]` settings.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawServerSection {
    /// Address `borrow-checker-server` listens on.
    bind: String,
    /// Hostnames the server answers besides IP literals and `localhost`.
    #[serde(default)]
    allowed_hosts: RawHostList,
}

/// `allowed-hosts` as a TOML array or, from the environment, one
/// comma-separated string.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
enum RawHostList {
    /// A TOML array of hostnames.
    List(Vec<String>),
    /// Comma-separated hostnames, as `BC_SERVER__ALLOWED_HOSTS` gives them.
    Joined(String),
}

impl Default for RawHostList {
    #[inline]
    fn default() -> Self {
        Self::List(Vec::new())
    }
}

impl RawHostList {
    /// Splits, trims and lowercases the entries, dropping empty ones and a
    /// trailing root dot.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Validation`] for an entry that carries a port,
    /// a scheme or a path.
    fn normalise(self) -> Result<Vec<String>, ConfigError> {
        let entries = match self {
            Self::List(list) => list,
            Self::Joined(joined) => joined.split(',').map(str::to_owned).collect(),
        };
        entries
            .iter()
            .map(|e| e.trim())
            .filter(|e| !e.is_empty())
            .map(|e| {
                if e.contains([':', '/', '[', ']']) {
                    return Err(ConfigError::Validation(format!(
                        "invalid server.allowed-hosts entry '{e}': give a hostname without a \
                         scheme, port or path"
                    )));
                }
                Ok(e.trim_end_matches('.').to_ascii_lowercase())
            })
            .collect()
    }
}

/// Web server settings from the `[server]` section.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ServerSection {
    /// Address `borrow-checker-server` listens on.
    bind: std::net::SocketAddr,
    /// Lowercase hostnames the server answers besides IP literals and
    /// `localhost`.
    #[serde(default)]
    allowed_hosts: Vec<String>,
}

impl ServerSection {
    /// Returns the listen address.
    #[inline]
    #[must_use]
    pub fn bind(&self) -> std::net::SocketAddr {
        self.bind
    }

    /// Returns the hostnames the server answers besides IP literals and
    /// `localhost`.
    ///
    /// # Returns
    ///
    /// Lowercase hostnames without a trailing dot; empty by default.
    #[inline]
    #[must_use]
    pub fn allowed_hosts(&self) -> &[String] {
        &self.allowed_hosts
    }
}

impl Default for ServerSection {
    #[inline]
    fn default() -> Self {
        Self {
            bind: std::net::SocketAddr::from(([127, 0, 0, 1], 7171)),
            allowed_hosts: Vec::new(),
        }
    }
}

/// Raw deserialized settings before validation.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawSettings {
    /// Display commodity code string.
    display_commodity: String,
    /// `[db]` table.
    #[serde(default)]
    db: RawDbSection,
    /// `[financial-year]` table.
    financial_year: RawFinancialYearSection,
    /// `[periods]` table.
    #[serde(default)]
    periods: RawPeriodsSection,
    /// `[import]` table.
    #[serde(default)]
    import: RawImportSection,
    /// `[plugins]` table.
    #[serde(default)]
    plugins: RawPluginsSection,
    /// `[cli]` table.
    cli: RawCliSection,
    /// `[backup]` table.
    #[serde(default = "default_raw_backup")]
    backup: RawBackupSection,
    /// `[server]` table.
    server: RawServerSection,
}

/// Validated application-wide settings.
#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// Financial year start month (1-based, 1–12).
    financial_year_start_month: u8,
    /// Financial year start day (1-based, 1–28).
    ///
    /// Capped at 28 to ensure the start day exists in every calendar month,
    /// including February (which has at minimum 28 days). Use 1 for the
    /// safest cross-month anchor.
    ///
    /// # Example
    ///
    /// A value of `1` is always safe; `28` is the maximum accepted value.
    /// Values of 29, 30, or 31 are rejected during validation because they
    /// do not exist in every month.
    financial_year_start_day: u8,
    /// Fortnightly anchor date, if configured.
    fortnightly_anchor: Option<Date>,
    /// Display commodity code.
    display_commodity: CommodityCode,
    /// Database file path override from the config file.
    ///
    /// When `None`, [`Settings::db_path`] falls back to [`default_db_path`].
    #[serde(default)]
    db_path: Option<std::path::PathBuf>,
    /// Ordered plugin search directories (user-configured, XDG data home).
    ///
    /// Directories are checked in order; a plugin found in an earlier directory
    /// takes precedence over the same-named plugin in a later one.
    #[serde(default)]
    plugin_dirs: Vec<std::path::PathBuf>,
    /// Root directory under which importer source documents live. Per-profile
    /// import-source locations are resolved relative to this root. `None` when
    /// unset.
    #[serde(default)]
    documents_root: Option<std::path::PathBuf>,
    /// CLI-specific settings from the `[cli]` section.
    #[serde(default)]
    cli: CliSection,
    /// Backup and rotation settings from the `[backup]` section.
    #[serde(default)]
    backup: BackupSection,
    /// Web server settings from the `[server]` section.
    #[serde(default)]
    server: ServerSection,
}

/// Resolves the ordered plugin search directories.
///
/// Priority: `BORROW_CHECKER_PLUGIN_DIR` env var (single dir) → `config_dirs`,
/// in order → the XDG data home default, appended if not already present.
fn resolve_plugin_dirs(config_dirs: &[String]) -> Vec<std::path::PathBuf> {
    let mut plugin_dirs: Vec<std::path::PathBuf> = Vec::new();

    if let Ok(dir) = std::env::var("BORROW_CHECKER_PLUGIN_DIR") {
        let p = std::path::PathBuf::from(&dir);
        if p.is_absolute() {
            tracing::debug!(dir = %p.display(), "plugin path: BORROW_CHECKER_PLUGIN_DIR override");
            plugin_dirs.push(p);
        } else {
            tracing::debug!(
                dir,
                "plugin path: BORROW_CHECKER_PLUGIN_DIR ignored (not absolute)"
            );
        }
    }

    for dir in config_dirs {
        let p = source::expand_home(dir);
        tracing::debug!(dir = %p.display(), "plugin path: from config file");
        plugin_dirs.push(p);
    }

    if let Some(dirs) = directories::BaseDirs::new() {
        let xdg_data = dirs.data_dir().join("borrow-checker").join("plugins");
        if !plugin_dirs.contains(&xdg_data) {
            tracing::debug!(dir = %xdg_data.display(), "plugin path: XDG data home default");
            plugin_dirs.push(xdg_data);
        }
    }

    plugin_dirs
}

/// Top-level keys that moved into a table, with the key that replaced each.
///
/// Keys are in their normalised kebab-case spelling, so a `snake_case`
/// leftover matches too.
const RETIRED_KEYS: &[(&str, &str)] = &[
    ("db-path", "[db] path"),
    ("financial-year-start-month", "[financial-year] start-month"),
    ("financial-year-start-day", "[financial-year] start-day"),
    ("fortnightly-anchor", "[periods] fortnightly-anchor"),
    ("documents-root", "[import] documents-root"),
    ("plugin-dirs", "[plugins] dirs"),
];

/// Rejects any retired top-level key in the merged configuration.
///
/// # Arguments
///
/// * `table` - The merged top-level table from every source.
///
/// # Errors
///
/// Returns [`ConfigError::Validation`] naming the source that set the key,
/// the key and its replacement.
fn reject_retired_keys(table: &config::Map<String, config::Value>) -> Result<(), ConfigError> {
    for (key, replacement) in RETIRED_KEYS {
        if let Some(value) = table.get(*key) {
            let origin = value.origin().unwrap_or("the configuration");
            return Err(ConfigError::Validation(format!(
                "{origin}: `{key}` is no longer read; set `{replacement}` instead"
            )));
        }
    }
    Ok(())
}

/// The tables a config may hold, in kebab-case.
const TABLES: &[&str] = &[
    "db",
    "financial-year",
    "periods",
    "import",
    "plugins",
    "cli",
    "backup",
    "server",
];

/// Rejects a top-level key that names a table and one of its keys.
///
/// `BC_BACKUP_DIR` separates table and key with one underscore, so it lands
/// at the top level as `backup-dir`, which serde would otherwise ignore.
///
/// # Arguments
///
/// * `table` - The merged top-level table from every source.
///
/// # Errors
///
/// Returns [`ConfigError::Validation`] naming the source, the key and the
/// spelling that reaches the table.
fn reject_flattened_table_keys(
    table: &config::Map<String, config::Value>,
) -> Result<(), ConfigError> {
    // The smallest key, so the error names the same key on every run.
    let flattened = table
        .iter()
        .filter_map(|(key, value)| {
            TABLES.iter().find_map(|section| {
                key.strip_prefix(section)
                    .and_then(|r| r.strip_prefix('-'))
                    .filter(|r| !r.is_empty())
                    .map(|rest| (key, value, *section, rest))
            })
        })
        .min_by_key(|(key, ..)| *key);
    let Some((key, value, section, rest)) = flattened else {
        return Ok(());
    };
    let origin = value.origin().unwrap_or("the configuration");
    let replacement = if origin == "the environment" {
        let var = |s: &str| s.replace('-', "_").to_uppercase();
        format!("BC_{}__{}", var(section), var(rest))
    } else {
        format!("[{section}] {rest}")
    };
    Err(ConfigError::Validation(format!(
        "{origin}: `{key}` is not a setting; set `{replacement}` instead"
    )))
}

/// Collects the process environment into a map of UTF-8 names and values.
///
/// A variable that is not valid UTF-8 is skipped unless its name starts with
/// the `BC_` prefix, matched case-insensitively as the environment source does.
///
/// # Arguments
///
/// * `vars` - Environment variables, as from [`std::env::vars_os`].
///
/// # Returns
///
/// The variables whose name and value are both valid UTF-8.
///
/// # Errors
///
/// Returns [`ConfigError::Validation`] when a `BC_` variable has a name or
/// value that is not valid UTF-8.
fn env_map(
    vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Result<config::Map<String, String>, ConfigError> {
    let mut map = config::Map::new();
    for (os_name, os_value) in vars {
        let is_bc = os_name
            .as_encoded_bytes()
            .get(..3)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"BC_"));
        match (os_name.into_string(), os_value.into_string()) {
            (Ok(name), Ok(value)) => {
                map.insert(name, value);
            }
            (Ok(name), Err(_)) if is_bc => {
                return Err(ConfigError::Validation(format!(
                    "environment variable {name} is not valid UTF-8"
                )));
            }
            (Err(bad_name), _) if is_bc => {
                return Err(ConfigError::Validation(format!(
                    "environment variable name {} is not valid UTF-8",
                    bad_name.to_string_lossy()
                )));
            }
            _ => {}
        }
    }
    Ok(map)
}

impl Settings {
    /// Loads settings from the configuration hierarchy.
    ///
    /// Sources (lowest to highest priority):
    /// 1. Built-in defaults
    /// 2. `$XDG_CONFIG_HOME/borrow-checker/config.toml` (or `~/.config/…`
    ///    when `XDG_CONFIG_HOME` is unset)
    /// 3. Platform-native config directory (e.g.
    ///    `~/Library/Application Support/borrow-checker/config.toml` on macOS)
    /// 4. `./borrow-checker.toml`
    /// 5. Environment variables: `BC_` then the key, with `__` between a
    ///    table and its key (`BC_DB__PATH`, `BC_BACKUP__RETAIN_COUNT`)
    ///
    /// Keys are kebab-case; `snake_case` is accepted. A relative path in a
    /// file resolves against the directory of that file's canonical path; one
    /// from the environment resolves against the working directory.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if any source fails to parse, a key is spelled
    /// both ways in one file, a value is out of range, a retired top-level key
    /// such as `db-path` or the retired `BC_DB_PATH` variable is set, a table
    /// key sits at the top level (`BC_BACKUP_DIR` for `BC_BACKUP__DIR`), or a
    /// `BC_` variable is not valid UTF-8.
    #[inline]
    pub fn load() -> Result<Self, ConfigError> {
        let mut files: Vec<PathBuf> = config_file_paths().collect();
        files.push(PathBuf::from("borrow-checker.toml"));
        Self::load_from(&files, env_map(std::env::vars_os())?)
    }

    /// Loads settings from explicit file sources and an environment map.
    ///
    /// # Arguments
    ///
    /// * `files` - Config files, lowest priority first; missing files are skipped.
    /// * `env` - Environment variables; only `BC_`-prefixed ones are read.
    ///
    /// # Errors
    ///
    /// As for [`Settings::load`].
    fn load_from(files: &[PathBuf], env: config::Map<String, String>) -> Result<Self, ConfigError> {
        if env.keys().any(|k| k.eq_ignore_ascii_case("BC_DB_PATH")) {
            return Err(ConfigError::Validation(
                "BC_DB_PATH is no longer read; set BC_DB__PATH instead".to_owned(),
            ));
        }

        let mut builder = config::Config::builder()
            .set_default("display-commodity", "AUD")?
            .set_default("financial-year.start-month", 7_i64)?
            .set_default("financial-year.start-day", 1_i64)?
            .set_default("cli.json", false)?
            .set_default("cli.log", Option::<String>::None)?
            .set_default("backup.dir", Option::<String>::None)?
            .set_default("backup.retain-count", 5_i64)?
            .set_default("backup.retain-days", Option::<i64>::None)?
            .set_default("backup.auto-pre-migration", true)?
            .set_default("backup.auto-pre-import", true)?
            .set_default("backup.auto-pre-discard", true)?
            .set_default("server.bind", "127.0.0.1:7171")?;

        for path in files {
            tracing::debug!(path = %path.display(), "config: adding source");
            builder = builder.add_source(source::ConfigFile::new(path));
        }
        tracing::debug!("config: adding source BC_* environment variables");
        builder = builder.add_source(
            config::Environment::with_prefix("BC")
                .prefix_separator("_")
                .separator("__")
                .convert_case(config::Case::Kebab)
                .source(Some(env)),
        );

        let merged = builder.build()?;
        let top_level = merged.collect()?;
        reject_retired_keys(&top_level)?;
        reject_flattened_table_keys(&top_level)?;
        let raw: RawSettings = merged.try_deserialize()?;
        Self::validate(raw)
    }

    /// Validates raw settings and returns a [`Settings`] instance.
    fn validate(raw: RawSettings) -> Result<Self, ConfigError> {
        let RawFinancialYearSection {
            start_month,
            start_day,
        } = raw.financial_year;
        if !(1..=12).contains(&start_month) {
            return Err(ConfigError::Validation(format!(
                "financial-year.start-month {start_month} is out of range 1–12"
            )));
        }
        if !(1..=28).contains(&start_day) {
            return Err(ConfigError::Validation(format!(
                "financial-year.start-day must be between 1 and 28 (capped at 28 so the day \
                 exists in every month, including February); got {start_day}"
            )));
        }
        tracing::debug!(start_month, start_day, "config: financial year");

        let fortnightly_anchor = raw
            .periods
            .fortnightly_anchor
            .map(|s| {
                s.parse::<Date>().map_err(|e| {
                    ConfigError::Validation(format!(
                        "invalid periods.fortnightly-anchor '{s}': {e}"
                    ))
                })
            })
            .transpose()?;
        tracing::debug!(fortnightly_anchor = ?fortnightly_anchor, "config: fortnightly anchor");

        if raw.display_commodity.is_empty() {
            return Err(ConfigError::Validation(
                "display-commodity must not be empty".into(),
            ));
        }
        tracing::debug!(display_commodity = %raw.display_commodity, "config: display commodity");

        let db_path = match raw.db.path.as_deref() {
            Some("") => {
                return Err(ConfigError::Validation("db.path must not be empty".into()));
            }
            Some(p) => Some(source::expand_home(p)),
            None => None,
        };
        tracing::debug!(db_path = ?db_path, "config: db.path");

        let documents_root = raw
            .import
            .documents_root
            .as_deref()
            .map(source::expand_home);
        tracing::debug!(documents_root = ?documents_root, "config: import.documents-root");

        let plugin_dirs = resolve_plugin_dirs(&raw.plugins.dirs);

        let cli = CliSection {
            json: raw.cli.json,
            log: raw.cli.log.clone(),
        };
        tracing::debug!(cli.json = cli.json, cli.log = ?cli.log, "config: cli section");

        // `retain-count = 0` (from any source: config file, env var, or the
        // 0-sentinel written by `write_backup_table` for a cleared value) is
        // treated as "no count limit", matching the on-disk convention that
        // disambiguates "user cleared it" (0) from "user never set it"
        // (absent key, defaulted to 5 by `Settings::load`).
        let backup = BackupSection {
            dir: raw.backup.dir.as_deref().map(source::expand_home),
            retain_count: raw.backup.retain_count.filter(|&n| n != 0),
            retain_days: raw.backup.retain_days,
            auto_pre_migration: raw.backup.auto_pre_migration,
            auto_pre_import: raw.backup.auto_pre_import,
            auto_pre_discard: raw.backup.auto_pre_discard,
        };
        tracing::debug!(
            backup.dir = ?backup.dir,
            backup.retain_count = ?backup.retain_count,
            backup.retain_days = ?backup.retain_days,
            backup.auto_pre_migration = backup.auto_pre_migration,
            backup.auto_pre_import = backup.auto_pre_import,
            backup.auto_pre_discard = backup.auto_pre_discard,
            "config: backup section"
        );

        let server_bind = raw
            .server
            .bind
            .parse::<std::net::SocketAddr>()
            .map_err(|e| {
                ConfigError::Validation(format!("invalid server.bind '{}': {e}", raw.server.bind))
            })?;
        let server = ServerSection {
            bind: server_bind,
            allowed_hosts: raw.server.allowed_hosts.normalise()?,
        };
        tracing::debug!(
            server.bind = %server.bind,
            server.allowed_hosts = ?server.allowed_hosts,
            "config: server section"
        );

        Ok(Self {
            financial_year_start_month: start_month,
            financial_year_start_day: start_day,
            fortnightly_anchor,
            display_commodity: CommodityCode::new(raw.display_commodity),
            db_path,
            plugin_dirs,
            documents_root,
            cli,
            backup,
            server,
        })
    }

    /// Returns the financial year start month (1-based).
    #[inline]
    #[must_use]
    pub fn financial_year_start_month(&self) -> u8 {
        self.financial_year_start_month
    }

    /// Returns the financial year start day (1-based).
    #[inline]
    #[must_use]
    pub fn financial_year_start_day(&self) -> u8 {
        self.financial_year_start_day
    }

    /// Returns the fortnightly anchor date, if configured.
    #[inline]
    #[must_use]
    pub fn fortnightly_anchor(&self) -> Option<Date> {
        self.fortnightly_anchor
    }

    /// Returns the display commodity code.
    #[inline]
    #[must_use]
    pub fn display_commodity(&self) -> &CommodityCode {
        &self.display_commodity
    }

    /// Returns the resolved database path.
    ///
    /// If a path was explicitly configured or set via [`Self::set_db_path`],
    /// that value is returned; otherwise falls back to [`default_db_path`].
    #[inline]
    #[must_use]
    pub fn db_path(&self) -> std::path::PathBuf {
        self.db_path.clone().unwrap_or_else(default_db_path)
    }

    /// Returns the ordered list of plugin search directories.
    ///
    /// Callers (typically the CLI) may append additional directories before
    /// passing the list to `PluginRegistry::load`.
    #[inline]
    #[must_use]
    pub fn plugin_paths(&self) -> &[std::path::PathBuf] {
        &self.plugin_dirs
    }

    /// Returns the configured import-sources root, or `None` when unset.
    #[inline]
    #[must_use]
    pub fn documents_root(&self) -> Option<&std::path::Path> {
        self.documents_root.as_deref()
    }

    /// Overrides the database path at runtime (e.g. from a CLI flag).
    ///
    /// This takes precedence over any value loaded from the config file.
    ///
    /// # Arguments
    ///
    /// * `path` - Absolute path to the SQLite database file.
    #[inline]
    pub fn set_db_path(&mut self, path: std::path::PathBuf) {
        self.db_path = Some(path);
    }

    /// Overrides the backup directory at runtime.
    ///
    /// This takes precedence over any `[backup] dir` loaded from the config
    /// file, and [`BackupSection::resolved_dir`] returns it.
    ///
    /// # Arguments
    ///
    /// * `dir` - Directory backups are written to and restored from.
    #[inline]
    pub fn set_backup_dir(&mut self, dir: std::path::PathBuf) {
        self.backup.dir = Some(dir);
    }

    /// Returns the CLI-specific settings from the `[cli]` config section.
    #[inline]
    #[must_use]
    pub fn cli(&self) -> &CliSection {
        &self.cli
    }

    /// Returns the backup settings from the `[backup]` config section.
    #[inline]
    #[must_use]
    pub fn backup(&self) -> &BackupSection {
        &self.backup
    }

    /// Returns the web server settings from the `[server]` config section.
    #[inline]
    #[must_use]
    pub fn server(&self) -> &ServerSection {
        &self.server
    }
}

impl Default for Settings {
    #[inline]
    fn default() -> Self {
        Self {
            financial_year_start_month: 7,
            financial_year_start_day: 1,
            fortnightly_anchor: None,
            display_commodity: CommodityCode::new("AUD"),
            db_path: None,
            plugin_dirs: Vec::new(),
            documents_root: None,
            cli: CliSection::default(),
            backup: BackupSection::default(),
            server: ServerSection::default(),
        }
    }
}

/// Returns an iterator over candidate config file paths, in priority order.
///
/// Priority (lowest first, so later entries override earlier ones):
/// 1. XDG path: `$XDG_CONFIG_HOME/borrow-checker/config.toml`, falling back
///    to `~/.config/borrow-checker/config.toml` when `XDG_CONFIG_HOME` is
///    unset (the XDG Base Directory default).
/// 2. Platform-native path from the [`directories`] crate (e.g.
///    `~/Library/Application Support/borrow-checker/config.toml` on macOS).
///
/// Duplicate paths are skipped; on Linux the two candidates typically resolve
/// to the same location (both honour `XDG_CONFIG_HOME`), so the iterator
/// yields a single path.
///
/// No file is required to exist — callers that want only existing paths should
/// filter with [`Path::exists`](std::path::Path::exists).
pub fn config_file_paths() -> impl Iterator<Item = PathBuf> {
    // XDG path: $XDG_CONFIG_HOME or fall back to $HOME/.config.
    // Per the XDG Base Directory Specification, XDG_CONFIG_HOME must be an
    // absolute path; non-absolute values are ignored.
    let xdg_base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join(".config")));
    let xdg_path = xdg_base.map(|b| b.join("borrow-checker").join("config.toml"));

    // Platform-native path via the directories crate
    let native_path = directories::ProjectDirs::from("", "", "borrow-checker")
        .map(|p| p.config_dir().join("config.toml"));

    let mut seen = std::collections::HashSet::new();
    [xdg_path, native_path]
        .into_iter()
        .flatten()
        .filter(move |p| seen.insert(p.clone()))
}

/// Returns the spelling `table` already uses for the kebab-case `key`.
///
/// An existing `snake_case` key is updated in place; a key the table lacks is
/// written in kebab-case.
fn spelling_in(table: &toml_edit::Table, key: &'static str) -> String {
    let snake = key.replace('-', "_");
    if table.contains_key(&snake) {
        snake
    } else {
        key.to_owned()
    }
}

/// Removes `key` from `table` in both spellings.
fn remove_either(table: &mut toml_edit::Table, key: &'static str) {
    table.remove(key);
    table.remove(&key.replace('-', "_"));
}

/// Returns whether the raw `item` in the config file at `path` loads as `dir`.
///
/// The raw value resolves as a load would: against the directory of the
/// file's canonical path, then with `~` expanded.
fn resolves_to(item: Option<&toml_edit::Item>, path: &std::path::Path, dir: &str) -> bool {
    let Some(raw) = item.and_then(toml_edit::Item::as_str) else {
        return false;
    };
    let Some(base) = std::fs::canonicalize(path)
        .ok()
        .and_then(|c| c.parent().map(std::path::Path::to_path_buf))
    else {
        return false;
    };
    source::resolve_from_file(raw, &base) == std::path::Path::new(dir)
}

/// Writes the `[backup]` table into the TOML document at `path`, preserving all
/// other content (comments, formatting, unrelated sections). Creates the file
/// and any parent directories if they do not exist.
///
/// `retain_count: None` is written as the sentinel `retain-count = 0` rather
/// than removing the key, so the "unlimited" choice survives the next
/// `Settings::load`, which would otherwise re-apply its `set_default` of `5`
/// to an absent key. `dir` and `retain_days` have no such default, so `None`
/// removes those keys as expected.
///
/// Keys the table already holds keep their spelling; new keys are kebab-case.
/// An existing `dir` that already resolves to the incoming `dir` keeps its
/// raw text, so a relative value is not replaced by this machine's absolute
/// path.
/// The file is rewritten with `std::fs::write`, which follows a symlink, so a
/// linked config file is updated at its target and the link survives.
fn write_backup_table(
    path: &std::path::Path,
    dir: Option<&str>,
    retain_count: Option<u32>,
    retain_days: Option<u32>,
    auto_pre_migration: bool,
) -> Result<(), ConfigError> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(ConfigError::Validation(format!(
                "cannot read config {}: {e}",
                path.display()
            )));
        }
    };
    let mut doc = existing
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| ConfigError::Validation(format!("config is not valid TOML: {e}")))?;

    let backup_item = doc
        .entry("backup")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let Some(backup_table) = backup_item.as_table_mut() else {
        return Err(ConfigError::Validation(
            "config `backup` key is not a table".to_owned(),
        ));
    };

    match dir {
        Some(d) if !resolves_to(backup_table.get("dir"), path, d) => {
            backup_table.insert("dir", toml_edit::value(d));
        }
        Some(_) => {}
        None => {
            backup_table.remove("dir");
        }
    }
    let count_key = spelling_in(backup_table, "retain-count");
    backup_table.insert(
        &count_key,
        toml_edit::value(i64::from(retain_count.unwrap_or(0))),
    );
    match retain_days {
        Some(n) => {
            let key = spelling_in(backup_table, "retain-days");
            backup_table.insert(&key, toml_edit::value(i64::from(n)));
        }
        None => remove_either(backup_table, "retain-days"),
    }
    let migration_key = spelling_in(backup_table, "auto-pre-migration");
    backup_table.insert(&migration_key, toml_edit::value(auto_pre_migration));

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| ConfigError::Validation(format!("cannot create config dir: {e}")))?;
    }
    std::fs::write(path, doc.to_string())
        .map_err(|e| ConfigError::Validation(format!("cannot write config: {e}")))?;
    Ok(())
}

/// Persists the `[backup]` section to the user config file.
///
/// Writes to the first existing config file among [`config_file_paths`], or the
/// first candidate path if none exist yet. Only the `[backup]` table is
/// modified; all other sections — including comments and their ordering — are
/// preserved by `toml_edit`, though exact whitespace is not guaranteed
/// byte-for-byte.
///
/// A value written here is not guaranteed to be what a subsequent
/// [`Settings::load`] returns: config layering means a higher-priority config
/// file or a matching `BC_*` environment variable can still shadow it.
///
/// # Arguments
///
/// * `dir` - Backup directory override, or `None` to clear it (use the default).
/// * `retain_count` - "Keep N newest" limit, or `None` to clear it (persisted
///   as the on-disk sentinel `retain-count = 0`).
/// * `retain_days` - "Keep newer than N days" limit, or `None` to clear it.
/// * `auto_pre_migration` - Whether automatic pre-migration snapshots are on.
///
/// # Returns
///
/// The path of the config file written.
///
/// # Errors
///
/// Returns [`ConfigError::Validation`] if no config path can be resolved, the
/// existing file cannot be read or is not valid TOML, or the file cannot be
/// written.
#[inline]
pub fn persist_backup_section(
    dir: Option<&str>,
    retain_count: Option<u32>,
    retain_days: Option<u32>,
    auto_pre_migration: bool,
) -> Result<std::path::PathBuf, ConfigError> {
    let candidates: Vec<std::path::PathBuf> = config_file_paths().collect();
    let target = candidates
        .iter()
        .find(|p| p.exists())
        .or_else(|| candidates.first())
        .cloned()
        .ok_or_else(|| ConfigError::Validation("no config file path available".to_owned()))?;
    write_backup_table(&target, dir, retain_count, retain_days, auto_pre_migration)?;
    Ok(target)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    #[cfg(unix)]
    use std::ffi::OsStr;
    #[cfg(unix)]
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt as _;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::Path;
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    /// Builds a fully-valid [`RawSettings`] that passes validation so individual
    /// tests can override exactly one field at a time.
    fn valid_raw() -> RawSettings {
        RawSettings {
            display_commodity: "AUD".to_owned(),
            db: RawDbSection { path: None },
            financial_year: RawFinancialYearSection {
                start_month: 7,
                start_day: 1,
            },
            periods: RawPeriodsSection {
                fortnightly_anchor: None,
            },
            import: RawImportSection {
                documents_root: None,
            },
            plugins: RawPluginsSection { dirs: Vec::new() },
            cli: RawCliSection {
                json: false,
                log: None,
            },
            backup: default_raw_backup(),
            server: RawServerSection {
                bind: "127.0.0.1:7171".into(),
                allowed_hosts: RawHostList::default(),
            },
        }
    }

    /// Writes `text` to `dir/name` and returns the path.
    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).expect("write config");
        path
    }

    /// Builds an environment map from `(name, value)` pairs.
    fn env(pairs: &[(&str, &str)]) -> config::Map<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn load_from_nothing_gives_defaults() {
        let s = Settings::load_from(&[], env(&[])).expect("load");
        assert_eq!(s.financial_year_start_month(), 7);
        assert_eq!(s.db_path(), default_db_path());
        assert_eq!(s.backup().retain_count(), Some(5));
    }

    #[test]
    fn kebab_file_sets_every_table() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = std::fs::canonicalize(dir.path()).expect("canonical");
        let db_path = base.join("data").join("db.sqlite");
        let docs_root = base.join("docs");
        let file = write(
            dir.path(),
            "c.toml",
            &format!(
                "display-commodity = \"USD\"\n\
                 [db]\npath = {:?}\n\
                 [financial-year]\nstart-month = 1\nstart-day = 2\n\
                 [periods]\nfortnightly-anchor = \"2024-01-05\"\n\
                 [import]\ndocuments-root = {:?}\n\
                 [cli]\njson = true\n\
                 [backup]\nretain-count = 9\nretain-days = 30\nauto-pre-import = false\n\
                 [server]\nbind = \"127.0.0.1:7200\"\n",
                db_path.to_string_lossy(),
                docs_root.to_string_lossy(),
            ),
        );
        let s = Settings::load_from(&[file], env(&[])).expect("load");
        assert_eq!(s.display_commodity().to_string(), "USD");
        assert_eq!(s.db_path(), db_path);
        assert_eq!(s.financial_year_start_month(), 1);
        assert_eq!(s.financial_year_start_day(), 2);
        assert_eq!(
            s.fortnightly_anchor().map(|d| d.to_string()),
            Some("2024-01-05".to_owned())
        );
        assert_eq!(s.documents_root(), Some(docs_root.as_path()));
        assert!(s.cli().json());
        assert_eq!(s.backup().retain_count(), Some(9));
        assert_eq!(s.backup().retain_days(), Some(30));
        assert!(!s.backup().auto_pre_import());
        assert_eq!(s.server().bind(), "127.0.0.1:7200".parse().expect("addr"));
    }

    #[test]
    fn snake_file_is_accepted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(
            dir.path(),
            "c.toml",
            "display_commodity = \"USD\"\n[financial_year]\nstart_month = 3\n[backup]\nretain_count = 2\n",
        );
        let s = Settings::load_from(&[file], env(&[])).expect("load");
        assert_eq!(s.display_commodity().to_string(), "USD");
        assert_eq!(s.financial_year_start_month(), 3);
        assert_eq!(s.backup().retain_count(), Some(2));
    }

    #[test]
    fn later_file_overrides_earlier_across_spellings() {
        let dir = tempfile::tempdir().expect("tempdir");
        let user = write(
            dir.path(),
            "user.toml",
            "[backup]\nretain_count = 2\nretain_days = 7\n",
        );
        let local = write(dir.path(), "local.toml", "[backup]\nretain-count = 8\n");
        let s = Settings::load_from(&[user, local], env(&[])).expect("load");
        assert_eq!(s.backup().retain_count(), Some(8));
        assert_eq!(s.backup().retain_days(), Some(7));
    }

    #[test]
    fn both_spellings_in_one_file_fails_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(
            dir.path(),
            "c.toml",
            "display_commodity = \"USD\"\ndisplay-commodity = \"EUR\"\n",
        );
        let err = Settings::load_from(&[file], env(&[])).expect_err("collision");
        let msg = err.to_string();
        assert!(
            msg.contains("display_commodity") && msg.contains("display-commodity"),
            "{msg}"
        );
    }

    #[rstest]
    #[case("BC_DISPLAY_COMMODITY", "EUR")]
    #[case("BC_DB__PATH", "/env/db.sqlite")]
    #[case("BC_FINANCIAL_YEAR__START_MONTH", "4")]
    #[case("BC_PERIODS__FORTNIGHTLY_ANCHOR", "2024-02-02")]
    #[case("BC_IMPORT__DOCUMENTS_ROOT", "/env/docs")]
    #[case("BC_CLI__JSON", "true")]
    #[case("BC_BACKUP__DIR", "/env/bk")]
    #[case("BC_BACKUP__RETAIN_COUNT", "11")]
    #[case("BC_BACKUP__AUTO_PRE_DISCARD", "false")]
    #[case("BC_SERVER__BIND", "127.0.0.1:7300")]
    fn env_reaches_every_table(#[case] name: &str, #[case] value: &str) {
        let s = Settings::load_from(&[], env(&[(name, value)])).expect("load");
        let observed = match name {
            "BC_DISPLAY_COMMODITY" => s.display_commodity().to_string(),
            "BC_DB__PATH" => s.db_path().to_string_lossy().into_owned(),
            "BC_FINANCIAL_YEAR__START_MONTH" => s.financial_year_start_month().to_string(),
            "BC_PERIODS__FORTNIGHTLY_ANCHOR" => s
                .fortnightly_anchor()
                .map(|d| d.to_string())
                .unwrap_or_default(),
            "BC_IMPORT__DOCUMENTS_ROOT" => s
                .documents_root()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            "BC_CLI__JSON" => s.cli().json().to_string(),
            "BC_BACKUP__DIR" => s.backup().resolved_dir().to_string_lossy().into_owned(),
            "BC_BACKUP__RETAIN_COUNT" => s
                .backup()
                .retain_count()
                .map(|n| n.to_string())
                .unwrap_or_default(),
            "BC_BACKUP__AUTO_PRE_DISCARD" => s.backup().auto_pre_discard().to_string(),
            "BC_SERVER__BIND" => s.server().bind().to_string(),
            other => panic!("unmapped case {other}"),
        };
        assert_eq!(observed, value);
    }

    #[test]
    fn env_overrides_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(dir.path(), "c.toml", "[db]\npath = \"/file/db.sqlite\"\n");
        let s =
            Settings::load_from(&[file], env(&[("BC_DB__PATH", "/env/db.sqlite")])).expect("load");
        assert_eq!(s.db_path(), PathBuf::from("/env/db.sqlite"));
    }

    #[test]
    fn relative_env_path_stays_relative_to_cwd() {
        let s = Settings::load_from(&[], env(&[("BC_DB__PATH", "rel/db.sqlite")])).expect("load");
        assert_eq!(s.db_path(), PathBuf::from("rel/db.sqlite"));
    }

    #[test]
    fn relative_file_path_resolves_beside_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(dir.path(), "c.toml", "[db]\npath = \"ledger/db.sqlite\"\n");
        let s = Settings::load_from(&[file], env(&[])).expect("load");
        let base = std::fs::canonicalize(dir.path()).expect("canonical");
        assert_eq!(s.db_path(), base.join("ledger/db.sqlite"));
    }

    #[test]
    fn tilde_db_path_expands_home() {
        let s =
            Settings::load_from(&[], env(&[("BC_DB__PATH", "~/ledger/db.sqlite")])).expect("load");
        let home = directories::BaseDirs::new()
            .expect("home")
            .home_dir()
            .to_owned();
        assert_eq!(s.db_path(), home.join("ledger/db.sqlite"));
    }

    #[rstest]
    #[case::file(Some("[db]\npath = \"\"\n"), &[])]
    #[case::env(None, &[("BC_DB__PATH", "")])]
    fn empty_db_path_is_rejected(#[case] file: Option<&str>, #[case] vars: &[(&str, &str)]) {
        let dir = tempfile::tempdir().expect("tempdir");
        let files: Vec<PathBuf> = file
            .map(|t| write(dir.path(), "c.toml", t))
            .into_iter()
            .collect();
        let err = Settings::load_from(&files, env(vars)).expect_err("empty");
        assert!(err.to_string().contains("db.path"), "{err}");
    }

    #[test]
    fn retired_db_path_variable_is_rejected() {
        let err =
            Settings::load_from(&[], env(&[("BC_DB_PATH", "/old.sqlite")])).expect_err("retired");
        assert!(err.to_string().contains("BC_DB__PATH"), "{err}");
    }

    #[test]
    fn retired_db_path_variable_is_rejected_in_any_case() {
        let err =
            Settings::load_from(&[], env(&[("bc_db_path", "/old.sqlite")])).expect_err("retired");
        assert!(err.to_string().contains("BC_DB__PATH"), "{err}");
    }

    #[rstest]
    #[case("BC_BACKUP_DIR", "BC_BACKUP__DIR")]
    #[case("BC_BACKUP_RETAIN_COUNT", "BC_BACKUP__RETAIN_COUNT")]
    #[case("BC_CLI_JSON", "BC_CLI__JSON")]
    #[case("bc_import_documents_root", "BC_IMPORT__DOCUMENTS_ROOT")]
    #[case("BC_SERVER_BIND", "BC_SERVER__BIND")]
    fn single_underscore_table_variable_is_rejected(#[case] var: &str, #[case] expected: &str) {
        let err = Settings::load_from(&[], env(&[(var, "1")])).expect_err("unknown key");
        assert!(err.to_string().contains(expected), "{err}");
    }

    #[test]
    fn table_key_at_top_level_of_a_file_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(dir.path(), "c.toml", "backup_dir = \"bk\"\n");
        let msg = Settings::load_from(&[file], env(&[]))
            .expect_err("unknown key")
            .to_string();
        assert!(msg.contains("[backup] dir"), "{msg}");
        assert!(msg.contains("c.toml"), "{msg}");
    }

    #[rstest]
    #[case("db_path", "[db] path")]
    #[case("financial-year-start-month", "[financial-year] start-month")]
    #[case("financial_year_start_day", "[financial-year] start-day")]
    #[case("fortnightly-anchor", "[periods] fortnightly-anchor")]
    #[case("documents_root", "[import] documents-root")]
    #[case("plugin-dirs", "[plugins] dirs")]
    fn retired_flat_keys_are_rejected(#[case] key: &str, #[case] replacement: &str) {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(dir.path(), "c.toml", &format!("{key} = \"old\"\n"));
        let err = Settings::load_from(&[file], env(&[])).expect_err("retired key");
        let msg = err.to_string();
        assert!(msg.contains(replacement), "{msg}");
        assert!(msg.contains(&key.replace('_', "-")), "{msg}");
        assert!(msg.contains("c.toml"), "{msg}");
    }

    #[test]
    fn retired_flat_key_in_the_environment_is_rejected() {
        let err = Settings::load_from(&[], env(&[("BC_FINANCIAL_YEAR_START_MONTH", "4")]))
            .expect_err("retired key");
        assert!(
            err.to_string().contains("[financial-year] start-month"),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[rstest]
    #[case::value(b"BC_DISPLAY_COMMODITY".as_slice(), b"\xffUSD".as_slice())]
    #[case::name(b"BC_\xff".as_slice(), b"USD".as_slice())]
    #[case::lowercase_name(b"bc_display_commodity".as_slice(), b"\xff".as_slice())]
    fn non_utf8_bc_variable_is_rejected(#[case] name: &[u8], #[case] value: &[u8]) {
        let vars = [(
            OsStr::from_bytes(name).to_owned(),
            OsStr::from_bytes(value).to_owned(),
        )];
        let err = env_map(vars).expect_err("non-UTF-8");
        assert!(err.to_string().contains("UTF-8"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_unrelated_variable_is_skipped() {
        let vars = [
            (
                OsStr::from_bytes(b"OTHER").to_owned(),
                OsStr::from_bytes(b"\xff").to_owned(),
            ),
            (OsString::from("BC_CLI__JSON"), OsString::from("true")),
        ];
        let map = env_map(vars).expect("unrelated variables are skipped");
        assert_eq!(map.get("BC_CLI__JSON").map(String::as_str), Some("true"));
        assert!(!map.contains_key("OTHER"));
    }

    #[test]
    fn default_settings_fy_start_is_july() {
        let s = Settings::default();
        assert_eq!(s.financial_year_start_month(), 7);
        assert_eq!(s.financial_year_start_day(), 1);
    }

    #[test]
    fn documents_root_defaults_to_none() {
        let settings = Settings::default();
        assert_eq!(settings.documents_root(), None);
    }

    #[test]
    #[cfg(not(windows))]
    fn config_file_paths_contains_xdg_path_when_env_is_set() {
        // SAFETY: Tests run in isolated processes under nextest; no concurrent
        // threads are reading environment variables.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", "/tmp/bc_test_xdg_9f3a") }
        let paths: Vec<_> = config_file_paths().collect();
        // SAFETY: Same as above — isolated process, no concurrent env access.
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") }
        assert!(
            paths.contains(&PathBuf::from(
                "/tmp/bc_test_xdg_9f3a/borrow-checker/config.toml"
            )),
            "expected XDG path in list, got: {paths:?}"
        );
    }

    #[test]
    fn config_file_paths_ignores_relative_xdg_config_home() {
        // SAFETY: Tests run in isolated processes under nextest; no concurrent
        // threads are reading environment variables.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", "relative/path") }
        let paths: Vec<_> = config_file_paths().collect();
        // SAFETY: Same as above — isolated process, no concurrent env access.
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") }
        assert!(
            !paths.iter().any(|p| p.starts_with("relative/path")),
            "relative XDG_CONFIG_HOME must be ignored per XDG spec; got: {paths:?}"
        );
    }

    #[test]
    fn config_file_paths_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for p in config_file_paths() {
            assert!(seen.insert(p.clone()), "duplicate path found: {p:?}");
        }
    }

    // --- Validation error paths ---

    #[test]
    fn invalid_fy_start_month_zero() {
        let raw = RawSettings {
            financial_year: RawFinancialYearSection {
                start_month: 0,
                start_day: 1,
            },
            ..valid_raw()
        };
        assert!(
            Settings::validate(raw).is_err(),
            "month 0 should fail validation"
        );
    }

    #[test]
    fn invalid_fy_start_month_thirteen() {
        let raw = RawSettings {
            financial_year: RawFinancialYearSection {
                start_month: 13,
                start_day: 1,
            },
            ..valid_raw()
        };
        assert!(
            Settings::validate(raw).is_err(),
            "month 13 should fail validation"
        );
    }

    #[test]
    fn invalid_fy_start_day_zero() {
        let raw = RawSettings {
            financial_year: RawFinancialYearSection {
                start_month: 7,
                start_day: 0,
            },
            ..valid_raw()
        };
        assert!(
            Settings::validate(raw).is_err(),
            "day 0 should fail validation"
        );
    }

    #[test]
    fn invalid_fy_start_day_twenty_nine() {
        let raw = RawSettings {
            financial_year: RawFinancialYearSection {
                start_month: 7,
                start_day: 29,
            },
            ..valid_raw()
        };
        let result = Settings::validate(raw);
        assert!(
            result.is_err(),
            "day 29 should fail validation (capped at 28)"
        );
        let err_msg = result.expect_err("already asserted is_err").to_string();
        assert!(
            err_msg.contains("28"),
            "error message should mention the cap of 28; got: {err_msg}"
        );
    }

    #[test]
    fn invalid_fortnightly_anchor_string() {
        let raw = RawSettings {
            periods: RawPeriodsSection {
                fortnightly_anchor: Some("not-a-date".to_owned()),
            },
            ..valid_raw()
        };
        assert!(
            Settings::validate(raw).is_err(),
            "invalid anchor date string should fail validation"
        );
    }

    #[test]
    fn invalid_empty_display_commodity() {
        let raw = RawSettings {
            display_commodity: String::new(),
            ..valid_raw()
        };
        assert!(
            Settings::validate(raw).is_err(),
            "empty display_commodity should fail validation"
        );
    }

    #[test]
    fn auto_pre_import_defaults_to_true() {
        let section = BackupSection::default();
        assert!(
            section.auto_pre_import(),
            "a snapshot before every import is the safe default"
        );
    }

    #[test]
    fn auto_pre_discard_defaults_to_true() {
        let section = BackupSection::default();
        assert!(
            section.auto_pre_discard(),
            "discard is irreversible; snapshot first unless told otherwise"
        );
    }

    #[test]
    fn default_backup_retain_count_is_five() {
        let s = Settings::default();
        assert_eq!(s.backup().retain_count(), Some(5));
        assert_eq!(s.backup().retain_days(), None);
        assert!(s.backup().auto_pre_migration());
    }

    #[test]
    fn default_backup_dir_ends_with_backups() {
        let dir = default_backup_dir();
        assert_eq!(dir.file_name().and_then(|n| n.to_str()), Some("backups"));
    }

    #[test]
    fn backup_section_resolves_default_dir_when_unset() {
        let s = Settings::default();
        assert_eq!(s.backup().resolved_dir(), default_backup_dir());
    }

    #[test]
    fn set_backup_dir_overrides_the_resolved_dir() {
        let mut s = Settings::default();
        s.set_backup_dir(PathBuf::from("/srv/bc/backups"));
        assert_eq!(s.backup().dir(), Some(Path::new("/srv/bc/backups")));
        assert_eq!(s.backup().resolved_dir(), PathBuf::from("/srv/bc/backups"));
    }

    #[test]
    fn writer_adds_kebab_keys_and_preserves_other_sections() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "display-commodity = \"USD\"\n\n[cli]\njson = true\n").expect("seed");

        write_backup_table(&cfg, Some("/tmp/bk"), Some(3), None, false).expect("write");

        let text = std::fs::read_to_string(&cfg).expect("read back");
        assert!(text.contains("[backup]"));
        assert!(text.contains("retain-count = 3"), "{text}");
        assert!(text.contains("auto-pre-migration = false"), "{text}");
        assert!(text.contains("dir = \"/tmp/bk\""));
        assert!(text.contains("display-commodity = \"USD\""));
        assert!(text.contains("json = true"));
    }

    #[test]
    fn writer_updates_snake_keys_in_place() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "[backup]\nretain_count = 3\nretain_days = 5\nauto_pre_migration = true\n",
        )
        .expect("seed");

        write_backup_table(&cfg, None, Some(7), Some(9), false).expect("write");

        let text = std::fs::read_to_string(&cfg).expect("read back");
        assert!(text.contains("retain_count = 7"), "{text}");
        assert!(text.contains("retain_days = 9"), "{text}");
        assert!(text.contains("auto_pre_migration = false"), "{text}");
        assert!(!text.contains("retain-count"), "{text}");
        assert!(!text.contains("retain-days"), "{text}");
        assert!(!text.contains("auto-pre-migration"), "{text}");
        let s = Settings::load_from(&[cfg], env(&[])).expect("reload");
        assert_eq!(s.backup().retain_count(), Some(7));
    }

    #[test]
    fn writer_removes_cleared_keys_in_either_spelling() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "[backup]\ndir = \"/old\"\nretain_days = 5\nretain-count = 3\n",
        )
        .expect("seed");

        write_backup_table(&cfg, None, None, None, true).expect("write");

        let text = std::fs::read_to_string(&cfg).expect("read back");
        assert!(!text.contains("retain_days"), "{text}");
        assert!(!text.contains("dir ="), "{text}");
        assert!(text.contains("retain-count = 0"), "{text}");
        let s = Settings::load_from(&[cfg], env(&[])).expect("reload");
        assert_eq!(s.backup().retain_count(), None);
    }

    #[cfg(unix)]
    #[test]
    fn writer_writes_through_a_symlink() {
        let target_dir = tempfile::tempdir().expect("tempdir");
        let link_dir = tempfile::tempdir().expect("tempdir");
        let target = target_dir.path().join("real.toml");
        std::fs::write(&target, "display-commodity = \"USD\"\n").expect("seed");
        let link = link_dir.path().join("config.toml");
        symlink(&target, &link).expect("symlink");

        write_backup_table(&link, None, Some(4), None, true).expect("write");

        assert!(
            std::fs::symlink_metadata(&link)
                .expect("meta")
                .file_type()
                .is_symlink()
        );
        let text = std::fs::read_to_string(&target).expect("read target");
        assert!(text.contains("retain-count = 4"), "{text}");
    }

    #[test]
    fn writer_keeps_a_relative_dir_that_resolves_to_the_incoming_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = write(
            dir.path(),
            "config.toml",
            "[backup]\ndir = \"bk\"\nretain-count = 5\n",
        );
        let resolved = std::fs::canonicalize(dir.path())
            .expect("canonical")
            .join("bk");

        write_backup_table(&cfg, resolved.to_str(), Some(3), None, true).expect("write");

        let text = std::fs::read_to_string(&cfg).expect("read back");
        assert!(text.contains("dir = \"bk\""), "{text}");
        assert!(text.contains("retain-count = 3"), "{text}");
    }

    #[test]
    fn writer_replaces_a_dir_that_resolves_elsewhere() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = write(dir.path(), "config.toml", "[backup]\ndir = \"bk\"\n");
        let elsewhere = dir.path().join("elsewhere");
        let other = elsewhere.to_str().expect("UTF-8 tempdir");

        write_backup_table(&cfg, Some(other), Some(3), None, true).expect("write");

        let text = std::fs::read_to_string(&cfg).expect("read back");
        assert!(!text.contains("dir = \"bk\""), "{text}");
        let s = Settings::load_from(&[cfg], env(&[])).expect("reload");
        assert_eq!(s.backup().dir(), Some(Path::new(other)));
    }

    #[cfg(unix)]
    #[test]
    fn writer_anchors_the_existing_dir_at_the_symlink_target() {
        let target_dir = tempfile::tempdir().expect("tempdir");
        let link_dir = tempfile::tempdir().expect("tempdir");
        let target = write(target_dir.path(), "real.toml", "[backup]\ndir = \"bk\"\n");
        let link = link_dir.path().join("config.toml");
        symlink(&target, &link).expect("symlink");
        let resolved = std::fs::canonicalize(target_dir.path())
            .expect("canonical")
            .join("bk");

        write_backup_table(&link, resolved.to_str(), Some(3), None, true).expect("write");

        let text = std::fs::read_to_string(&target).expect("read target");
        assert!(text.contains("dir = \"bk\""), "{text}");
        assert!(text.contains("retain-count = 3"), "{text}");
    }

    #[test]
    fn writer_leaves_an_unreadable_file_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = dir.path().join("config.toml");
        let bytes = b"display-commodity = \"\xff\"\n";
        std::fs::write(&cfg, bytes).expect("seed");

        write_backup_table(&cfg, None, Some(3), None, true).expect_err("unreadable");

        assert_eq!(std::fs::read(&cfg).expect("read back"), bytes);
    }

    #[test]
    fn fresh_config_defaults_retain_count_to_five() {
        let raw = valid_raw();
        let s = Settings::validate(raw).expect("valid_raw should validate");
        assert_eq!(s.backup().retain_count(), Some(5));
    }

    #[test]
    fn zero_retain_count_is_treated_as_unlimited() {
        let raw = RawSettings {
            backup: RawBackupSection {
                dir: None,
                retain_count: Some(0),
                retain_days: None,
                auto_pre_migration: true,
                auto_pre_import: true,
                auto_pre_discard: true,
            },
            ..valid_raw()
        };
        let s = Settings::validate(raw).expect("validate should succeed");
        assert_eq!(s.backup().retain_count(), None);
    }

    #[test]
    fn server_bind_defaults_to_loopback() {
        let s = Settings::load_from(&[], env(&[])).expect("load");
        assert_eq!(s.server().bind(), "127.0.0.1:7171".parse().expect("addr"));
    }

    #[test]
    fn server_bind_reads_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(dir.path(), "c.toml", "[server]\nbind = \"0.0.0.0:8080\"\n");
        let s = Settings::load_from(&[file], env(&[])).expect("load");
        assert_eq!(s.server().bind(), "0.0.0.0:8080".parse().expect("addr"));
    }

    #[test]
    fn server_bind_env_overrides_the_file() {
        let s =
            Settings::load_from(&[], env(&[("BC_SERVER__BIND", "127.0.0.1:9000")])).expect("load");
        assert_eq!(s.server().bind(), "127.0.0.1:9000".parse().expect("addr"));
    }

    #[test]
    fn server_bind_rejects_a_non_address() {
        let err =
            Settings::load_from(&[], env(&[("BC_SERVER__BIND", "localhost")])).expect_err("bad");
        assert!(err.to_string().contains("server.bind"), "{err}");
    }

    #[test]
    fn server_allowed_hosts_default_to_empty() {
        let s = Settings::load_from(&[], env(&[])).expect("load");
        assert_eq!(s.server().allowed_hosts(), Vec::<String>::new());
    }

    #[test]
    fn server_allowed_hosts_read_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(
            dir.path(),
            "c.toml",
            "[server]\nallowed-hosts = [\"Ledger.Example.ts.net.\", \"nas\"]\n",
        );
        let s = Settings::load_from(&[file], env(&[])).expect("load");
        assert_eq!(s.server().allowed_hosts(), ["ledger.example.ts.net", "nas"]);
    }

    #[rstest]
    #[case::one("nas", &["nas"])]
    #[case::several(" ledger.example.ts.net , nas,", &["ledger.example.ts.net", "nas"])]
    #[case::empty("", &[])]
    fn server_allowed_hosts_read_the_environment(#[case] value: &str, #[case] expected: &[&str]) {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(
            dir.path(),
            "c.toml",
            "[server]\nallowed-hosts = [\"other\"]\n",
        );
        let s = Settings::load_from(&[file], env(&[("BC_SERVER__ALLOWED_HOSTS", value)]))
            .expect("load");
        assert_eq!(s.server().allowed_hosts(), expected);
    }

    #[rstest]
    #[case::port("nas:7171")]
    #[case::scheme("http://nas")]
    fn server_allowed_hosts_reject_more_than_a_hostname(#[case] value: &str) {
        let err =
            Settings::load_from(&[], env(&[("BC_SERVER__ALLOWED_HOSTS", value)])).expect_err("bad");
        assert!(err.to_string().contains("server.allowed-hosts"), "{err}");
    }
}
