# Fuzzing

Coverage-guided fuzzing of the entry points that take untrusted bytes, with
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer). This crate is not part of the
stable build or of CI: it needs a nightly toolchain.

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run bundle_open -- -max_len=16384 -timeout=10
```

| Target | Entry point | Input |
|---|---|---|
| `bundle_open` | `Bundle::new`, `read_entry`, `read_entry_range`, then the serialized files inside | raw bytes |
| `bundle_info` | the same | `[mode][len u16 BE][block info][blocks]` inside a valid header (mode bit 0: block info stored, else LZ4) |
| `serialized_parse` | `SerializedFile::parse`, `raw`, `read` (struct, `IgnoredAny`, `serde_json::Value`) | raw bytes |
| `serialized_tree` | the type tree blob reader and the deserializer | `[mode][len u16 BE][tree blob][object data]` inside a valid file |
| `de_tree` | `de::from_slice` with trees built through `TypeTreeNode::new` | a generated tree, then the data |

Besides panics and overflows, every target fails when an iteration holds more live memory at once than
its input justifies (`unity_bundle_fuzz::bounded`: 600 bytes per input byte plus 8 MiB; LZ4 can
stand for 255 bytes per stored byte).

The seed corpora in `fuzz/corpus/` are synthetic, built by `cargo run --manifest-path fuzz/Cargo.toml --bin gen_seeds`
with the writers of `tests/common`; no game data is used. `fuzz/corpus/<target>/crash-*` files are the minimised
inputs of the bugs that were fixed (each one also has a regular test).
