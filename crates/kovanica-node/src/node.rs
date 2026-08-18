//! The node: an in-memory [`Ledger`] plus the high-level operations a node
//! offers (bring up a genesis, submit spends, query balances and tips, and
//! save/load its state).
//!
//! For demonstration and testing, actors are identified by a small integer
//! *seed* — the node derives `KeyPair::from_u64(seed)` for them and signs on
//! their behalf (single-UTXO coin selection: it spends one existing output that
//! covers the amount and returns the change). A real node never holds spending
//! keys or does wallet work; that lives client-side. This keeps the binary a
//! runnable, self-contained demo of the whole stack.

use std::fs;

use kovanica_dag::BlockId;
use kovanica_state::{
    Address, KeyPair, Ledger, LedgerError, LedgerInsertError, OutPoint, Transaction, TxId, TxOutput,
};

/// Why a node operation failed.
#[derive(Debug)]
pub enum NodeError {
    /// An operation needed a ledger, but no genesis has been created yet.
    NotInitialized,
    /// `genesis` was called on an already-initialised node.
    AlreadyInitialized,
    /// A spend of zero value was requested.
    ZeroAmount,
    /// No single unspent output owned by the sender covers the amount (this node
    /// does not combine multiple outputs).
    InsufficientFunds,
    /// Building the genesis ledger failed.
    Ledger(LedgerError),
    /// Submitting the block failed (structure or stateful validation).
    Insert(LedgerInsertError),
    /// Reading or writing the snapshot file failed.
    Io(String),
    /// Decoding a snapshot failed.
    Snapshot(String),
}

impl core::fmt::Display for NodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            NodeError::NotInitialized => f.write_str("no ledger yet — run `genesis` first"),
            NodeError::AlreadyInitialized => f.write_str("already initialised"),
            NodeError::ZeroAmount => f.write_str("amount must be non-zero"),
            NodeError::InsufficientFunds => f.write_str("no single output covers the amount"),
            NodeError::Ledger(e) => write!(f, "genesis invalid: {e}"),
            NodeError::Insert(e) => write!(f, "{e}"),
            NodeError::Io(e) => write!(f, "io error: {e}"),
            NodeError::Snapshot(e) => write!(f, "bad snapshot: {e}"),
        }
    }
}

impl std::error::Error for NodeError {}

/// The result of a successful [`Node::send`]: the block that carried the spend
/// and the transaction's id.
#[derive(Clone, Copy, Debug)]
pub struct Sent {
    /// Id of the block that was inserted.
    pub block: BlockId,
    /// Id of the transfer transaction.
    pub tx: TxId,
}

/// A running node holding the ledger in memory.
#[derive(Default)]
pub struct Node {
    ledger: Option<Ledger>,
}

impl Node {
    /// A fresh node with no ledger yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a genesis has been created.
    pub fn is_initialized(&self) -> bool {
        self.ledger.is_some()
    }

    /// The address the node uses for actor `seed`.
    pub fn address(seed: u64) -> Address {
        KeyPair::from_u64(seed).address()
    }

    /// Bring up the ledger: a genesis block whose coinbase mints `amount` to
    /// actor `founder_seed`, with GHOSTDAG parameter `k` and per-block `subsidy`.
    /// Returns the genesis block id and the founder's address.
    pub fn genesis(
        &mut self,
        k: u16,
        subsidy: u64,
        amount: u64,
        founder_seed: u64,
    ) -> Result<(BlockId, Address), NodeError> {
        if self.ledger.is_some() {
            return Err(NodeError::AlreadyInitialized);
        }
        let founder = Self::address(founder_seed);
        let coinbase =
            Transaction::coinbase(vec![TxOutput::new(amount, founder)], b"genesis".to_vec());
        let ledger = Ledger::new(k, subsidy, &[coinbase]).map_err(NodeError::Ledger)?;
        let genesis = ledger.genesis();
        self.ledger = Some(ledger);
        Ok((genesis, founder))
    }

    fn ledger(&self) -> Result<&Ledger, NodeError> {
        self.ledger.as_ref().ok_or(NodeError::NotInitialized)
    }

    /// The spendable balance of `owner` in the current full ledger state.
    pub fn balance(&self, owner: &Address) -> Result<u128, NodeError> {
        Ok(self.ledger()?.ledger_state().balance(owner))
    }

    /// The current tips.
    pub fn tips(&self) -> Result<Vec<BlockId>, NodeError> {
        Ok(self.ledger()?.dag().tips())
    }

    /// The selected (heaviest) tip.
    pub fn selected_tip(&self) -> Result<BlockId, NodeError> {
        Ok(self.ledger()?.dag().selected_tip())
    }

    /// Number of blocks in the DAG (including genesis).
    pub fn block_count(&self) -> Result<usize, NodeError> {
        Ok(self.ledger()?.dag().len())
    }

    /// Send `amount` from actor `from_seed` to actor `to_seed`, as a new block
    /// built on the current tips. Selects one existing output of the sender that
    /// covers the amount and returns the change to the sender.
    pub fn send(&mut self, from_seed: u64, amount: u64, to_seed: u64) -> Result<Sent, NodeError> {
        if amount == 0 {
            return Err(NodeError::ZeroAmount);
        }
        let from = KeyPair::from_u64(from_seed);
        let from_addr = from.address();
        let to_addr = Self::address(to_seed);

        let ledger = self.ledger.as_mut().ok_or(NodeError::NotInitialized)?;
        let state = ledger.ledger_state();

        // Best-fit: the smallest output of the sender that covers `amount`, with a
        // deterministic tie-break on the outpoint so selection is reproducible.
        let mut candidates: Vec<(OutPoint, u64)> = state
            .iter()
            .filter(|(_, out)| out.owner == from_addr && out.value >= amount)
            .map(|(op, out)| (*op, out.value))
            .collect();
        candidates.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        let (outpoint, value) = *candidates.first().ok_or(NodeError::InsufficientFunds)?;

        let mut outputs = vec![TxOutput::new(amount, to_addr)];
        if value > amount {
            outputs.push(TxOutput::new(value - amount, from_addr));
        }
        let tx = Transaction::signed(&[(outpoint, &from)], outputs, Vec::new());
        let tx_id = tx.id();

        let parents = ledger.dag().tips();
        let block = ledger
            .insert(parents, 1, &[tx])
            .map_err(NodeError::Insert)?;
        Ok(Sent { block, tx: tx_id })
    }

    /// Write the ledger snapshot to `path`.
    pub fn save(&self, path: &str) -> Result<(), NodeError> {
        let bytes = self.ledger()?.write_snapshot();
        fs::write(path, bytes).map_err(|e| NodeError::Io(e.to_string()))
    }

    /// Replace the node's ledger with one loaded from the snapshot at `path`.
    pub fn load(&mut self, path: &str) -> Result<(), NodeError> {
        let bytes = fs::read(path).map_err(|e| NodeError::Io(e.to_string()))?;
        let ledger =
            Ledger::read_snapshot(&bytes).map_err(|e| NodeError::Snapshot(e.to_string()))?;
        self.ledger = Some(ledger);
        Ok(())
    }
}
