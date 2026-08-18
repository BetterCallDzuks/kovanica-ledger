//! Differential tests for the reachability oracle (now the DAG's backing for
//! `is_ancestor`): on many DAGs — structured and randomly generated adversarial
//! ones — the oracle must agree with an **independent** ground truth (a naive
//! backward walk over parent edges) for *every* ordered pair of blocks, and its
//! chain-ancestor answer must match walking selected parents.

use std::collections::{HashSet, VecDeque};

use kovanica_dag::{Block, BlockId, Dag, Reachability};

/// A tiny deterministic PRNG (SplitMix-ish LCG) so tests are reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.next() >> 33) as usize) % n.max(1)
    }
}

/// Build a random DAG of `n` non-genesis blocks with parameter `k`. Each block
/// references 1–3 distinct existing blocks and has a small random work, producing
/// varied shapes (chains, wide forks, deep merges).
fn random_dag(seed: u64, n: usize, k: u16) -> (Dag, Vec<BlockId>) {
    let genesis = Block::genesis(1, 0, b"genesis".to_vec());
    let genesis_id = genesis.id();
    let mut dag = Dag::new(k, genesis);
    let mut ids = vec![genesis_id];
    let mut rng = Rng(seed.wrapping_add(0x9E3779B97F4A7C15));

    for i in 0..n {
        let want = 1 + rng.below(3);
        let mut parents: Vec<BlockId> = Vec::new();
        for _ in 0..(want * 4) {
            if parents.len() == want {
                break;
            }
            let cand = ids[rng.below(ids.len())];
            if !parents.contains(&cand) {
                parents.push(cand);
            }
        }
        let work = 1 + rng.below(4) as u128;
        let id = dag
            .insert(Block::new(parents, work, 0, format!("b{i}").into_bytes()))
            .expect("random block is valid");
        ids.push(id);
    }
    (dag, ids)
}

/// Independent ground truth: is `a` a strict DAG-ancestor of `b`? Naive backward
/// BFS over parent edges (this is what the oracle must reproduce).
fn naive_is_ancestor(dag: &Dag, a: &BlockId, b: &BlockId) -> bool {
    if a == b {
        return false;
    }
    let mut seen: HashSet<BlockId> = HashSet::new();
    let mut queue: VecDeque<BlockId> = dag.block(b).unwrap().parents().iter().copied().collect();
    while let Some(x) = queue.pop_front() {
        if x == *a {
            return true;
        }
        if !seen.insert(x) {
            continue;
        }
        for parent in dag.block(&x).unwrap().parents() {
            queue.push_back(*parent);
        }
    }
    false
}

/// Reference: is `a` a strict selected-parent (chain) ancestor of `b`?
fn walk_chain_ancestor(dag: &Dag, a: &BlockId, b: &BlockId) -> bool {
    if a == b {
        return false;
    }
    let mut cur = dag.ghostdag(b).and_then(|g| g.selected_parent);
    while let Some(c) = cur {
        if c == *a {
            return true;
        }
        cur = dag.ghostdag(&c).and_then(|g| g.selected_parent);
    }
    false
}

/// Assert the oracle (via both `Dag::is_ancestor` and a freshly-built
/// `Reachability`) agrees with the naive ground truth on every ordered pair.
fn assert_oracle_matches(dag: &Dag, ids: &[BlockId]) {
    let oracle = Reachability::build(dag);
    for a in ids {
        for b in ids {
            let truth = naive_is_ancestor(dag, a, b);
            assert_eq!(dag.is_ancestor(a, b), truth, "Dag::is_ancestor ({a}, {b})");
            assert_eq!(
                oracle.is_ancestor(a, b),
                truth,
                "oracle.is_ancestor ({a}, {b})"
            );
            assert_eq!(
                oracle.is_chain_ancestor(a, b),
                walk_chain_ancestor(dag, a, b),
                "is_chain_ancestor mismatch for ({a}, {b})"
            );
        }
    }
}

#[test]
fn matches_past_sets_on_random_dags() {
    // A spread of seeds, sizes and k values, including k = 0 (aggressive reds).
    for seed in 0..40u64 {
        let k = (seed % 4) as u16;
        let n = 12 + (seed as usize % 25);
        let (dag, ids) = random_dag(seed, n, k);
        assert_oracle_matches(&dag, &ids);
    }
}

#[test]
fn matches_on_a_linear_chain() {
    let genesis = Block::genesis(1, 0, b"genesis".to_vec());
    let mut dag = Dag::new(3, genesis);
    let mut ids = vec![dag.genesis()];
    for i in 0..20 {
        let parent = *ids.last().unwrap();
        ids.push(
            dag.insert(Block::new(vec![parent], 1, 0, format!("c{i}").into_bytes()))
                .unwrap(),
        );
    }
    assert_oracle_matches(&dag, &ids);
}

#[test]
fn matches_on_a_wide_fork_and_merge() {
    let genesis = Block::genesis(1, 0, b"genesis".to_vec());
    let mut dag = Dag::new(2, genesis);
    let g = dag.genesis();
    let parallel: Vec<BlockId> = (0..8)
        .map(|i| {
            dag.insert(Block::new(vec![g], 1, 0, format!("w{i}").into_bytes()))
                .unwrap()
        })
        .collect();
    let merge = dag
        .insert(Block::new(parallel.clone(), 1, 0, b"m".to_vec()))
        .unwrap();

    let mut ids = vec![g];
    ids.extend(parallel);
    ids.push(merge);
    assert_oracle_matches(&dag, &ids);
}

#[test]
fn matches_on_a_diamond() {
    let genesis = Block::genesis(1, 0, b"genesis".to_vec());
    let mut dag = Dag::new(3, genesis);
    let g = dag.genesis();
    let a = dag
        .insert(Block::new(vec![g], 1, 0, b"a".to_vec()))
        .unwrap();
    let b = dag
        .insert(Block::new(vec![g], 1, 0, b"b".to_vec()))
        .unwrap();
    let m = dag
        .insert(Block::new(vec![a, b], 1, 0, b"m".to_vec()))
        .unwrap();
    // A tail block off only one side, to exercise a non-tree covering path.
    let c = dag
        .insert(Block::new(vec![a], 1, 0, b"c".to_vec()))
        .unwrap();
    assert_oracle_matches(&dag, &[g, a, b, m, c]);
}
