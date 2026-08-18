//! A reachability oracle: answer "is A an ancestor of B?" without a full
//! per-block `past` set.
//!
//! Today [`Dag`] answers reachability from a per-block `past` set stored in full
//! — O(1) per query but O(n²) memory. This module is the standard replacement
//! (as used by Kaspa): a **reachability tree** with interval labels, plus a
//! **future-covering set** per block for the edges the tree does not capture.
//!
//! ## How it works
//!
//! * The **reachability tree** is the selected-parent tree (each block's tree
//!   parent is its GHOSTDAG selected parent; genesis is the root). A DFS labels
//!   every node with an interval `[start, end]` where the subtree of a node is
//!   exactly the contiguous range `[start, end]`. So `X` is a tree-ancestor of
//!   `Y` iff `X.start <= Y.start <= X.end` — an O(1) check, O(n) memory.
//!
//! * DAG reachability has edges the tree misses (a block's non-selected
//!   parents). For each block `A` the **future-covering set** `fcs(A)` holds the
//!   tree-roots of `future(A) \ tree_subtree(A)` — the minimal blocks whose tree
//!   subtrees cover the rest of `A`'s future. Then `A` reaches `B` iff `B` is in
//!   `A`'s tree subtree, or `B` is in the tree subtree of some `fcs(A)` member.
//!
//! ## Status
//!
//! This is built from the DAG's public structure and is **verified against the
//! existing `past`-set reachability** by a differential test over many random
//! adversarial DAGs (see `tests/reachability.rs`). It does not yet replace the
//! `past` sets — the build is a straightforward O(n²) pass and the sets are
//! still what `Dag` uses. Swapping `Dag` over to the oracle (incremental
//! maintenance with interval reindexing, and dropping the `past` sets) is the
//! follow-up this proves correct first.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::block::BlockId;
use crate::dag::Dag;

/// An interval-labelled reachability tree plus per-block future-covering sets.
#[derive(Clone, Debug)]
pub struct Reachability {
    /// Tree interval `[start, end]` per block: the block's subtree is the
    /// contiguous range `[start, end]` of `start` labels.
    intervals: HashMap<BlockId, (u64, u64)>,
    /// Future-covering set per block: the tree-roots of `future(b) \ subtree(b)`,
    /// sorted by interval start.
    fcs: HashMap<BlockId, Vec<BlockId>>,
}

impl Reachability {
    /// Build the oracle from `dag`, using only its public structure (the blocks,
    /// their parents, and their GHOSTDAG selected parents).
    pub fn build(dag: &Dag) -> Self {
        let order = dag.linearize(); // topological: genesis first, parents before children

        // Reachability tree = selected-parent tree. Collect tree children.
        let mut tree_children: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
        // DAG children = inverse of parents (for walking futures).
        let mut dag_children: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
        for id in &order {
            if let Some(sp) = dag.ghostdag(id).and_then(|g| g.selected_parent) {
                tree_children.entry(sp).or_default().push(*id);
            }
            for parent in dag
                .block(id)
                .expect("id from linearize is present")
                .parents()
            {
                dag_children.entry(*parent).or_default().push(*id);
            }
        }
        // Deterministic child order → deterministic interval labels.
        for children in tree_children.values_mut() {
            children.sort_unstable();
        }

        let intervals = label_intervals(dag.genesis(), &tree_children);
        let fcs = build_fcs(&order, dag, &dag_children, &intervals);

        Self { intervals, fcs }
    }

    /// Whether `x` is a tree- (selected-chain-) ancestor of, or equal to, `y`.
    fn tree_reaches(&self, x: &BlockId, y: &BlockId) -> bool {
        match (self.intervals.get(x), self.intervals.get(y)) {
            (Some(&(xs, xe)), Some(&(ys, _))) => xs <= ys && ys <= xe,
            _ => false,
        }
    }

    /// Whether `ancestor` is a strict tree- (selected-chain-) ancestor of
    /// `descendant`.
    pub fn is_chain_ancestor(&self, ancestor: &BlockId, descendant: &BlockId) -> bool {
        ancestor != descendant && self.tree_reaches(ancestor, descendant)
    }

