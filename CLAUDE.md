# CLAUDE.md

Guidance for AI assistants (and humans) working in the **kovanica-ledger** repository.

> **Status: early implementation.** The first vertical slice — the block DAG and
> the GHOSTDAG consensus core — exists, builds, and is tested (`crates/kovanica-dag`).
> Layers above it (networking, mempool, UTXO/state, node binary, RPC) do not exist
> yet and are marked **TODO** below. Keep this file in sync with the code: update
> it in the same change that adds or moves the structure it describes.

---

## 1. What this project is

**kovanica-ledger** is a **DAG-based distributed ledger** — a high-throughput,
parallel-block cryptocurrency/ledger protocol built on a **Directed Acyclic Graph**
(BlockDAG) rather than a single linear chain. The name *kovanica* is
Serbo-Croatian for "coin / mint." Blocks reference **multiple parents**, so many
blocks can be produced in parallel and later merged, which is what enables high
block rates (BPS).

The consensus core follows **GHOSTDAG** (Sompolinsky, Wyborski & Zohar — the
protocol behind Kaspa, a refinement of PHANTOM). Related systems in the design
space, for reference when extending consensus: IOTA Tangle, Hedera Hashgraph,
Fantom Lachesis/Sonic, Avalanche Snow family, Conflux, OHIE, and the DAG-BFT
mempool/ordering line (Narwhal–Tusk, Bullshark, Mysticeti, Sailfish). When
implementing a mechanism from any of these, **name it** in comments so reviewers
can check it against the paper.

## 2. Core domain concepts (shared vocabulary)

Keep these terms precise and consistent across code, comments, and docs:

- **DAG / BlockDAG** — the ledger is a directed acyclic graph; a block references
  **multiple parents/tips**, enabling parallel block creation.
- **Tip** — a block with no children yet; new blocks reference current tips.
- **Past / ancestors** — all blocks reachable by following parent edges.
- **Anticone** — blocks neither ancestor nor descendant of a given block ("parallel").
- **Selected parent** — the parent with the heaviest blue work; forms the chain backbone.
- **Mergeset** — the blocks a new block merges in: `past(B) \ (past(sp) ∪ {sp})`.
- **Blue set / red set** — the well-connected honest cluster (blue) vs. blocks left
  too far to the side (red), decided by the **k-cluster rule**.
- **k parameter** — max tolerated *blue anticone size*: every blue block may have at
  most `k` blue blocks in its anticone. Larger `k` tolerates higher latency/BPS at a
  wider security margin.
- **Blue score / blue work** — size / total work of a block's blue set; drives chain
  selection and ordering.
- **Partial order → total order (linearization)** — consensus deterministically
  linearizes the DAG so every honest node agrees on one sequence.

## 3. Repository layout

```
Cargo.toml                     Workspace manifest (resolver 2); shared deps: blake3, hex
crates/
  kovanica-dag/                The DAG + GHOSTDAG consensus core (first slice)
    src/
      lib.rs                   Crate docs + re-exports + a doctest quick tour
      block.rs                 Block (multi-parent vertex) and BlockId (BLAKE3 hash)
      dag.rs                   Dag store: insert/validate, past sets, tips, GhostdagData, chain_key
      ghostdag.rs              compute_ghostdag(): selected parent, mergeset, k-cluster blue/red colouring
      ordering.rs              linearize() (deterministic topological order), selected_tip/selected_chain
    tests/
      consensus.rs             Integration + adversarial tests (wide fork, determinism, k-cluster invariant)
```

Not built yet (**TODO**, add crates under `crates/` as they land): `network` (p2p
gossip), `mempool` (tx dissemination), `ledger`/`state` (UTXO or accounts,
validation), `crypto` (signatures/VRF beyond block hashing), `node` (binary, config,
RPC). Update this tree when you add them.

### Deliberate first-slice simplifications (do not mistake for the final design)

- **Reachability** is answered from a per-block `past` set stored in full: O(1)
  queries but O(n²) memory. Production replaces this with a reachability oracle
  (interval labels). See `dag.rs` module docs.
- **In-memory only** — no persistence layer yet.
- **Linearization** is a deterministic topological sort keyed by the GHOSTDAG chain
  key (blue work, blue score, id). It is a valid total order in the GHOSTDAG spirit,
  not a bit-for-bit reproduction of Kaspa's mergeset ordering.

## 4. Build, test & run

Rust workspace (edition 2021, `rust-version` 1.75). From the repo root:

- **Build:** `cargo build`
- **Test (all: unit + integration + doctests):** `cargo test`
- **Single test:** `cargo test <name>` (e.g. `cargo test adversarial_wide_fork`)
- **Lint:** `cargo clippy --all-targets` (keep it warning-clean)
- **Format:** `cargo fmt` (CI-style check: `cargo fmt --check`)

`unsafe` code is **forbidden** crate-wide (`#![forbid(unsafe_code)]` via
`[lints.rust]`). There is no node binary to run yet; the crate is a library.

## 5. Engineering conventions

- **Consensus correctness is paramount.** Any change to selected-parent choice,
  mergeset, k-cluster colouring, blue score/work, or linearization can break safety
  (double-spend) or liveness. Such changes require: a written rationale naming the
  protocol semantics being followed, and **deterministic + adversarial tests**
  (Byzantine/equivocating parents, wide forks beyond `k`, tie-breaks, partitions).
- **Determinism.** Consensus output must be a pure function of the DAG — identical on
  every node. Never let HashMap iteration order, wall-clock time, or unstable sorts
  affect a consensus result. (The colouring iterates a HashMap but its *outcome* is
  order-independent; preserve that property.)
- **Tie-breaks** fall back to `BlockId` byte order — keep it that way for determinism.
- **Tests:** prefer property/invariant and adversarial tests for graph/consensus code.
  The k-cluster invariant (`blue_anticone_size <= k` for every blue block) is a good
  general assertion — see `tests/consensus.rs`.
- **Style:** match surrounding code; document *why*. Keep consensus-affecting changes
  in focused commits.

## 6. Git workflow

- **Never commit to the default branch directly.** Develop on a feature branch and
  open a **draft PR**.
- **Branch naming:** short, kebab-case, scoped — `consensus/…`, `dag/…`, `ledger/…`,
  or `claude/<topic>` for assistant-driven work.
- **Commits:** clear, imperative subject lines describing *why*. Run `cargo fmt`,
  `cargo clippy --all-targets`, and `cargo test` before pushing.
- **Push:** `git push -u origin <branch-name>`; open a PR if none exists for the branch.

## 7. For AI assistants — working notes

- This file is the **source of truth for conventions**; when reality and this file
  disagree, fix one or the other in the same PR — don't silently diverge.
- **Do not invent** APIs, module paths, or commands. If a section here is TODO, say so
  rather than fabricating specifics.
- When reasoning about consensus, cite the concrete reference system (GHOSTDAG,
  Bullshark, Avalanche, …) so the design stays auditable.
- Before claiming tests/builds pass, actually run them and report real output.

---

## Roadmap / next slices (in rough order)

- [ ] Transactions + a UTXO or account state layer; apply state in linearized order.
- [ ] Signatures (ed25519) and block/tx validation beyond structural checks.
- [ ] Reachability oracle to replace full per-block `past` sets.
- [ ] Persistence (an on-disk store) for the DAG and consensus data.
- [ ] p2p networking / gossip and a runnable node binary with RPC.
- [ ] Pruning / finality depth; difficulty adjustment for `work`.
