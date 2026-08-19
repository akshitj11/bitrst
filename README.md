# bitrst


<img width="2283" height="1717" alt="image" src="https://github.com/user-attachments/assets/788c9b45-ac30-4e0c-ad66-275525ab96e3" />

Bitcoin from scratch, in Rust.

Local mining, P2PKH wallet, bounded mempool, and a P2P `node` that relays blocks and txs. CLI chains stay in memory. `FileBlockStore` is the disk API for library callers.

## Demo

![bitrst mine finding two local blocks](docs/assets/mine-demo.gif)

`bitrst mine --count 2 --network-time 1231007105`. Network time has to sit strictly above genesis MTP or the first mined block is rejected. Regenerate with `scripts/render-demo.sh` (ImageMagick plus a built `bitrst` binary).

## Architecture

```mermaid
flowchart TB
  subgraph cli [bitrst CLI]
    Main[src/main.rs]
    Tip[tip]
    Mine[mine]
    Wallet[wallet]
    NodeCmd[node]
  end

  subgraph wallet_layer [bitrst-wallet]
    WalletCore[Wallet + UTXO watch]
    Sign[sign_p2pkh_input]
    Addr[P2PKH Address]
  end

  subgraph core [bitrst-core]
    Handle[ChainHandle]
    Chain[Chain connect / reorg / orphans]
    Validate[Validate: size PoW Merkle coinbase time bits UTXO script]
    Utxo[UtxoSet]
    Events[ChainEvent journal + cursor]
    Mempool[Mempool admission / eviction]
    MempoolH[MempoolHandle]
    StoreTrait[BlockStore trait]
    MemStore[MemoryBlockStore]
    FileStore[FileBlockStore]
    DiscJournal[DisconnectedBlockJournal]
  end

  subgraph script [bitrst-script]
    VM[P2PKH stack interpreter]
  end

  subgraph crypto [bitrst-crypto]
    Hash[SHA256d HASH160 Base58 ECDSA]
  end

  subgraph miner [bitrst-miner]
    MineCrate[nonce search]
  end

  subgraph net [bitrst-net P2P]
    PM[PeerManager]
    HS[handshake]
    Relay[relay: inv / getdata / tx / block]
    Track[BlockRequestTracker + TxRequestTracker]
  end

  Main --> Tip
  Main --> Mine
  Main --> Wallet
  Main --> NodeCmd
  Tip --> Handle
  Mine --> Handle
  Mine --> MineCrate
  Wallet --> WalletCore
  WalletCore --> Sign
  WalletCore --> Handle
  Sign --> Hash
  Sign --> VM
  Addr --> Hash
  NodeCmd --> PM
  Handle --> Chain
  Chain --> Validate
  Validate --> Utxo
  Validate --> VM
  Validate --> Hash
  Chain --> Events
  Chain --> StoreTrait
  Chain --> DiscJournal
  StoreTrait --> MemStore
  StoreTrait --> FileStore
  MempoolH --> Mempool
  Mempool --> Validate
  Mempool --> Utxo
  Mempool --> Events
  Mempool --> DiscJournal
  VM --> Hash
  MineCrate --> Chain
  PM --> HS
  PM --> Relay
  PM --> Handle
  PM --> MempoolH
  Relay --> Track
  Relay --> Handle
  Relay --> MempoolH
```

Diagram source: [`docs/architecture-diagram.mmd`](docs/architecture-diagram.mmd). Full write-up: [`ARCHITECTURE.md`](ARCHITECTURE.md).

## CLI

The `bitrst` binary builds a fresh in-memory chain per process:

| Command | Purpose |
|---------|---------|
| `tip` | Print the active chain tip hash |
| `mine` | Mine one or more blocks on a local chain |
| `wallet new` | Generate a P2PKH address (secrets hidden by default) |
| `wallet address` | Derive an address from a private key (`--private-key-stdin`, `BITRST_PRIVATE_KEY`, or `--private-key`) |
| `wallet balance` | Report balance for an address on a genesis-only chain |
| `node` | Run a P2P node via `PeerManager` with shared mempool (Ctrl-C / SIGTERM shutdown) |

