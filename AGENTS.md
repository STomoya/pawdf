## Commands

```sh
# Python
# lint
uvx ruff check --fix
# format
uvx ruff format
# type check
uvx ty check --fix
# test
uv run pytest

# Rust
# Test requires LD_LIBRARY_PATH to be set in a venv. The Makefile automates this.
# Test (cargo test --workspace)
make test
# coverage (cargo llvm-cov --show-missing-lines)
make coverage
# format (cargo fmt --check)
make format
# clippy (cargo clippy --workspace --all-targets -- -D warnings)
make clippy
```

## Scratch pad

The folder `local_scratch` is for throw away scripts, or holding reference files. This folder is gitignored.
