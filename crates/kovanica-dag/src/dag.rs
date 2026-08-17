//! The append-only block DAG store and its GHOSTDAG metadata.
//!
//! The [`Dag`] owns every block plus the consensus data GHOSTDAG derives for it
//! (see [`crate::ghostdag`]) and the total order it induces (see
//! [`crate::ordering`]). Blocks are inserted one at a time; each insert is
//! validated and immediately coloured, so the store always holds a fully
//! processed DAG.
//!
//! ## First-slice simplifications (intentional, documented as TODO)
//!
//! * Reachability is answered from a per-block `past` set stored in full. This
//!   is O(1) per query but O(n²) memory. A production node replaces this with a
//!   reachability oracle (interval labels), tracked in `CLAUDE.md`.
//! * Everything lives in memory; there is no persistence layer yet.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::block::{Block, BlockId};
use crate::validation::BlockValidator;

/// Errors returned when inserting a block into the [`Dag`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DagError {
    /// A block with this id is already present.
    DuplicateBlock(BlockId),
    /// A referenced parent is not in the DAG.
    MissingParent(BlockId),
    /// A non-genesis block referenced no parents.
    NoParents(BlockId),
    /// `insert_genesis` was called on a DAG that already has a genesis.
    GenesisAlreadySet,
    /// The installed [`BlockValidator`] rejected the block, with its reason.
    InvalidBlock { id: BlockId, reason: String },
}

impl core::fmt::Display for DagError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DagError::DuplicateBlock(id) => write!(f, "duplicate block {id}"),
            DagError::MissingParent(id) => write!(f, "missing parent {id}"),
            DagError::NoParents(id) => write!(f, "non-genesis block {id} has no parents"),
            DagError::GenesisAlreadySet => write!(f, "genesis already set"),
            DagError::InvalidBlock { id, reason } => {
                write!(f, "block {id} rejected by validator: {reason}")
            }
        }
    }
}

impl std::error::Error for DagError {}

/// The `k` parameter of GHOSTDAG: the maximum tolerated blue anticone size.
///
/// It bounds how many well-connected blocks may be mutually "parallel" (in each
/// other's anticone) while still all counting as blue. Larger `k` tolerates
/// higher block rates / latency at the cost of a wider security margin.
pub type KParam = u16;

/// Consensus metadata GHOSTDAG derives for a single block.
///
/// A block's *blue set* is the set of blue blocks in its past. It is built from
/// its selected parent's blue set plus the blues found in its mergeset.
#[derive(Clone, Debug)]
pub struct GhostdagData {
    /// The parent with the heaviest blue work (see [`Dag::chain_key`]).
    /// `None` only for genesis.
    pub selected_parent: Option<BlockId>,
    /// Mergeset blocks coloured blue, in the topological order they were added.
    pub mergeset_blues: Vec<BlockId>,
    /// Mergeset blocks coloured red, in topological order.
    pub mergeset_reds: Vec<BlockId>,
    /// Size of this block's blue set (number of blue blocks in its past).
    pub blue_score: u64,
    /// Total work of this block's blue set.
    pub blue_work: u128,
    /// For each blue block in this block's blue set, the number of blue blocks
    /// in *its* anticone (restricted to this blue set). The invariant GHOSTDAG
    /// maintains is that every value here is `<= k`.
    pub blue_anticone_sizes: HashMap<BlockId, KParam>,
}

/// The GHOSTDAG data a block *would* receive, computed by [`Dag::preview`]
/// without inserting the block.
#[derive(Clone, Debug)]
pub struct BlockPreview {
    /// The parent that would be the block's selected parent.
    pub selected_parent: BlockId,
    /// The block's mergeset, in the deterministic order the linearization uses.
    pub mergeset: Vec<BlockId>,
}

/// A stored block: the block itself plus derived DAG/consensus data.
pub(crate) struct Node {
    pub(crate) block: Block,
    /// Strict ancestors of this block (its past), stored in full — see the
    /// module docs for the memory trade-off.
    pub(crate) past: HashSet<BlockId>,
    /// Direct children, for tip maintenance and total-order traversal.
    pub(crate) children: BTreeSet<BlockId>,
    pub(crate) ghostdag: GhostdagData,
}

