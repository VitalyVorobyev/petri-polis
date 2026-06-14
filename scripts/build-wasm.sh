#!/usr/bin/env bash
# Build the Rust sim (crates/petri-wasm) → WASM into app/src/wasm/ for Vite to import.
# Usage: bash scripts/build-wasm.sh [--release]   (default is a fast --dev build)
set -euo pipefail

PROFILE_FLAG="--dev"
for arg in "$@"; do
  [ "$arg" = "--release" ] && PROFILE_FLAG="--release"
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

wasm-pack build crates/petri-wasm \
  "$PROFILE_FLAG" \
  --target web \
  --out-dir "$ROOT/app/src/wasm" \
  --out-name petri_wasm

echo "✓ wasm built ($PROFILE_FLAG) → app/src/wasm/"
