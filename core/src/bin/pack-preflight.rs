//! Checks a mod pack before anyone plays it. Read-only; nothing is changed, downloaded or uploaded.
//!
//!     pack-preflight scan DIR [--masters masters.json] [--rules launcher-rules.json] [--ledger FILE] [--json] [--strict]
//!     pack-preflight triage SKSE64.LOG        (ledger lines proposed from a play-test's skse64.log)
//!     pack-preflight addrlib versionlib-1-6-1170-0.bin ID...   (is each Address Library ID in the file?)
//!
//! scan: DIR holds the pack's files (a Data folder, or extracted mod archives). --rules takes the
//! collection's plugin order from a launcher-rules.json. Exit 0 unless --strict and a check failed.

use launcher_core::preflight::{self, Ledger, Sev};

fn die(m: &str) -> ! {
    eprintln!("pack-preflight: {m}");
    std::process::exit(2);
}

fn json_file(p: &str) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(p).unwrap_or_else(|e| die(&format!("can't read {p}: {e}")))).unwrap_or_else(|e| die(&format!("{p} isn't JSON: {e}")))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();
    match args.first().map(String::as_str) {
        Some("scan") => {
            let dir = args.get(1).unwrap_or_else(|| die("scan needs a folder"));
            let server: Vec<String> = flag("--masters")
                .map(|p| launcher_core::serverorder::server_order(&json_file(&p)).into_iter().map(|s| s.name).skip(launcher_core::serverorder::BASE.len()).collect())
                .unwrap_or_default();
            let order: Vec<String> = flag("--rules")
                .map(|p| json_file(&p)["collection"]["pluginOrder"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default())
                .unwrap_or_default();
            let ledger = match flag("--ledger") {
                Some(p) => Ledger::parse(&std::fs::read(&p).unwrap_or_else(|e| die(&format!("can't read {p}: {e}")))).unwrap_or_else(|e| die(&e)),
                None => Ledger::built_in(),
            };
            let found = preflight::scan(&preflight::Input { root: std::path::Path::new(dir), server: &server, plugin_order: &order, ledger: &ledger });
            if args.iter().any(|a| a == "--json") {
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "result": preflight::worst(&found), "findings": found })).expect("plain data"));
            } else {
                print!("{}", preflight::render(&found));
            }
            if args.iter().any(|a| a == "--strict") && preflight::worst(&found) == Sev::Fail {
                std::process::exit(1);
            }
        }
        Some("triage") => {
            let p = args.get(1).unwrap_or_else(|| die("triage needs an skse64.log"));
            let text = String::from_utf8_lossy(&std::fs::read(p).unwrap_or_else(|e| die(&format!("can't read {p}: {e}")))).into_owned();
            println!("{}", serde_json::to_string_pretty(&preflight::triage(&text)).expect("plain data"));
        }
        Some("addrlib") => {
            let p = args.get(1).unwrap_or_else(|| die("addrlib needs a file"));
            let db = launcher_core::addrlib::parse(&std::fs::read(p).unwrap_or_else(|e| die(&format!("can't read {p}: {e}")))).unwrap_or_else(|e| die(&e));
            println!("{p}: version {:?}, {} IDs", db.version, db.ids.len());
            for id in &args[2..] {
                let n: u64 = id.parse().unwrap_or_else(|_| die(&format!("{id} isn't a number")));
                match db.ids.get(&n) {
                    Some(o) => println!("ID {n}: present, offset {o:#x}"),
                    None => println!("ID {n}: NOT in this file"),
                }
            }
        }
        _ => die("usage: pack-preflight scan DIR [--masters F] [--rules F] [--ledger F] [--json] [--strict] | triage LOG | addrlib BIN ID..."),
    }
}
