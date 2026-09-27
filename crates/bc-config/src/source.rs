//! A TOML config file as a `config` source.
//!
//! Each file is normalised on its own, before config-rs merges the layers, so
//! `retain_count` in one file and `retain-count` in another name the same key
//! and precedence holds between them.
use std::path::Path;
use std::path::PathBuf;

use config::Map;
use config::Source;
use config::Value;
use config::ValueKind;

/// Keys whose string values are filesystem paths, as `(table, key)`.
///
/// A relative value in a file resolves against that file's directory.
pub(crate) const PATH_KEYS: &[(&str, &str)] = &[
    ("db", "path"),
    ("backup", "dir"),
    ("import", "documents-root"),
    ("plugins", "dirs"),
];

/// A TOML config file whose keys are normalised to kebab-case and whose
/// relative path values are resolved against the file's canonical directory.
///
/// A missing file contributes nothing. A symlink resolves to its target, so a
/// linked `~/.config/borrow-checker/config.toml` anchors paths beside the file
/// it points at.
#[derive(Debug, Clone)]
pub(crate) struct ConfigFile {
    /// The path as given; canonicalised when collected.
    path: PathBuf,
}

impl ConfigFile {
    /// Creates a source for the TOML file at `path`.
    ///
    /// # Arguments
    ///
    /// * `path` - Location of the file; it need not exist.
    ///
    /// # Returns
    ///
    /// A source that reads the file when the config is built.
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the canonical path, or `None` when nothing exists at the path.
    ///
    /// A symlink whose target is missing is an error rather than `None`: it
    /// names a config the user meant to load.
    fn canonical(&self) -> Result<Option<PathBuf>, config::ConfigError> {
        match std::fs::canonicalize(&self.path) {
            Ok(path) => Ok(Some(path)),
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound
                    && std::fs::symlink_metadata(&self.path).is_err() =>
            {
                Ok(None)
            }
            Err(e) => Err(config::ConfigError::Message(format!(
                "cannot resolve config file {}: {e}",
                self.path.display()
            ))),
        }
    }
}

impl Source for ConfigFile {
    fn clone_into_box(&self) -> Box<dyn Source + Send + Sync> {
        Box::new(self.clone())
    }

    fn collect(&self) -> Result<Map<String, Value>, config::ConfigError> {
        let Some(canonical) = self.canonical()? else {
            return Ok(Map::new());
        };
        let origin = canonical.display().to_string();
        let raw = config::File::from(canonical.as_path())
            .format(config::FileFormat::Toml)
            .collect()?;
        let mut table = normalise_table(raw, &origin)?;
        if let Some(base) = canonical.parent() {
            anchor_paths(&mut table, base);
        }
        Ok(table)
    }
}

/// Rewrites every key in `table`, recursively, from `snake_case` to kebab-case.
///
/// # Errors
///
/// Returns an error naming `origin` and both spellings when two keys in one
/// table normalise to the same name.
fn normalise_table(
    table: Map<String, Value>,
    origin: &str,
) -> Result<Map<String, Value>, config::ConfigError> {
    let mut out = Map::new();
    let mut spelled: Map<String, String> = Map::new();
    #[expect(
        clippy::iter_over_hash_type,
        reason = "each key is inserted into `out` regardless of iteration order"
    )]
    for (key, value) in table {
        let kebab = key.replace('_', "-");
        if let Some(first) = spelled.get(&kebab) {
            let (a, b) = if *first < key {
                (first.as_str(), key.as_str())
            } else {
                (key.as_str(), first.as_str())
            };
            return Err(config::ConfigError::Message(format!(
                "{origin}: `{a}` and `{b}` name the same key; keep one"
            )));
        }
        out.insert(kebab.clone(), normalise_value(value, origin)?);
        spelled.insert(kebab, key);
    }
    Ok(out)
}

/// Normalises the keys of any table nested in `value`, tagging it with `origin`.
fn normalise_value(value: Value, origin: &str) -> Result<Value, config::ConfigError> {
    let kind = match value.kind {
        ValueKind::Table(table) => ValueKind::Table(normalise_table(table, origin)?),
        ValueKind::Array(items) => ValueKind::Array(
            items
                .into_iter()
                .map(|item| normalise_value(item, origin))
                .collect::<Result<_, _>>()?,
        ),
        other @ (ValueKind::Nil
        | ValueKind::Boolean(_)
        | ValueKind::I64(_)
        | ValueKind::I128(_)
        | ValueKind::U64(_)
        | ValueKind::U128(_)
        | ValueKind::Float(_)
        | ValueKind::String(_)) => other,
    };
    Ok(Value::new(Some(&origin.to_owned()), kind))
}

