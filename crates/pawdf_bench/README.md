# pawdf_bench

Timing harness for performance work. Not published, no dependencies beyond the workspace.

```sh
cargo build --profile profiling -p pawdf_bench   # release + debug symbols
REPS=5 target/profiling/pawdf_bench <mode> <file.pdf>
```

Each mode prints one line with the median of `REPS` runs (default 5). Pass PDFs on the
command line; never commit them.

| Mode | Measures |
|---|---|
| `open` | `Document::from_bytes` |
| `resolve` | resolving every object: cold (parse) and warm (cache hits) |
| `lex` | `parse_content` alone on pre-decoded page content streams |
| `stages` | content (fonts warm), `pdfminer::analyze`, `table::ruled`, `table::borderless` over all pages |
| `firstcontent` | content on a fresh document, font loading included |
| `flow` | `analyze` with default params vs `boxes_flow = None` (reading-order cost) |
| `cells` | borderless table and cell counts (empty, one line) with analyze vs borderless cost |
| `tagged` | `table::tagged` over all pages, and with empty content (tree-walk cost) |
| `annots` | annotations on all pages |
| `docapi` | outline, form fields, structure tree |

For an A/B, build the base commit in a `git worktree` and run both binaries alternately;
expect about ±10% run-to-run noise.

## Sampling stacks without `perf`

When `perf` is blocked (`kernel.perf_event_paranoid`), sample with gdb. gdb runs the harness
as its child, so `kernel.yama.ptrace_scope=1` is fine, and a shell loop stops it with `SIGUSR1`
to take each sample:

```sh
cat > /tmp/sample.gdb <<'GDB'
handle SIGUSR1 stop print nopass
run
while 1
  bt 1
  continue
end
GDB
(sleep 0.5; while pkill -USR1 -x pawdf_bench; do sleep 0.02; done) &
REPS=30 gdb -batch -x /tmp/sample.gdb --args target/profiling/pawdf_bench stages file.pdf \
  </dev/null 2>/dev/null | grep '^#0' | sed -E 's/^#0 +(0x[0-9a-f]+ in )?//; s/ \(.*//' \
  | sort | uniq -c | sort -rn | head -30
```

This counts the innermost frame (self time). Use `bt 30` and count every frame for inclusive
time. Raise `REPS` until the run lasts a few seconds; gdb takes about 25 samples a second.
