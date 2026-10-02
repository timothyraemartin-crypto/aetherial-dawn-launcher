//! The server's copies of the listed plugins under the names every player's
//! launcher runs them as, with the same bytes (SERVER-PLUGINS.md):
//!
//!     canonical-plugins <folder with the mods' plugins> <output folder> [--esl-as-esm <name.esl>]...
//!
//! Writes each plugin, and its archives and string files, to the output
//! folder, plus canonical.json listing original name, new name, whether the
//! masters were rewritten and the sha256. The server thread copies the
//! output folder into the server's Data; nobody renames by hand.
//!
//! Each `--esl-as-esm` names a light plugin the server's list runs as a full
//! plugin, "<stem>.esm" (desync/esl-on-server.md): the export's
//! `export.json` lists them under `light_as_full`.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = || {
        eprintln!("usage: canonical-plugins <plugins folder> <output folder> [--esl-as-esm <name.esl>]...");
        std::process::exit(2);
    };
    if args.len() < 3 {
        usage();
    }
    let mut full = Vec::new();
    let mut rest = args[3..].iter();
    while let Some(a) = rest.next() {
        match (a.as_str(), rest.next()) {
            ("--esl-as-esm", Some(n)) if n.to_ascii_lowercase().ends_with(".esl") && !n.contains(['/', '\\']) => full.push(n.clone()),
            _ => usage(),
        }
    }
    let (from, to) = (std::path::Path::new(&args[1]), std::path::Path::new(&args[2]));
    match launcher_core::aliases::canonicalize_dir_full(from, to, &full) {
        Ok(list) => {
            let json = serde_json::to_string_pretty(&list).expect("plain data");
            if let Err(e) = std::fs::write(to.join("canonical.json"), &json) {
                eprintln!("couldn't write canonical.json: {e}");
                std::process::exit(1);
            }
            println!("{json}");
        }
        Err(e) => {
            eprintln!("canonical-plugins: {e}");
            std::process::exit(1);
        }
    }
}
