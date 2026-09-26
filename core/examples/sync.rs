//! Runs one update from the command line, for testing against a server:
//! cargo run -p launcher-core --example sync -- <base-url> <skyrim-dir> [--verify]

use launcher_core::{manifest::Manifest, sync};

#[tokio::main]
async fn main() -> launcher_core::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (base, dir) = (&args[1], std::path::Path::new(&args[2]));
    let verify = args.iter().any(|a| a == "--verify");
    let http = reqwest::Client::new();
    let m = Manifest::fetch(&http, base).await?;
    let plan = sync::plan(dir, &m, verify).await?;
    println!("build {}: {} to download ({} bytes), {} to remove", m.build, plan.download.len(), plan.download_bytes, plan.remove.len());
    sync::apply(&http, base, dir, &plan, |_| {}).await?;
    let again = sync::plan(dir, &m, true).await?;
    println!("after update: {} files still differ", again.download.len() + again.remove.len());
    Ok(())
}
