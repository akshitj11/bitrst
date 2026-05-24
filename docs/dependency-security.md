# bitrst — CI Dependency Security Guide

This document covers the setup for `cargo audit`, `cargo deny`, and `cargo outdated` in CI. It explains what each tool does, what to configure, and how to handle failures — specific to a Bitcoin node codebase where a bad dependency is a consensus risk, not just a bug.

## Why this matters more for bitrst than a normal Rust project

A compromised or vulnerable dependency in a web API means data leaks. In a Bitcoin node it means:

- A broken `sha2` or hash crate → invalid blocks accepted, valid blocks rejected
- A vulnerable crypto primitive → private keys or signatures forged
- A supply-chain attack on any consensus-critical crate → network split

**Rule:** every dependency is a consensus surface. Treat new dependencies like new consensus rules — with suspicion and explicit justification.

## Repository layout

```
bitrst/
├── .github/workflows/
│   ├── ci.yml           # fmt, clippy, tests
│   ├── security.yml     # cargo audit + cargo deny
│   └── outdated.yml     # weekly cargo outdated (non-blocking)
├── deny.toml            # cargo deny policy
└── audit.toml           # cargo audit suppressions (reviewed only)
```

## The three tools

| Tool | Checks against | Blocks CI? | When |
|------|----------------|------------|------|
| `cargo audit` | RustSec CVE database | Yes | Every push to `main`; PRs that touch deps |
| `cargo deny` | Licenses, duplicates, banned crates, sources | Yes (except advisories matrix leg — warn only) | Same as audit |
| `cargo outdated` | Latest versions on crates.io | No | Weekly (Mon 09:00 UTC) + manual dispatch |

**Why `cargo outdated` is warning-only:** outdated ≠ vulnerable. Blocking CI on every new upstream release creates churn. The weekly workflow uses `--exit-code 0`.

## CI workflows

### `security.yml`

- **Push to `main`:** full security run (no path filter).
- **Pull requests:** only when `Cargo.toml`, `Cargo.lock`, `deny.toml`, `audit.toml`, or `security.yml` change.

Jobs:

1. **`audit`** — `rustsec/audit-check@v2` (hard fail).
2. **`deny`** — matrix over `advisories`, `bans`, `licenses`, `sources`. The `advisories` leg uses `continue-on-error: true` so a newly published advisory does not block unrelated work; `cargo audit` still hard-fails on CVEs.

### `outdated.yml`

- Schedule: `0 9 * * 1` (Monday 09:00 UTC).
- `workflow_dispatch` for manual runs.
- `cargo outdated --workspace --exit-code 0 --depth 1` (direct deps only).

## Configuration files

### `deny.toml`

- **Advisories (v2):** `unmaintained = "workspace"`, `unsound = "all"`, `yanked = "deny"`.
- **Licenses (v2):** allow-list only (unlisted licenses denied by default). Workspace crates declare `license = "MIT"`.
- **Bans:** `multiple-versions = "deny"`. Use `skip` with comments for unavoidable duplicates.
- **Sources:** only crates.io; no unknown git/registry.

### `audit.toml`

CVE suppressions belong **here only**, after review. Every `ignore` entry needs a comment and a tracking issue. Do not duplicate ignores in `deny.toml`.

## Local checks (before push)

```bash
cargo audit
cargo deny check advisories
cargo deny check bans
cargo deny check licenses
cargo deny check sources
cargo outdated --workspace --depth 1   # informational
```

With [`.cargo/config.toml`](../.cargo/config.toml), `cargo test` matches CI test flags.

## When CI fails

### `cargo audit` finds a CVE

1. Read the advisory at https://rustsec.org/advisories/
2. If the vulnerable path is reachable from bitrst: update the dependency immediately.
3. If unreachable: add to `audit.toml` with a comment; open a tracking issue to update within 30 days.

### `cargo deny` — `bans` (duplicate crate)

```bash
cargo tree -d -p <crate-name>
cargo update -p <crate-name>
```

If unresolvable, add a documented `skip` in `deny.toml`.

### `cargo deny` — `licenses`

- Permissive license missing from allow list → add after review.
- GPL or unknown → find an alternative; do not add GPL to bitrst.

### `cargo outdated`

Triage weekly: prefer **Compat** updates; treat crypto crate staleness as urgent.

## Golden rule for new dependencies

Before adding any crate:

1. Published on crates.io? (no git sources)
2. Permissive license (MIT/Apache)?
3. Actively maintained? (roughly &lt; 1 year since last release)
4. Is there already a crate in the tree that does the same thing?

For crypto or hashing crates also ask:

5. Same implementation already used elsewhere? (avoid duplicate SHA256 stacks)
6. Checked on https://rustsec.org and the crate README?

## Pre-merge checklist (dependencies)

- [ ] No new crate without the golden-rule questions answered
- [ ] `cargo audit` passes locally
- [ ] `cargo deny check` passes for `bans`, `licenses`, `sources`
- [ ] `Cargo.lock` is committed
- [ ] Any `audit.toml` ignore has a comment and tracking issue

## Summary

| What | Tool | When | Hard fail? |
|------|------|------|------------|
| Known CVEs | `cargo audit` | Push to `main` + dep PRs | Yes |
| CVEs (secondary) | `cargo deny advisories` | Same | Warn only |
| Duplicate crates | `cargo deny bans` | Same | Yes |
| Licenses | `cargo deny licenses` | Same | Yes |
| Non-crates.io sources | `cargo deny sources` | Same | Yes |
| Stale versions | `cargo outdated` | Weekly | No |
