#!/usr/bin/env bash
# Build all WASM importer plugins and stage them under target/plugins/.
#
# For each plugin:
#   1. Compile to wasm32-wasip2 via cargo.
#   2. Optionally optimise with wasm-opt -O2.
#   3. Generate a sidecar manifest from [package.metadata.bc-plugin] in Cargo.toml.
#
# Requires: cargo, python3 (3.11+ for tomllib), optionally wasm-opt.
set -euo pipefail

WORKSPACE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PLUGINS_OUT="$WORKSPACE_ROOT/target/plugins"
PLUGINS=(csv ledger beancount ofx)

# Ensure the wasm32-wasip2 target is installed (idempotent).
rustup target add wasm32-wasip2

mkdir -p "$PLUGINS_OUT"

if command -v wasm-opt &>/dev/null && wasm-opt --version &>/dev/null 2>&1; then
  WASM_OPT=true
  echo "info: wasm-opt found — optimising wasm binaries with -O2"
else
  WASM_OPT=false
  echo "info: wasm-opt not found — copying wasm binaries without optimisation"
fi

for name in "${PLUGINS[@]}"; do
  echo "==> Building plugin: $name"

  manifest="$WORKSPACE_ROOT/plugins/$name/Cargo.toml"

  cargo build --release --target wasm32-wasip2 --manifest-path "$manifest"

  # Cargo uses underscores in artifact names: bc-plugin-csv → bc_plugin_csv.wasm
  wasm_src="$WORKSPACE_ROOT/plugins/$name/target/wasm32-wasip2/release/bc_plugin_${name}.wasm"
  wasm_dest="$PLUGINS_OUT/${name}.wasm"

  if [[ ! -f "$wasm_src" ]]; then
    echo "error: expected wasm artifact not found: $wasm_src" >&2
    exit 1
  fi

  if $WASM_OPT; then
    wasm-opt -O2 -o "$wasm_dest" "$wasm_src"
  else
    cp "$wasm_src" "$wasm_dest"
  fi
  echo "    wasm  → $wasm_dest"

  # Generate sidecar manifest from [package.metadata.bc-plugin] in Cargo.toml,
  # merging in the package version so PluginManifest.version is populated.
  toml_dest="$PLUGINS_OUT/${name}.toml"
  jaq '{plugin: (.package.metadata."bc-plugin" + {version: .package.version})}' "$manifest" --to toml >"$toml_dest"
  echo "    toml  → $toml_dest"
done

echo "Done. Plugins staged in $PLUGINS_OUT"
