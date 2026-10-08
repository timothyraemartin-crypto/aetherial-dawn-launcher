//! Capture test for the character portrait, to run on a real PC with Skyrim
//! open: `cargo run -p launcher-core --example portrait_grab`. Click the game
//! window within 10 seconds; the portrait is saved as portrait-test.png in the
//! current folder and its size is printed.

#[cfg(windows)]
fn main() {
    use launcher_core::portrait;
    println!("Click the Skyrim window and stand where your character is in view...");
    for _ in 0..20 {
        match portrait::grab::portrait() {
            Ok(png) => {
                std::fs::write("portrait-test.png", &png).expect("can't write portrait-test.png");
                println!("OK: portrait-test.png, {} bytes, {:?}", png.len(), portrait::check_png(&png).unwrap());
                return;
            }
            Err(e) => {
                println!("{e}");
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }
    println!("FAILED: no portrait after 10 seconds");
    std::process::exit(1);
}

#[cfg(not(windows))]
fn main() {
    println!("The portrait grab only works on Windows.");
}
