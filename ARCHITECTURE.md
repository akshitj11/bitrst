# Architecture

This repo starts as a small Rust workspace for building Bitcoin concepts in layers.

- `bitrst-core`: blocks, transactions, UTXO set, consensus rules, and chain validation
- `bitrst-crypto`: hashing and cryptographic helpers
- `bitrst-miner`: nonce search (imports `Target`, difficulty, and time rules from core)
- `bitrst`: CLI entry point

Current data flow:

1. Core structs serialize protocol data into Bitcoin wire bytes.
2. `bitrst-crypto` hashes those bytes with SHA-256d.
3. `bitrst-core` validates headers (PoW, Merkle root, bits, timestamps) and transactions against the UTXO set.
4. `Chain::connect_block` extends the active tip, stores orphans, or reorganizes to a heavier fork.
5. `bitrst-miner` searches nonces for headers that satisfy a `Target` from core.
