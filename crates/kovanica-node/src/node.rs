//! The node: an in-memory [`Ledger`] and [`Mempool`], plus the operations a node
//! offers — bring up a genesis, build/pack/submit spends, produce blocks, gossip
//! blocks with peers, query balances and tips, and save/load its state.
//!
//! For demonstration and testing, actors are identified by a small integer
//! *seed* — the node derives `KeyPair::from_u64(seed)` for them and signs on
//! their behalf (single-UTXO coin selection: it spends one existing output that
//! covers the amount and returns the change). A real node never holds spending
//! keys or does wallet work; that lives client-side. This keeps the binary a
//! runnable, self-contained demo of the whole stack.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use kovanica_dag::{pow, Block, BlockId, Dag, DagError};
use kovanica_state::{
    apply_block, decode_block_payload, encode_block_payload, Address, KeyPair, Ledger, LedgerError,
    LedgerInsertError, LedgerStore, OutPoint, Transaction, TxId, TxOutput,
};

use crate::mempool::Mempool;

/// How far ahead of the local wall clock a received block's timestamp may sit
/// before the node rejects it: two hours, in milliseconds. This is **node
/// policy**, not pure-DAG consensus — it depends on the local clock, so it lives
/// at the block-acceptance layer, not in [`kovanica_dag`]. (Bitcoin uses the
/// same two-hour future-time bound.)
const MAX_FUTURE_DRIFT_MS: u64 = 2 * 60 * 60 * 1000; // 2 hours

/// The node's source of wall-clock time. Injectable so production timestamps and
/// the future-time bound are deterministic in tests (and controllable in
/// simulated environments) — see [`Node::set_now_ms`].
#[derive(Default)]
enum Clock {
    /// Real UNIX wall-clock time.
    #[default]
    Wall,
    /// A pinned time in milliseconds since the UNIX epoch.
    Fixed(u64),
}

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
    /// A coinbase transaction was submitted where a spend was expected.
    UnexpectedCoinbase,
    /// Building the genesis ledger failed.
    Ledger(LedgerError),
    /// Submitting the block failed (structure or stateful validation).
    Insert(LedgerInsertError),
    /// Reading or writing the snapshot file failed.
    Io(String),
    /// Decoding a snapshot failed.
    Snapshot(String),
    /// A received block's timestamp is further ahead of the local wall clock than
    /// [`MAX_FUTURE_DRIFT_MS`] allows (node policy, not pure-DAG consensus).
    TimestampTooFarInFuture {
        /// The block's timestamp, in milliseconds.
        timestamp_ms: u64,
        /// The local wall-clock time it was checked against, in milliseconds.
        now_ms: u64,
    },
}

impl core::fmt::Display for NodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            NodeError::NotInitialized => f.write_str("no ledger yet — run `genesis` first"),
            NodeError::AlreadyInitialized => f.write_str("already initialised"),
            NodeError::ZeroAmount => f.write_str("amount must be non-zero"),
            NodeError::InsufficientFunds => f.write_str("no single output covers the amount"),
            NodeError::UnexpectedCoinbase => f.write_str("coinbase transactions are not accepted"),
            NodeError::Ledger(e) => write!(f, "genesis invalid: {e}"),
            NodeError::Insert(e) => write!(f, "{e}"),
            NodeError::Io(e) => write!(f, "io error: {e}"),
            NodeError::Snapshot(e) => write!(f, "bad snapshot: {e}"),
            NodeError::TimestampTooFarInFuture { timestamp_ms, now_ms } => write!(
                f,
                "block timestamp {timestamp_ms}ms is more than {MAX_FUTURE_DRIFT_MS}ms ahead of local time {now_ms}ms"
            ),
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

/// The wire form of a block for gossip: everything a peer needs to re-insert it.
#[derive(Clone, Debug)]
pub struct BlockRecord {
    /// The block's parents.
    pub parents: Vec<BlockId>,
    /// The block's work weight.
    pub work: u128,
    /// The block's timestamp, in milliseconds.
    pub timestamp_ms: u64,
    /// The block's proof-of-work nonce. Carried so a peer reconstructs the exact
    /// same id (and, under enforced PoW, the block still meets its target).
    pub nonce: u64,
    /// The block's transactions.
    pub txs: Vec<Transaction>,
}

