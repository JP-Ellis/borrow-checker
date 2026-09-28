//! Runs `borrow-checker-<name>` for a subcommand clap does not know.

use std::env::consts::EXE_SUFFIX;
use std::ffi::OsStr;
use std::ffi::OsString;
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

    let mut file = OsString::from("borrow-checker-");
    file.push(name);
    file.push(EXE_SUFFIX);
    exe_dir
        .into_iter()
        .map(Path::to_path_buf)
        .chain(path_var.into_iter().flat_map(std::env::split_paths))
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
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
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
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
    use std::ffi::OsStr;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::resolve;

    fn touch_exe(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let p = dir.join(name);
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
