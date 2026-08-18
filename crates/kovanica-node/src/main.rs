//! The `kovanica-node` binary.
//!
//! * `kovanica-node` (or `kovanica-node serve`) — a REPL: read one command per
//!   line from stdin, print the response to stdout. `quit`/`exit` ends it.
//! * `kovanica-node demo` — replay a scripted end-to-end scenario, printing each
//!   command and its response, so the whole stack can be exercised in one run.

use std::io::{self, BufRead, Write};

use kovanica_node::{rpc, Node};

fn main() {
    let mode = std::env::args().nth(1);
    match mode.as_deref() {
        None | Some("serve") => serve(),
        Some("demo") => demo(),
        Some("help") | Some("-h") | Some("--help") => {
            println!("usage: kovanica-node [serve|demo]");
            println!("  serve  read commands from stdin (default)");
            println!("  demo   run a scripted end-to-end scenario");
            println!();
            println!("{}", rpc::HELP);
        }
        Some(other) => {
            eprintln!("unknown mode '{other}' (use: serve | demo | help)");
            std::process::exit(2);
        }
    }
}

/// Read commands from stdin, print one response line each.
fn serve() {
    let mut node = Node::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let _ = writeln!(stdout, "kovanica-node ready — type `help`, `quit` to exit");
    let _ = stdout.flush();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed == "quit" || trimmed == "exit" {
            break;
        }
        let response = rpc::execute_line(&mut node, trimmed);
        if writeln!(stdout, "{response}").is_err() {
            break;
        }
        let _ = stdout.flush();
    }
}

/// Replay a fixed scenario, printing a transcript.
fn demo() {
    let mut node = Node::new();
    let script = [
        "genesis 3 1000 500 1", // actor 1 is funded with 500
        "balance 1",
        "send 1 200 2", // immediate block: 1 -> 2 (200), change 300 back to 1
        "balance 1",
        "balance 2",
        "pool 2 50 3", // queue in the mempool instead of building a block now
        "pending",
        "produce", // pack the mempool into a block
        "pending",
        "balance 2",
        "balance 3",
        "tip",
        "len",
    ];
    for cmd in script {
        println!("> {cmd}");
        println!("{}", rpc::execute_line(&mut node, cmd));
    }
}
