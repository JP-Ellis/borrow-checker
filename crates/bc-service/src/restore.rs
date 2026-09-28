//! Confines a restore candidate to the backup directory in server mode.

use std::path::Path;
use std::path::PathBuf;

use bc_ipc::BcError;

/// Returns `candidate` canonicalised, if it lies inside `dir`.
///
/// The web server applies this before a restore, so a network caller can only
/// restore a file from the backup directory. Canonicalising resolves `..` and
/// symlinks before the containment check.
///
/// # Arguments
///
/// * `candidate` - Path the client asked to restore.
/// * `dir` - The backup directory.
///
/// # Returns
///
/// The canonical candidate path.
///
/// # Errors
///
/// Returns [`BcError::Validation`] if either path does not resolve or the
/// candidate lies outside `dir`.
#[inline]
pub fn confine_to_dir(candidate: &Path, dir: &Path) -> Result<PathBuf, BcError> {
    let invalid =
        |why: &str| BcError::Validation(format!("cannot restore {}: {why}", candidate.display()));
    let dir_canon = dir
        .canonicalize()
        .map_err(|_err| invalid("backup directory not found"))?;
    let resolved = candidate
        .canonicalize()
        .map_err(|_err| invalid("file not found"))?;
    if resolved.starts_with(&dir_canon) {
        Ok(resolved)
    } else {
        Err(invalid("not in the backup directory"))
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use bc_ipc::BcError;

    use super::confine_to_dir;

    #[test]
    fn a_file_inside_the_directory_is_accepted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("b.db");
        std::fs::write(&file, b"x").expect("write");

        let ok = confine_to_dir(&file, dir.path()).expect("inside");

        assert!(ok.ends_with("b.db"));
    }

    #[test]
    fn a_dot_dot_escape_is_rejected() {
        let root = tempfile::tempdir().expect("tempdir");
        let backups = root.path().join("backups");
        std::fs::create_dir_all(&backups).expect("mkdir");
        std::fs::write(root.path().join("secret.db"), b"x").expect("write");

        let err = confine_to_dir(&backups.join("../secret.db"), &backups);

        assert!(matches!(err, Err(BcError::Validation(_))), "{err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_directory_is_rejected() {
        let root = tempfile::tempdir().expect("tempdir");
        let backups = root.path().join("backups");
        std::fs::create_dir_all(&backups).expect("mkdir");
        std::fs::write(root.path().join("secret.db"), b"x").expect("write");
        symlink(root.path().join("secret.db"), backups.join("link.db")).expect("symlink");

        let err = confine_to_dir(&backups.join("link.db"), &backups);

        assert!(matches!(err, Err(BcError::Validation(_))), "{err:?}");
    }

    #[test]
    fn a_missing_file_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = confine_to_dir(&dir.path().join("nope.db"), dir.path());
        assert!(matches!(err, Err(BcError::Validation(_))), "{err:?}");
    }
}
