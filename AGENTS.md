# Agent guide

unity-bundle reads UnityFS bundles and type-tree objects for Rust developers working with Unity 2018.4/2019.4 assets.
It supplies nocturne-unpack's container layer and deserializes objects through serde.

## Toolchain, build and CI

- Rust 1.71 minimum (`Cargo.toml` and CI); normal CI uses stable on Linux, macOS and Windows.
- The library depends only on serde, forbids unsafe code and declares no feature switches.
- Exact CI checks, including the MSRV example build:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --examples --release
cargo build --lib --examples
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

The MSRV job runs `cargo build --lib --examples` and `cargo test` with Rust 1.71.0.
Normal tests include synthetic bundles and hostile inputs; the external oracle is separate from CI.

## Code map

- `src/bundle.rs`, `lz4.rs`: UnityFS container and LZ4 blocks.
- `src/serialized.rs`, `typetree.rs`, `de.rs`: serialized files, trees and serde deserialization.
- `src/cursor.rs`, `error.rs`, `class_id.rs`, `common_strings.rs`: checked reads and format tables.
- `tests/`, `examples/inspect.rs`: synthetic/hostile tests and inspection utility.
- `docs/FORMATS.md`, `SUBSET.md`, `tools/oracle/`: evidence, supported subset and comparisons.

## Project rules

- Support only the README's checked scope: UnityFS 6, serialized formats 17/21 and the stated Unity versions.
- Refuse unsupported versions, compression and flags with an explanatory error.
- Read only blocks covering requested byte ranges; do not expand a large `.resS` file for a small range.
- Check counts against available input before allocation and retain string/visit budgets.
- Allocate what a count or length from the input sizes through `reserve`, `grow`, `zeroed` and `copy_of` in
  `src/error.rs`, so that running out of memory is `Error::OutOfMemory` rather than an abort.
- A change to what a table costs in memory changes the table in the README's "Hostile input" and its test.
- Malformed input must return errors rather than panic.
- Preserve the README's provenance and `NOTICE`: rabex informed the design; this is not a clean-room implementation.
- Oracle/game-derived data stays outside Git; keep synthetic tests independent of an installed game.

## Public files

- Never commit game files, extracted assets, renders, decompiled code or secrets.
- The public-tree gate needs Python 3.9+ and Git; run it from the repository root:
  `python3 tools/check_public_tree.py` (checks Git's index, including staged new files).
- CI checks committed changes with `python3 tools/check_public_tree.py --range BASE..HEAD`.

## Contributing, license and AI

- Open an issue to discuss a problem or send a pull request with the change.
- Before a pull request, run the applicable CI commands above and the public-tree gate.
- License: MIT OR Apache-2.0, at your option; see `LICENSE-MIT` and `LICENSE-APACHE`.

The code was written with AI coding agents (Anthropic's Claude and OpenAI's GPT models) under the maintainer's direction.
AI-assisted contributions are welcome when their use is disclosed and the changes are reviewed.
