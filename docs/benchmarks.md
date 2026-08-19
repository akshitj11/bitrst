# Benchmarks

Criterion measures SHA-256d and bounded nonce search on this machine only. Numbers from another host are not a target. Record your own run in the template below.

## Commands

Compile benchmarks without running them (CI-safe):

```bash
cargo bench --no-run
```

Run all benchmark suites:

```bash
cargo bench
```

Run a single suite or filter:

```bash
cargo bench --bench hashing
cargo bench --bench mining
cargo bench -- sha256d
cargo bench -- bounded_nonce
```

Save Criterion output to a directory (useful before pasting results):

```bash
cargo bench -- --save-baseline local
```

## Suites

| Bench file | What is measured | Setup (excluded) |
|------------|------------------|------------------|
| `benches/hashing.rs` | `sha256d` on 80-byte header bytes; `BlockHeader::hash()` (serialize + SHA-256d) | Header construction and serialization buffer |
| `benches/mining.rs` | 1,000-attempt bounded nonce search on test `bits`; easy-target search capped at 1,000 attempts | Fresh header clone and target decode per iteration (`iter_batched`) |

## Results template

Copy this block after a local run and fill in your environment. Do not commit filled numbers unless intentionally refreshing a maintainer snapshot.

```text
Date:
Host CPU:
Rust toolchain (rustc -V):
OS:

cargo bench --bench hashing
  sha256d_payload/80_byte_header_bytes     time: ______ ns  thrpt: ______ MB/s
  sha256d_header/serialize_and_hash        time: ______ ns  thrpt: ______ MB/s

cargo bench --bench mining
  bounded_nonce_search/1000_header_hashes    time: ______ ns
  bounded_nonce_search/easy_target_until_solution  time: ______ ns

Notes:
```

## Related tests

Authoritative vectors and mainnet genesis replay are covered by integration tests:

```bash
cargo test --test known_vectors
cargo test --test mainnet_genesis
```
