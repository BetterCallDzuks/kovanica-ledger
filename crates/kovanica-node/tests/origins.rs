//! Integration tests for geographic origin tracking: a node may declare an ISO
//! 3166-1 alpha-3 [`Origin`](kovanica_node::Origin), peers record pulses on
//! in-process gossip, and none of this is consensus. Origin never enters a
//! block's encoding, never changes GHOSTDAG colouring or linearization, and
//! two honest nodes that disagree on a peer's origin still agree on the DAG.

use kovanica_node::{net, rpc::execute_line, Node, Origin};

/// A node with the standard genesis (mints 1000 to actor 1), matching the setup
/// in `network.rs`.
fn genesis_node() -> Node {
    let mut node = Node::new();
    node.genesis(3, 1000, 1000, 1).unwrap();
    node
}

fn run(node: &mut Node, line: &str) -> String {
    execute_line(node, line)
}

#[test]
fn rpc_sets_and_gets_origin_and_rejects_junk() {
    let mut node = Node::new();
    assert_eq!(run(&mut node, "origin"), "ok none");
    assert_eq!(run(&mut node, "origin hrv"), "ok HRV");
    assert_eq!(run(&mut node, "origin"), "ok HRV");
    // Replaces any previous value.
    assert_eq!(run(&mut node, "origin USA"), "ok USA");
    assert_eq!(run(&mut node, "origin"), "ok USA");

    assert!(run(&mut node, "origin hr").starts_with("err origin must be"));
    assert!(run(&mut node, "origin HRVV").starts_with("err origin must be"));
    assert!(run(&mut node, "origin 12A").starts_with("err origin must be"));
    assert!(run(&mut node, "origin HRV extra").starts_with("err expected 0 or 1"));
    // A rejected parse leaves the previous origin in place.
    assert_eq!(run(&mut node, "origin"), "ok USA");
}

#[test]
fn rpc_origins_is_empty_until_a_peer_pulses() {
    let mut node = Node::new();
    assert_eq!(run(&mut node, "origins"), "ok");
    run(&mut node, "origin HRV");
    // Own origin is not a pulse.
    assert_eq!(run(&mut node, "origins"), "ok");
}

#[test]
fn gossip_records_a_pulse_from_the_senders_origin() {
    let mut from = genesis_node();
    from.set_origin(Origin::parse("HRV").unwrap());
    from.send(1, 400, 2).unwrap();

    let mut to = genesis_node();
    assert!(to.origin_pulses().is_empty());

    let applied = net::gossip(&from, &mut to).unwrap();
    assert_eq!(applied, 1);
    assert_eq!(to.origin_pulses(), vec![(Origin::parse("HRV").unwrap(), 1)]);
    assert_eq!(run(&mut to, "origins"), "ok HRV 1");

    // Repeated gossip from the same origin just increments.
    net::gossip(&from, &mut to).unwrap();
    assert_eq!(to.origin_pulses(), vec![(Origin::parse("HRV").unwrap(), 2)]);
}

#[test]
fn gossip_without_an_origin_records_no_pulse() {
    let mut from = genesis_node();
    from.send(1, 400, 2).unwrap();
    let mut to = genesis_node();
    net::gossip(&from, &mut to).unwrap();
    assert!(to.origin_pulses().is_empty());
}

#[test]
fn origin_pulses_sort_by_count_then_iso3() {
    let mut observer = Node::new();
    let hrv = Origin::parse("HRV").unwrap();
    let usa = Origin::parse("USA").unwrap();
    let deu = Origin::parse("DEU").unwrap();
    observer.observe_origin(usa);
    observer.observe_origin(hrv);
    observer.observe_origin(deu);
    observer.observe_origin(hrv);
    // HRV has 2; DEU and USA tie at 1, so ISO-3 order: DEU then USA.
    assert_eq!(observer.origin_pulses(), vec![(hrv, 2), (deu, 1), (usa, 1)]);
    assert_eq!(run(&mut observer, "origins"), "ok HRV 2 DEU 1 USA 1");
}

#[test]
fn origin_does_not_change_the_dag_or_utxo_state() {
    // Two producers, identical spends, pinned clocks, different origins: the
    // selected tip and balances match. Origin is node policy — not a consensus
    // input — so it cannot change a block id.
    let now = 1_700_000_000_000;
    let mut a = genesis_node();
    a.set_now_ms(now);
    a.set_origin(Origin::parse("HRV").unwrap());
    a.send(1, 400, 2).unwrap();

    let mut b = genesis_node();
    b.set_now_ms(now);
    b.set_origin(Origin::parse("USA").unwrap());
    b.send(1, 400, 2).unwrap();

    assert_eq!(a.selected_tip().unwrap(), b.selected_tip().unwrap());
    assert_eq!(
        a.balance(&Node::address(2)).unwrap(),
        b.balance(&Node::address(2)).unwrap()
    );

    let mut receiver = genesis_node();
    net::gossip(&a, &mut receiver).unwrap();
    assert_eq!(receiver.selected_tip().unwrap(), a.selected_tip().unwrap());
    assert_eq!(receiver.balance(&Node::address(2)).unwrap(), 400);
    // The pulse is recorded, but it is a local view — the DAG is the same.
    assert_eq!(
        receiver.origin_pulses(),
        vec![(Origin::parse("HRV").unwrap(), 1)]
    );
}

#[test]
fn tcp_pull_sync_does_not_record_origin() {
    // The one-shot TCP wire format is block records only. Origin announcement
    // is in-process gossip today; a continuous p2p overlay would carry it.
    use std::net::TcpListener;
    use std::thread;

    let mut server = genesis_node();
    server.set_origin(Origin::parse("HRV").unwrap());
    server.send(1, 400, 2).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let records = server.export();
    let handle = thread::spawn(move || {
        net::serve_records(&listener, &records).unwrap();
    });

    let mut client = genesis_node();
    let applied = net::pull_blocks(addr, &mut client).unwrap();
    handle.join().unwrap();

    assert_eq!(applied, 1);
    assert_eq!(client.balance(&Node::address(2)).unwrap(), 400);
    assert!(
        client.origin_pulses().is_empty(),
        "TCP block sync must not smuggle origin onto the wire"
    );
}

#[test]
fn help_lists_origin_commands() {
    let mut node = Node::new();
    let help = run(&mut node, "help");
    assert!(help.contains("origin [ISO3]"), "{help}");
    assert!(help.contains("origins"), "{help}");
}
