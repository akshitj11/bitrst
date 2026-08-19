# Consensus and production standards (bitrst)

Every crate that handles **bytes from outside** the process (peers, CLI, disk) must follow these rules.

## Correctness

- Implement consensus formulas exactly as in Bitcoin Core or the relevant BIP.
- Cite the authoritative source in comments for non-obvious math (e.g. `GetBlockProof` in `chain.cpp`).
- Cross-check with known test vectors before claiming mainnet compatibility.

## Errors, not panics

- Public APIs return `Result` for invalid external input.
- `expect` / `unwrap` are allowed in tests and for internal invariants that cannot fail if earlier validation succeeded.
- Reorg, orphan, and fork-walking paths must not panic on malformed metadata.

## Bounded work (DoS resistance)

| Limit | Value | Notes |
|-------|-------|--------|
| Max block serialized size | 4_000_000 bytes | Bitcoin `MAX_BLOCK_SERIALIZED_SIZE` |
| Max transaction serialized size | 4_000_000 bytes | Defensive standalone decode ceiling; a transaction cannot exceed its containing block |
| Max inputs per transaction | 25_000 | Defensive decode/allocation limit, not a consensus rule |
| Max outputs per transaction | 25_000 | Defensive decode/allocation limit, not a consensus rule |
| Max orphan pool | 100 blocks | Evict oldest when full |
| Max transactions per block | 25_000 | Protocol upper bound |
| Max script size | 10_000 bytes | Per-script push (simplified) |
| Mining nonce search | 10_000_000 attempts | Returns `MineError::AttemptsExceeded` |

## Validation order

Cheapest checks first on untrusted blocks:

1. Serialized size
2. Proof of work
3. Merkle root
4. Coinbase rules
5. Timestamp (MTP + future drift)
6. Compact `bits`
7. UTXO / transaction rules
8. Script (M5+)

## Testing

- Unit tests per module with happy path + negative cases.
- Integration tests under `tests/` for chain reorg, orphans, UTXO undo symmetry.
- Property tests optional (`proptest`) for invariants.
- Mainnet block replay and legacy known vectors (M8).

## Documentation

Consensus comments explain **why** (rule purpose, BIP reference), not only what the code does.
