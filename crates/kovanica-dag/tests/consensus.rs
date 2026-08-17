//! Integration tests for the GHOSTDAG consensus core: colouring under the
//! k-cluster rule (including an adversarial wide fork), the k-cluster
//! invariant, determinism across insertion order, and topological validity of
//! the linearization.

use std::collections::HashSet;

use kovanica_dag::{Block, BlockId, Dag};

/// Build a DAG with the given `k` and a fixed genesis.
fn new_dag(k: u16) -> (Dag, BlockId) {
    let genesis = Block::genesis(1, b"kovanica-genesis".to_vec());
    let id = genesis.id();
    (Dag::new(k, genesis), id)
}

/// Insert a unit-work block with the given parents and label.
fn add(dag: &mut Dag, parents: &[BlockId], label: &str) -> BlockId {
    dag.insert(Block::new(parents.to_vec(), 1, label.as_bytes().to_vec()))
        .expect("insert should succeed")
}

/// Assert `order` is a valid topological order of `dag`: every block appears
/// after all of its parents.
fn assert_topological(dag: &Dag, order: &[BlockId]) {
    let mut seen: HashSet<BlockId> = HashSet::new();
    for id in order {
        for parent in dag.block(id).unwrap().parents() {
            assert!(
                seen.contains(parent),
                "block {id} emitted before its parent {parent}"
            );
        }
        seen.insert(*id);
    }
    assert_eq!(
        seen.len(),
        dag.len(),
        "order must cover every block exactly once"
    );
}

/// The k-cluster invariant: for every block, every entry in its blue anticone
/// map is `<= k`, and the map covers exactly `blue_score` blocks.
fn assert_k_cluster_invariant(dag: &Dag, order: &[BlockId]) {
    let k = dag.k();
    for id in order {
        let gd = dag.ghostdag(id).unwrap();
        assert_eq!(
            gd.blue_anticone_sizes.len() as u64,
            gd.blue_score,
            "blue map size must equal blue score for {id}"
        );
        for (&blue, &size) in &gd.blue_anticone_sizes {
            assert!(
                size <= k,
                "blue block {blue} has blue anticone {size} > k={k} in the view of {id}"
            );
        }
    }
}

#[test]
fn genesis_only() {
    let (dag, genesis) = new_dag(3);
    assert_eq!(dag.len(), 1);
    assert_eq!(dag.tips(), vec![genesis]);
    assert_eq!(dag.selected_tip(), genesis);
    assert_eq!(dag.linearize(), vec![genesis]);
    assert_eq!(dag.ghostdag(&genesis).unwrap().blue_score, 0);
}

#[test]
fn linear_chain_increments_blue_score() {
    let (mut dag, genesis) = new_dag(3);
    let a = add(&mut dag, &[genesis], "a");
    let b = add(&mut dag, &[a], "b");
    let c = add(&mut dag, &[b], "c");

    assert_eq!(dag.ghostdag(&a).unwrap().blue_score, 1); // {genesis}
    assert_eq!(dag.ghostdag(&b).unwrap().blue_score, 2); // {genesis, a}
    assert_eq!(dag.ghostdag(&c).unwrap().blue_score, 3); // {genesis, a, b}

    assert_eq!(dag.selected_tip(), c);
    assert_eq!(dag.selected_chain(), vec![genesis, a, b, c]);
    assert_eq!(dag.linearize(), vec![genesis, a, b, c]);
}

#[test]
fn parallel_blocks_merge_all_blue_when_k_large() {
    let (mut dag, genesis) = new_dag(3);
    let a = add(&mut dag, &[genesis], "a");
    let b = add(&mut dag, &[genesis], "b");
    assert!(dag.in_anticone(&a, &b), "a and b are parallel");
    let m = add(&mut dag, &[a, b], "m");

    let gd = dag.ghostdag(&m).unwrap();
    assert_eq!(gd.blue_score, 3, "genesis + a + b are all blue");
    assert!(
        gd.mergeset_reds.is_empty(),
        "nothing is red when k is generous"
    );

    let order = dag.linearize();
    assert_topological(&dag, &order);
    assert_eq!(order[0], genesis);
    assert_eq!(*order.last().unwrap(), m);
}

