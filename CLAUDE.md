# CLAUDE.md

Guidance for AI assistants (and humans) working in the **kovanica-ledger** repository.

> **Status: greenfield.** As of this file's creation the repository contains no
> source code, build files, or history beyond this document. Sections describing
> concrete implementation details are marked **TODO** and MUST be filled in from
> the real code as it lands — do not treat the aspirational parts of this file as
> established fact, and update this file in the same change that introduces the
> structure it describes.

---

## 1. What this project is

**kovanica-ledger** is a **DAG-based distributed ledger** — a high-throughput,
parallel-block cryptocurrency/ledger protocol built on a **Directed Acyclic Graph**
(BlockDAG / TxDAG) rather than a single linear chain. The name *kovanica* is
Serbo-Croatian for "coin / mint," and *ledger* signals the distributed-ledger
core.

The design space this project draws from — and the vocabulary the codebase and
docs are expected to use — is the modern DAG-ledger and DAG-BFT literature:

- **BlockDAG / GHOSTDAG family:** Kaspa's GHOSTDAG, and its theoretical
  predecessors PHANTOM and SPECTRE (k-cluster, *blue set* / *red set*,
  blue-work ordering, turning a partial order into a total order).
- **Tangle:** IOTA's DAG-of-transactions model (tip selection, cumulative weight).
- **aBFT / hashgraph:** Hedera Hashgraph (gossip-about-gossip, virtual voting).
- **Lachesis / Sonic:** Fantom's aBFT DAG consensus.
- **Snow family:** Avalanche (Snowball / Snowman metastable sampling consensus).
- **Parallel-chain / sharded-DAG:** Conflux (GHOST + pivot chain), OHIE.
- **DAG-BFT mempool + ordering:** Narwhal–Tusk, Bullshark, Mysticeti, Sailfish —
  separating **data dissemination (mempool DAG)** from **consensus/ordering**.

If you are researching, designing, implementing, evaluating, or comparing any
part of the consensus/ordering layer, ground your reasoning in these systems and
name the mechanism you are borrowing (e.g. "GHOSTDAG-style blue-set selection,"
"Bullshark-style DAG-round wave commit").

## 2. Core domain concepts (shared vocabulary)

Keep these terms precise and consistent across code, comments, and docs:

- **DAG / BlockDAG / TxDAG** — the ledger is a directed acyclic graph; a block
  (or transaction) may reference **multiple parents/tips**, enabling parallel
  block creation and high BPS (blocks per second).
- **Tip** — a DAG vertex with no children yet; new blocks reference current tips.
- **Partial order → total order** — the DAG is a partial order; consensus's job is
  to deterministically **linearize** it into a total order every honest node agrees on.
- **Blue set / red set (PHANTOM/GHOSTDAG)** — the well-connected "honest-looking"
  cluster (blue) vs. blocks left out (red); ordering follows accumulated blue work.
- **k-cluster parameter (k)** — anticone-size bound; a security/throughput knob.
- **Anticone** — the set of blocks not ordered relative to a given block (neither
  ancestor nor descendant).
- **Wave / round (DAG-BFT)** — Narwhal/Bullshark/Mysticeti structure the DAG into
  rounds; consensus commits *anchors* to derive a total order.
- **Finality** — probabilistic (Nakamoto/Tangle/Avalanche) vs. deterministic/BFT
  (Hashgraph/Lachesis/Bullshark). State which model any given component provides.

## 3. Repository layout

**TODO — populate once code exists.** When adding the first real modules, replace
this section with the actual tree and a one-line purpose per top-level directory.
A likely shape for a DAG ledger (adjust to reality — do **not** create empty dirs
to match this):

```
/            TODO: language/build manifest (Cargo.toml / go.mod / package.json / pyproject.toml)
/consensus   TODO: DAG ordering & finality (GHOSTDAG-style selection, blue-set, linearization)
/dag         TODO: graph data structure, tips, parents, anticone/ancestry queries
/mempool     TODO: transaction pool / data dissemination layer
/network     TODO: p2p gossip, block/tx propagation
/ledger      TODO: UTXO or account state, validation, pruning
/crypto      TODO: hashing, signatures, VRF/sortition if used
/node        TODO: node binary, config, RPC/API
/tests       TODO: unit, property, and adversarial/consensus-safety tests
```

## 4. Build, test & run

**TODO — no build system yet.** Once the stack is chosen, document the exact
commands here so an assistant can build and validate without guessing. Fill in:

- **Language / toolchain & version:** _TODO_
- **Build:** _TODO (e.g. `cargo build`, `go build ./...`, `npm run build`)_
- **Test:** _TODO — include how to run the full suite AND a single test_
- **Lint / format / typecheck:** _TODO_
- **Run a node locally / devnet:** _TODO_

Until this section is real, **verify commands before claiming they work**, and
never report a build or test as passing that you did not actually run.

## 5. Engineering conventions

- **Correctness of consensus is paramount.** Changes to DAG ordering, blue-set
  selection, finality, or the partial-order→total-order linearization can break
  safety (double-spend) or liveness. Such changes require: a written rationale
  referencing the protocol being followed, and **deterministic/adversarial tests**
  (Byzantine parents, equivocation, network partition, tip flooding).
- **Determinism.** Node linearization must be reproducible across nodes given the
  same DAG. Avoid nondeterministic iteration order, wall-clock dependence in
  ordering, and unstable sorts in consensus paths.
- **Name your source protocol** in comments when implementing a known mechanism,
  so reviewers can check it against the paper.
- **Match surrounding style** once code exists (naming, error handling, comment
  density). Until then, keep new code idiomatic for the chosen language.

## 6. Git workflow

- **Never commit to the default branch directly.** Develop on a feature branch and
  open a **draft PR**.
- **Branch naming:** short, kebab-case, scoped (e.g. `consensus/ghostdag-blue-set`,
  `dag/anticone-query`, `claude/<topic>` for assistant-driven work).
- **Commits:** clear, imperative subject lines describing *why*, not just *what*.
  Keep consensus-affecting changes in focused commits.
- **Push:** `git push -u origin <branch-name>`; open a PR if none exists for the branch.
- Because this repo currently has **no default branch / no history**, the first
  merged PR effectively establishes `main`. Confirm the default branch name after
  the first merge and update the "default branch" reference here if it isn't `main`.

## 7. For AI assistants — working notes

- This file is the **source of truth for conventions**; when reality and this file
  disagree, fix the code or fix this file — don't silently diverge. Update CLAUDE.md
  in the same PR that changes structure, build, or workflow.
- **Do not invent** APIs, module paths, or commands. If a section here is still
  TODO, say so rather than fabricating specifics.
- When reasoning about consensus/ordering, cite the concrete reference system
  (GHOSTDAG, Bullshark, Avalanche, Hashgraph, …) so the design is auditable.
- Prefer property-based and adversarial tests for graph/consensus code over
  example-based tests alone.

---

## Maintenance checklist (remove items as they're done)

- [ ] Choose language/toolchain; add build manifest and fill in **§4 Build/test/run**.
- [ ] Replace **§3 Repository layout** with the real tree.
- [ ] Document the chosen consensus mechanism and finality model concretely.
- [ ] Confirm the default branch name and update **§6**.
- [ ] Remove the "greenfield" banner once the codebase is real.
