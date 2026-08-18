# Kovanica Ledger

A **DAG-based distributed ledger**: a BlockDAG where blocks reference multiple
parents so they can be produced in parallel and merged, rather than forming a
single linear chain. Consensus follows **GHOSTDAG** (the PHANTOM/GHOSTDAG
protocol behind Kaspa).

> Early stage. The block DAG + GHOSTDAG consensus core, a UTXO ledger applied in
> GHOSTDAG order (with per-block state and snapshot persistence), and a runnable
> node binary with a mempool and multi-node block gossip are implemented and
> tested; continuous p2p (peer discovery, relay) is not built yet. See
> [`CLAUDE.md`](./CLAUDE.md) for architecture, conventions, and roadmap.

## What's here

`crates/kovanica-dag` — the consensus core:

- **Block DAG** with multi-parent blocks and BLAKE3 block ids.
- **GHOSTDAG**: selected parent, mergeset, and the k-cluster blue/red colouring
  that identifies the well-connected cluster and bounds an attacker's influence.
- **Linearization**: a deterministic total order over the whole DAG, plus the
  selected (heaviest) chain.
- **Difficulty** (`difficulty::Retarget` + `Dag::set_difficulty`): computes the
  `work` the next block should target from the timestamps + work of recent blocks
  to hold a steady block rate, and **enforces it in consensus** — with difficulty
  enabled, `Dag::insert` requires each block's `work` to equal
  `Dag::next_work_target` and its `timestamp` not to precede any parent's.
  Enforcement is opt-in; blocks now carry a `timestamp`.
- **Reachability oracle** (`reachability::Reachability`): an interval-tree +
  future-covering-set index for O(1) ancestor queries — now the DAG's backing for
  ancestor and mergeset computation, so the O(n²) per-block `past` sets are gone
  (each block keeps only its `past_size` count). **Maintained incrementally** —
  each insert folds in just the one new block (Kaspa reachability / interval
  reindexing) instead of rebuilding. Differentially verified against an
  independent naive parent-walk and against a freshly-built oracle after every
  insert.

`crates/kovanica-state` — the UTXO ledger:

- **Transactions** in the UTXO model (inputs spend previous outputs; coinbase
  transactions mint), with a canonical encoding that doubles as a block payload.
- **Ed25519 spend authorisation**: each input is signed; spends verify against the
  spent output's owner.
- **State transition**: transactions are applied in the DAG's GHOSTDAG-linearized
  order, so a double-spend split across parallel blocks is resolved
  deterministically — the block that wins the linearization spends the output;
  the loser is rejected.
- **Per-block UTXO state** (`Ledger`): each block's view state is maintained
  incrementally from its selected parent, so a block invalid in its own view
  (double-spending an ancestor, a bad signature) is rejected *at insert* and never
  enters the DAG. The full state matches the batch `apply_dag`.
- **Persistence**: `write_snapshot`/`read_snapshot` on both the `Dag` and the
  `Ledger` serialise a compact replay log (blocks in topological order); loading
  recomputes all consensus and UTXO state, so nothing derived is trusted from disk.
- **Finality & re-orgs** (`Ledger::with_finality`): blocks more than a finality
  depth below the selected tip are final — their per-block state is pruned and
  they can't be built on (a deep re-org is rejected). Above the finality point the
  current state simply follows the selected tip, so a heavier branch takes over
  with no explicit revert.

`crates/kovanica-node` — a runnable node tying the stack together behind a small
line RPC (`serve` reads commands from stdin; `demo` replays a scripted scenario).
A **mempool** queues transfers (`pool`) that `produce` packs into a block, and
nodes exchange blocks to converge on one DAG — in-process (`net::gossip`) or over
TCP (`net::serve_blocks` / `pull_blocks`). Produced blocks are stamped with the
node's wall clock (clamped monotone above their parents), and `receive_block`
rejects a peer's block dated more than two hours ahead of local time — node
policy, not pure-DAG consensus. Snapshot-backed via `save`/`load`. Continuous p2p
(peer discovery, relay) is not built yet.

## Run the node

```sh
cargo run -p kovanica-node -- demo   # scripted end-to-end scenario
cargo run -p kovanica-node           # REPL: type commands, `help`, `quit`
```

```text
> genesis 3 1000 500 1     # mint 500 to actor 1 (k=3, subsidy=1000)
> send 1 200 2             # immediate block: actor 1 sends 200 to actor 2
> pool 2 50 3              # queue a transfer in the mempool
> produce                  # pack the mempool into a block
> balance 3                # ok 50
> save ledger.snap         # persist; `load ledger.snap` restores it
```

Two `Node`s that share a genesis converge by exchanging blocks:

```rust
use kovanica_node::{net, Node};

let mut a = Node::new(); a.genesis(3, 1000, 1000, 1).unwrap();
let mut b = Node::new(); b.genesis(3, 1000, 1000, 1).unwrap();
a.send(1, 400, 2).unwrap();      // a produces a block
net::gossip(&a, &mut b).unwrap(); // b catches up
assert_eq!(b.balance(&Node::address(2)).unwrap(), 400);
```

## Build & test

```sh
cargo build
cargo test            # unit + integration + doctests
cargo clippy --all-targets
```

## Example — consensus (`kovanica-dag`)

```rust
use kovanica_dag::{Block, Dag};

let genesis = Block::genesis(1, b"kovanica-genesis".to_vec());
let genesis_id = genesis.id();
let mut dag = Dag::new(3, genesis); // k = 3

let a = dag.insert(Block::new(vec![genesis_id], 1, b"a".to_vec())).unwrap();
let b = dag.insert(Block::new(vec![genesis_id], 1, b"b".to_vec())).unwrap();
let c = dag.insert(Block::new(vec![a, b], 1, b"c".to_vec())).unwrap();

assert_eq!(dag.ghostdag(&c).unwrap().blue_score, 3); // genesis + a + b
let order = dag.linearize();                          // deterministic total order
```

## Example — ledger (`kovanica-state`)

Blocks carry transactions; consensus orders them; the ledger applies them.

```rust
use kovanica_dag::{Block, Dag};
use kovanica_state::{apply_dag, encode_block_payload, KeyPair, OutPoint, Transaction, TxOutput};

let miner = KeyPair::from_u64(1);
let alice = KeyPair::from_u64(2);

// Genesis coinbase mints 100 to the miner.
let coinbase = Transaction::coinbase(vec![TxOutput::new(100, miner.address())], b"genesis".to_vec());
let coin = OutPoint::new(coinbase.id(), 0);
let genesis = Block::genesis(1, encode_block_payload(&[coinbase]));
let genesis_id = genesis.id();
let mut dag = Dag::new(3, genesis);

// The miner sends 100 to Alice, spending the coinbase output.
let pay = Transaction::signed(&[(coin, &miner)], vec![TxOutput::new(100, alice.address())], vec![]);
dag.insert(Block::new(vec![genesis_id], 1, encode_block_payload(&[pay]))).unwrap();

let run = apply_dag(&dag, 100); // subsidy = 100 per block
assert_eq!(run.utxo.balance(&alice.address()), 100);
```

## License

Dual-licensed under MIT or Apache-2.0.
