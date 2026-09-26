//! Prints the version of a Windows executable: `cargo run --example gamever -- SkyrimSE.exe`
fn main() {
    let p = std::env::args().nth(1).expect("path to an .exe");
    match launcher_core::version::exe_version(std::path::Path::new(&p)) {
        Some(v) => println!("{}", launcher_core::version::show(v)),
        None => println!("no version resource"),
    }
}
