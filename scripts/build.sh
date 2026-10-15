#!/usr/bin/env bash

set -euo pipefail

TARGETS=(
  "x86_64-unknown-linux-gnu"
  "aarch64-unknown-linux-gnu"
)
OUTPUT="dist"

for target in "${TARGETS[@]}"; do
  rustup target add "${target}" >/dev/null

  echo "==> Building ${target}"
  uv run --group build maturin build --release --zig --target ${target}
done

echo
echo "Wheels in dist/:"
ls -1 dist/*.whl
