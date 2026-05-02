//! `cargo xtask` — custom build automation for the borrow-checker workspace.
//!
//! Run via `cargo xtask <task>`.  Available tasks:
//!
//! - `build-plugins` — compile all WASM importer plugins and stage them under
//!   `target/plugins/`.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::exit,
    clippy::expect_used,
    reason = "build tool: direct output and exit-on-error are intentional"
)]

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let task = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: cargo xtask <task>");
        eprintln!("Available tasks: build-plugins");
        std::process::exit(1);
    });

    match task.as_str() {
        "build-plugins" => build_plugins(),
        other => {
            eprintln!("Unknown task: {other}");
            eprintln!("Available tasks: build-plugins");
            std::process::exit(1);
        }
    }
}

/// The short names of the importer plugins.
///
/// Each name `N` maps to:
/// - source crate:    `plugins/{N}/Cargo.toml`
/// - sidecar manifest: `plugins/{N}/plugin.toml`
/// - wasm package:    `bc-plugin-{N}` → output `bc_plugin_{N}.wasm`
const PLUGINS: &[&str] = &["csv", "ledger", "beancount", "ofx"];

/// Build all WASM importer plugins and stage them under `target/plugins/`.
fn build_plugins() {
    // Workspace root is the parent of this crate's manifest directory.
    let workspace_root = workspace_root();

    let plugins_out = workspace_root.join("target").join("plugins");
    if !plugins_out.exists() {
        fs::create_dir_all(&plugins_out).unwrap_or_else(|err| {
            eprintln!(
                "error: failed to create output directory {}: {err}",
                plugins_out.display()
            );
            std::process::exit(1);
        });
    }

    let wasm_opt_available = wasm_opt_on_path();
    if wasm_opt_available {
        println!("info: wasm-opt found — optimising wasm binaries with -O2");
    } else {
        println!("info: wasm-opt not found — copying wasm binaries without optimisation");
    }

    for name in PLUGINS {
        build_plugin(name, &workspace_root, &plugins_out, wasm_opt_available);
    }

    println!("Done. Plugins staged in {}", plugins_out.display());
}

/// Build a single plugin, then copy (and optionally optimise) the wasm artifact
/// and its sidecar manifest into `plugins_out`.
fn build_plugin(name: &str, workspace_root: &Path, plugins_out: &Path, wasm_opt: bool) {
    let manifest_path = workspace_root.join("plugins").join(name).join("Cargo.toml");

    println!("==> Building plugin: {name}");

    // `bc-plugin-{name}` Cargo.toml lives outside the workspace, so we use
    // `--manifest-path` to point directly at the plugin's own Cargo.toml.
    // The build output lands in `plugins/{name}/target/`.
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
            "--manifest-path",
        ])
        .arg(&manifest_path)
        .current_dir(workspace_root)
        .status()
        .unwrap_or_else(|err| {
            eprintln!("error: failed to spawn cargo for plugin {name}: {err}");
            std::process::exit(1);
        });

    if !status.success() {
        eprintln!("error: cargo build failed for plugin {name}");
        std::process::exit(1);
    }

    // The wasm filename uses underscores: `bc-plugin-csv` → `bc_plugin_csv.wasm`.
    let wasm_name = format!("bc_plugin_{name}.wasm");
    let wasm_src = workspace_root
        .join("plugins")
        .join(name)
        .join("target")
        .join("wasm32-wasip2")
        .join("release")
        .join(&wasm_name);
    let wasm_dest = plugins_out.join(format!("{name}.wasm"));

    if !wasm_src.exists() {
        eprintln!(
            "error: expected wasm artifact not found: {}",
            wasm_src.display()
        );
        std::process::exit(1);
    }

    if wasm_opt {
        run_wasm_opt(&wasm_src, &wasm_dest, name);
    } else {
        fs::copy(&wasm_src, &wasm_dest).unwrap_or_else(|err| {
            eprintln!(
                "error: failed to copy wasm for plugin {name} → {}: {err}",
                wasm_dest.display()
            );
            std::process::exit(1);
        });
    }

    println!("    wasm  → {}", wasm_dest.display());

    // Copy the sidecar manifest.
    let toml_src = workspace_root
        .join("plugins")
        .join(name)
        .join("plugin.toml");
    let toml_dest = plugins_out.join(format!("{name}.toml"));

    if !toml_src.exists() {
        eprintln!("error: sidecar manifest not found: {}", toml_src.display());
        std::process::exit(1);
    }

    fs::copy(&toml_src, &toml_dest).unwrap_or_else(|err| {
        eprintln!(
            "error: failed to copy manifest for plugin {name} → {}: {err}",
            toml_dest.display()
        );
        std::process::exit(1);
    });

    println!("    toml  → {}", toml_dest.display());
}

/// Run `wasm-opt -O2 -o <dest> <src>` to optimise the wasm binary.
fn run_wasm_opt(src: &Path, dest: &Path, name: &str) {
    let status = Command::new("wasm-opt")
        .args(["-O2", "-o"])
        .arg(dest)
        .arg(src)
        .status()
        .unwrap_or_else(|err| {
            eprintln!("error: failed to spawn wasm-opt for plugin {name}: {err}");
            std::process::exit(1);
        });

    if !status.success() {
        eprintln!("error: wasm-opt failed for plugin {name}");
        std::process::exit(1);
    }
}

/// Return the workspace root (parent directory of this crate's manifest).
fn workspace_root() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .parent()
        .expect("xtask manifest dir should have a parent")
        .to_owned()
}

/// Return `true` if `wasm-opt` is available on `PATH`.
fn wasm_opt_on_path() -> bool {
    Command::new("wasm-opt")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}
