# Consensus and production standards (bitrst)

Bytes from peers, CLI, or disk are untrusted. Public APIs return `Result`. Consensus math matches Bitcoin Core or a cited BIP. Comments say why the rule exists.

`unwrap` / `expect` stay in tests and in paths where a prior check already made failure impossible. Reorg, orphan, and fork walks must not panic on malformed metadata.

## Bounded work

| Limit | Value | Notes |
|-------|-------|--------|
| Max block serialized size | 4_000_000 bytes | Bitcoin `MAX_BLOCK_SERIALIZED_SIZE` |
| Max transaction serialized size | 4_000_000 bytes | Same ceiling as a containing block |
| Max inputs per transaction | 25_000 | Decode/allocation bound, not a consensus rule |
| Max outputs per transaction | 25_000 | Decode/allocation bound, not a consensus rule |
| Max orphan blocks | 256 | Evict oldest when full |
| Max transactions per block | 25_000 | Protocol upper bound |
| Max script size | 10_000 bytes | Per-script push (simplified) |
| Mempool tx count | 5_000 | Default; lowest fee rate evicted first |
| Mempool serialized bytes | 300_000_000 | Default |
| Chain event journal | 256 entries | Multi-consumer replay; overrun is an error |
| Disconnected block journal | 256 blocks | Must cover the event journal window |
| Mining nonce search | 10_000_000 attempts | `MineError::AttemptsExceeded` |

## Validation order

Cheapest checks first:

1. Serialized size
2. Proof of work
3. Merkle root
4. Coinbase rules
5. Timestamp (MTP + future drift)
6. Compact `bits`
7. UTXO / transaction rules
8. Script (P2PKH, legacy `SIGHASH_ALL`)

## Testing

Module tests cover happy path and rejection. Integration tests under `tests/` cover reorg, orphans, UTXO undo, mempool restore, and P2P relay. Mainnet genesis replay and legacy known vectors are M8.
