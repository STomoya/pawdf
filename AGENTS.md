## Overview

pawdf is a PDF reading and parsing library written in pure Rust, with a Python binding. PDF creation, writing, and editing are out of scope.

## Project Structure

```text
container/  # Container files.
crates/  # Rust packages.
crates/pawdf_pyo3/  # PyO3 related codes.
crates/pawdf_*/  # Pure Rust implementations.
crates/pawdf_*/src/  # Rust codes.
crates/pawdf_*/tests/  # Rust test codes.
crates/pawdf_reader/data/  # Generated third-party data and its licenses. Regenerate with scripts/gen_reader_data.py (see README.md).
fuzz/  # cargo-fuzz targets. Outside the workspace, needs nightly. See fuzz/README.md for corpus rules.
python/  # Python packages.
python/pawdf/  # Python binding.
python/pawdf/_core.pyi  # Hand-written stubs for the native module.
python/pawdf/layout.py  # Re-exports of the layout functions and types.
python/tests/  # pytest codes.
python/validation/  # Comparison scripts against pdfium and pdfminer.six.
scripts/  # Wheel build, container test matrix, reader data generation.
pyproject.toml  # Includes linter/formatter/type checker configs.
Cargo.toml
Makefile  # Contains commands for the Rust toolchain. See the Commands section.
```

## Commands

```sh
# Python
# lint
uvx ruff check --fix
# format
uvx ruff format
# type check with pyrefly (primary)
uvx pyrefly check
# type hint coverage
uvx pyrefly coverage check
# type check with ty
uvx ty check --fix
# test
uv run pytest
# test including the validation comparison tests (skipped by plain pytest)
uv run --group validation pytest

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

# Fuzz (nightly). Targets and corpus rules are in fuzz/README.md.
cargo +nightly fuzz run <target>
```

## Architecture

- `pawdf_core`: lexer, object model, stream filters, xref, encryption (standard security handler), content-stream tokenizing, text-string decoding, diagnostics.
- `pawdf_reader`: document- and page-level API built on `pawdf_core`: content interpretation (glyphs, paths, images, clips), fonts and CMaps, annotations, AcroForm, outlines and destinations, structure tree, optional content.
- `pawdf_layout`: layout analysis on `pawdf_reader` output: words, lines, blocks (`pdfminer`, a port of pdfminer.six) and tables (`table`: tagged, ruled, borderless). One module per algorithm family.
- Dependencies flow one way: `pawdf_pyo3` → `pawdf_layout` → `pawdf_reader` → `pawdf_core`.
- Keep the binding thin. All parsing logic lives in the pure Rust crates. `pawdf_pyo3` only converts types and errors. `python/pawdf` only re-exports, provides type stubs, and adds small Pythonic conveniences (context managers, `__repr__`).
- Core APIs take bytes (`&[u8]` or an owned buffer). File I/O and memory-mapping stay at the edges (the binding or the caller).
- Load objects lazily through the xref. Do not parse the whole file up front.
- Public types own their data. Borrowing from the input is fine inside a crate but must not leak lifetimes into the public API.
- Correctness first. Optimize only when a benchmark shows the need.

## Parsing Rules

- ISO 32000-2 (PDF 2.0) is the primary reference. Where behavior is non-obvious, cite the section number in a comment.
- Be lenient: recover from malformed input (broken xref, wrong offsets, etc.) the way mature readers do.
- Report every recovery as a collected diagnostic (kind + `Location`) that the caller can inspect, including from Python. Do not add a logging dependency.
  - Kind: reuse the kind of the structure being read (`BadFont`, `BadImage`, `BadOperands`, ...). Add a new kind only for a new domain.
  - Location: the most specific available. A file offset or a decoded-stream offset if one exists, else `Location::Object` of the object holding the bad entry. `Location::File(0)` only when there is no ref.
  - An explicit null or a dangling reference means absent (§7.3.7, §7.3.10). The inherited value or default applies, with no diagnostic.
- Never panic on any input. No `unwrap`/`expect` outside tests.
- Encryption: the standard security handler is supported, with an optional password. Unsupported handlers or revisions return a clear error (`UnsupportedEncryption`). Ask before adding public-key or other security handlers.

## Rust Rules

- Every `pawdf_*` crate except `pawdf_pyo3` uses `#![forbid(unsafe_code)]`. In `pawdf_pyo3`, every `unsafe` block needs a `// SAFETY:` justification.
- Errors are typed per-crate enums. Carry byte-offset context where a position exists.
- Keep dependencies minimal. Ask before adding any new crate (Rust or Python). Already approved:
  - Rust: `miniz_oxide`, `ttf-parser`, `pyo3`, and RustCrypto `md-5`, `sha2`, `aes`, `cbc`, `rc4`, plus `stringprep`.
  - Python: the `validation` group (`pdfminer.six` pinned, `pypdfium2`). Validation only, never a runtime dependency.
- Edition 2024, latest stable toolchain. No MSRV guarantee before 1.0.
- Before 1.0, break APIs freely: no deprecation shims or back-compat code. Update callers instead.
- Write rustdoc on `pub` items. Internals get comments only for non-obvious "why".

## Python Rules

- Maintain hand-written `.pyi` stubs and `py.typed` for `pawdf._core`. Update the stubs with every binding change, and `uvx pyrefly check` and `uvx ty check` must pass. `pyrefly` is the primary source. Use `ty` as secondary and reporting status on PRs is not required.
- Write docstrings on public Python API.

## Testing

- Every bug fix adds a regression test that reproduces the failing input.
- Fixtures are small, hand-crafted PDFs or inline byte strings. Never commit large or copyrighted files.
- Parser entry points have `cargo-fuzz` targets. Add a target when adding a new parser.
- Rust tests cover parsing logic. pytest covers the binding surface, plus regression tests for the `python/validation` scripts (`test_compare_*.py`).

## Workflow

- Before declaring a task done, run all checks in the Commands section (Rust and Python) and confirm they pass.
- Never commit to `main`. Work on a feature branch and open a PR.
- Use Conventional Commits (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`).
- Stay in scope. Ask before touching anything outside the task. Never add PDF writing or editing features.

## Scratch pad

The folder `local_scratch` is for throw away scripts, or holding reference files. This folder is gitignored.
