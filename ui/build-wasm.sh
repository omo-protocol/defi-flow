#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
wasm-pack build --target web --out-dir ui/src/wasm/pkg -- --features wasm --no-default-features
