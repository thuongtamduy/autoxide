# Fuzzing

Fuzz targets ([cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz), libFuzzer) for the code that reads untrusted
input. A target feeds arbitrary bytes to a parser and fails if it panics, overflows or hangs. This directory is its own
Cargo workspace because libFuzzer needs a nightly compiler.

| Target          | What is fuzzed                                                                  |
| --------------- | ------------------------------------------------------------------------------- |
| `page_query`    | `limit`, `cursor`, `order`, `count` of every list endpoint (pagination cursors) |
| `tenant_hint`   | The `Host` and `Autoxide-Tenant` headers that select a tenant                   |
| `search_query`  | The search-box query language (`is:`, `-word`, comparisons, quoted phrases)     |
| `csv_read`      | CSV import files, and write-then-read round trips                                |
| `decimal_money` | Decimal amounts, money and currency codes                                        |
| `timestamp`     | RFC 3339 timestamps and clock times                                              |

## Run

```bash
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cd fuzz
cargo +nightly fuzz run page_query -- -max_total_time=60
cargo +nightly fuzz list          # all targets
```

A crash is saved under `fuzz/artifacts/<target>/`. Reproduce with `cargo +nightly fuzz run <target> <that file>`, then
copy the input into a regular unit test next to the function, fix the bug, and keep the test.
CI runs every target weekly (`.github/workflows/fuzz.yml`) and opens an issue when one fails.

## Add a target

1. Pick a function that takes untrusted input and is part of the **public** API (`pub` in `autoxide-kernel` or a
   module's `api`).
2. Add `fuzz/fuzz_targets/<name>.rs` (copy a small one) and a `[[bin]]` entry in `fuzz/Cargo.toml`.
3. Add the name to the `matrix.target` list of `.github/workflows/fuzz.yml` and to the table above.

Not yet covered: the `Idempotency-Key` header and JSON bodies are handled inside the HTTP layer (private functions); they
are exercised through the property tests of the layer today and become fuzz targets when the layer's input parsing is
exposed as a public function.
