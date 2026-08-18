# Kovanica Ledger

A **DAG-based distributed ledger**: a BlockDAG where blocks reference multiple
parents so they can be produced in parallel and merged, rather than forming a
single linear chain. Consensus follows **GHOSTDAG** (the PHANTOM/GHOSTDAG
protocol behind Kaspa).

> Early stage. The block DAG + GHOSTDAG consensus core, a UTXO ledger applied in
> GHOSTDAG order (with per-block state and snapshot persistence), and a runnable
> single-node binary are implemented and tested; p2p networking is not built yet.
> See [`CLAUDE.md`](./CLAUDE.md) for architecture, conventions, and roadmap.

## What's here

`crates/kovanica-dag` — the consensus core:

- **Block DAG** with multi-parent blocks and BLAKE3 block ids.
- **GHOSTDAG**: selected parent, mergeset, and the k-cluster blue/red colouring
  that identifies the well-connected cluster and bounds an attacker's influence.
- **Linearization**: a deterministic total order over the whole DAG, plus the
  selected (heaviest) chain.

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

`crates/kovanica-node` — a runnable single-node binary tying the stack together
behind a small line RPC (`serve` reads commands from stdin; `demo` replays a
scripted scenario). Snapshot-backed via `save`/`load`. No p2p peers yet.

## Run the node

```sh
cargo run -p kovanica-node -- demo   # scripted end-to-end scenario
cargo run -p kovanica-node           # REPL: type commands, `help`, `quit`
```

```text
> genesis 3 1000 500 1     # mint 500 to actor 1 (k=3, subsidy=1000)
> send 1 200 2             # actor 1 sends 200 to actor 2 (as a new block)
> balance 1                # ok 300
> balance 2                # ok 200
> save ledger.snap         # persist; `load ledger.snap` restores it
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