/// An append-only block DAG with GHOSTDAG consensus data.
pub struct Dag {
    k: KParam,
    genesis: BlockId,
    pub(crate) nodes: HashMap<BlockId, Node>,
    /// Blocks with no children yet — the current tips.
    tips: BTreeSet<BlockId>,
    /// Optional payload-aware validator run on each [`Dag::insert`]. See
    /// [`crate::validation`].
    validator: Option<Box<dyn BlockValidator>>,
}

impl Dag {
    /// Create a DAG seeded with `genesis` and the GHOSTDAG parameter `k`.
    pub fn new(k: KParam, genesis: Block) -> Self {
        let genesis_id = genesis.id();
        let mut nodes = HashMap::new();
        nodes.insert(
            genesis_id,
            Node {
                block: genesis,
                past: HashSet::new(),
                children: BTreeSet::new(),
                ghostdag: GhostdagData {
                    selected_parent: None,
                    mergeset_blues: Vec::new(),
                    mergeset_reds: Vec::new(),
                    blue_score: 0,
                    blue_work: 0,
                    blue_anticone_sizes: HashMap::new(),
                },
            },
        );
        let mut tips = BTreeSet::new();
        tips.insert(genesis_id);
        Self {
            k,
            genesis: genesis_id,
            nodes,
            tips,
            validator: None,
        }
    }

    /// Create a DAG as [`Dag::new`] but with a [`BlockValidator`] installed, so
    /// every subsequent [`Dag::insert`] must pass `validator`. The genesis block
    /// itself is not validated.
    pub fn with_validator(k: KParam, genesis: Block, validator: Box<dyn BlockValidator>) -> Self {
        let mut dag = Self::new(k, genesis);
        dag.validator = Some(validator);
        dag
    }

    /// Install (or replace) the block validator run on every [`Dag::insert`].
    pub fn set_validator(&mut self, validator: Box<dyn BlockValidator>) {
        self.validator = Some(validator);
    }

    /// The GHOSTDAG `k` parameter.
    pub fn k(&self) -> KParam {
        self.k
    }

    /// The genesis block id.
    pub fn genesis(&self) -> BlockId {
        self.genesis
    }

    /// Number of blocks in the DAG (including genesis).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the DAG only contains genesis.
    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    /// Whether a block is present.
    pub fn contains(&self, id: &BlockId) -> bool {
        self.nodes.contains_key(id)
    }

    /// The current tips (blocks with no children), sorted by id.
    pub fn tips(&self) -> Vec<BlockId> {
        self.tips.iter().copied().collect()
    }

    /// Borrow a stored block.
    pub fn block(&self, id: &BlockId) -> Option<&Block> {
        self.nodes.get(id).map(|n| &n.block)
    }

    /// Borrow the GHOSTDAG data derived for a block.
    pub fn ghostdag(&self, id: &BlockId) -> Option<&GhostdagData> {
        self.nodes.get(id).map(|n| &n.ghostdag)
    }

    /// `true` iff `ancestor` is a strict ancestor of `descendant`
    /// (i.e. `ancestor` is in `descendant`'s past). `false` for equal ids.
    pub fn is_ancestor(&self, ancestor: &BlockId, descendant: &BlockId) -> bool {
        self.nodes
            .get(descendant)
            .is_some_and(|n| n.past.contains(ancestor))
    }

    /// `true` iff `a` and `b` are in each other's anticone: distinct blocks
    /// where neither is an ancestor of the other (they are "parallel").
    pub fn in_anticone(&self, a: &BlockId, b: &BlockId) -> bool {
        a != b && !self.is_ancestor(a, b) && !self.is_ancestor(b, a)
    }

    /// The chain-selection key used to rank blocks: heavier blue work wins,
    /// then higher blue score, then larger id as a deterministic final tiebreak.
    ///
    /// Used to pick a block's selected parent and the DAG's selected tip.
    pub(crate) fn chain_key(&self, id: &BlockId) -> (u128, u64, BlockId) {
        let g = &self.nodes[id].ghostdag;
        (g.blue_work, g.blue_score, *id)
    }

