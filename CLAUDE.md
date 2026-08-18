# CLAUDE.md

Guidance for AI assistants (and humans) working in the **kovanica-ledger** repository.

> **Status: early implementation.** Three vertical slices exist, build, and are
> tested: the block DAG and GHOSTDAG consensus core (`crates/kovanica-dag`); the
> UTXO ledger/state layer that applies transactions in GHOSTDAG-linearized order
> with ed25519 spend authorisation, per-block state, snapshot persistence, and
> finality-depth pruning / re-orgs (`crates/kovanica-state`); and a runnable node
> binary with a line RPC, a
> mempool, block production, and multi-node block gossip (`crates/kovanica-node`).
> Difficulty is both an algorithm (`kovanica-dag::difficulty`) and, now,
> **consensus-enforced**: blocks carry a `timestamp`, and an opt-in policy
> (`Dag::set_difficulty`) requires each block's `work` to equal the target its
> past implies and its timestamp not to precede any parent's. Separately, the
> **node** now enforces a wall-clock future-time bound on block timestamps
> (`Node::receive_block` rejects a block dated more than two hours ahead of the
> local clock) — deliberately node policy, not a pure function of the DAG. Still
> **TODO** (below): continuous p2p gossip with peer discovery.
> Keep this file in sync with the code: update it in the same change that adds or
> moves the structure it describes.

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
      block.rs                 Block (multi-parent vertex, work + timestamp) and BlockId (BLAKE3 hash)
      dag.rs                   Dag store: insert/validate, oracle-backed reachability + mergeset, past_size, tips, GhostdagData, preview(), chain_key, set_difficulty/next_work_target
      ghostdag.rs              compute_ghostdag(): selected parent, mergeset, k-cluster blue/red colouring
      ordering.rs              linearize() (recursive GHOSTDAG order), selected_tip/selected_chain
      validation.rs            BlockValidator trait + Dag::with_validator: pluggable insert-time validation
      snapshot.rs              Dag::write_snapshot()/read_snapshot(): replay-log persistence
      difficulty.rs            Retarget::next_work(): difficulty retargeting for block work (algorithm); enforced via Dag::set_difficulty
      reachability.rs          Reachability oracle: interval-tree + future-covering sets (the Dag's backing for is_ancestor + mergeset)
    tests/
      consensus.rs             Integration + adversarial tests (wide fork, determinism, k-cluster invariant, validator hook)
      reachability.rs          Differential: Dag/oracle is_ancestor == naive parent-walk over random adversarial DAGs
      difficulty.rs            Integration + adversarial: enforced work/timestamp (understate/overstate/backdate rejected, target deterministic)
  kovanica-state/              UTXO ledger applied in GHOSTDAG order (second slice)
    src/
      lib.rs                   Crate docs + re-exports + an end-to-end doctest
      keys.rs                  Address, KeyPair, verify() — ed25519 spend authorisation
      tx.rs                    Transaction/TxId/OutPoint/TxInput/TxOutput; canonical encoding; sighash
      utxo.rs                  UtxoSet: the unspent-output state, with balance/total_value
      ledger.rs                apply_block()/apply_dag() (batch) + Ledger (per-block state, stateful insert, snapshot, finality/pruning)
      validation.rs            TxStructureValidator: context-free structural checks (a BlockValidator)
    tests/
      ledger.rs                Integration + adversarial (double-spend across parallel blocks, order-independence)
      validation.rs            Integration: structural rejection at insert vs stateful rejection at apply
      perblock.rs              Integration: per-block state, stateful insert rejection, apply_dag consistency
      persistence.rs           Integration: Ledger snapshot round-trip (state recomputed by replay)
      finality.rs              Integration: finality-depth pruning, deep-reorg rejection, implicit re-org
      difficulty.rs            Integration: Ledger::set_difficulty enforces work/timestamp end-to-end
  kovanica-node/               Runnable node binary, mempool, and block gossip (third slice + multi-node)
    src/
      lib.rs                   Crate docs + re-exports + a doctest of the RPC
      node.rs                  Node: Ledger + Mempool; genesis/send/pool/produce/balance/tips/save/load + gossip
      mempool.rs               Mempool: pending txs, deterministic (id) ordering for block assembly
      net.rs                   gossip() (in-process) + serve_blocks/pull_blocks (one-shot TCP sync)
      rpc.rs                   execute_line(): the text command protocol (string in, string out)
      main.rs                  Binary: `serve` (stdin/stdout REPL) and `demo` (scripted scenario)
    tests/
      rpc.rs                   Integration: end-to-end transfers, errors, snapshot round-trip via RPC
      mempool.rs               Integration: pool/produce assembly, conflict partial-inclusion
      network.rs               Integration: multi-node convergence (in-process + conflict + TCP loopback)
      timestamps.rs            Integration: wall-clock timestamp policy (pinned clock, monotone stamps, far-future reject)
```

Not built yet (**TODO**, add crates under `crates/` as they land): continuous p2p
gossip with peer discovery — `kovanica-node` today does one-shot block sync
(pull-all) between two nodes, no peer set or relay loop. Some `crypto` exists
(ed25519 spend signatures in
`kovanica-state`); VRF and beyond remain TODO. Update this tree when you add them.

### Deliberate first-slice simplifications (do not mistake for the final design)

- **Reachability** is answered by the interval-tree + future-covering-set oracle
  (`reachability::Reachability`): the selected-parent tree carries DFS interval
  labels and each block a future-covering set for the non-tree edges. This **is**
  the `Dag`'s backing — the per-block `past` sets are gone; each block keeps only
  its `past_size` (the ancestor *count*, all the topological sort key needs), and
  the mergeset is recomputed by a `sp`-bounded backward walk over parent edges
  (`Dag::mergeset_ordered`). The oracle is currently **rebuilt from scratch after
  every insert** (an O(n²) pass); incremental maintenance with interval
  **reindexing** is the remaining optimisation. Correctness is guarded by a
  differential test against an independent naive parent-walk over random
  adversarial DAGs, plus the whole consensus/ledger suite (unchanged by the
  cutover). See `dag.rs` and `reachability.rs` module docs.
- **In-memory working state**, with **replay-log persistence**: `Dag`/`Ledger`
  `write_snapshot`/`read_snapshot` serialise only `k`, the subsidy, and the blocks
  in topological order; loading replays inserts so all derived state (the
  reachability oracle, colouring, per-block UTXO state) is recomputed, never
  trusted from disk. There
  is no incremental on-disk store or mmap yet — a snapshot is written/read whole.
- **Linearization** is the recursive GHOSTDAG order:
  `order(B) = order(selected_parent(B)) ++ mergeset_order(B) ++ [B]`, unrolled over
  the selected chain and closed with the selected tip's anticone (the virtual
  block's mergeset). The selected chain is a subsequence and each merged block
  sits directly before its merger. Mergeset order within a block is a deterministic
  topological sort by `(past_size, id)` — a valid GHOSTDAG-spirit order, not Kaspa's
  exact blue-work mergeset tiebreak. See `ordering.rs` module docs.
- **State (`kovanica-state`)** applies transactions in GHOSTDAG order. Two views:
  `apply_dag` folds a finished DAG from scratch; `Ledger` maintains each block's
  view state incrementally from its selected parent (per-block state stored in
  full — an O(n²) memory trade-off; the DAG's own `past` sets have since been
  replaced by the reachability oracle, but the per-block UTXO state has not). A
  subsidy is a single per-block
  constant (no halving schedule); coinbase maturity is only "not spendable in the
  same block"; there are no tx size/weight limits. `Ledger::with_finality` prunes
  the per-block state of final blocks (more than `finality_depth` blue score below
  the tip) and rejects blocks built on final history; re-orgs above the finality
  point are implicit (`ledger_state` follows the selected tip, no revert). Pruning
  is of the per-block *state* only — the DAG stays append-only (DAG/`past`-set
  pruning waits on the reachability oracle). See `ledger.rs` module docs.
- **Insert-time validation** now has both layers. `Dag::with_validator` +
  `TxStructureValidator` reject malformed/structurally-invalid blocks; `Ledger`
  additionally runs the **stateful** rules (input existence, signatures, value
  conservation, coinbase amount) against a block's view state before it enters the
  DAG (via `Dag::preview`), so a block invalid in its own view is rejected at
  insert. Two *parallel* blocks that spend the same output are each valid in their
  own view and both admitted; their conflict resolves only in a merger's view.
- **Difficulty** now has both the algorithm and consensus enforcement. `Block`
  carries a `timestamp_ms` (in the canonical id encoding); there is still no
  proof-of-work, so `work` is caller-set *unless* difficulty is enabled.
  `difficulty::Retarget::next_work` is the retargeting *algorithm*;
  `Dag::set_difficulty(retarget)` opts a DAG into **enforcement**, after which
  `Dag::insert` requires every non-genesis block's `work` to equal
  `Dag::next_work_target(parents)` (the retarget over the last `window + 1` blocks
  of the selected-parent chain — a pure function of the DAG) and its timestamp not
  to precede any parent's. Genesis is exempt. `Ledger::set_difficulty` threads the
  same switch through the state layer, and the node mines produced blocks to
  `next_work_target` when it is set. Enforcement is **opt-in**, so a DAG built
  without it accepts any `work`, exactly as before. The wall-clock "not too far
  in the future" bound on timestamps is *not* here — that is node policy, not a
  pure function of the DAG. It lives in `kovanica-node` (`Node::receive_block`
  rejects a block whose `timestamp_ms` exceeds the node's clock by more than
  `MAX_FUTURE_DRIFT_MS` = 2h); the node's clock is injectable (`Node::set_now_ms`)
  so production timestamps and the bound are deterministic in tests, and produced
  blocks now stamp wall-clock now clamped monotone above their parents.

## 4. Build, test & run

Rust workspace (edition 2021, `rust-version` 1.75). From the repo root:

- **Build:** `cargo build`
- **Test (all: unit + integration + doctests):** `cargo test`
- **Single test:** `cargo test <name>` (e.g. `cargo test adversarial_wide_fork`)
- **Lint:** `cargo clippy --all-targets` (keep it warning-clean)
- **Format:** `cargo fmt` (CI-style check: `cargo fmt --check`)
- **Run the node:** `cargo run -p kovanica-node -- demo` (scripted end-to-end
  scenario) or `cargo run -p kovanica-node` (a `serve` REPL reading commands from
  stdin; try `help`).

`unsafe` code is **forbidden** crate-wide (`#![forbid(unsafe_code)]` via
`[lints.rust]`). `kovanica-dag`/`kovanica-state` are libraries; `kovanica-node`
is a library plus a binary (`serve`/`demo`).

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
- [x] Finality-depth pruning + re-orgs (`Ledger::with_finality`): prune the
      per-block state of final blocks, reject blocks built on final history, and
      follow the selected tip (implicit re-org, no revert).
