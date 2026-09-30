//! The list hash the launcher gives a server-lane list, after the same
//! checks the export runs before downloading:
//!
//!     cargo run -p launcher-core --no-default-features --bin lane-hash -- <server-lane.json>
//!
//! Prints the hash, or why the launcher would refuse the list. The Mods chat
//! uses it for a revision's `list_hash`, so it matches the exporter exactly.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: lane-hash <server-lane.json>");
        std::process::exit(2);
    }
    let lane: launcher_core::serverlane::ServerLane = match std::fs::read(&args[1]).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string())) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("couldn't read {}: {e}", args[1]);
            std::process::exit(1);
        }
    };
    if let Err(e) = launcher_core::serverlane::check(&lane) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    println!("{}", launcher_core::serverlane::list_hash(&lane));
}