    /// The mergeset for a block with selected parent `sp` and past set `past`,
    /// in deterministic topological order:
    /// `past \ (past(sp) ∪ {sp})`, sorted by `(|past|, id)`.
    ///
    /// Shared by GHOSTDAG colouring, the linearization, and [`Dag::preview`] so
    /// all three agree on the mergeset and its order.
    pub(crate) fn mergeset_ordered(&self, sp: BlockId, past: &HashSet<BlockId>) -> Vec<BlockId> {
        let sp_past = &self.nodes[&sp].past;
        let mut mergeset: Vec<BlockId> = past
            .iter()
            .copied()
            .filter(|b| *b != sp && !sp_past.contains(b))
            .collect();
        mergeset.sort_by_key(|b| (self.nodes[b].past.len(), *b));
        mergeset
    }

    /// Preview the GHOSTDAG selected parent and mergeset a block would get if it
    /// were inserted with `block`'s parents — **without** inserting it.
    ///
    /// Runs the same structural checks as [`Dag::insert`] (duplicate, no parents,
    /// missing parent) so a caller can validate a prospective block against its
    /// view before committing it. This is what lets the state layer apply a
    /// block's transactions on top of its selected parent's UTXO state and reject
    /// an invalid block before it enters the DAG.
    pub fn preview(&self, block: &Block) -> Result<BlockPreview, DagError> {
        let id = block.id();
        if self.nodes.contains_key(&id) {
            return Err(DagError::DuplicateBlock(id));
        }
        if block.parents().is_empty() {
            return Err(DagError::NoParents(id));
        }
        for parent in block.parents() {
            if !self.nodes.contains_key(parent) {
                return Err(DagError::MissingParent(*parent));
            }
        }
        let selected_parent = *block
            .parents()
            .iter()
            .max_by_key(|p| self.chain_key(p))
            .expect("non-empty parents");
        let mut past: HashSet<BlockId> = HashSet::new();
        for parent in block.parents() {
            past.insert(*parent);
            past.extend(self.nodes[parent].past.iter().copied());
        }
        let mergeset = self.mergeset_ordered(selected_parent, &past);
        Ok(BlockPreview {
            selected_parent,
            mergeset,
        })
    }

    /// Insert `block`, validating and colouring it. Returns its id.
    ///
    /// Fails if the block is a duplicate, references a missing parent, (for a
    /// non-genesis block) references no parents, or is rejected by the installed
    /// [`BlockValidator`] (if any). The structural DAG checks run first, so a
    /// validator only ever sees a block whose parents are present.
    pub fn insert(&mut self, block: Block) -> Result<BlockId, DagError> {
        let id = block.id();
        if self.nodes.contains_key(&id) {
            return Err(DagError::DuplicateBlock(id));
        }
        if block.parents().is_empty() {
            return Err(DagError::NoParents(id));
        }
        for parent in block.parents() {
            if !self.nodes.contains_key(parent) {
                return Err(DagError::MissingParent(*parent));
            }
        }

        // Payload-aware validation, before the block is added to the DAG. Both
        // borrows of `self` here are shared, which the borrow checker allows.
        if let Some(validator) = self.validator.as_deref() {
            validator
                .validate(&block, self)
                .map_err(|reason| DagError::InvalidBlock { id, reason })?;
        }

        // Past = union over parents of ({parent} ∪ past(parent)).
        let mut past: HashSet<BlockId> = HashSet::new();
        for parent in block.parents() {
            past.insert(*parent);
            past.extend(self.nodes[parent].past.iter().copied());
        }

        // Derive GHOSTDAG data (selected parent, mergeset, colouring).
        let ghostdag = self.compute_ghostdag(block.parents(), &past);

        // Wire the block in: attach to parents, refresh tips.
        for parent in block.parents() {
            self.nodes.get_mut(parent).unwrap().children.insert(id);
            self.tips.remove(parent);
        }
        self.tips.insert(id);

        self.nodes.insert(
            id,
            Node {
                block,
                past,
                children: BTreeSet::new(),
                ghostdag,
            },
        );
        Ok(id)
    }
}
