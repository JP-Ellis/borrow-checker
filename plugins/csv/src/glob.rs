//! Filename globbing for source-file selection (single `*` wildcard).

use std::path::PathBuf;

use bc_sdk::ImportError;

/// Returns the paths in `dir` whose file name matches `pattern`, sorted
/// lexicographically for deterministic transaction order.
///
/// `dir` is interpreted in the plugin's filesystem namespace (relative to the
/// host-preopened documents root). Non-recursive.
///
/// # Errors
///
/// Returns [`ImportError`] if `dir` cannot be read.
pub fn matching_files(dir: &str, pattern: &str) -> Result<Vec<PathBuf>, ImportError> {
    let entries = std::fs::read_dir(dir).map_err(|e| ImportError::BadValue {
        field: "source_dir".to_owned(),
        detail: format!("cannot read directory {dir:?}: {e}"),
    })?;

    let mut matched: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| ImportError::Parse(format!("directory entry error: {e}")))?;
        // Skip entries that are not regular files (e.g. subdirectories); on a
        // file-type read error, skip the entry rather than fail the whole scan.
        if !entry.file_type().is_ok_and(|ft| ft.is_file()) {
            continue;
        }
        // Filenames that are not valid UTF-8 are skipped (`to_str` yields `None`).
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| glob_match(pattern, name))
        {
            matched.push(entry.path());
        }
    }
    matched.sort();
    Ok(matched)
}

/// Matches `name` against `pattern`, supporting a single `*` wildcard.
///
/// With no `*`, the match is exact. With one `*`, the text before it must be a
/// prefix and the text after it a suffix of `name`. Extra `*` beyond the first
/// are treated as literal (split occurs on the first `*` only).
#[must_use]
pub fn glob_match(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((prefix, suffix)) => {
            name.len() >= prefix.len().saturating_add(suffix.len())
                && name.starts_with(prefix)
                && name.ends_with(suffix)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use pretty_assertions::assert_eq;

    use super::glob_match;

    #[test]
    fn wildcard_matches_by_prefix_and_suffix() {
        assert!(glob_match("*.csv", "2025-06.csv"));
        assert!(glob_match("2025-*.csv", "2025-06.csv"));
        assert!(glob_match("*", "anything.txt"));
        assert!(glob_match("exact.csv", "exact.csv"));
    }

    #[test]
    fn wildcard_rejects_non_matches() {
        assert_eq!(glob_match("*.csv", "notes.txt"), false);
        assert_eq!(glob_match("2025-*.csv", "2024-06.csv"), false);
        assert_eq!(glob_match("exact.csv", "other.csv"), false);
    }

    #[test]
    fn matching_files_filters_and_sorts() {
        let dir = std::env::temp_dir().join("bc-csv-glob-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        for name in ["2025-02.csv", "2025-01.csv", "notes.txt"] {
            let mut f = std::fs::File::create(dir.join(name)).expect("create");
            f.write_all(b"x").expect("write");
        }
        std::fs::create_dir_all(dir.join("archive.csv")).expect("mkdir subdir");

        let got = super::matching_files(dir.to_str().expect("utf8"), "*.csv").expect("glob");
        let names: Vec<String> = got
            .iter()
            .map(|p| p.file_name().expect("name").to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["2025-01.csv".to_owned(), "2025-02.csv".to_owned()]
        );
    }

    #[test]
    fn matching_files_errors_on_missing_dir() {
        assert!(super::matching_files("/no/such/bc/dir", "*.csv").is_err());
    }
}
