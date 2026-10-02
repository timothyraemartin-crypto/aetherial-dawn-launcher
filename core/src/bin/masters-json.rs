//! The served masters.json for an export (serverorder::masters_json):
//!
//!     masters-json <current masters.json> <export.json> > masters.json
//!
//! Keeps the current file's five base masters as they are, then lists every
//! master past them and every lane plugin from the exporter's export.json in
//! load order, under the names PCs run them as, with the canonical sha256,
//! size and crc32 each PC's copy must match. Nothing is uploaded.

fn read(path: &str) -> serde_json::Value {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("can't read {path}: {e}");
        std::process::exit(1);
    });
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        eprintln!("{path} isn't JSON: {e}");
        std::process::exit(1);
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: masters-json <current masters.json> <export.json>");
        std::process::exit(2);
    }
    let current = read(&args[1]);
    let list = current.get("masters").or_else(|| current.get("files")).unwrap_or(&current);
    let base: Vec<serde_json::Value> = list.as_array().map(|a| a.iter().take(launcher_core::serverorder::BASE.len()).cloned().collect()).unwrap_or_default();
    match launcher_core::serverorder::masters_json(&base, &read(&args[2])) {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v).expect("plain data")),
        Err(e) => {
            eprintln!("masters-json: {e}");
            std::process::exit(1);
        }
    }
}
