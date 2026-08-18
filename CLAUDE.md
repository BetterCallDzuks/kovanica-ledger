# CLAUDE.md

Guidance for AI assistants (and humans) working in the **kovanica-ledger** repository.

> **Status: early implementation.** Two vertical slices exist, build, and are
> tested: the block DAG and GHOSTDAG consensus core (`crates/kovanica-dag`), and
> the UTXO ledger/state layer that applies transactions in GHOSTDAG-linearized
> order with ed25519 spend authorisation (`crates/kovanica-state`). Layers around
> them (networking, mempool, node binary, RPC, persistence) do not exist yet and
> are marked **TODO** below. Keep this file in sync with the code: update it in
> the same change that adds or moves the structure it describes.

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
Cargo.toml                     Workspace manifest (resolver 2); shared deps: blake3, hex, ed25519-dalek
crates/
  kovanica-dag/                The DAG + GHOSTDAG consensus core (first slice)
    src/
      lib.rs                   Crate docs + re-exports + a doctest quick tour
      block.rs                 Block (multi-parent vertex) and BlockId (BLAKE3 hash)
      dag.rs                   Dag store: insert/validate, past sets, tips, GhostdagData, preview(), chain_key
      ghostdag.rs              compute_ghostdag(): selected parent, mergeset, k-cluster blue/red colouring
      ordering.rs              linearize() (recursive GHOSTDAG order), selected_tip/selected_chain
      validation.rs            BlockValidator trait + Dag::with_validator: pluggable insert-time validation
      snapshot.rs              Dag::write_snapshot()/read_snapshot(): replay-log persistence
    tests/
      consensus.rs             Integration + adversarial tests (wide fork, determinism, k-cluster invariant, validator hook)
  kovanica-state/              UTXO ledger applied in GHOSTDAG order (second slice)
    src/
      lib.rs                   Crate docs + re-exports + an end-to-end doctest
      keys.rs                  Address, KeyPair, verify() — ed25519 spend authorisation
      tx.rs                    Transaction/TxId/OutPoint/TxInput/TxOutput; canonical encoding; sighash
      utxo.rs                  UtxoSet: the unspent-output state, with balance/total_value
      ledger.rs                apply_block()/apply_dag() (batch) + Ledger (per-block UTXO state, stateful insert, snapshot)
      validation.rs            TxStructureValidator: context-free structural checks (a BlockValidator)
    tests/
      ledger.rs                Integration + adversarial (double-spend across parallel blocks, order-independence)
      validation.rs            Integration: structural rejection at insert vs stateful rejection at apply
      perblock.rs              Integration: per-block state, stateful insert rejection, apply_dag consistency
      persistence.rs           Integration: Ledger snapshot round-trip (state recomputed by replay)
```

Not built yet (**TODO**, add crates under `crates/` as they land): `network` (p2p
gossip), `mempool` (tx dissemination), `node` (binary, config, RPC). Some `crypto`
exists (ed25519 spend signatures in `kovanica-state`); VRF and beyond remain TODO.
Update this tree when you add them.

### Deliberate first-slice simplifications (do not mistake for the final design)

- **Reachability** is answered from a per-block `past` set stored in full: O(1)
  queries but O(n²) memory. Production replaces this with a reachability oracle
  (interval labels). See `dag.rs` module docs.
- **In-memory working state**, with **replay-log persistence**: `Dag`/`Ledger`
  `write_snapshot`/`read_snapshot` serialise only `k`, the subsidy, and the blocks
  in topological order; loading replays inserts so all derived state (past sets,
  colouring, per-block UTXO state) is recomputed, never trusted from disk. There
  is no incremental on-disk store or mmap yet — a snapshot is written/read whole.
- **Linearization** is the recursive GHOSTDAG order:
  `order(B) = order(selected_parent(B)) ++ mergeset_order(B) ++ [B]`, unrolled over
  the selected chain and closed with the selected tip's anticone (the virtual
  block's mergeset). The selected chain is a subsequence and each merged block
  sits directly before its merger. Mergeset order within a block is a deterministic
  topological sort by `(|past|, id)` — a valid GHOSTDAG-spirit order, not Kaspa's
  exact blue-work mergeset tiebreak. See `ordering.rs` module docs.
- **State (`kovanica-state`)** applies transactions in GHOSTDAG order. Two views:
  `apply_dag` folds a finished DAG from scratch; `Ledger` maintains each block's
  view state incrementally from its selected parent (per-block state stored in
  full — the same O(n²) trade-off as `past` sets). A subsidy is a single per-block
  constant (no halving schedule); coinbase maturity is only "not spendable in the
  same block"; there are no tx size/weight limits. `Ledger` is append-only — no
  pruning or re-org/revert of stored states yet. See `ledger.rs` module docs.
- **Insert-time validation** now has both layers. `Dag::with_validator` +
  `TxStructureValidator` reject malformed/structurally-invalid blocks; `Ledger`
  additionally runs the **stateful** rules (input existence, signatures, value
  conservation, coinbase amount) against a block's view state before it enters the
  DAG (via `Dag::preview`), so a block invalid in its own view is rejected at
  insert. Two *parallel* blocks that spend the same output are each valid in their
  own view and both admitted; their conflict resolves only in a merger's view.

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

- [x] Transactions + a UTXO state layer; apply state in linearized order (`kovanica-state`).
- [x] Signatures (ed25519) for spend authorisation.
- [x] Block-level validation at insert time: context-free structural validation
      (`BlockValidator` hook + `TxStructureValidator`) and stateful (UTXO-aware)
      validation (`Ledger` + `Dag::preview`).
- [x] Recursive GHOSTDAG linearization (`order(B) = order(sp) ++ mergeset ++ [B]`),
      the ordering per-block UTXO state composes along.
- [x] Per-block UTXO state built incrementally from each block's selected parent
      (`Ledger`), matching `apply_dag`; enables stateful validation at insert.
- [ ] Incremental re-orgs / state revert + pruning (finality depth); `Ledger` is
      append-only today and stores each block's state in full.
- [x] Persistence: replay-log snapshots of the DAG and ledger
      (`Dag`/`Ledger` `write_snapshot`/`read_snapshot`) — state recomputed on load.
- [ ] Reachability oracle to replace full per-block `past` sets (interval labels +
      future-covering sets); also rewires mergeset computation and the ordering key.
- [ ] Incremental / streaming on-disk store (today a snapshot is written & read whole).
- [ ] p2p networking / gossip and a runnable node binary with RPC.
- [ ] Difficulty adjustment for `work`.
