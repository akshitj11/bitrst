# bitrst release thread (2026-W34)

Post as a thread. Each block is one tweet.

---

M7 and M8 are in. bitrst now has a working P2P stack, mainnet genesis replay, bounded mempool admission, and atomic disk block storage. CLI chains are still ephemeral; the store trait is real.

---

`PeerManager` shares one `ChainHandle` and one `MempoolHandle` across peers. Relay handles `inv`, `getdata`, `tx`, and `block`. Block and tx request trackers cap outstanding `getdata` with TTL expiry so loops do not accumulate.

---

Mempool admits against active UTXO plus in-pool spends. At capacity it evicts lowest fee rate first. Reorgs drop confirmed txs. Disconnected blocks go into a bounded journal so a heavier fork can put valid spends back without guessing.

---

`FileBlockStore` writes one hex file per block hash. Temp file in the same directory, fsync, rename. Open sweeps `.tmp` residue and invalid filenames. Library API only for now.

---

Failure modes that actually bit us: event cursor lag desyncing mempool until explicit resync, disconnect journal overrun when recovery needs blocks outside the window, serving mempool txs without revalidation after reorg, MTP validation rejecting `mine` when network time equals genesis time.

---

Fast tests: `cargo test --all --features test-short-period`. Repo: https://github.com/akshitj11/bitrst