#[test]
fn k_zero_reds_the_second_parallel_block() {
    // With k = 0 no two blues may be parallel, so a merge of two parallel
    // blocks keeps only its selected parent blue and reds the other.
    let (mut dag, genesis) = new_dag(0);
    let a = add(&mut dag, &[genesis], "a");
    let b = add(&mut dag, &[genesis], "b");
    let m = add(&mut dag, &[a, b], "m");

    let gd = dag.ghostdag(&m).unwrap();
    assert_eq!(
        gd.mergeset_blues.len(),
        0,
        "the non-selected parallel block is red"
    );
    assert_eq!(gd.mergeset_reds.len(), 1);
    assert_eq!(gd.blue_score, 2, "only genesis + selected parent are blue");

    // The selected parent is the heavier-keyed of the two parallel tips.
    let sp = gd.selected_parent.unwrap();
    assert!(sp == a || sp == b);
    assert_eq!(gd.mergeset_reds[0], if sp == a { b } else { a });
}

#[test]
fn adversarial_wide_fork_bounds_blue_set_by_k() {
    // A "wide attacker" mines many blocks in parallel directly off genesis,
    // then a block merges them all. Under the k-cluster rule the blue set can
    // absorb only k of the parallel blocks beyond the selected parent; the rest
    // must be red no matter the insertion order.
    let k = 2u16;
    let (mut dag, genesis) = new_dag(k);

    let parallel: Vec<BlockId> = (0..5)
        .map(|i| add(&mut dag, &[genesis], &format!("w{i}")))
        .collect();
    // All five are mutually parallel.
    for i in 0..parallel.len() {
        for j in (i + 1)..parallel.len() {
            assert!(dag.in_anticone(&parallel[i], &parallel[j]));
        }
    }

    let m = add(&mut dag, &parallel, "merge");
    let gd = dag.ghostdag(&m).unwrap();

    // Blue set = genesis + selected parent + exactly k of the remaining
    // parallel blocks = k + 2 blues; the other (5 - 1 - k) are red.
    assert_eq!(gd.blue_score, u64::from(k) + 2);
    assert_eq!(gd.mergeset_blues.len(), usize::from(k));
    assert_eq!(gd.mergeset_reds.len(), 5 - 1 - usize::from(k));

    let order = dag.linearize();
    assert_topological(&dag, &order);
    assert_k_cluster_invariant(&dag, &order);
}

#[test]
fn linearization_is_deterministic_across_insertion_order() {
    // Build the same logical DAG two ways, inserting the two parallel blocks in
    // opposite order. The resulting consensus data and total order must match.
    let build = |swap: bool| {
        let (mut dag, genesis) = new_dag(3);
        let (a, b) = if swap {
            let b = add(&mut dag, &[genesis], "b");
            let a = add(&mut dag, &[genesis], "a");
            (a, b)
        } else {
            let a = add(&mut dag, &[genesis], "a");
            let b = add(&mut dag, &[genesis], "b");
            (a, b)
        };
        let _m = add(&mut dag, &[a, b], "m");
        dag
    };

    let dag1 = build(false);
    let dag2 = build(true);

    assert_eq!(dag1.selected_tip(), dag2.selected_tip());
    assert_eq!(dag1.linearize(), dag2.linearize());
    // linearize is itself a pure function of the DAG.
    assert_eq!(dag1.linearize(), dag1.linearize());
}

#[test]
fn heavier_branch_wins_selected_chain() {
    // genesis -> a -> a2 (length-2 branch) vs genesis -> b (length-1 branch).
    // The longer/heavier branch's tip is selected.
    let (mut dag, genesis) = new_dag(3);
    let a = add(&mut dag, &[genesis], "a");
    let a2 = add(&mut dag, &[a], "a2");
    let _b = add(&mut dag, &[genesis], "b");

    assert_eq!(dag.selected_tip(), a2);
    assert_eq!(dag.selected_chain(), vec![genesis, a, a2]);
}

#[test]
fn insert_validations() {
    let (mut dag, genesis) = new_dag(3);
    // Duplicate.
    let a_block = Block::new(vec![genesis], 1, b"a".to_vec());
    let a = dag.insert(a_block.clone()).unwrap();
    assert!(dag.insert(a_block).is_err());
    // Missing parent.
    let phantom = Block::genesis(1, b"not-in-dag".to_vec()).id();
    assert!(dag
        .insert(Block::new(vec![phantom], 1, b"x".to_vec()))
        .is_err());
    // Non-genesis with no parents.
    assert!(dag.insert(Block::new(vec![], 1, b"y".to_vec())).is_err());
    // Sanity: the one good block is a tip.
    assert_eq!(dag.tips(), vec![a]);
}
