# Architecture

This repo starts as a small Rust workspace for building Bitcoin concepts in layers.

- `bitrst-core`: blocks, transactions, and chain state
- `bitrst-crypto`: hashing and cryptographic helpers
- `bitrst-miner`: proof-of-work target checks and nonce search
- `bitrst`: CLI entry point

Current data flow:

1. Core structs serialize protocol data into Bitcoin wire bytes.
2. `bitrst-crypto` hashes those bytes with SHA-256d.
3. `bitrst-miner` checks block header hashes against a target while searching nonces.
