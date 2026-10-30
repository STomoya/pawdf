# Fuzzing

`cargo-fuzz` targets for the parser entry points. This crate is outside the main workspace and needs nightly.

| Target     | Entry point                                       |
| ---------- | ------------------------------------------------- |
| `lexer`    | `parse_indirect`: lexer and object parser         |
| `file`     | `File::from_bytes` and decoding every stream      |
| `filters`  | stream filters                                    |
| `content`  | one page's content stream, interpreted            |
| `document` | `Document`: every page's content                  |
| `cmap`     | an embedded CMap on a Type0 font                  |
| `crypt`    | an encryption dictionary and password, then decryption of a string and a stream |
| `outline`    | the document outline                            |
| `acroform`   | AcroForm fields and widgets                     |
| `structure`  | the structure tree                              |
| `annotation` | a page's annotations                            |
| `oc`         | optional content, from marked content and annotations |
| `names`      | named destinations, from link annotations        |

```sh
cargo +nightly fuzz run document -- -max_total_time=300
```

A crash is saved to `artifacts/<target>/`. Add it as a regression test before fixing it.

## Corpus

`corpus/` and `artifacts/` are local and gitignored.

Keep each `corpus/<target>` small. If no `-max_len` is given, libFuzzer uses the size of the largest corpus input as the limit (up to 1 MB). One large real PDF therefore fills the corpus with huge inputs and slows a run by about 30×.

Do not copy real PDFs into `corpus/<target>`. Fuzz them as a separate corpus instead:

```sh
cargo +nightly fuzz run document /path/to/large_pdfs -- -max_len=4194304 -max_total_time=300
```

If a corpus has grown large, merge it into a new directory capped at 64 KiB, then replace the old one:

```sh
mkdir corpus/document_min
cargo +nightly fuzz run document corpus/document_min corpus/document -- -merge=1 -max_len=65536
```