    /// Whether `ancestor` is a strict DAG-ancestor of `descendant` (i.e.
    /// `ancestor ∈ past(descendant)`). `false` for equal ids.
    pub fn is_ancestor(&self, ancestor: &BlockId, descendant: &BlockId) -> bool {
        if ancestor == descendant {
            return false;
        }
        if self.tree_reaches(ancestor, descendant) {
            return true; // in the tree subtree
        }
        // Otherwise `descendant` must sit under one of `ancestor`'s future-covering
        // blocks' tree subtrees.
        self.fcs
            .get(ancestor)
            .is_some_and(|covers| covers.iter().any(|c| self.tree_reaches(c, descendant)))
    }
}

/// DFS interval labelling of the reachability tree rooted at `root`. Iterative to
/// tolerate deep chains. A node's subtree is the contiguous `start` range
/// `[start, end]`.
fn label_intervals(
    root: BlockId,
    tree_children: &HashMap<BlockId, Vec<BlockId>>,
) -> HashMap<BlockId, (u64, u64)> {
    let mut intervals: HashMap<BlockId, (u64, u64)> = HashMap::new();
    let mut counter: u64 = 0;

    enum Step {
        Enter(BlockId),
        Exit(BlockId),
    }
    let mut stack = vec![Step::Enter(root)];
    while let Some(step) = stack.pop() {
        match step {
            Step::Enter(id) => {
                let start = counter;
                counter += 1;
                intervals.insert(id, (start, start));
                stack.push(Step::Exit(id));
                if let Some(children) = tree_children.get(&id) {
                    // Push in reverse so children are entered in sorted order.
                    for child in children.iter().rev() {
                        stack.push(Step::Enter(*child));
                    }
                }
            }
            Step::Exit(id) => {
                // Every start assigned so far within this subtree is <= counter-1,
                // and the subtree is contiguous, so end = counter - 1.
                let end = counter - 1;
                intervals.get_mut(&id).expect("entered before exit").1 = end;
            }
        }
    }
    intervals
}

/// For each block, the tree-roots of `future(block) \ subtree(block)`.
fn build_fcs(
    order: &[BlockId],
    dag: &Dag,
    dag_children: &HashMap<BlockId, Vec<BlockId>>,
    intervals: &HashMap<BlockId, (u64, u64)>,
) -> HashMap<BlockId, Vec<BlockId>> {
    let subtree_contains = |x: &BlockId, y: &BlockId| -> bool {
        match (intervals.get(x), intervals.get(y)) {
            (Some(&(xs, xe)), Some(&(ys, _))) => xs <= ys && ys <= xe,
            _ => false,
        }
    };

    let mut fcs: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
    for a in order {
        // future(a) = everything reachable forward over DAG child edges, minus a.
        let mut future: HashSet<BlockId> = HashSet::new();
        let mut queue: VecDeque<BlockId> = VecDeque::new();
        queue.push_back(*a);
        while let Some(cur) = queue.pop_front() {
            if let Some(children) = dag_children.get(&cur) {
                for child in children {
                    if future.insert(*child) {
                        queue.push_back(*child);
                    }
                }
            }
        }

        // candidate = future(a) outside a's tree subtree. Its tree-roots (members
        // whose selected parent is not itself a candidate) are the covering set.
        // The candidate set is closed under tree-descendants, so this is exact.
        let mut covers: Vec<BlockId> = future
            .iter()
            .copied()
            .filter(|c| !subtree_contains(a, c))
            .filter(|c| {
                let tree_parent = dag.ghostdag(c).and_then(|g| g.selected_parent);
                match tree_parent {
                    // Root of the candidate set iff its tree parent is not also a
                    // candidate (either in a's subtree, or not in a's future).
                    Some(tp) => subtree_contains(a, &tp) || !future.contains(&tp),
                    None => true,
                }
            })
            .collect();
        covers.sort_unstable_by_key(|c| intervals.get(c).map_or(0, |iv| iv.0));
        if !covers.is_empty() {
            fcs.insert(*a, covers);
        }
    }
    fcs
}