/// Resolves the relative values of every [`PATH_KEYS`] entry against `base`.
fn anchor_paths(table: &mut Map<String, Value>, base: &Path) {
    for (section, key) in PATH_KEYS {
        let Some(ValueKind::Table(inner)) = table.get_mut(*section).map(|v| &mut v.kind) else {
            continue;
        };
        let Some(value) = inner.get_mut(*key) else {
            continue;
        };
        match &mut value.kind {
            ValueKind::String(raw) => *raw = anchor(raw, base),
            ValueKind::Array(items) => {
                for item in items {
                    if let ValueKind::String(raw) = &mut item.kind {
                        *raw = anchor(raw, base);
                    }
                }
            }
            ValueKind::Nil
            | ValueKind::Boolean(_)
            | ValueKind::I64(_)
            | ValueKind::I128(_)
            | ValueKind::U64(_)
            | ValueKind::U128(_)
            | ValueKind::Float(_)
            | ValueKind::Table(_) => {}
        }
    }
}

/// Joins a relative `raw` onto `base`.
///
/// Absolute values, home-relative values (expanded later by [`expand_home`])
/// and empty values pass through unchanged. Validation rejects an empty
/// `db.path`; the other path keys keep an empty value as given.
/// A `~user` value is relative like any other.
fn anchor(raw: &str, base: &Path) -> String {
    if raw.is_empty() || home_relative(raw).is_some() || Path::new(raw).is_absolute() {
        return raw.to_owned();
    }
    base.join(raw).to_string_lossy().into_owned()
}

/// Resolves a path value read from a file whose canonical directory is `base`.
///
/// This is the resolution [`ConfigFile`] and settings validation apply
/// together: anchor a relative value, then expand a leading `~`.
///
/// # Arguments
///
/// * `raw` - The value as written in the file.
/// * `base` - The directory of the file's canonical path.
///
/// # Returns
///
/// The path a load of that file yields for `raw`.
pub(crate) fn resolve_from_file(raw: &str, base: &Path) -> PathBuf {
    expand_home(&anchor(raw, base))
}

/// Returns the part of `raw` after a leading `~`, `~/` or, on Windows, `~\`.
///
/// # Arguments
///
/// * `raw` - A path value from any config layer.
///
/// # Returns
///
/// The remainder, empty for a bare `~`; `None` when `raw` is not
/// home-relative.
fn home_relative(raw: &str) -> Option<&str> {
    if raw == "~" {
        return Some("");
    }
    raw.strip_prefix("~/")
        .or_else(|| raw.strip_prefix("~\\").filter(|_| cfg!(windows)))
}

