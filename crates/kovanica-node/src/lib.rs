//! # kovanica-node
//!
//! A runnable node for the Kovanica DAG ledger. It ties the stack together —
//! `kovanica_dag` (the block DAG + GHOSTDAG consensus) and `kovanica_state` (the
//! UTXO [`Ledger`](kovanica_state::Ledger), signatures, per-block state,
//! snapshots) — behind a small, testable interface.
//!
//! * [`Node`] holds the ledger in memory and exposes the high-level operations:
//!   bring up a genesis, submit signed transfers, query balances / tips, and
//!   save or load a snapshot.
//! * [`rpc::execute_line`] is a line-based command protocol (string in, string
//!   out) — the node's "RPC". The binary wires it to stdin/stdout (`serve`) or
//!   replays a scripted `demo`.
//!
//! Nodes are multi-node aware: a [`Mempool`](mempool::Mempool) holds pending
//! transactions, [`Node::produce_block`] packs the valid ones into a block, and
//! blocks gossip between nodes ([`net::gossip`] in-process, or
//! [`net::serve_blocks`] / [`net::pull_blocks`] over TCP) so peers converge on
//! the same DAG. A node may declare an [`Origin`](origin::Origin) (ISO 3166-1
//! alpha-3) so operators can track where users come from; that announcement is
//! **node policy**, never consensus. There is no peer discovery or continuous
//! gossip loop yet (later slices). Actors are integer *seeds* the node signs
//! for — a demo convenience, not how a real node handles keys (see [`node`]).
//!
//! ```
//! use kovanica_node::{rpc, Node};
//!
//! let mut node = Node::new();
//! // Genesis mints 500 to actor 1; actor 1 sends 200 to actor 2.
//! assert!(rpc::execute_line(&mut node, "genesis 3 1000 500 1").starts_with("ok genesis"));
//! assert!(rpc::execute_line(&mut node, "send 1 200 2").starts_with("ok block"));
//! assert_eq!(rpc::execute_line(&mut node, "balance 1"), "ok 300");
//! assert_eq!(rpc::execute_line(&mut node, "balance 2"), "ok 200");
//! // Geographic origin is node policy: set it, list observed pulses (none yet).
//! assert_eq!(rpc::execute_line(&mut node, "origin HRV"), "ok HRV");
//! assert_eq!(rpc::execute_line(&mut node, "origins"), "ok");
//! ```

pub mod mempool;
pub mod net;
pub mod node;
pub mod origin;
pub mod rpc;

pub use mempool::Mempool;
pub use net::NetError;
pub use node::{BlockRecord, Node, NodeError, Sent};
pub use origin::{Origin, OriginError};
