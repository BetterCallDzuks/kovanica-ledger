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
//! This is a single-process node: there is no p2p gossip yet (a later slice), so
//! it maintains one local DAG. Actors are integer *seeds* the node signs for —
//! a demo convenience, not how a real node handles keys (see [`node`]).
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
//! ```

pub mod node;
pub mod rpc;

pub use node::{Node, NodeError, Sent};
