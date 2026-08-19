# bitrst release thread (2026-W34)

Post as a thread. Each block is one tweet.

---

bitrst can now talk to peers, replay mainnet genesis, admit txs into a bounded mempool, and write blocks to disk. The CLI still keeps the chain in RAM. Persistence is a library API, not a node flag.

---

Every peer shares one `ChainHandle` and one `MempoolHandle`. Relay answers `inv`, `getdata`, `tx`, and `block`. Outstanding `getdata` is capped and TTL-expired, so a looping peer cannot queue unbounded fetches.

---

Mempool admission checks the active UTXO set plus in-pool spends. At 5,000 txs or 300 MB it evicts lowest fee rate first, oldest admission as the tie break. A reorg that disconnects a block puts those spends back only if they still validate. The disconnect journal is 256 blocks. Older than that and resync returns an error instead of guessing.

---

`FileBlockStore` names each block by its 64-char hash. Write a `.tmp` in the same directory, fsync, rename. Open deletes leftover temps and junk filenames. The CLI does not use it yet.

---

The bugs that actually showed up: a lagging event cursor left mempool stale until explicit resync. Serving a mempool tx after reorg without rechecking it. Mining with `--network-time` equal to genesis time, which MTP rejects because the next block must be strictly later.

---

Fast tests: `cargo test --all --features test-short-period`. Repo: https://github.com/akshitj11/bitrst
