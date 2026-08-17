//! Turning the partial order into a total order (linearization).
//!
//! GHOSTDAG gives every block a blue score and a selected-parent chain. To run a
//! ledger you still need a single, agreed **total order** over every block, so
//! that (in a full system) transactions can be applied deterministically.
//!
//! [`Dag::linearize`] produces that order as a deterministic topological sort:
//! repeatedly emit the highest-priority block all of whose parents are already
//! emitted, where priority is the GHOSTDAG chain key — heavier blue work first,
//! then higher blue score, then smaller id. Because the rule is a pure function
//! of the DAG, every node derives the identical sequence, and because it only
//! emits a block after its parents it is always a valid topological order.
//!
//! [`Dag::selected_tip`] and [`Dag::selected_chain`] expose the GHOSTDAG
//! backbone (the heaviest chain) that this order is built around.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::block::BlockId;
use crate::dag::Dag;

impl Dag {
    /// The selected tip: the tip with the heaviest [`Dag::chain_key`]. This is
    /// the head of the heaviest blue chain and the tip a new block would choose
    /// as its selected parent.
    pub fn selected_tip(&self) -> BlockId {
        self.tips()
            .into_iter()
            .max_by_key(|t| self.chain_key(t))
            .unwrap_or_else(|| self.genesis())
    }

    /// The selected-parent chain from genesis up to the selected tip, in order.
    pub fn selected_chain(&self) -> Vec<BlockId> {
        let mut chain = vec![self.selected_tip()];
        while let Some(parent) = self
            .ghostdag(chain.last().unwrap())
            .and_then(|g| g.selected_parent)
        {
            chain.push(parent);
        }
        chain.reverse();
        chain
    }

    /// A deterministic total order over every block in the DAG.
    ///
    /// The result is a topological sort (every block appears after all its
    /// parents) and is identical on any node holding the same DAG.
    pub fn linearize(&self) -> Vec<BlockId> {
        // Kahn's algorithm with a priority queue. `remaining` counts each
        // block's not-yet-emitted parents; a block becomes ready at zero.
        let mut remaining: HashMap<BlockId, usize> = HashMap::with_capacity(self.nodes.len());
        let mut ready: BinaryHeap<PriorityKey> = BinaryHeap::new();

        for (id, node) in &self.nodes {
            let pending = node.block.parents().len();
            remaining.insert(*id, pending);
            if pending == 0 {
                ready.push(self.priority_key(id));
            }
        }

        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(PriorityKey { id, .. }) = ready.pop() {
            order.push(id);
            for child in &self.nodes[&id].children {
                let slot = remaining.get_mut(child).unwrap();
                *slot -= 1;
                if *slot == 0 {
                    ready.push(self.priority_key(child));
                }
            }
        }

        debug_assert_eq!(
            order.len(),
            self.nodes.len(),
            "linearization dropped blocks"
        );
        order
    }

    fn priority_key(&self, id: &BlockId) -> PriorityKey {
        let (blue_work, blue_score, _) = self.chain_key(id);
        PriorityKey {
            blue_work,
            blue_score,
            id: *id,
        }
    }
}

/// Max-heap ordering key: heavier blue work first, then higher blue score, then
/// smaller id (via `Reverse`) so the whole order is deterministic.
struct PriorityKey {
    blue_work: u128,
    blue_score: u64,
    id: BlockId,
}

impl PartialEq for PriorityKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}
impl Eq for PriorityKey {}
impl PartialOrd for PriorityKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PriorityKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.blue_work
            .cmp(&other.blue_work)
            .then(self.blue_score.cmp(&other.blue_score))
            .then(Reverse(self.id).cmp(&Reverse(other.id)))
    }
}
