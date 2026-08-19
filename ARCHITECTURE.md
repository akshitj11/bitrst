# Architecture

Visual diagram: [README Architecture section](README.md#architecture). Mermaid source: [`docs/architecture-diagram.mmd`](docs/architecture-diagram.mmd).

The workspace layers Bitcoin primitives from wire encoding through chain validation, wallet signing, mempool admission, and P2P relay. Each crate owns one boundary; the CLI wires them for local experimentation.

## Crates

- `bitrst-core`: blocks, transactions, UTXO set, consensus rules, chain validation, mempool, and block storage
- `bitrst-crypto`: SHA-256d, HASH160, Base58Check, and ECDSA helpers
- `bitrst-script`: P2PKH script templates and stack-based script verification
- `bitrst-miner`: nonce search (imports `Target`, difficulty, and time rules from core)
- `bitrst-wallet`: secp256k1 keys, P2PKH addresses, transaction signing, and wallet UTXO tracking
- `bitrst-net`: Bitcoin P2P handshake, peer manager, inventory relay, and graceful shutdown
- `bitrst`: CLI entry point (`tip`, `mine`, `wallet`, `node`)

See [docs/CONSENSUS_STANDARDS.md](docs/CONSENSUS_STANDARDS.md) for production-quality rules applied across crates.

## Data flow

1. Core structs serialize protocol data into Bitcoin wire bytes.
2. `bitrst-crypto` hashes those bytes with SHA-256d.
3. Untrusted blocks are size-checked, then validated: PoW, Merkle, coinbase, timestamp, bits, UTXO.
4. `Chain::connect_block` extends the active tip, stores orphans (capped), or reorganizes to heavier cumulative work.
5. `ChainHandle` (`Arc<RwLock<Chain>>`) exposes thread-safe tip reads and exclusive connects for CLI and P2P.
6. `BlockStore` abstracts persistence. `MemoryBlockStore` backs tests and ephemeral CLI chains. `FileBlockStore` writes one hex-named file per block hash with same-directory temp file, `fsync`, and `rename`.
7. `ChainEvent` records connects, disconnects, reorgs, and orphan pool changes in a bounded journal. `ChainEventCursor` lets consumers read without draining the log.
8. `DisconnectedBlockJournal` retains recently disconnected active-chain blocks so mempool resync can replay exact disconnect history after cursor lag.
9. `Mempool` admits non-coinbase transactions against the active UTXO view plus in-pool spends. At capacity it evicts the lowest fee-rate resident; ties break on oldest admission. `MempoolHandle` wraps the pool for concurrent P2P access.
10. `bitrst-script` verifies P2PKH spends using legacy sighash and ECDSA.
11. `bitrst-wallet` watches P2PKH addresses, signs local spends, and updates balances from active-chain events.
12. `bitrst-miner` searches nonces with bounded attempts (`MAX_NONCE_ATTEMPTS`).
13. `bitrst-net` `PeerManager` accepts inbound peers, dials seeds, and runs per-peer relay tasks. `relay.rs` handles `inv`, `getdata`, `tx`, and `block` messages against shared `ChainHandle` and `MempoolHandle`.
14. CLI commands build ephemeral in-memory chains. They print an explicit notice. `FileBlockStore` is available to library callers and integration tests; the CLI does not persist chain state yet.

## Block and transaction relay

`PeerRelayState` tracks outstanding block and transaction `getdata` requests with TTL and capacity limits. On `inv`, the relay layer requests unknown block hashes and unseen mempool transaction IDs. On `getdata`, it serves stored blocks from chain and validated transactions from mempool, revalidating mempool entries before reply. Accepted `tx` messages run through mempool admission; successful accepts fan out `inv` to other peers. Each peer keeps a `ChainEventCursor` so mempool state resyncs when the consumer falls behind the event journal.

## CLI

| Command | Chain backend | Notes |
|---------|---------------|-------|
| `tip` | Ephemeral `ChainHandle` | Prints active tip hash |
| `mine` | Ephemeral | Mines coinbase blocks with easy `bits` default |
| `wallet` | Ephemeral | Keygen, address derivation, genesis-only balance |
| `node` | Ephemeral + `MempoolHandle` | Runs `PeerManager` until Ctrl-C / SIGTERM |

`node` shares one `ChainHandle` and one `MempoolHandle` across all peer tasks. Listen address, seed list, inbound/outbound caps, and network magic are CLI flags.

## Chain work

Per-block work uses Bitcoin Core `GetBlockProof`: `(~target / (target + 1)) + 1` on 256-bit little-endian targets (`Target::to_work` in `pow.rs`).
