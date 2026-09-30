//! Runs `borrow-checker-<name>` for a subcommand clap does not know.

use std::collections::BTreeSet;
use std::env::consts::EXE_SUFFIX;
use std::ffi::OsStr;
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(unix)]
use std::os::unix::process::CommandExt as _;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

/// Finds `borrow-checker-<name>`: beside the running CLI first, then on `PATH`.
///
/// # Arguments
///
/// * `name` - The subcommand name.
/// * `exe_dir` - Directory of the running CLI.
/// * `path_var` - The `PATH` value.
///
/// # Returns
///
/// The binary's path, or `None` — including when `name` is not a single
/// plain path component, which would otherwise let it escape `exe_dir` and
/// every `path_var` directory (e.g. `../../usr/bin/id`).
pub(crate) fn resolve(
    name: &OsStr,
    exe_dir: Option<&Path>,
    path_var: Option<&OsStr>,
) -> Option<PathBuf> {
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return None;
    }

    let mut file = OsString::from(PREFIX);
    file.push(name);
    file.push(EXE_SUFFIX);
    search_dirs(exe_dir, path_var)
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

/// Lists the external subcommands [`resolve`] can find, sorted and without
/// duplicates.
///
/// # Arguments
///
/// * `exe_dir` - Directory of the running CLI.
/// * `path_var` - The `PATH` value.
///
/// # Returns
///
/// The `<name>` of every executable `borrow-checker-<name>` beside the CLI
/// or on `PATH`.
pub(crate) fn discover(exe_dir: Option<&Path>, path_var: Option<&OsStr>) -> Vec<String> {
    let names: BTreeSet<String> = search_dirs(exe_dir, path_var)
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file = entry.file_name().into_string().ok()?;
            let name = file.strip_prefix(PREFIX)?.strip_suffix(EXE_SUFFIX)?;
            (!name.is_empty() && is_executable(&entry.path())).then(|| name.to_owned())
        })
        .collect();
    names.into_iter().collect()
}

/// Directory of the running CLI, searched before `PATH`.
pub(crate) fn exe_dir() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.to_path_buf())
}

/// The file-name prefix of every external subcommand.
const PREFIX: &str = "borrow-checker-";

/// `exe_dir`, then each directory in `path_var`.
fn search_dirs(exe_dir: Option<&Path>, path_var: Option<&OsStr>) -> impl Iterator<Item = PathBuf> {
    exe_dir
        .into_iter()
        .map(Path::to_path_buf)
        .chain(path_var.into_iter().flat_map(std::env::split_paths))
}

/// Whether `path` is a file the CLI could run. On Unix this excludes
/// non-executable files, such as the `.d` dependency files Cargo writes
/// beside its binaries.
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Replaces this process with the external subcommand, or exits with an error.
///
/// A global `--db-path` given before the subcommand reaches the child as
/// `BC_DB__PATH`, which every BorrowChecker binary reads.
///
/// # Arguments
///
/// * `args` - The subcommand name followed by its arguments.
/// * `db_path` - The global `--db-path`, if given.
#[expect(clippy::print_stderr, clippy::exit, reason = "CLI entry point")]
pub(crate) fn run(args: Vec<OsString>, db_path: Option<&Path>) -> ! {
    let mut iter = args.into_iter();
    let name = iter.next().unwrap_or_default();
    let exe_dir = exe_dir();
    let Some(bin) = resolve(
        &name,
        exe_dir.as_deref(),
        std::env::var_os("PATH").as_deref(),
    ) else {
        crate::cli::external_subcommand_error(&name.to_string_lossy());
    };
    let mut cmd = std::process::Command::new(&bin);
    cmd.args(iter);
    if let Some(db) = db_path {
        cmd.env("BC_DB__PATH", db);
    }
    #[cfg(unix)]
    {
        let err = cmd.exec();
        eprintln!("error: cannot run {}: {err}", bin.display());
        std::process::exit(1);
    }
    #[cfg(not(unix))]
    match cmd.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(err) => {
            eprintln!("error: cannot run {}: {err}", bin.display());
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::env::consts::EXE_SUFFIX;
    use std::ffi::OsStr;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::discover;
    use super::resolve;

    fn touch_exe(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let p = dir.join(format!("{name}{EXE_SUFFIX}"));
        std::fs::write(&p, b"#!/bin/sh\n").expect("write");
        #[cfg(unix)]
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        p
    }

    #[test]
    fn prefers_the_binary_beside_the_cli() {
        let beside = tempfile::tempdir().expect("tempdir");
        let on_path = tempfile::tempdir().expect("tempdir");
        let want = touch_exe(beside.path(), "borrow-checker-server");
        touch_exe(on_path.path(), "borrow-checker-server");

        let got = resolve(
            OsStr::new("server"),
            Some(beside.path()),
            Some(on_path.path().as_os_str()),
        );

        assert_eq!(got, Some(want));
    }

    #[test]
    fn falls_back_to_path() {
        let beside = tempfile::tempdir().expect("tempdir");
        let on_path = tempfile::tempdir().expect("tempdir");
        let want = touch_exe(on_path.path(), "borrow-checker-server");

        let got = resolve(
            OsStr::new("server"),
            Some(beside.path()),
            Some(on_path.path().as_os_str()),
        );

        assert_eq!(got, Some(want));
    }

    #[test]
    fn unknown_name_resolves_to_none() {
        let beside = tempfile::tempdir().expect("tempdir");
        assert_eq!(resolve(OsStr::new("nope"), Some(beside.path()), None), None);
    }

    #[test]
    fn discover_lists_each_name_once_in_order() {
        let beside = tempfile::tempdir().expect("tempdir");
        let on_path = tempfile::tempdir().expect("tempdir");
        touch_exe(beside.path(), "borrow-checker-server");
        touch_exe(on_path.path(), "borrow-checker-server");
        touch_exe(on_path.path(), "borrow-checker-alpha");
        touch_exe(on_path.path(), "unrelated");

        let got = discover(Some(beside.path()), Some(on_path.path().as_os_str()));

        assert_eq!(got, ["alpha", "server"]);
    }

    #[cfg(unix)]
    #[test]
    fn discover_skips_files_that_are_not_executable() {
        let beside = tempfile::tempdir().expect("tempdir");
        std::fs::write(beside.path().join("borrow-checker-server.d"), b"").expect("write");

        assert_eq!(discover(Some(beside.path()), None), Vec::<String>::new());
    }

    #[rstest]
    #[case("../x")]
    #[case("a/b")]
    #[case("..")]
    #[case("")]
    fn a_non_plain_name_resolves_to_none(#[case] name: &str) {
        let beside = tempfile::tempdir().expect("tempdir");
        assert_eq!(resolve(OsStr::new(name), Some(beside.path()), None), None);
    }
}