- [x] Persistence: replay-log snapshots of the DAG and ledger
      (`Dag`/`Ledger` `write_snapshot`/`read_snapshot`) — state recomputed on load.
- [x] Reachability oracle (`reachability::Reachability`): interval-tree +
      future-covering sets, now the `Dag`'s backing for ancestor queries and
      mergeset computation. The per-block `past` sets are dropped (each block keeps
      only `past_size`); mergeset is a selected-parent-bounded backward walk. The
      oracle is rebuilt after each insert; incremental maintenance with interval
      **reindexing** (and the DAG-level pruning it unlocks) is the remaining
      optimisation.
- [ ] Incremental / streaming on-disk store (today a snapshot is written & read whole).
- [x] Runnable node binary + a line RPC over the ledger (`kovanica-node`:
      `serve`/`demo`, snapshot-backed).
- [x] Mempool + block production (`pool`/`produce`), and multi-node block
      dissemination — in-process `gossip` and a one-shot TCP pull sync — with
      nodes converging on the same DAG (conflicts resolved identically).
- [ ] Continuous p2p gossip: peer discovery, a relay loop, tx (not just block)
      dissemination; mempool eviction of permanently-invalid txs.
- [x] Difficulty adjustment for `work`: the retargeting algorithm
      (`difficulty::Retarget::next_work`) plus **consensus enforcement**. `Block`
      now carries a `timestamp_ms`; `Dag::set_difficulty` opts a DAG into
      validating each block's `work` against `Dag::next_work_target` (the retarget
      over the selected-parent chain — a pure function of the DAG) and its
      timestamp against its parents'. Threaded through `Ledger::set_difficulty` and
      the node's miner.
- [x] Wall-clock future-time bound on block timestamps (**node policy**, not
      pure-DAG): `Node::receive_block` rejects a block whose `timestamp_ms` is more
      than `MAX_FUTURE_DRIFT_MS` (2h) ahead of the node's clock. The clock is
      injectable (`Node::set_now_ms`) for deterministic tests; produced blocks
      stamp wall-clock now clamped monotone above their parents
      (`crates/kovanica-node/tests/timestamps.rs`).
