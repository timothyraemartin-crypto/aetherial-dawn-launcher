//! The server's copies of the listed plugins under the names every player's
//! launcher runs them as, with the same bytes (SERVER-PLUGINS.md):
//!
//!     canonical-plugins <folder with the mods' plugins> <output folder>
//!
//! Writes each plugin, and its archives and string files, to the output
//! folder, plus canonical.json listing original name, new name, whether the
//! masters were rewritten and the sha256. The server thread copies the
//! output folder into the server's Data; nobody renames by hand.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: canonical-plugins <plugins folder> <output folder>");
        std::process::exit(2);
    }
    let (from, to) = (std::path::Path::new(&args[1]), std::path::Path::new(&args[2]));
    match launcher_core::aliases::canonicalize_dir(from, to) {
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