/// Expands a leading `~`, `~/` or, on Windows, `~\` to the home directory.
///
/// `~user` forms are left alone.
///
/// # Arguments
///
/// * `raw` - A path value from any config layer.
///
/// # Returns
///
/// The expanded path, or `raw` unchanged.
pub(crate) fn expand_home(raw: &str) -> PathBuf {
    if let Some(rest) = home_relative(raw)
        && let Some(dirs) = directories::BaseDirs::new()
    {
        let home = dirs.home_dir();
        return if rest.is_empty() {
            home.to_owned()
        } else {
            home.join(rest)
        };
    }
    PathBuf::from(raw)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::Path;

    use pretty_assertions::assert_eq;

    use super::*;

    /// Writes `text` to `dir/name` and returns the path.
    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).expect("write config");
        path
    }

    /// Returns the string at `table.key` in a collected map.
    fn string_at(map: &Map<String, Value>, table: &str, key: &str) -> String {
        let ValueKind::Table(inner) = &map.get(table).expect("table present").kind else {
            panic!("{table} is not a table");
        };
        inner
            .get(key)
            .expect("key present")
            .clone()
            .into_string()
            .expect("string value")
    }

    #[test]
    fn missing_file_contributes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let map = ConfigFile::new(dir.path().join("absent.toml"))
            .collect()
            .expect("collect");
        assert!(map.is_empty());
    }

    #[test]
    fn snake_case_keys_become_kebab_case() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(dir.path(), "c.toml", "[backup]\nretain_count = 3\n");
        let map = ConfigFile::new(path).collect().expect("collect");
        let ValueKind::Table(backup) = &map.get("backup").expect("backup present").kind else {
            panic!("backup is not a table");
        };
        assert!(backup.contains_key("retain-count"));
        assert!(!backup.contains_key("retain_count"));
    }

    #[test]
    fn both_spellings_in_one_file_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(
            dir.path(),
            "c.toml",
            "[backup]\nretain_count = 3\nretain-count = 4\n",
        );
        let err = ConfigFile::new(&path).collect().expect_err("collision");
        let msg = err.to_string();
        assert!(msg.contains("retain_count"), "{msg}");
        assert!(msg.contains("retain-count"), "{msg}");
        assert!(msg.contains("c.toml"), "{msg}");
    }

    #[test]
    fn relative_path_resolves_against_file_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(dir.path(), "c.toml", "[db]\npath = \"data/db.sqlite\"\n");
        let map = ConfigFile::new(path).collect().expect("collect");
        let base = std::fs::canonicalize(dir.path()).expect("canonical");
        assert_eq!(
            string_at(&map, "db", "path"),
            base.join("data/db.sqlite").to_string_lossy()
        );
    }

    #[test]
    fn every_path_key_is_anchored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = std::fs::canonicalize(dir.path()).expect("canonical");
        let abs_p2 = base.join("abs/p2");
        let path = write(
            dir.path(),
            "c.toml",
            &format!(
                "[backup]\ndir = \"bk\"\n[import]\ndocuments_root = \"docs\"\n[plugins]\ndirs = [\"p1\", {:?}]\n",
                abs_p2.to_string_lossy()
            ),
        );
        let map = ConfigFile::new(path).collect().expect("collect");
        assert_eq!(
            string_at(&map, "backup", "dir"),
            base.join("bk").to_string_lossy()
        );
        assert_eq!(
            string_at(&map, "import", "documents-root"),
            base.join("docs").to_string_lossy()
        );
        let ValueKind::Table(plugins) = &map.get("plugins").expect("plugins present").kind else {
            panic!("plugins is not a table");
        };
        let dirs: Vec<String> = plugins
            .get("dirs")
            .expect("dirs present")
            .clone()
            .into_array()
            .expect("array")
            .into_iter()
            .map(|v| v.into_string().expect("string"))
            .collect();
        assert_eq!(
            dirs,
            vec![
                base.join("p1").to_string_lossy().into_owned(),
                abs_p2.to_string_lossy().into_owned()
            ]
        );
    }

    #[test]
    fn absolute_tilde_and_empty_values_are_left_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = std::fs::canonicalize(dir.path()).expect("canonical");
        let abs_db = base.join("abs/db.sqlite");
        for raw in [
            abs_db.to_string_lossy().into_owned(),
            "~".to_owned(),
            "~/db.sqlite".to_owned(),
            String::new(),
        ] {
            let path = write(dir.path(), "c.toml", &format!("[db]\npath = {raw:?}\n"));
            let map = ConfigFile::new(path).collect().expect("collect");
            assert_eq!(string_at(&map, "db", "path"), raw);
        }
    }

    #[test]
    fn tilde_user_value_is_anchored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(dir.path(), "c.toml", "[db]\npath = \"~user/db.sqlite\"\n");
        let map = ConfigFile::new(path).collect().expect("collect");
        let base = std::fs::canonicalize(dir.path()).expect("canonical");
        assert_eq!(
            string_at(&map, "db", "path"),
            base.join("~user/db.sqlite").to_string_lossy()
        );
    }

    #[cfg(windows)]
    #[test]
    fn backslash_tilde_value_is_left_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(dir.path(), "c.toml", "[db]\npath = '~\\db.sqlite'\n");
        let map = ConfigFile::new(path).collect().expect("collect");
        assert_eq!(string_at(&map, "db", "path"), "~\\db.sqlite");
    }

    #[test]
    fn non_path_keys_are_not_anchored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(dir.path(), "c.toml", "[cli]\nlog = \"info\"\n");
        let map = ConfigFile::new(path).collect().expect("collect");
        assert_eq!(string_at(&map, "cli", "log"), "info");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_file_resolves_against_target_directory() {
        let target_dir = tempfile::tempdir().expect("tempdir");
        let link_dir = tempfile::tempdir().expect("tempdir");
        let target = write(
            target_dir.path(),
            "real.toml",
            "[db]\npath = \"db.sqlite\"\n",
        );
        let link = link_dir.path().join("config.toml");
        symlink(&target, &link).expect("symlink");

        let map = ConfigFile::new(link).collect().expect("collect");
        let base = std::fs::canonicalize(target_dir.path()).expect("canonical");
        assert_eq!(
            string_at(&map, "db", "path"),
            base.join("db.sqlite").to_string_lossy()
        );
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let link = dir.path().join("config.toml");
        symlink(dir.path().join("gone.toml"), &link).expect("symlink");
        let err = ConfigFile::new(&link).collect().expect_err("dangling");
        assert!(err.to_string().contains("config.toml"), "{err}");
    }

    #[rstest::rstest]
    #[case("~", "")]
    #[case("~/finance/db.sqlite", "finance/db.sqlite")]
    fn expand_home_joins_home(#[case] raw: &str, #[case] rest: &str) {
        let home = directories::BaseDirs::new()
            .expect("home")
            .home_dir()
            .to_owned();
        let expected = if rest.is_empty() {
            home
        } else {
            home.join(rest)
        };
        assert_eq!(expand_home(raw), expected);
    }

    #[cfg(windows)]
    #[test]
    fn expand_home_accepts_backslash() {
        let home = directories::BaseDirs::new()
            .expect("home")
            .home_dir()
            .to_owned();
        assert_eq!(
            expand_home("~\\finance\\db.sqlite"),
            home.join("finance\\db.sqlite")
        );
    }

    #[rstest::rstest]
    #[case("/abs/db.sqlite")]
    #[case("rel/db.sqlite")]
    #[case("~user/db.sqlite")]
    fn expand_home_leaves_other_values(#[case] raw: &str) {
        assert_eq!(expand_home(raw), PathBuf::from(raw));
    }
}
