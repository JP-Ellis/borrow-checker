//! Plugin manifest types and loader.
//!
//! Each `.wasm` plugin file ships with a sidecar `.toml` manifest that
//! declares its name, version, ABI, and host requirements.

use std::path::Path;

/// Current ABI version supported by this host.
pub const HOST_ABI_VERSION: u32 = 1;

/// Minimum ABI version that this host supports (hard floor — below this is an error).
pub const HOST_ABI_MIN: u32 = 1;

/// Minimum ABI version that is in the deprecation grace window.
///
/// Plugins whose `sdk_abi` is in the range `HOST_ABI_DEPRECATED_MIN ..< HOST_ABI_MIN`
/// are still loaded but emit a warning indicating that support will be dropped
/// in a future release. Currently this equals [`HOST_ABI_MIN`] (no grace window
/// exists yet), but the three-tier validation logic is wired up and ready.
pub const HOST_ABI_DEPRECATED_MIN: u32 = 1;

/// Errors that can occur when loading a plugin manifest.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// The sidecar `.toml` file could not be read.
    #[error("cannot read manifest at {path}: {source}")]
    Io {
        /// Path to the manifest file.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The manifest could not be parsed as TOML.
    #[error("invalid manifest TOML at {path}: {source}")]
    Parse {
        /// Path to the manifest file.
        path: String,
        /// The TOML parse error.
        #[source]
        source: toml::de::Error,
    },
    /// The plugin's ABI version is not supported by this host.
    #[error(
        "plugin '{name}' requires ABI v{sdk_abi}, host supports \
         v{HOST_ABI_DEPRECATED_MIN}–v{HOST_ABI_VERSION} \
         (deprecated floor v{HOST_ABI_DEPRECATED_MIN}, hard floor v{HOST_ABI_MIN})"
    )]
    UnsupportedAbi {
        /// The plugin name.
        name: String,
        /// The ABI version the plugin was compiled against.
        sdk_abi: u32,
    },
    /// The plugin requires a newer version of BorrowChecker.
    #[error(
        "plugin '{name}' requires BorrowChecker v{min_host} or newer, but host is v{host_version}"
    )]
    UnsupportedHost {
        /// The plugin name.
        name: String,
        /// The minimum host version required.
        min_host: String,
        /// The current host version.
        host_version: String,
    },
}

/// Parses a semantic version string into a tuple of `(major, minor, patch)`.
fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut parts = s.split('-');
    let base = parts.next()?;
    let mut nums = base.split('.');
    let major = nums.next()?.parse().ok()?;
    let minor = nums.next()?.parse().ok()?;
    let patch = nums.next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// Validates if the given `host` version is compatible with the `min` version.
fn is_compatible_host(host: &str, min: &str) -> bool {
    let host_ver = parse_version(host).unwrap_or((0, 0, 0));
    let min_ver = parse_version(min).unwrap_or((0, 0, 0));
    host_ver >= min_ver
}

/// The `[plugin]` section of a sidecar manifest `.toml`.
#[non_exhaustive]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PluginManifest {
    /// Stable plugin name (e.g. `"csv"`). Must match `Importer::name()`.
    pub name: String,
    /// Semver plugin version (informational only).
    pub version: String,
    /// Integer ABI version the plugin was compiled against.
    pub sdk_abi: u32,
    /// Minimum BorrowChecker version required (informational only).
    pub min_host: String,
}

/// Top-level TOML wrapper containing the `[plugin]` table.
#[derive(serde::Deserialize)]
struct ManifestFile {
    /// The `[plugin]` section of the manifest.
    plugin: PluginManifest,
}

impl PluginManifest {
    /// Loads and validates a manifest from a `.toml` sidecar path.
    ///
    /// # Arguments
    ///
    /// * `toml_path` - Path to the `.toml` manifest file to load.
    ///
    /// # Returns
    ///
    /// A [`PluginManifest`] if the file was successfully read, parsed, and
    /// its ABI version is supported by this host.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError`] if the file cannot be read, parsed, or
    /// if its ABI version is not supported by this host.
    #[inline]
    pub fn load(toml_path: &Path) -> Result<Self, ManifestError> {
        let text = std::fs::read_to_string(toml_path).map_err(|source| ManifestError::Io {
            path: toml_path.display().to_string(),
            source,
        })?;
        let file: ManifestFile = toml::from_str(&text).map_err(|source| ManifestError::Parse {
            path: toml_path.display().to_string(),
            source,
        })?;
        let manifest = file.plugin;

        let host_version = env!("CARGO_PKG_VERSION");
        if !is_compatible_host(host_version, &manifest.min_host) {
            return Err(ManifestError::UnsupportedHost {
                name: manifest.name.clone(),
                min_host: manifest.min_host,
                host_version: host_version.to_owned(),
            });
        }

        if manifest.sdk_abi > HOST_ABI_VERSION || manifest.sdk_abi < HOST_ABI_DEPRECATED_MIN {
            // Completely out of range — hard error.
            return Err(ManifestError::UnsupportedAbi {
                name: manifest.name,
                sdk_abi: manifest.sdk_abi,
            });
        }
        if manifest.sdk_abi < HOST_ABI_MIN {
            // In the grace window: warn but still load.
            tracing::warn!(
                plugin = %manifest.name,
                sdk_abi = manifest.sdk_abi,
                host_abi_min = HOST_ABI_MIN,
                "plugin uses a deprecated ABI version and will not be loadable in a future release"
            );
        }
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn parses_valid_manifest() {
        let toml = r#"
[plugin]
name     = "csv"
version  = "1.0.0"
sdk_abi  = 1
min_host = "0.1.0"
"#;
        let file: ManifestFile = toml::from_str(toml).expect("valid toml");
        assert_eq!(file.plugin.name, "csv");
        assert_eq!(file.plugin.sdk_abi, 1_u32);
    }

    #[test]
    fn load_rejects_unsupported_abi_above_max() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("plugin.toml");
        std::fs::write(
            &path,
            r#"[plugin]
name = "future"
version = "1.0.0"
sdk_abi = 99
min_host = "0.1.0"
"#,
        )
        .expect("write");
        let result = PluginManifest::load(&path);
        let err = result.expect_err("should fail for too-new ABI");
        assert!(matches!(err, ManifestError::UnsupportedAbi { .. }));
    }

    #[test]
    fn load_rejects_unsupported_abi_below_deprecated_min() {
        // ABI 0 is below HOST_ABI_DEPRECATED_MIN (1), so it is a hard error.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("plugin.toml");
        std::fs::write(
            &path,
            r#"[plugin]
name = "ancient"
version = "0.0.1"
sdk_abi = 0
min_host = "0.0.1"
"#,
        )
        .expect("write");
        let result = PluginManifest::load(&path);
        let err = result.expect_err("should fail for too-old ABI");
        assert!(matches!(err, ManifestError::UnsupportedAbi { .. }));
    }

    #[test]
    fn load_rejects_unsupported_host() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("plugin.toml");
        std::fs::write(
            &path,
            r#"[plugin]
name = "too_new"
version = "1.0.0"
sdk_abi = 1
min_host = "99.99.99"
"#,
        )
        .expect("write");
        let result = PluginManifest::load(&path);
        let err = result.expect_err("should fail for too-new min_host");
        assert!(matches!(err, ManifestError::UnsupportedHost { .. }));
    }
}
