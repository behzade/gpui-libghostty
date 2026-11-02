#[path = "../../crates/gpui-ghostty/build_support/ghostty.rs"]
mod ghostty;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let manifest = root.join("crates/gpui-ghostty");
    let work = root.join("target/ghostty-source");
    match std::env::args().nth(1).as_deref() {
        Some("prepare-ghostty") => {
            ghostty::materialize(&manifest, &manifest.join("upstream/ghostty"), &work)?;
            let patched = ghostty::prepare(&manifest, &work, &work)?;
            println!(
                "Verified upstream/ghostty and patches; prepared {}",
                patched.display()
            );
        }
        Some("check-ghostty") => {
            let patched = ghostty::prepare(&manifest, &work, &work)?;
            println!("{}", patched.display());
        }
        _ => return Err("usage: cargo run -p xtask -- {prepare-ghostty|check-ghostty}".into()),
    }
    Ok(())
}
