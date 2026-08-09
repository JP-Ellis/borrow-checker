//! Filesystem-facing loader: reads a ledger and expands its includes.
//!
//! The parser in [`crate::parser`] is deliberately pure. Everything that
//! touches the filesystem lives here, so include resolution, path handling and
//! recursion can be tested independently of the grammar.

use std::collections::BTreeMap;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use bc_sdk::ImportError;

use crate::ast::Directive;
use crate::parser::parse;

/// Maximum include nesting depth.
///
/// A backstop, not a design limit: real ledgers nest two or three deep. It
/// also catches a symlinked cycle, which [`normalise`] cannot see because it
/// resolves paths lexically rather than through the filesystem.
const MAX_DEPTH: usize = 64;

/// A directive together with the file it was read from.
///
/// The path is shared rather than cloned: one file contributes many
/// directives, and each needs its own source location.
#[derive(Debug)]
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

/// Bookkeeping carried through a recursive expansion.
#[derive(Debug, Default)]
struct State {
    /// Normalised paths currently being expanded, outermost first.
    ///
    /// A path reappearing here is a true cycle. Its length is the depth.
    stack: Vec<PathBuf>,
    /// Every normalised path already expanded, mapped to the site that first
    /// pulled it in, so a second inclusion can name both.
    seen: BTreeMap<PathBuf, String>,
    /// How many directives the parser produced across every file visited,
    /// including includes and unrecognised keywords.
    directives_seen: usize,
}

/// The `include` directive that pulled a file in.
///
/// Both halves are needed for diagnostics: the site locates the directive, and
/// the literal text is what the user can actually grep for in their ledger.
#[derive(Debug, Clone, Copy)]
struct Origin<'a> {
    /// Where the include was written, as `file:line`.
    site: &'a str,
    /// The path text exactly as it appears in the ledger.
    literal: &'a str,
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
    let mut state = State::default();
    let root_path = normalise(Path::new(root));
    expand(&root_path, None, &mut state, &mut loaded)?;

    let transactions = loaded
        .directives
        .iter()
        .filter(|sourced| matches!(sourced.directive, Directive::Transaction(_)))
        .count();
    if transactions == 0 {
        loaded.warnings.push(format!(
            "{}: visited {} file(s), saw {} directive(s) and found 0 transactions",
            root_path.display(),
            state.seen.len(),
            state.directives_seen
        ));
    }

    Ok(loaded)
}

/// Reads one file, appends its directives, and recurses into its includes.
///
/// # Arguments
///
/// * `path` - The file to read, already lexically normalised.
/// * `origin` - The `include` that pulled this file in, or `None` for the root
///   file.
/// * `state` - Cycle, depth and duplicate bookkeeping.
/// * `loaded` - Accumulator for directives and warnings.
///
/// # Errors
///
/// As [`load`], plus [`ImportError::BadValue`] if the ledger includes itself
/// cyclically or nests deeper than [`MAX_DEPTH`].
fn expand(
    path: &Path,
    origin: Option<Origin<'_>>,
    state: &mut State,
    loaded: &mut Loaded,
) -> Result<(), ImportError> {
    let display: Rc<str> = Rc::from(path.display().to_string());
    let key = path.to_path_buf();

    if state.stack.contains(&key) {
        let mut chain: Vec<String> = state
            .stack
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        chain.push(display.to_string());
        return Err(ImportError::BadValue {
            field: "source_file".to_owned(),
            detail: format!("include cycle: {}", chain.join(" -> ")),
        });
    }

    if state.stack.len() >= MAX_DEPTH {
        let head = state
            .stack
            .first()
            .map_or_else(|| display.to_string(), |p| p.display().to_string());
        return Err(ImportError::BadValue {
            field: "source_file".to_owned(),
            detail: format!(
                "includes nested too deeply (limit {MAX_DEPTH}) at {display}, starting from {head}"
            ),
        });
    }

    if let Some(first) = state.seen.get(&key) {
        if let Some(Origin { site, .. }) = origin {
            loaded.warnings.push(format!(
                "{display} is included twice, first from {first} and again from {site}; \
                 expanding it once"
            ));
        }
        return Ok(());
    }
    state.seen.insert(
        key.clone(),
        origin.map_or("the import profile", |o| o.site).to_owned(),
    );

    let bytes = std::fs::read(path).map_err(|e| ImportError::BadValue {
        field: "source_file".to_owned(),
        detail: match origin {
            Some(Origin { site, literal }) => {
                format!("include \"{literal}\" at {site} cannot read {display}: {e}")
            }
            None => format!("cannot read {display}: {e}"),
        },
    })?;
    let text = core::str::from_utf8(&bytes)
        .map_err(|e| ImportError::Parse(format!("{display} is not valid UTF-8: {e}")))?;

    let directives = parse(text).map_err(|e| ImportError::Parse(format!("{display}: {e}")))?;
    let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();

    state.stack.push(key);
    state.directives_seen = state.directives_seen.saturating_add(directives.len());
    for directive in directives {
        match directive {
            Directive::Include { path: rel, line } => {
                let site = format!("{display}:{line}");
                let target = normalise(&parent.join(&rel));
                let origin = Origin {
                    site: &site,
                    literal: &rel,
                };
                expand(&target, Some(origin), state, loaded)?;
            }
            Directive::Unknown { keyword, line } => {
                loaded.warnings.push(format!(
                    "{display}:{line}: ignoring unrecognised directive '{keyword}'"
                ));
            }
            other => loaded.directives.push(Sourced {
                file: Rc::clone(&display),
                directive: other,
            }),
        }
    }
    state.stack.pop();

    Ok(())
}