/// A running node holding the ledger and mempool in memory.
#[derive(Default)]
pub struct Node {
    ledger: Option<Ledger>,
    mempool: Mempool,
    clock: Clock,
}

impl Node {
    /// A fresh node with no ledger yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The node's current wall-clock time in milliseconds since the UNIX epoch.
    /// With the default [`Clock::Wall`] this reads the system clock (returning 0
    /// if it is somehow before the epoch — no panic); a pinned clock returns its
    /// fixed value.
    fn now_ms(&self) -> u64 {
        match self.clock {
            Clock::Wall => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
            Clock::Fixed(n) => n,
        }
    }

    /// Pin the node's clock to a fixed time (milliseconds since the UNIX epoch),
    /// making both produced-block timestamps and the future-time bound
    /// deterministic. Primarily for tests and controlled/simulated environments;
    /// a real node runs on the default wall clock.
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.clock = Clock::Fixed(now_ms);
    }

    /// The timestamp to stamp on a new block built on `parents`: the node's
    /// wall-clock now, clamped up to stay strictly after the latest parent
    /// (genesis is at 0). The wall clock makes timestamps meaningful; the clamp
    /// keeps them monotone even if the clock is behind or a parent is ahead, so
    /// they still satisfy the difficulty layer's "not older than any parent" rule
    /// (see [`kovanica_dag::Dag::set_difficulty`]).
    fn next_timestamp(&self, dag: &Dag, parents: &[BlockId]) -> u64 {
        let floor = parents
            .iter()
            .filter_map(|p| dag.block(p).map(|b| b.timestamp_ms()))
            .max()
            .map_or(0, |latest| latest + 1);
        self.now_ms().max(floor)
    }

    /// The proof-of-work nonce to stamp on a new block built on `parents` with
    /// `work`, `timestamp_ms`, and transactions `txs`.
    ///
    /// When proof-of-work is enforced on the ledger's DAG, mine the block —
    /// Nakamoto-style hash-target search over the nonce
    /// ([`kovanica_dag::pow::mine`]) — so its id meets its `work` target and it
    /// passes insert; otherwise `0` (no mining). The template here must be
    /// byte-identical to the block [`Ledger::insert`] will build (same parents,
    /// work, timestamp, and payload encoding) so the winning nonce carries over
    /// to exactly the same id.
    fn mine_nonce(
        dag: &Dag,
        parents: &[BlockId],
        work: u128,
        timestamp_ms: u64,
        txs: &[Transaction],
    ) -> u64 {
        if !dag.proof_of_work_enabled() {
            return 0;
        }
        let template = Block::new(
            parents.to_vec(),
            work,
            timestamp_ms,
            0,
            encode_block_payload(txs),
        );
        pow::mine(&template).nonce()
    }

    /// Enable (or disable) consensus-enforced proof-of-work on the ledger. Once
    /// enabled, produced blocks are mined and received blocks must meet their
    /// target. See [`Ledger::set_proof_of_work`]. Errors if not initialised.
    pub fn set_proof_of_work(&mut self, enabled: bool) -> Result<(), NodeError> {
        self.ledger
            .as_mut()
            .ok_or(NodeError::NotInitialized)?
            .set_proof_of_work(enabled);
        Ok(())
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

    /// Number of pending transactions in the mempool.
    pub fn pending_count(&self) -> usize {
        self.mempool.len()
    }

    /// Build a signed transfer of `amount` from actor `from_seed` to actor
    /// `to_seed`, selecting one of the sender's outputs that covers it and
    /// returning the change. Does not touch the ledger or mempool.
    fn build_transfer(
        &self,
        from_seed: u64,
        amount: u64,
        to_seed: u64,
    ) -> Result<Transaction, NodeError> {
        if amount == 0 {
            return Err(NodeError::ZeroAmount);
        }
        let from = KeyPair::from_u64(from_seed);
        let from_addr = from.address();
        let to_addr = Self::address(to_seed);

        let state = self.ledger()?.ledger_state();
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
        Ok(Transaction::signed(
            &[(outpoint, &from)],
            outputs,
            Vec::new(),
        ))
    }

    /// Send `amount` from actor `from_seed` to actor `to_seed` **immediately**,
    /// as a new block built on the current tips. (For the mempool flow use
    /// [`Node::pool`] then [`Node::produce_block`].)
    pub fn send(&mut self, from_seed: u64, amount: u64, to_seed: u64) -> Result<Sent, NodeError> {
        let tx = self.build_transfer(from_seed, amount, to_seed)?;
        let tx_id = tx.id();
        // Compute parents / timestamp / work / nonce while holding the shared
        // borrow (the wall-clock timestamp needs `&self`, and mining needs the
        // DAG's PoW switch), before the `&mut` insert.
        let parents = self.ledger()?.dag().tips();
        let timestamp = self.next_timestamp(self.ledger()?.dag(), &parents);
        let dag = self.ledger()?.dag();
        let work = dag.next_work_target(&parents).unwrap_or(1);
        let nonce = Self::mine_nonce(dag, &parents, work, timestamp, std::slice::from_ref(&tx));
        let ledger = self.ledger.as_mut().ok_or(NodeError::NotInitialized)?;
        let block = ledger
            .insert(parents, work, timestamp, nonce, &[tx])
            .map_err(NodeError::Insert)?;
        self.evict_mempool();
        Ok(Sent { block, tx: tx_id })
    }

    /// Build a transfer and add it to the mempool (not yet in a block). Returns
    /// its transaction id.
    pub fn pool(&mut self, from_seed: u64, amount: u64, to_seed: u64) -> Result<TxId, NodeError> {
        let tx = self.build_transfer(from_seed, amount, to_seed)?;
        let id = tx.id();
        self.mempool.add(tx);
        Ok(id)
    }

    /// Accept an externally-formed transaction into the mempool (e.g. relayed by
    /// a peer). Rejects coinbase transactions. Returns its id.
    pub fn submit_tx(&mut self, tx: Transaction) -> Result<TxId, NodeError> {
        if tx.is_coinbase() {
            return Err(NodeError::UnexpectedCoinbase);
        }
        let id = tx.id();
        self.mempool.add(tx);
        Ok(id)
    }

    /// Assemble the largest valid prefix of the mempool into a block on the
    /// current tips, insert it, and drop the included transactions.
    ///
    /// Candidates are tried in deterministic (id) order against the current UTXO
    /// state; any that conflict are not included. After insert, the mempool
    /// evicts transactions whose inputs are gone from the selected-tip view
    /// (permanently invalid on this branch). Returns the new block id, or
    /// `None` if nothing could be included.
    pub fn produce_block(&mut self) -> Result<Option<BlockId>, NodeError> {
        if self.ledger.is_none() {
            return Err(NodeError::NotInitialized);
        }
        if self.mempool.is_empty() {
            return Ok(None);
        }

        let (subsidy, mut working) = {
            let ledger = self.ledger.as_ref().expect("checked above");
            (ledger.subsidy(), ledger.ledger_state())
        };
        let mut selected = Vec::new();
        let mut selected_ids = Vec::new();
        for tx in self.mempool.ordered() {
            // Apply each candidate on top of the ones already chosen; keep it only
            // if it holds, so the assembled block validates as a whole.
            if apply_block(&mut working, std::slice::from_ref(&tx), subsidy).is_ok() {
                selected_ids.push(tx.id());
                selected.push(tx);
            }
        }
        if selected.is_empty() {
            return Ok(None);
        }

        // Compute parents / timestamp / work / nonce under a shared borrow (the
        // wall-clock timestamp needs `&self`, and mining needs the DAG's PoW
        // switch) before taking the `&mut` insert.
        let dag = self.ledger.as_ref().expect("checked above").dag();
        let parents = dag.tips();
        let timestamp = self.next_timestamp(dag, &parents);
        let work = dag.next_work_target(&parents).unwrap_or(1);
        let nonce = Self::mine_nonce(dag, &parents, work, timestamp, &selected);
        let ledger = self.ledger.as_mut().expect("checked above");
        let block = ledger
            .insert(parents, work, timestamp, nonce, &selected)
            .map_err(NodeError::Insert)?;
        self.mempool.remove_all(&selected_ids);
        self.evict_mempool();
        Ok(Some(block))
    }

    /// A pending mempool transaction by id, if present.
    pub fn mempool_tx(&self, id: &TxId) -> Option<Transaction> {
        self.mempool.get(id)
    }

    fn evict_mempool(&mut self) {
        let Some(ledger) = self.ledger.as_ref() else {
            return;
        };
        let utxo = ledger.ledger_state();
        self.mempool.evict_invalid(&utxo);
    }

    /// The gossip record for a block, if present.
    pub fn block_record(&self, id: &BlockId) -> Option<BlockRecord> {
        let dag = self.ledger.as_ref()?.dag();
        let block = dag.block(id)?;
        let txs = decode_block_payload(block.payload()).ok()?;
        Some(BlockRecord {
            parents: block.parents().to_vec(),
            work: block.work(),
            timestamp_ms: block.timestamp_ms(),
            nonce: block.nonce(),
            txs,
        })
    }

    /// Every non-genesis block as a gossip record, in topological order — what a
    /// peer needs to catch up (genesis is shared out of band). Suitable to feed,
    /// in order, into [`Node::receive_block`] on another node.
    pub fn export(&self) -> Vec<BlockRecord> {
        let Some(ledger) = self.ledger.as_ref() else {
            return Vec::new();
        };
        let genesis = ledger.genesis();
        ledger
            .dag()
            .linearize()
            .into_iter()
            .filter(|id| *id != genesis)
            .filter_map(|id| self.block_record(&id))
            .collect()
    }

    /// Insert a block received from a peer. Idempotent: a block already present
    /// returns its id rather than an error. The block's parents must already be
    /// present (feed records in topological order).
    pub fn receive_block(&mut self, record: BlockRecord) -> Result<BlockId, NodeError> {
        // Node policy (not pure-DAG consensus): reject a block dated too far ahead
        // of our local wall clock. This depends on the local clock, so it cannot
        // live in `kovanica_dag`; it is applied here, before the ledger insert.
        let now_ms = self.now_ms();
        if record.timestamp_ms > now_ms.saturating_add(MAX_FUTURE_DRIFT_MS) {
            return Err(NodeError::TimestampTooFarInFuture {
                timestamp_ms: record.timestamp_ms,
                now_ms,
            });
        }
        let ledger = self.ledger.as_mut().ok_or(NodeError::NotInitialized)?;
        match ledger.insert(
            record.parents,
            record.work,
            record.timestamp_ms,
            record.nonce,
            &record.txs,
        ) {
            Ok(id) => {
                self.evict_mempool();
                Ok(id)
            }
            Err(LedgerInsertError::Dag(DagError::DuplicateBlock(id))) => Ok(id),
            Err(e) => Err(NodeError::Insert(e)),
        }
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

    /// Write an incremental append-only log of this node's ledger at `path`.
    pub fn create_log(&self, path: &str) -> Result<LedgerStore, NodeError> {
        LedgerStore::create(path, self.ledger()?).map_err(|e| NodeError::Io(e.to_string()))
    }

    /// Rebuild the node from an incremental log at `path`. The store is
    /// returned so the caller can [`persist_block`](Self::persist_block) new
    /// inserts without rewriting the file.
    pub fn load_log(path: &str) -> Result<(Self, LedgerStore), NodeError> {
        let (store, ledger) =
            LedgerStore::open(path).map_err(|e| NodeError::Snapshot(e.to_string()))?;
        Ok((
            Self {
                ledger: Some(ledger),
                mempool: Mempool::new(),
                clock: Clock::default(),
            },
            store,
        ))
    }

    /// Append `id`'s block to an open log. No-op-level error if the block is
    /// missing (it must already be in this node).
    pub fn persist_block(&self, store: &mut LedgerStore, id: &BlockId) -> Result<(), NodeError> {
        let block = self
            .ledger()?
            .dag()
            .block(id)
            .ok_or_else(|| NodeError::Io("unknown block".into()))?;
        store
            .append(block)
            .map_err(|e| NodeError::Io(e.to_string()))
    }
}
