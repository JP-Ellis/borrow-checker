//! Filesystem-facing loader: reads a ledger and expands its includes.
//!
//! The parser in [`crate::parser`] is deliberately pure. Everything that
//! touches the filesystem lives here, so include resolution, path handling and
//! recursion can be tested independently of the grammar.

use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use bc_sdk::ImportError;

use crate::ast::Directive;
use crate::parser::parse;

/// A directive together with the file it was read from.
///
/// The path is shared rather than cloned: one file contributes many
/// directives, and each needs its own source location.
#[derive(Debug, Clone)]
pub(crate) struct Sourced {
    /// Display path of the file this directive was read from.
    pub file: Rc<str>,
    /// The directive itself.
    pub directive: Directive,
}

/// The result of expanding a ledger and everything it includes.
#[derive(Debug, Default)]
pub(crate) struct Loaded {
    /// Every directive from the root file and its includes, in document order.
    pub directives: Vec<Sourced>,
    /// Human-readable warnings about the ledger, for the caller to emit.
    ///
    /// Returned as data rather than logged here so they can be asserted in
    /// native unit tests, where `bc_sdk::warn!` compiles to a no-op.
    pub warnings: Vec<String>,
}

/// Reads `root` and recursively splices in every file it includes.
///
/// # Arguments
///
/// * `root` - Path to the root ledger file, as supplied in the import config.
///
/// # Returns
///
/// Every directive in document order, each paired with its originating file,
/// plus any warnings about the ledger.
///
/// # Errors
///
/// Returns [`ImportError::BadValue`] if a file cannot be read, naming the
/// include that referred to it, and [`ImportError::Parse`] if a file is not
/// valid UTF-8 or fails to parse.
pub(crate) fn load(root: &str) -> Result<Loaded, ImportError> {
    let mut loaded = Loaded::default();
    expand(Path::new(root), None, &mut loaded)?;
    Ok(loaded)
}

/// Reads one file, appends its directives, and recurses into its includes.
///
/// # Arguments
///
/// * `path` - The file to read.
/// * `origin` - Where this file was included from, as `file:line`, or `None`
///   for the root file.
/// * `loaded` - Accumulator for directives and warnings.
///
/// # Errors
///
/// As [`load`].
fn expand(path: &Path, origin: Option<&str>, loaded: &mut Loaded) -> Result<(), ImportError> {
    let display: Rc<str> = Rc::from(path.display().to_string());

    let bytes = std::fs::read(path).map_err(|e| ImportError::BadValue {
        field: "source_file".to_owned(),
        detail: match origin {
            Some(site) => format!("include at {site} cannot read {display}: {e}"),
            None => format!("cannot read {display}: {e}"),
        },
    })?;
    let text = core::str::from_utf8(&bytes)
        .map_err(|e| ImportError::Parse(format!("{display} is not valid UTF-8: {e}")))?;

    let directives = parse(text).map_err(|e| ImportError::Parse(format!("{display}: {e}")))?;
    let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();

    for directive in directives {
        if let Directive::Include { path: rel, line } = directive {
            let site = format!("{display}:{line}");
            let target = normalise(&parent.join(&rel));
            expand(&target, Some(&site), loaded)?;
        } else {
            loaded.directives.push(Sourced {
                file: Rc::clone(&display),
                directive,
            });
        }
    }

    Ok(())
}

/// Folds `.` and `..` out of a path without touching the filesystem.
///
/// `std::fs::canonicalize` is avoided deliberately: symlink resolution is
/// unreliable under a wasip2 preopen, and this path is only ever used for
/// opening files and for comparing them.
///
/// # Arguments
///
/// * `path` - The path to normalise.
///
/// # Returns
///
/// The lexically normalised path.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::ast::Directive;

    /// Writes `files` into a fresh directory unique to `test_name` and returns
    /// that directory. Each entry is a `(relative path, contents)` pair;
    /// intermediate directories are created as needed.
    fn fixture(test_name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bc-beancount-src-{test_name}"));
        let _ = std::fs::remove_dir_all(&dir);
        for (rel, contents) in files {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("mkdir");
            }
            std::fs::write(&path, contents).expect("write");
        }
        dir
    }

    /// Renders `load`'s output as `(file stem, description)` pairs for the
    /// transactions it found, in document order.
    fn transactions(loaded: &Loaded) -> Vec<(String, String)> {
        loaded
            .directives
            .iter()
            .filter_map(|sourced| match &sourced.directive {
                Directive::Transaction(tx) => Some((
                    Path::new(sourced.file.as_ref())
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    tx.narration.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    /// A one-line transaction body, parameterised by narration.
    fn tx(narration: &str) -> String {
        format!(
            "2025-01-15 * \"{narration}\"\n  Expenses:Food   1.00 AUD\n  Assets:Bank  -1.00 AUD\n"
        )
    }

    #[test]
    fn expands_nested_includes_in_document_order() {
        let dir = fixture(
            "nested",
            &[
                (
                    "main.bean",
                    &format!("{}include \"fy2025.bean\"\n{}", tx("before"), tx("after")),
                ),
                ("fy2025.bean", "include \"2025-01.bean\"\n"),
                ("2025-01.bean", &tx("innermost")),
            ],
        );
        let root = dir.join("main.bean");
        let loaded = load(root.to_str().expect("utf8")).expect("load");
        assert_eq!(
            transactions(&loaded),
            vec![
                ("main".to_owned(), "before".to_owned()),
                ("2025-01".to_owned(), "innermost".to_owned()),
                ("main".to_owned(), "after".to_owned()),
            ]
        );
    }

    #[test]
    fn resolves_relative_to_the_including_file_not_the_root() {
        // Two files in different directories each include "shared.bean".
        // Resolving against the root — or the process cwd — would make both
        // reach the same file.
        let dir = fixture(
            "relative",
            &[
                (
                    "main.bean",
                    "include \"a/sub.bean\"\ninclude \"b/sub.bean\"\n",
                ),
                ("a/sub.bean", "include \"shared.bean\"\n"),
                ("a/shared.bean", &tx("from a")),
                ("b/sub.bean", "include \"shared.bean\"\n"),
                ("b/shared.bean", &tx("from b")),
            ],
        );
        let root = dir.join("main.bean");
        let loaded = load(root.to_str().expect("utf8")).expect("load");
        assert_eq!(
            transactions(&loaded)
                .into_iter()
                .map(|(_, narration)| narration)
                .collect::<Vec<_>>(),
            vec!["from a".to_owned(), "from b".to_owned()]
        );
    }

    #[test]
    fn missing_include_errors_naming_file_line_and_path() {
        let dir = fixture(
            "missing",
            &[("main.bean", "; header\ninclude \"fy2019.bean\"\n")],
        );
        let root = dir.join("main.bean");
        let err = load(root.to_str().expect("utf8")).expect_err("missing include must fail");
        let message = err.to_string();
        assert!(
            message.contains("main.bean"),
            "names the including file: {message}"
        );
        assert!(
            message.contains(":2"),
            "names the include's line: {message}"
        );
        assert!(message.contains("fy2019.bean"), "names the path: {message}");
    }

    #[test]
    fn included_file_that_is_not_utf8_errors_naming_that_file() {
        let dir = fixture("badutf8", &[("main.bean", "include \"broken.bean\"\n")]);
        std::fs::write(dir.join("broken.bean"), [0xff_u8, 0xfe]).expect("write");
        let root = dir.join("main.bean");
        let err = load(root.to_str().expect("utf8")).expect_err("bad utf8 must fail");
        assert!(
            err.to_string().contains("broken.bean"),
            "the error names the included file, not the root: {err}"
        );
    }
}