/// Folds `.` and `..` out of a path without touching the filesystem.
///
/// `std::fs::canonicalize` is avoided deliberately: symlink resolution is
/// unreliable under a wasip2 preopen, and this path is only ever used for
/// opening files and for comparing them.
///
/// A `..` that cannot be cancelled against a preceding named component is
/// kept, so an escaping path stays distinct from the file it would otherwise
/// be folded onto — the normalised path doubles as the cycle and diamond key.
/// Rooted paths clamp at the root instead, as POSIX resolution does.
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
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
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

    #[test]
    fn self_inclusion_errors_with_the_chain() {
        let dir = fixture(
            "cycle",
            &[
                ("main.bean", "include \"fy2020.bean\"\n"),
                ("fy2020.bean", "include \"main.bean\"\n"),
            ],
        );
        let root = dir.join("main.bean");
        let err = load(root.to_str().expect("utf8")).expect_err("a cycle must fail");
        let message = err.to_string();
        assert!(
            message.contains("cycle"),
            "the error says what went wrong: {message}"
        );
        assert!(
            message.contains("main.bean") && message.contains("fy2020.bean"),
            "the error prints the chain: {message}"
        );
    }

    #[test]
    fn diamond_include_expands_once_and_warns() {
        // `shared.bean` is reachable via two branches. Expanding it twice
        // would silently double-import every transaction in it.
        let dir = fixture(
            "diamond",
            &[
                ("main.bean", "include \"a.bean\"\ninclude \"b.bean\"\n"),
                ("a.bean", "include \"shared.bean\"\n"),
                ("b.bean", "include \"shared.bean\"\n"),
                ("shared.bean", &tx("only once")),
            ],
        );
        let root = dir.join("main.bean");
        let loaded = load(root.to_str().expect("utf8")).expect("a diamond is not an error");
        assert_eq!(
            transactions(&loaded)
                .into_iter()
                .map(|(_, narration)| narration)
                .collect::<Vec<_>>(),
            vec!["only once".to_owned()],
            "the shared file is expanded exactly once"
        );
        assert_eq!(loaded.warnings.len(), 1);
        let warning = loaded.warnings.first().expect("one warning");
        assert!(
            warning.contains("shared.bean")
                && warning.contains("a.bean")
                && warning.contains("b.bean"),
            "the warning names the file and both including sites: {warning}"
        );
    }

    #[test]
    fn excessive_nesting_errors() {
        let mut files: Vec<(String, String)> = Vec::new();
        files.push(("main.bean".to_owned(), "include \"d0.bean\"\n".to_owned()));
        for depth in 0..70_usize {
            files.push((
                format!("d{depth}.bean"),
                format!("include \"d{}.bean\"\n", depth.saturating_add(1)),
            ));
        }
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(name, contents)| (name.as_str(), contents.as_str()))
            .collect();
        let dir = fixture("deep", &refs);
        let root = dir.join("main.bean");
        let err = load(root.to_str().expect("utf8")).expect_err("excessive nesting must fail");
        let message = err.to_string();
        assert!(
            message.contains("nested too deeply"),
            "the error explains the limit: {message}"
        );
        assert!(
            message.contains("main.bean"),
            "the error names the chain head, not just where it tripped: {message}"
        );
    }

    #[test]
    fn unknown_directive_produces_a_warning_naming_file_and_line() {
        let dir = fixture(
            "unknown",
            &[("main.bean", "; header\nfrobnicate whatever\n")],
        );
        let root = dir.join("main.bean");
        let loaded = load(root.to_str().expect("utf8")).expect("an unknown directive is not fatal");
        let warning = loaded
            .warnings
            .iter()
            .find(|w| w.contains("frobnicate"))
            .expect("the unknown keyword is warned about");
        assert!(
            warning.contains("main.bean:2"),
            "the warning names file and line: {warning}"
        );
    }

    #[test]
    fn zero_transactions_warns_with_the_file_count() {
        // A genuinely empty ledger is legitimate and must not fail. But
        // "visited 3 files, found 0 transactions" reads very differently from
        // "visited 1 file", and that difference is the point of #401.
        let dir = fixture(
            "empty",
            &[
                ("main.bean", "include \"a.bean\"\ninclude \"b.bean\"\n"),
                ("a.bean", "2025-01-01 open Assets:Bank AUD\n"),
                ("b.bean", "option \"title\" \"Household\"\n"),
            ],
        );
        let root = dir.join("main.bean");
        let loaded = load(root.to_str().expect("utf8")).expect("an empty ledger is not an error");
        let warning = loaded
            .warnings
            .iter()
            .find(|w| w.contains("0 transactions"))
            .expect("an empty result warns");
        assert!(
            warning.contains("3 file(s)"),
            "the warning counts files visited: {warning}"
        );
        assert!(
            warning.contains("4 directive(s)"),
            "the warning counts directives seen, so 'parsed nothing' and \
             'parsed plenty, none of them transactions' read differently: {warning}"
        );
        assert!(
            warning.contains(root.to_str().expect("utf8")),
            "the warning names the root ledger, so a user importing several \
             profiles knows which one warned: {warning}"
        );
    }

    #[test]
    fn normalise_folds_dot_and_parent_components() {
        // A `..` that escapes must survive folding: the normalised path is the
        // cycle and diamond key, so collapsing `../../shared.bean` onto
        // `shared.bean` would misreport two different files as one and skip
        // the second as a diamond.
        let cases = [
            ("ledger/2025-01.bean", "ledger/2025-01.bean"),
            ("./ledger/./2025-01.bean", "ledger/2025-01.bean"),
            ("ledger/sub/../2025-01.bean", "ledger/2025-01.bean"),
            ("ledger/../2025-01.bean", "2025-01.bean"),
            ("../x.bean", "../x.bean"),
            ("../../x.bean", "../../x.bean"),
            ("ledger/../../x.bean", "../x.bean"),
            ("a/b/../../../../c", "../../c"),
            ("ledger/../../../../etc/passwd", "../../../etc/passwd"),
            ("/a/b/../c", "/a/c"),
            ("/x/../../../etc", "/etc"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                normalise(Path::new(input)),
                PathBuf::from(expected),
                "normalising {input}"
            );
        }
    }

    #[test]
    fn unreadable_include_error_quotes_the_literal_include_text() {
        // The user can only grep for what they typed, so the diagnostic must
        // carry the literal text as well as the path it resolved to.
        let dir = fixture(
            "literal",
            &[("ledger/main.bean", "include \"../../nope/fy2019.bean\"\n")],
        );
        let root = dir.join("ledger").join("main.bean");
        let err = load(root.to_str().expect("utf8")).expect_err("missing include must fail");
        let message = err.to_string();
        assert!(
            message.contains("include \"../../nope/fy2019.bean\""),
            "quotes the literal include text verbatim: {message}"
        );
        assert!(
            message.contains("main.bean:1"),
            "names the including file and its line: {message}"
        );
    }

    #[test]
    fn escaping_include_stays_distinct_from_a_same_named_sibling() {
        // Regression for #401. `normalise()` folds `..` lexically, and it is
        // the key used for both diamond and cycle detection. Before 1df8da7,
        // consecutive `..` past what a preceding named component could
        // cancel folded away in pairs: an include with two unmatched `..`
        // beyond its real nesting silently normalised to the same *bare*
        // filename as a shallower, unrelated sibling — so the escaping
        // include was misclassified as an already-seen diamond and its file
        // was never read, without any error.
        //
        // The include's relative-path arithmetic only reproduces this
        // against a *relative* root: joined onto an absolute temp-dir
        // prefix, folding always bottoms out at the real filesystem root,
        // where old and fixed code agree. So this test `chdir`s into a
        // directory nested inside the fixture and loads a relative root,
        // matching how the bug actually manifested in a real repository
        // tree. `cargo-nextest` runs each test in its own process, so
        // mutating the process's working directory here cannot race another
        // test.
        let dir = fixture(
            "escape",
            &[
                (
                    "w/x/top/ledger/main.bean",
                    "include \"../../shared.bean\"\ninclude \"../../../../shared.bean\"\n",
                ),
                ("w/x/shared.bean", &tx("sibling")),
                ("shared.bean", &tx("escaped")),
            ],
        );
        let original_dir = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(dir.join("w").join("x")).expect("chdir into fixture");
        let loaded = load("top/ledger/main.bean");
        std::env::set_current_dir(original_dir).expect("restore original dir");
        let loaded = loaded.expect("load");

        let narrations: Vec<String> = transactions(&loaded)
            .into_iter()
            .map(|(_, narration)| narration)
            .collect();
        assert_eq!(
            narrations,
            vec!["sibling".to_owned(), "escaped".to_owned()],
            "the doubly-escaping include must reach the distinct file two \
             real directories up, not be folded onto the sibling already \
             read via the singly-escaping include: {narrations:?}"
        );
        assert!(
            loaded
                .warnings
                .iter()
                .all(|warning| !warning.contains("included twice")),
            "two genuinely different files must not be reported as a \
             diamond: {:?}",
            loaded.warnings
        );
    }

    #[test]
    fn a_ledger_with_transactions_does_not_warn_about_emptiness() {
        let dir = fixture("nonempty", &[("main.bean", &tx("something"))]);
        let root = dir.join("main.bean");
        let loaded = load(root.to_str().expect("utf8")).expect("load");
        assert_eq!(loaded.warnings, Vec::<String>::new());
    }
}
