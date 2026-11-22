#!/usr/bin/env bash
# Trío de calidad del DoD (ROADMAP §1). Ejecutar antes de cada commit.
set -euo pipefail

echo "== cargo fmt --check =="
cargo fmt --check

echo "== cargo clippy --all-targets --all-features --locked -- -D warnings =="
cargo clippy --all-targets --all-features --locked -- -D warnings

echo "== cargo test =="
cargo test --quiet

echo "== TODO VERDE =="
