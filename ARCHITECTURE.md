# Architecture

This repo starts as a small Rust workspace for building Bitcoin concepts in layers.

- `bitrst-core`: blocks, transactions, UTXO set, consensus rules, and chain validation
- `bitrst-crypto`: SHA-256d, HASH160, Base58Check, and ECDSA helpers
- `bitrst-script`: P2PKH script templates and stack-based script verification
- `bitrst-miner`: nonce search (imports `Target`, difficulty, and time rules from core)
- `bitrst-wallet`: secp256k1 keys, P2PKH addresses, transaction signing, and wallet UTXO tracking
- `bitrst`: CLI entry point

See [docs/CONSENSUS_STANDARDS.md](docs/CONSENSUS_STANDARDS.md) for production-quality rules applied across crates.

## Data flow

1. Core structs serialize protocol data into Bitcoin wire bytes.
2. `bitrst-crypto` hashes those bytes with SHA-256d.
3. Untrusted blocks are size-checked, then validated: PoW → Merkle → coinbase → timestamp → bits → UTXO.
4. `Chain::connect_block` extends the active tip, stores orphans (capped), or reorganizes to heavier cumulative work.
5. `ChainHandle` (`Arc<RwLock<Chain>>`) exposes thread-safe tip reads and exclusive connects for CLI / future P2P.
6. `MemoryBlockStore` implements `BlockStore` for block persistence API (disk backend later).
7. `ChainEvent` records connects, disconnects, reorgs, and orphan pool changes.
8. `bitrst-script` verifies P2PKH spends using legacy sighash and ECDSA.
9. `bitrst-wallet` watches P2PKH addresses, signs local spends, and updates balances from active-chain events.
10. `bitrst-miner` searches nonces with bounded attempts (`MAX_NONCE_ATTEMPTS`).

## Chain work

Per-block work uses Bitcoin Core `GetBlockProof`: `(~target / (target + 1)) + 1` on 256-bit little-endian targets (`Target::to_work` in `pow.rs`).
