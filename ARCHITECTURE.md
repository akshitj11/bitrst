# Architecture

Visual diagram: [README Architecture section](README.md#architecture). Mermaid source: [`docs/architecture-diagram.mmd`](docs/architecture-diagram.mmd).

Untrusted bytes enter through P2P or CLI. Size bounds run first. Then PoW, Merkle, coinbase, time, bits, UTXO, script. Only after that does chain state move.

## Crates

- `bitrst-core`: blocks, transactions, UTXO, consensus, mempool, event journals, block storage
- `bitrst-crypto`: SHA-256d, HASH160, Base58Check, ECDSA
- `bitrst-script`: P2PKH stack interpreter
- `bitrst-miner`: bounded nonce search against core `Target` rules
- `bitrst-wallet`: keys, P2PKH addresses, signing, UTXO watch
- `bitrst-net`: handshake, `PeerManager`, `inv`/`getdata`/`tx`/`block` relay
- `bitrst`: CLI (`tip`, `mine`, `wallet`, `node`)

Consensus comments cite Bitcoin Core or a BIP. See [docs/CONSENSUS_STANDARDS.md](docs/CONSENSUS_STANDARDS.md).

## Data flow

1. Core types serialize to Bitcoin wire bytes. Decode is bounded and rejects non-canonical CompactSize.
2. `bitrst-crypto` hashes those bytes with SHA-256d.
3. Incoming blocks: serialized size, PoW, Merkle, coinbase, MTP and future drift, compact `bits`, UTXO, P2PKH script.
4. `Chain::connect_block` extends the tip, parks orphans (cap 256, oldest evicted), or reorgs to more cumulative work. A failed reorg restores a snapshot.
5. `ChainHandle` (`Arc<RwLock<Chain>>`) is the thread-safe entry for CLI and P2P.
6. `BlockStore` is the persistence trait. `MemoryBlockStore` backs tests and CLI. `FileBlockStore` writes one lowercase-hex file per hash: same-directory temp, `fsync`, `rename`. Open sweeps `.tmp` and invalid names.
7. `ChainEvent` is a bounded journal (256 entries). `ChainEventCursor` reads without draining. `take_events` is the wallet drain. If the wallet high-water mark falls off the window, it gets a lag error instead of a silent gap.
8. `DisconnectedBlockJournal` keeps the same 256-entry window of recently disconnected active-chain blocks so mempool resync can replay exact disconnect history. Overrun is an error.
9. `Mempool` admits non-coinbase txs against the active UTXO view plus in-pool spends. Duplicate inputs, missing UTXOs, output > input, and P2PKH failures reject. At 5,000 txs or 300 MB it evicts lowest fee rate, then oldest. Evicting a parent removes its descendants in the same step. `MempoolHandle` is the concurrent wrapper.
10. `bitrst-script` verifies P2PKH with legacy `SIGHASH_ALL`.
11. `bitrst-wallet` watches addresses and applies connect/disconnect events. Side-chain blocks do not credit balances.
12. `bitrst-miner` searches nonces up to `MAX_NONCE_ATTEMPTS`.
13. `bitrst-net` `PeerManager` accepts inbound peers, dials seeds, and runs per-peer tasks. `relay.rs` talks to shared `ChainHandle` and `MempoolHandle`. Chain work that can block runs on `spawn_blocking`.
14. CLI commands build an in-memory chain and print that fact. `FileBlockStore` is for library callers and tests.

## Relay

`PeerRelayState` tracks outstanding block and tx `getdata` with TTL and a cap. On `inv`, unknown block hashes and unseen mempool txids request `getdata`. On `getdata`, chain serves stored blocks. Mempool revalidates a tx against the current UTXO view before serving it and drops it if stale. Accepted `tx` messages fan out `inv` to other ready peers, excluding the origin. Each peer holds a `ChainEventCursor`. Cursor lag triggers `resync_to_active_chain` from the last collected sequence. If the disconnect journal no longer holds that history, relay returns an error instead of a partial pool.

## CLI

| Command | Backend | Notes |
|---------|---------|-------|
| `tip` | Ephemeral `ChainHandle` | Prints tip hash |
| `mine` | Ephemeral | Coinbase blocks. Default easy `bits`. `--network-time` must be strictly after genesis MTP |
| `wallet` | Ephemeral | Keygen, address derivation, genesis-only balance |
| `node` | Ephemeral + `MempoolHandle` | `PeerManager` until Ctrl-C / SIGTERM |

`node` shares one chain and one mempool across peer tasks. Listen address, seeds, inbound/outbound caps, and network magic are flags.

## Chain work

Per-block work is Bitcoin Core `GetBlockProof`: `(~target / (target + 1)) + 1` on 256-bit little-endian targets (`Target::to_work` in `pow.rs`).
