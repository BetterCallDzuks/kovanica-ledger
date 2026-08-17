# Kovanica Ledger

A **DAG-based distributed ledger**: a BlockDAG where blocks reference multiple
parents so they can be produced in parallel and merged, rather than forming a
single linear chain. Consensus follows **GHOSTDAG** (the PHANTOM/GHOSTDAG
protocol behind Kaspa).

> Early stage. The block DAG and the GHOSTDAG consensus core are implemented and
> tested; higher layers (state, networking, node) are not built yet. See
> [`CLAUDE.md`](./CLAUDE.md) for architecture, conventions, and roadmap.

## What's here

`crates/kovanica-dag` — the consensus core:

- **Block DAG** with multi-parent blocks and BLAKE3 block ids.
- **GHOSTDAG**: selected parent, mergeset, and the k-cluster blue/red colouring
  that identifies the well-connected cluster and bounds an attacker's influence.
- **Linearization**: a deterministic total order over the whole DAG, plus the
  selected (heaviest) chain.

## Build & test

```sh
cargo build
cargo test            # unit + integration + doctests
cargo clippy --all-targets
```

## Example

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

## License

Dual-licensed under MIT or Apache-2.0.