Each command prints that the chain is ephemeral. `FileBlockStore` writes one hex file per block hash (temp, fsync, rename). The CLI does not call it yet.

```bash
bitrst tip
bitrst mine --count 2 --network-time 1231007105
bitrst wallet new
bitrst wallet address --private-key-stdin   # hex on stdin
bitrst wallet balance --address <addr> --network-time 1231007105
bitrst node --listen 127.0.0.1:8333 --network testnet
```

## Current scope

- Workspace scaffold
- Core, crypto, and miner crates
- SHA-256d hashing with the Bitcoin genesis header test vector
- Block header serialization and hashing
- Full `Block` struct with transaction list and Merkle root validation
- Transaction and UTXO basics
- First proof-of-work nonce search pass
- Difficulty adjustment over 2,016-block periods
- Block timestamp validation (MTP and future-drift limits)
- `Chain` validation: connect blocks, UTXO checks, orphans, reorg by cumulative work
- M4.5 workspace hardening: spec-aligned `block_work`, DoS limits, fork-aware MTP, `ChainHandle`, events, block store trait
- M4.6 chain robustness: reorg snapshot rollback, iterative orphan promotion, `active_hashes`, analytic `serialized_size`
- Universal-guide chain consensus integration tests (reorg safety, orphans, difficulty, validation, events)
- M5 Script VM: P2PKH script verification, legacy sighash, `bitrst-script` stack interpreter
- M6 Wallet: secp256k1 key generation, Base58Check P2PKH addresses, P2PKH signing, and active-chain UTXO tracking
- M7 P2P networking: `bitrst-net` peer manager, handshake, block and transaction relay, and CLI `node` command
- M8 Benchmarks, genesis replay, and known vectors
- `FileBlockStore`: atomic one-file-per-hash disk persistence
- Bounded mempool with fee-rate eviction, reorg sync, and P2P relay integration
- CI for tests, clippy, and dependency security (`cargo audit`, `cargo deny`)

## Docs and progress

- Architecture: [`ARCHITECTURE.md`](ARCHITECTURE.md)
- Devlog: [`devlog/2026-W34.md`](devlog/2026-W34.md)
- Machine-readable stats: [`docs/progress.json`](docs/progress.json)
- Validate docs locally: `scripts/validate-docs.sh`

### mdBook (optional)

Install [mdBook](https://rust-lang.github.io/mdBook/) locally, then from the repo root:

```bash
mdbook build
```

Output lands in `book/`. mdBook is not required for CI; the repository docs work as plain Markdown without it.

## Testing

Fast workspace suite (short difficulty-adjustment period):

```bash
cargo test --all --features test-short-period
```

Use `cargo ci-fast` for the same fast test command with locked dependencies. A full mainnet-interval boundary run (`cargo test --all` without features) is slower but supported.

Legacy known vectors and mainnet genesis replay:

```bash
cargo test --test known_vectors
cargo test --test mainnet_genesis
```

Benchmarks (compile-only in CI; run locally for timings):

```bash
cargo bench --no-run
cargo bench
```

See [`docs/benchmarks.md`](docs/benchmarks.md) for filters and a machine-dependent results template.

## Security

Dependency policy and CI behavior: [`docs/dependency-security.md`](docs/dependency-security.md).

Before pushing dependency changes:

```bash
cargo audit
cargo deny check
```

## Roadmap

1. Block + SHA256d: done
2. Transactions + UTXO: done
3. Proof of work: done
4. Chain validation: done
5. Workspace hardening (M4.5): done
6. Chain robustness (M4.6): done
7. Script VM (M5): done
8. Wallet (M6): done
9. P2P networking (M7): done
10. Benchmarks, genesis replay, and known vectors (M8): done
11. FileBlockStore and mempool relay (M8.1–M8.2): done
12. CLI persistent chain, addrman, headers-first sync: planned
